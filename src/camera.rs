//! 摄像头轨道 —— **V1 未实现的扩展点**。
//!
//! SPEC §21 把 Camera 定位为"演练复盘"的一部分（"我讲这一段的时候，我本人是什么状态"），
//! 并且 SPEC §14 要求一次 Rehearsal 同时记录 Microphone + Camera。
//!
//! 本次交付**不实现**摄像头采集（见 README「未实现项」），原因是硬性约束里
//! 禁止引入需要 cmake / C++ / 外部二进制的重依赖（`nokhwa` 在 Windows 上会拉入
//! Media Foundation 绑定与构建脚本，且无法保证默认 `cargo build` 纯 Rust 可构建）。
//!
//! 为了让以后接入摄像头不需要改数据模型，这里先固定两件事：
//!
//! 1. **数据位**：[`crate::model::Rehearsal`] 已经为"多条轨道"留好了形状——
//!    音频是 `audio: Option<AudioRef>`，未来的 `video: Option<VideoRef>` 与之并列，
//!    路径约定为 `rehearsals/<id>/camera.<ext>`。
//! 2. **接口位**：[`crate::audio::Recorder`] 已经把"录制源"抽象成
//!    `elapsed()` + `finish(dir)`。摄像头只需要实现同一个 trait，
//!    把 `finish` 写成"把帧序列封装成文件"，就能与麦克风共享同一套
//!    `Instant` 无关的时间语义（时间戳由"已落盘帧数 / 帧率"给出）。
//!
//! 上层（`src/app.rs`）在录制结束时只做一件事：遍历 `Vec<Box<dyn Recorder>>`，
//! 对每个录制源调用 `finish`。因此接入摄像头时 app.rs 的改动量 ≈ 0。
// 本模块是刻意保留的「核心 API 层」：它被单元测试完整覆盖，但并不要求每个
// 访问器都被当前 UI 调用（例如 `Rehearsal::push_event` / `Stroke::length` 这类
// 查询函数，未来接入时间轴编辑、摄像头轨道时会直接用上）。
// 这里统一放行 dead_code，避免 `cargo build` 输出被几十条"未被 UI 调用"淹没；
// UI 层（src/app.rs、src/main.rs）**不加**这个放行，真正的死代码仍会被指出。
#![allow(dead_code)]

/// 摄像头轨道的占位实现。所有方法都返回"未实现"，但类型已经存在，
/// 这样调用方可以先按最终形状写代码（V1 里没有任何地方调用它）。
pub struct CameraTrack;

/// 摄像头轨道在 `Rehearsal` 目录下的文件命名约定。
pub const CAMERA_FILE_NAME: &str = "camera.mp4";

impl CameraTrack {
    /// 本版本恒返回 `None`：V1 不采集摄像头。
    pub fn probe() -> Option<CameraDeviceInfo> {
        None
    }
}

/// 摄像头设备信息（预留给 Review 界面的"摄像头"面板标题）。
#[derive(Clone, Debug, PartialEq)]
pub struct CameraDeviceInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_is_explicitly_unimplemented_in_v1() {
        assert!(CameraTrack::probe().is_none());
        assert_eq!(CAMERA_FILE_NAME, "camera.mp4");
    }
}
