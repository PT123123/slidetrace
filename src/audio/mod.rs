//! 音频层。
//!
//! - [`record`]：cpal 采集麦克风 → hound 写 WAV
//! - [`play`]：rodio 播放 WAV，支持 seek + 当前位置
//!
//! ## 录制器抽象
//!
//! V1 只实现麦克风。摄像头录制（SPEC §21）**本次不实现**，但接口已经预留：
//! [`Recorder`] 描述"一个能产出带时间戳数据的录制源"，将来的 `camera` 模块
//! 只要实现同一个 trait，就能作为第二条轨道加入同一次 Rehearsal，
//! 而上层（app.rs）不需要改动数据模型。详见 [`crate::camera`]。
// 本模块是刻意保留的「核心 API 层」：它被单元测试完整覆盖，但并不要求每个
// 访问器都被当前 UI 调用（例如 `Rehearsal::push_event` / `Stroke::length` 这类
// 查询函数，未来接入时间轴编辑、摄像头轨道时会直接用上）。
// 这里统一放行 dead_code，避免 `cargo build` 输出被几十条"未被 UI 调用"淹没；
// UI 层（src/app.rs、src/main.rs）**不加**这个放行，真正的死代码仍会被指出。
#![allow(dead_code)]

pub mod play;
pub mod record;

use std::path::Path;
use std::time::Duration;

/// 录制源的统一抽象。时间语义：`elapsed()` 是**已经确定性落入介质的时长**，
/// 而不是墙上时钟。麦克风用"已采集帧数 / 采样率"，这样事件时间戳与
/// WAV 内容天然对齐（不会因为缓冲区延迟而漂移）。
pub trait Recorder: Send {
    /// 这个录制源的名字（用于 UI 与日志），例如 `"mic"`。
    fn name(&self) -> &str;

    /// 已经确定录下的时长。
    fn elapsed(&self) -> Duration;

    /// 录制过程中是否发生过流错误；一次性取出（取走后返回 None）。
    /// 默认实现用于没有额外错误通道的录制源。
    fn take_error(&self) -> Option<String> {
        None
    }

    /// 停止采集并把结果写到 `dir` 下，返回写入的字节数与帧数。
    fn finish(self: Box<Self>, dir: &Path) -> Result<FinishedRecording, String>;
}

/// 录制结束后的产物描述。
#[derive(Clone, Debug)]
pub struct FinishedRecording {
    pub file_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
}

impl FinishedRecording {
    pub fn duration(&self) -> f64 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0.0;
        }
        self.frames as f64 / (self.sample_rate as f64 * self.channels as f64)
    }
}

/// 读取 WAV 的所有样本，降混成单声道 f32（时间轴波形用）。
///
/// 直接用 hound 而不是 rodio：这里只需要数据，不需要播放管线。
pub fn read_wav_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| format!("打开 WAV 失败：{e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let mut mono: Vec<f32> = Vec::new();

    match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample.saturating_sub(1))) as f32;
            let mut acc = 0f32;
            let mut ch = 0usize;
            for s in reader.samples::<i32>() {
                let v = s.map_err(|e| format!("读取样本失败：{e}"))? as f32 / max;
                acc += v;
                ch += 1;
                if ch == channels {
                    mono.push(acc / channels as f32);
                    acc = 0.0;
                    ch = 0;
                }
            }
        }
        hound::SampleFormat::Float => {
            let mut acc = 0f32;
            let mut ch = 0usize;
            for s in reader.samples::<f32>() {
                let v = s.map_err(|e| format!("读取样本失败：{e}"))?;
                acc += v;
                ch += 1;
                if ch == channels {
                    mono.push(acc / channels as f32);
                    acc = 0.0;
                    ch = 0;
                }
            }
        }
    }
    Ok((mono, spec.sample_rate))
}

/// 读取 WAV 的时长（秒）。不回放、不解码样本，只读头部。
pub fn wav_duration(path: &Path) -> Option<f64> {
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.sample_rate == 0 || spec.channels == 0 {
        return None;
    }
    let frames = reader.len() as f64 / spec.channels as f64;
    Some(frames / spec.sample_rate as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_roundtrip_read_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let samples: Vec<f32> = (0..1600)
            .map(|i| (i as f32 / 1600.0 * std::f32::consts::TAU * 4.0).sin() * 0.5)
            .collect();
        crate::demo::write_wav_mono_i16(&path, 16_000, &samples).unwrap();

        let (mono, rate) = read_wav_mono(&path).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(mono.len(), samples.len());
        // 16bit 量化误差以内
        for (a, b) in mono.iter().zip(samples.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        let d = wav_duration(&path).unwrap();
        assert!((d - 0.1).abs() < 1e-6);
    }

    #[test]
    fn wav_duration_of_missing_file_is_none() {
        assert!(wav_duration(Path::new("definitely-not-here.wav")).is_none());
    }

    #[test]
    fn finished_recording_duration() {
        let f = FinishedRecording {
            file_name: "audio.wav".into(),
            sample_rate: 44_100,
            channels: 2,
            frames: 44_100 * 2 * 2,
        };
        assert_eq!(f.duration(), 2.0);
    }
}
