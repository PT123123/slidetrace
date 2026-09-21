//! 数据模型：Project / Page / Element / Stroke / Script / Rehearsal。
//!
//! 设计原则（对应 SPEC §26）：
//! - `Page` 只是**空间容器**，不是产品的时间核心；
//! - 一切"什么时候发生了什么"都表达为 [`TimelineEvent`](crate::timeline::TimelineEvent)；
//! - `Stroke` 不是静态图形，而是 `Geometry + Time`（逐点时间戳）；
//! - 本模块**不依赖任何 UI / 音频库**，可以被纯单测覆盖。
// 本模块是刻意保留的「核心 API 层」：它被单元测试完整覆盖，但并不要求每个
// 访问器都被当前 UI 调用（例如 `Rehearsal::push_event` / `Stroke::length` 这类
// 查询函数，未来接入时间轴编辑、摄像头轨道时会直接用上）。
// 这里统一放行 dead_code，避免 `cargo build` 输出被几十条"未被 UI 调用"淹没；
// UI 层（src/app.rs、src/main.rs）**不加**这个放行，真正的死代码仍会被指出。
#![allow(dead_code)]

pub mod asset;
pub mod element;
pub mod page;
pub mod project;
pub mod rehearsal;
pub mod script;
pub mod stroke;

pub use asset::AssetRef;
pub use element::{Element, ElementId, ElementKind, TextAlign};
pub use page::{Page, PageId, Size};
pub use project::Project;
pub use rehearsal::{AudioRef, Rehearsal, RehearsalMeta};
pub use script::{ScriptDoc, ScriptSection, SectionId};
pub use stroke::Stroke;

/// 当前 project.json 的 schema 版本。读盘时若版本更高则拒绝加载。
pub const SCHEMA_VERSION: u32 = 1;

/// 默认页面坐标系（元素的 x/y/w/h 与笔迹点都在这个坐标系内）。
pub const DEFAULT_PAGE_WIDTH: f64 = 1280.0;
pub const DEFAULT_PAGE_HEIGHT: f64 = 720.0;

/// 生成给人类的相对时间字符串，例如 `2026-09-21 21:12:03`。
///
/// 自己实现是为了避免为一个时间戳引入 chrono/time 这类额外依赖。
pub fn format_unix_time(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        m,
        d,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Howard Hinnant 的 `civil_from_days` 算法（days since 1970-01-01 → y/m/d）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 当前 Unix 时间戳（秒）。
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 把任意标题转成可用于目录名的 slug（保留 ASCII 字母数字与 `-`）。
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' || ch == ' ' {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    // 纯中文标题会被清空，此时退化为时间戳 slug，保证目录唯一且可预测。
    if trimmed.is_empty() {
        format!("project-{}", now_unix())
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_formatting_is_correct() {
        assert_eq!(format_unix_time(0), "1970-01-01 00:00:00");
        // 2021-01-01 00:00:00 UTC
        assert_eq!(format_unix_time(1_609_459_200), "2021-01-01 00:00:00");
        // 2024-02-29 12:34:56 UTC（闰年 2 月 29 日）
        assert_eq!(format_unix_time(1_709_210_096), "2024-02-29 12:34:56");
    }

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(slugify("TiDB 架构分享"), "tidb");
        assert_eq!(slugify("My Talk 01"), "my-talk-01");
        assert_eq!(slugify("a//b"), "ab");
        // 纯中文标题无 ASCII 字符可用，退化为时间戳 slug
        assert!(slugify("架构分享").starts_with("project-"));
    }
}
