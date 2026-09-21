use serde::{Deserialize, Serialize};

/// 项目的资源文件引用。V1 只做「复制进 `<project>/assets/`」这一种策略，
/// `original_path` 仅用于展示来源。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetRef {
    /// 相对 `assets/` 的文件名。
    pub name: String,
    #[serde(default)]
    pub original_path: Option<String>,
    #[serde(default)]
    pub added_at_unix: u64,
    /// 图片像素尺寸，Create 模式插入时用来计算默认宽高比。
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

impl AssetRef {
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            name: name.into(),
            original_path: None,
            added_at_unix: super::now_unix(),
            width,
            height,
        }
    }
}
