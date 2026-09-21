//! cpal 麦克风录音 → WAV。
//!
//! 时间戳来源：**已采集帧数 / 采样率**，而不是墙上时钟。
//! 这样"按下快捷键的时间"与"WAV 里的位置"天然对齐，不会因为驱动缓冲区
//! 而出现几十毫秒的系统性偏移——这是本产品时间轴精度的基础。
// 本模块是刻意保留的「核心 API 层」：它被单元测试完整覆盖，但并不要求每个
// 访问器都被当前 UI 调用（例如 `Rehearsal::push_event` / `Stroke::length` 这类
// 查询函数，未来接入时间轴编辑、摄像头轨道时会直接用上）。
// 这里统一放行 dead_code，避免 `cargo build` 输出被几十条"未被 UI 调用"淹没；
// UI 层（src/app.rs、src/main.rs）**不加**这个放行，真正的死代码仍会被指出。
#![allow(dead_code)]

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
// `f32::from_sample` 来自 cpal 重新导出的 dasp_sample::Sample
use cpal::Sample as _;

use super::{FinishedRecording, Recorder};

/// 录音过程中会被音频回调线程与 UI 线程共享的数据。
struct Shared {
    samples: Mutex<Vec<f32>>,
    frames: AtomicU64,
    error: Mutex<Option<String>>,
}

/// 麦克风录制器。
pub struct MicRecorder {
    /// `cpal::Stream` 必须一直存活，drop 即停止采集。
    stream: Option<cpal::Stream>,
    shared: Arc<Shared>,
    device_name: String,
    sample_rate: u32,
    channels: u16,
}

impl MicRecorder {
    /// 打开默认输入设备并开始采集。
    ///
    /// 失败时返回可读的中文原因（没有麦克风、设备被占用、格式不支持等），
    /// 由上层决定是否降级成"无声演练"。
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "找不到可用的麦克风输入设备".to_string())?;
        let device_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "未知设备".into());

        let supported = device
            .default_input_config()
            .map_err(|e| format!("读取麦克风默认格式失败：{e}"))?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels();
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let shared = Arc::new(Shared {
            samples: Mutex::new(Vec::with_capacity(sample_rate as usize * channels as usize * 8)),
            frames: AtomicU64::new(0),
            error: Mutex::new(None),
        });

        let err_shared = shared.clone();
        let err_cb = move |e: cpal::StreamError| {
            if let Ok(mut slot) = err_shared.error.lock() {
                *slot = Some(format!("音频流错误：{e}"));
            }
        };

        let nch = channels as usize;
        // cpal 的输入流是泛型的（按采样格式单态化），这里把常见格式都覆盖掉。
        macro_rules! build {
            ($t:ty) => {
                device.build_input_stream(
                    &config,
                    {
                        let shared = shared.clone();
                        move |data: &[$t], _: &cpal::InputCallbackInfo| {
                            if let Ok(mut buf) = shared.samples.lock() {
                                buf.reserve(data.len());
                                for &s in data {
                                    buf.push(f32::from_sample(s));
                                }
                            }
                            shared
                                .frames
                                .fetch_add((data.len() / nch.max(1)) as u64, Ordering::Relaxed);
                        }
                    },
                    err_cb,
                    None,
                )
            };
        }

        let stream = match sample_format {
            cpal::SampleFormat::F32 => build!(f32),
            cpal::SampleFormat::I16 => build!(i16),
            cpal::SampleFormat::U16 => build!(u16),
            cpal::SampleFormat::I32 => build!(i32),
            cpal::SampleFormat::I8 => build!(i8),
            cpal::SampleFormat::U8 => build!(u8),
            cpal::SampleFormat::F64 => build!(f64),
            other => return Err(format!("暂不支持的麦克风采样格式：{other:?}")),
        }
        .map_err(|e| format!("无法打开麦克风输入流：{e}"))?;

        stream.play().map_err(|e| format!("无法启动录音：{e}"))?;

        Ok(Self {
            stream: Some(stream),
            shared,
            device_name,
            sample_rate,
            channels,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// 当前缓冲区里的采样帧数。
    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }

    /// 停止采集并取走样本（不写盘）。写盘由 [`MicRecorder::finish`] 完成。
    fn take_samples(&mut self) -> Vec<f32> {
        self.stream = None; // drop 掉 stream，回调不再触发
        let mut guard = self.shared.samples.lock().unwrap();
        std::mem::take(&mut *guard)
    }
}

impl Recorder for MicRecorder {
    fn name(&self) -> &str {
        "mic"
    }

    fn take_error(&self) -> Option<String> {
        self.shared.error.lock().ok().and_then(|mut g| g.take())
    }

    fn elapsed(&self) -> std::time::Duration {
        let f = self.frames();
        if self.sample_rate == 0 {
            return std::time::Duration::ZERO;
        }
        std::time::Duration::from_secs_f64(f as f64 / self.sample_rate as f64)
    }

    fn finish(mut self: Box<Self>, dir: &Path) -> Result<FinishedRecording, String> {
        let samples = self.take_samples();
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
        let path = dir.join("audio.wav");
        write_wav_i16(&path, self.sample_rate, self.channels, &samples)?;
        let frames = (samples.len() / self.channels.max(1) as usize) as u64;
        Ok(FinishedRecording {
            file_name: "audio.wav".to_string(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            frames,
        })
    }
}

/// 把交错 f32 样本写成 16-bit PCM WAV（体积小、兼容性最好）。
pub fn write_wav_i16(
    path: &Path,
    sample_rate: u32,
    channels: u16,
    samples: &[f32],
) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| format!("创建 WAV 失败：{e}"))?;
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(v)
            .map_err(|e| format!("写入 WAV 失败：{e}"))?;
    }
    writer.finalize().map_err(|e| format!("收尾 WAV 失败：{e}"))?;
    Ok(())
}

/// 探测默认麦克风是否可用（UI 用来提前提示用户）。
pub fn input_device_available() -> Option<String> {
    let host = cpal::default_host();
    host.default_input_device()
        .and_then(|d| d.description().ok().map(|desc| desc.name().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_back_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.wav");
        let samples: Vec<f32> = vec![0.0, 0.5, -0.5, 1.0, -1.0, 0.25];
        write_wav_i16(&path, 8_000, 2, &samples).unwrap();
        let (mono, rate) = super::super::read_wav_mono(&path).unwrap();
        assert_eq!(rate, 8_000);
        assert_eq!(mono.len(), 3); // 6 个交错样本 = 3 帧
        assert!((mono[0] - 0.25).abs() < 1e-3); // (0.0 + 0.5)/2
        assert!((mono[1] - 0.25).abs() < 1e-3); // (-0.5 + 1.0)/2
        assert!((mono[2] + 0.375).abs() < 1e-3); // (-1.0 + 0.25)/2
    }

    #[test]
    fn elapsed_is_derived_from_frames() {
        let shared = Arc::new(Shared {
            samples: Mutex::new(Vec::new()),
            frames: AtomicU64::new(8000),
            error: Mutex::new(None),
        });
        let r = MicRecorder {
            stream: None,
            shared,
            device_name: "test".into(),
            sample_rate: 8000,
            channels: 1,
        };
        assert_eq!(r.elapsed().as_secs_f64(), 1.0);
        assert_eq!(r.sample_rate(), 8000);
        assert_eq!(r.name(), "mic");
    }
}
