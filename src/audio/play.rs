//! rodio 播放 + seek + 播放位置。
//!
//! ## 为什么播放位置不用 `rodio::Player::get_pos()`
//!
//! rodio 的 `get_pos()` 依赖音频线程推进，暂停/空队列/设备异常时语义不好把握；
//! 而且**没有声卡的环境下整个播放管线根本无法初始化**，但我们仍然希望时间轴
//! 能拖动、能回放画面与笔迹。
//!
//! 所以这里自己维护一个 [`PlaybackClock`]：由 `Instant` 推进，rodio 只负责出声。
//! 拖动时间轴时同时调用 [`AudioPlayer::seek`]，音频与画面因此保持同步。
//! 没有音频设备时 [`AudioPlayer`] 处于"静音模式"（`device_available == false`），
//! 除不出声以外所有行为完全一致。
// 本模块是刻意保留的「核心 API 层」：它被单元测试完整覆盖，但并不要求每个
// 访问器都被当前 UI 调用（例如 `Rehearsal::push_event` / `Stroke::length` 这类
// 查询函数，未来接入时间轴编辑、摄像头轨道时会直接用上）。
// 这里统一放行 dead_code，避免 `cargo build` 输出被几十条"未被 UI 调用"淹没；
// UI 层（src/app.rs、src/main.rs）**不加**这个放行，真正的死代码仍会被指出。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rodio::{Decoder, Player as RodioPlayer};

/// 播放时钟：以 `Instant` 为唯一真相，可暂停、可任意 seek。
#[derive(Debug, Clone)]
pub struct PlaybackClock {
    /// 上次 play/seek 时的基准位置（秒）。
    base: f64,
    /// 正在播放时的起点；`None` 表示暂停。
    started_at: Option<Instant>,
    /// 媒体总时长（秒），0 表示未知。
    total: f64,
}

impl PlaybackClock {
    pub fn new(total: f64) -> Self {
        Self {
            base: 0.0,
            started_at: None,
            total: total.max(0.0),
        }
    }

    pub fn set_total(&mut self, total: f64) {
        self.total = total.max(0.0);
        if self.base > self.total {
            self.base = self.total;
        }
    }

    pub fn is_playing(&self) -> bool {
        self.started_at.is_some()
    }

    /// 当前播放位置（秒），永远落在 `0..=total`。
    pub fn position(&self) -> f64 {
        let raw = match self.started_at {
            Some(t0) => self.base + t0.elapsed().as_secs_f64(),
            None => self.base,
        };
        if self.total > 0.0 {
            raw.clamp(0.0, self.total)
        } else {
            raw.max(0.0)
        }
    }

    pub fn play(&mut self) {
        if self.started_at.is_none() {
            // 播到头了再按播放 → 从头开始
            if self.total > 0.0 && self.base >= self.total - 1e-6 {
                self.base = 0.0;
            }
            self.started_at = Some(Instant::now());
        }
    }

    pub fn pause(&mut self) {
        if self.started_at.is_some() {
            self.base = self.position();
            self.started_at = None;
        }
    }

    pub fn seek(&mut self, t: f64) {
        let t = if self.total > 0.0 {
            t.clamp(0.0, self.total)
        } else {
            t.max(0.0)
        };
        self.base = t;
        if self.started_at.is_some() {
            self.started_at = Some(Instant::now());
        }
    }

    pub fn rewind(&mut self) {
        self.base = 0.0;
        if self.started_at.is_some() {
            self.started_at = Some(Instant::now());
        }
    }

    /// 播放是否已经走到结尾（用于自动暂停）。
    pub fn reached_end(&self) -> bool {
        self.total > 0.0 && self.is_playing() && self.position() >= self.total - 1e-3
    }
}

/// 音频播放器。没有可用输出设备时自动降级为静音模式。
pub struct AudioPlayer {
    /// 必须一直持有：drop 掉 `MixerDeviceSink` 播放就停了。
    device: Option<rodio::MixerDeviceSink>,
    player: Option<RodioPlayer>,
    clock: PlaybackClock,
    loaded: Option<PathBuf>,
    /// 当前是否处于"哑"状态（有声卡但解码失败，或根本没有声卡）。
    note: Option<String>,
}

impl AudioPlayer {
    /// 尝试打开默认输出设备。**任何失败都只记录到 `note`，不返回 Err**，
    /// 这样无音频环境下整个应用依然可用。
    pub fn open() -> Self {
        match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(sink) => {
                let mut sink = sink;
                // 默认在 drop 时会往 stderr 打一行提示；这是个正常关闭的库内部通知，
                // 对本应用只是噪音。
                sink.log_on_drop(false);
                let player = RodioPlayer::connect_new(sink.mixer());
                player.pause();
                Self {
                    device: Some(sink),
                    player: Some(player),
                    clock: PlaybackClock::new(0.0),
                    loaded: None,
                    note: None,
                }
            }
            Err(e) => Self {
                device: None,
                player: None,
                clock: PlaybackClock::new(0.0),
                loaded: None,
                note: Some(format!("没有可用的音频输出设备（{e}），将以静音方式回放")),
            },
        }
    }

    pub fn device_available(&self) -> bool {
        self.device.is_some()
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// 载入一个 WAV 文件并把播放头复位到 `start_at` 秒。
    ///
    /// 即使解码失败（文件损坏 / 格式不支持），时钟依然按 `hint_duration` 工作，
    /// 保证画面与笔迹仍能正常回放。
    pub fn load(&mut self, path: &Path, hint_duration: f64) {
        let total = crate::audio::wav_duration(path).unwrap_or(hint_duration);
        self.clock.set_total(total);
        self.clock.rewind();
        self.loaded = None;
        self.note = None;

        if let Some(player) = &self.player {
            player.clear();
            player.pause();
            match std::fs::File::open(path).map_err(|e| e.to_string()).and_then(|f| {
                Decoder::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())
            }) {
                Ok(decoder) => {
                    player.append(decoder);
                    self.loaded = Some(path.to_path_buf());
                }
                Err(e) => {
                    self.note = Some(format!("音频解码失败：{e}（画面仍可回放）"));
                }
            }
        }
    }

    /// 没有音频文件时也能建立一条时间轴。
    pub fn set_duration(&mut self, total: f64) {
        self.clock.set_total(total);
    }

    pub fn duration(&self) -> f64 {
        self.clock.total
    }

    pub fn position(&self) -> f64 {
        self.clock.position()
    }

    pub fn is_playing(&self) -> bool {
        self.clock.is_playing()
    }

    pub fn play(&mut self) {
        self.clock.play();
        if let Some(p) = &self.player {
            // seek 到时钟位置，避免暂停期间画面被拖动后音频还在原处
            let _ = p.try_seek(Duration::from_secs_f64(self.clock.position()));
            p.play();
        }
    }

    pub fn pause(&mut self) {
        self.clock.pause();
        if let Some(p) = &self.player {
            p.pause();
        }
    }

    pub fn toggle(&mut self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    /// 跳转到 `t` 秒（画面与音频同步）。
    pub fn seek(&mut self, t: f64) {
        self.clock.seek(t);
        self.seek_device();
    }

    /// 画面拖动时高频调用：每次都更新时钟，但只在必要时才真正 seek 声卡，
    /// 避免每秒几十次 `try_seek` 造成爆音。
    pub fn seek_throttled(&mut self, t: f64) {
        let before = self.clock.base;
        self.clock.seek(t);
        if (self.clock.base - before).abs() > 0.02 {
            self.seek_device();
        }
    }

    fn seek_device(&self) {
        if let Some(p) = &self.player {
            if self.loaded.is_some() {
                // rodio 的 seek 用 `Result` 表达"这个源不支持随机访问"，
                // 对我们来说不是致命错误：画面/笔迹的定位不依赖它。
                let _ = p.try_seek(Duration::from_secs_f64(self.clock.position()));
            }
        }
    }

    /// 播放到结尾时自动停下（由 UI 定时器每帧调用）。
    pub fn tick(&mut self) {
        if self.clock.reached_end() {
            self.pause();
        }
    }

    /// rodio 自己报告的播放位置，仅用于诊断对比。
    pub fn device_position(&self) -> f64 {
        self.player
            .as_ref()
            .map(|p| p.get_pos().as_secs_f64())
            .unwrap_or(0.0)
    }

    /// 清空当前音频并复位时钟（切换到没有音频的演练时使用）。
    pub fn stop_and_clear(&mut self) {
        self.clock.pause();
        self.clock.seek(0.0);
        self.clock.set_total(0.0);
        if let Some(p) = &self.player {
            p.pause();
            p.clear();
        }
        self.loaded = None;
    }

    pub fn has_audio(&self) -> bool {
        self.loaded.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_advances_and_pauses() {
        let mut c = PlaybackClock::new(10.0);
        assert!(!c.is_playing());
        assert_eq!(c.position(), 0.0);
        c.play();
        assert!(c.is_playing());
        std::thread::sleep(Duration::from_millis(30));
        let p = c.position();
        assert!(p > 0.0 && p < 1.0, "position={p}");
        c.pause();
        let paused = c.position();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(c.position(), paused, "暂停后位置不应继续走");
    }

    #[test]
    fn clock_seek_clamps_to_range() {
        let mut c = PlaybackClock::new(5.0);
        c.seek(3.0);
        assert!((c.position() - 3.0).abs() < 1e-9);
        c.seek(-10.0);
        assert_eq!(c.position(), 0.0);
        c.seek(100.0);
        assert_eq!(c.position(), 5.0);
    }

    #[test]
    fn seek_while_playing_keeps_playing_from_new_base() {
        let mut c = PlaybackClock::new(20.0);
        c.play();
        c.seek(12.0);
        std::thread::sleep(Duration::from_millis(20));
        let p = c.position();
        assert!(p >= 12.0 && p < 13.0, "position={p}");
        assert!(c.is_playing());
    }

    #[test]
    fn replay_from_start_when_finished() {
        let mut c = PlaybackClock::new(1.0);
        c.seek(1.0);
        assert!(!c.is_playing());
        c.play(); // 已经到结尾 → 从头播
        assert!(c.position() < 0.5);
    }

    #[test]
    fn reached_end_detection() {
        let mut c = PlaybackClock::new(0.05);
        c.play();
        std::thread::sleep(Duration::from_millis(80));
        assert!(c.reached_end());
        assert_eq!(c.position(), 0.05);
    }

    #[test]
    fn player_without_device_still_has_working_clock() {
        // 不依赖真实声卡：即使没有设备，时钟与时长也必须可用
        let mut p = AudioPlayer {
            device: None,
            player: None,
            clock: PlaybackClock::new(0.0),
            loaded: None,
            note: Some("test".into()),
        };
        p.set_duration(30.0);
        assert_eq!(p.duration(), 30.0);
        p.seek(12.0);
        assert!((p.position() - 12.0).abs() < 1e-9);
        p.play();
        assert!(p.is_playing());
        p.pause();
        assert!(!p.is_playing());
        p.tick();
        assert!(!p.has_audio());
        assert!(!p.device_available());
        assert_eq!(p.note(), Some("test"));
    }

    #[test]
    fn throttled_seek_updates_clock_every_time() {
        let mut p = AudioPlayer {
            device: None,
            player: None,
            clock: PlaybackClock::new(0.0),
            loaded: None,
            note: None,
        };
        p.set_duration(10.0);
        p.seek_throttled(1.0);
        assert!((p.position() - 1.0).abs() < 1e-9);
        p.seek_throttled(1.005); // 变化很小
        assert!((p.position() - 1.005).abs() < 1e-9);
        p.seek_throttled(5.0);
        assert!((p.position() - 5.0).abs() < 1e-9);
    }
}
