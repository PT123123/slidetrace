use serde::{Deserialize, Serialize};

use super::Element;

/// 页面 id。
pub type PageId = String;

/// 页面 = 空间容器。页面本身**没有时间属性**，页面的切换由 `TimelineEvent::PageChange` 表达。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: PageId,
    pub name: String,
    #[serde(default)]
    pub elements: Vec<Element>,
    /// 备注：只用于 Create 模式的提示，不参与播放。
    #[serde(default)]
    pub note: String,
}

impl Page {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            elements: Vec::new(),
            note: String::new(),
        }
    }

    pub fn element(&self, id: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.id == id)
    }

    pub fn element_mut(&mut self, id: &str) -> Option<&mut Element> {
        self.elements.iter_mut().find(|e| e.id == id)
    }

    /// 按 (z, 插入顺序) 升序，即绘制顺序：后面的画在上面。
    pub fn draw_order(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.elements.len()).collect();
        idx.sort_by_key(|&i| (self.elements[i].z, i as i32));
        idx
    }

    /// 下一个 z 值，保证新元素在最上层。
    pub fn next_z(&self) -> i32 {
        self.elements.iter().map(|e| e.z).max().unwrap_or(0) + 1
    }
}

/// 页面尺寸（逻辑坐标系）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

impl Size {
    pub const fn new(w: f64, h: f64) -> Self {
        Self { w, h }
    }

    /// 等比例适配到目标框，返回 (scale, offset_x, offset_y)，即 letterbox 变换。
    ///
    /// 画布渲染、鼠标坐标反变换、笔迹坐标都共用这一个函数，保证三者永不脱节。
    pub fn fit_into(&self, target_w: f64, target_h: f64) -> (f64, f64, f64) {
        if self.w <= 0.0 || self.h <= 0.0 || target_w <= 0.0 || target_h <= 0.0 {
            return (1.0, 0.0, 0.0);
        }
        let scale = (target_w / self.w).min(target_h / self.h);
        let off_x = (target_w - self.w * scale) / 2.0;
        let off_y = (target_h - self.h * scale) / 2.0;
        (scale, off_x, off_y)
    }
}

impl Default for Size {
    fn default() -> Self {
        Size::new(super::DEFAULT_PAGE_WIDTH, super::DEFAULT_PAGE_HEIGHT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Element;

    #[test]
    fn fit_into_letterboxes() {
        let page = Size::new(1280.0, 720.0);
        // 目标框比页面更宽 → 高度受限，左右留黑边
        let (s, ox, oy) = page.fit_into(2000.0, 720.0);
        assert_eq!(s, 1.0);
        assert_eq!(oy, 0.0);
        assert_eq!(ox, 360.0);
        // 目标框更矮
        let (s2, _, oy2) = page.fit_into(1280.0, 360.0);
        assert!((s2 - 0.5).abs() < 1e-9);
        assert_eq!(oy2, 0.0);
    }

    #[test]
    fn draw_order_respects_z_then_insertion() {
        let mut p = Page::new("p1", "第一页");
        let mut a = Element::text("a", 0.0, 0.0, 10.0, 10.0, "a");
        let mut b = Element::text("b", 0.0, 0.0, 10.0, 10.0, "b");
        let c = Element::text("c", 0.0, 0.0, 10.0, 10.0, "c");
        a.z = 5;
        b.z = 5;
        p.elements = vec![a, b, c];
        // c(z=0) 最先画，a/b(z=5) 后画且 a 在 b 之前
        assert_eq!(p.draw_order(), vec![2, 0, 1]);
        assert_eq!(p.next_z(), 6);
    }
}
