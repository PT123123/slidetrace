use serde::{Deserialize, Serialize};

/// 元素 id。使用可读字符串（`el3`）而不是 UUID，方便人工检查 project.json。
pub type ElementId = String;

/// 元素的**空间**描述。时间维度一律由 `TimelineEvent` 承担。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: ElementId,
    pub kind: ElementKind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// 层级：数值越大越靠上。命中测试按 (z, 数组顺序) 取最上面的。
    #[serde(default)]
    pub z: i32,
    #[serde(default)]
    pub style: ElementStyle,
    /// 录制开始时是否默认隐藏。`true` 表示"必须按快捷键才会出现"。
    #[serde(default)]
    pub hidden_by_default: bool,
}

impl Element {
    pub fn text(id: impl Into<String>, x: f64, y: f64, w: f64, h: f64, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: ElementKind::Text { text: text.into() },
            x,
            y,
            w,
            h,
            z: 0,
            style: ElementStyle::default(),
            hidden_by_default: true,
        }
    }

    pub fn image(
        id: impl Into<String>,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        asset: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: ElementKind::Image { asset: asset.into() },
            x,
            y,
            w,
            h,
            z: 0,
            style: ElementStyle::default(),
            hidden_by_default: true,
        }
    }

    /// 元素文本（仅 Text 有）。用于日志 / 命中提示。
    pub fn text_content(&self) -> Option<&str> {
        match &self.kind {
            ElementKind::Text { text } => Some(text),
            _ => None,
        }
    }

    /// 图片元素引用的 asset 名称（相对 `assets/`，或绝对路径）。
    pub fn asset_name(&self) -> Option<&str> {
        match &self.kind {
            ElementKind::Image { asset } => Some(asset),
            _ => None,
        }
    }

    pub fn set_text(&mut self, new_text: impl Into<String>) {
        if let ElementKind::Text { text } = &mut self.kind {
            *text = new_text.into();
        }
    }

    /// 点是否落在元素矩形内（坐标系为页面坐标系）。
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }

    pub fn rect(&self) -> (f64, f64, f64, f64) {
        (self.x, self.y, self.w, self.h)
    }

    /// 归一化：宽度/高度不允许为负，避免拖动时出现"反向矩形"。
    pub fn clamp_size(&mut self, min_w: f64, min_h: f64) {
        self.w = self.w.max(min_w);
        self.h = self.h.max(min_h);
    }
}

/// 元素类型。`Shape` 是 V1 预留的扩展位（SPEC §17 的 Temporal Visual Event 未来可覆盖箭头/高亮）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ElementKind {
    Text { text: String },
    Image { asset: String },
    /// V1 未实现渲染，仅保留数据位。（见 README「未实现项」）
    Shape { shape: ShapeKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Arrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign::Left
    }
}

/// 元素样式。颜色统一用 `#RRGGBB` 字符串，避免模型层依赖 slint 类型。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementStyle {
    pub color: String,
    pub font_size: f64,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub align: TextAlign,
    /// 背景色；`None` 表示透明。
    #[serde(default)]
    pub background: Option<String>,
}

impl Default for ElementStyle {
    fn default() -> Self {
        Self {
            color: "#1f2933".to_string(),
            font_size: 28.0,
            bold: false,
            align: TextAlign::Left,
            background: None,
        }
    }
}

/// 把 `#RRGGBB` / `#AARRGGBB` 解析成 (r, g, b, a)。
///
/// 自己解析而不是依赖 slint 的 `Color::from_str`，这样模型层保持无 UI 依赖。
pub fn parse_hex_color(s: &str) -> Option<(u8, u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    let hex = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
    match h.len() {
        3 => {
            let r = hex(0, 1)?;
            let g = hex(1, 1)?;
            let b = hex(2, 1)?;
            // #abc → #aabbcc
            Some((r * 17, g * 17, b * 17, 255))
        }
        6 => Some((hex(0, 2)?, hex(2, 2)?, hex(4, 2)?, 255)),
        8 => Some((hex(2, 2)?, hex(4, 2)?, hex(6, 2)?, hex(0, 2)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_parse() {
        assert_eq!(parse_hex_color("#ff8800"), Some((255, 136, 0, 255)));
        assert_eq!(parse_hex_color("#f80"), Some((255, 136, 0, 255)));
        assert_eq!(parse_hex_color("#80ff8800"), Some((255, 136, 0, 128)));
        assert_eq!(parse_hex_color("nope"), None);
        assert_eq!(parse_hex_color("#12345"), None);
    }

    #[test]
    fn contains_and_clamp() {
        let mut e = Element::text("el1", 10.0, 20.0, 100.0, 50.0, "hi");
        assert!(e.contains(10.0, 20.0));
        assert!(e.contains(110.0, 70.0));
        assert!(!e.contains(111.0, 70.0));
        e.w = -5.0;
        e.h = 0.0;
        e.clamp_size(4.0, 4.0);
        assert_eq!((e.w, e.h), (4.0, 4.0));
    }
}
