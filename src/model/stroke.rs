use serde::{Deserialize, Serialize};

use super::PageId;

/// 笔迹 id。
pub type StrokeId = String;

/// 笔迹上的一个点。`t` 是**录音时间轴上的绝对秒数**（不是相对笔迹起点的偏移）。
///
/// 这一点很关键：`state_at(t)` 只需要 `p.t <= t` 就可决定"画到第几个点"，
/// 不需要知道笔迹何时开始（SPEC §18）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    pub t: f64,
}

impl StrokePoint {
    pub fn new(x: f64, y: f64, t: f64) -> Self {
        Self { x, y, t }
    }
}

/// 手写轨迹 = Geometry + Time。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: StrokeId,
    /// 笔迹属于哪个页面；只有当前页面的笔迹会被绘制。
    pub page_id: PageId,
    /// `#RRGGBB`
    pub color: String,
    /// 线宽，单位为页面坐标系里的单位（会随画布缩放）。
    pub width: f64,
    pub points: Vec<StrokePoint>,
    /// 冗余保存一份起止时间，便于时间轴统计与快速过滤。
    #[serde(default)]
    pub start_time: f64,
    #[serde(default)]
    pub end_time: f64,
}

impl Stroke {
    pub fn new(id: impl Into<String>, page_id: impl Into<String>, color: impl Into<String>, width: f64) -> Self {
        Self {
            id: id.into(),
            page_id: page_id.into(),
            color: color.into(),
            width,
            points: Vec::new(),
            start_time: 0.0,
            end_time: 0.0,
        }
    }

    pub fn push(&mut self, x: f64, y: f64, t: f64) {
        if self.points.is_empty() {
            self.start_time = t;
        }
        self.end_time = t;
        self.points.push(StrokePoint::new(x, y, t));
    }

    /// 重新计算 start/end（反序列化旧数据或直接改 points 后调用）。
    pub fn refresh_bounds(&mut self) {
        self.start_time = self.points.first().map(|p| p.t).unwrap_or(0.0);
        self.end_time = self.points.last().map(|p| p.t).unwrap_or(0.0);
    }

    /// 在时间 `t` 时这条笔迹应该画出的点数（前缀长度）。
    ///
    /// 由于点一定是按时间递增采集的，这里用二分查找即可，O(log n)。
    pub fn point_count_at(&self, t: f64) -> usize {
        if self.points.is_empty() || t < self.points[0].t {
            return 0;
        }
        // partition_point 返回第一个 `t <= p.t` 的位置，也就是已完成的点数。
        self.points.partition_point(|p| p.t <= t)
    }

    /// 生成 SVG path 的 `d` 字符串，只包含前 `count` 个点。
    ///
    /// 单点笔迹（比如点一下）会退化成一个极短的线段，保证在画布上依然可见。
    pub fn to_svg_path(&self, count: usize) -> String {
        let n = count.min(self.points.len());
        if n == 0 {
            return String::new();
        }
        let mut out = String::with_capacity(n * 16 + 16);
        for (i, p) in self.points[..n].iter().enumerate() {
            if i == 0 {
                out.push_str(&format!("M {:.2} {:.2}", p.x, p.y));
            } else {
                out.push_str(&format!(" L {:.2} {:.2}", p.x, p.y));
            }
        }
        if n == 1 {
            out.push_str(&format!(" L {:.2} {:.2}", self.points[0].x + 0.01, self.points[0].y));
        }
        out
    }

    /// 画笔在整条笔迹上走过的总长度（页面坐标系）。
    pub fn length(&self) -> f64 {
        self.points
            .windows(2)
            .map(|w| {
                let dx = w[1].x - w[0].x;
                let dy = w[1].y - w[0].y;
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Stroke {
        let mut s = Stroke::new("st1", "p1", "#e11d48", 3.0);
        for (x, y, t) in [(10.0, 10.0, 1.0), (20.0, 10.0, 2.0), (30.0, 10.0, 3.0)] {
            s.push(x, y, t);
        }
        s
    }

    #[test]
    fn point_count_prefix() {
        let s = sample();
        assert_eq!(s.point_count_at(0.5), 0);
        assert_eq!(s.point_count_at(1.0), 1);
        assert_eq!(s.point_count_at(1.5), 1);
        assert_eq!(s.point_count_at(3.0), 3);
        assert_eq!(s.point_count_at(99.0), 3);
    }

    #[test]
    fn svg_path_generation() {
        let s = sample();
        assert_eq!(s.to_svg_path(0), "");
        assert_eq!(s.to_svg_path(1), "M 10.00 10.00 L 10.01 10.00");
        assert_eq!(s.to_svg_path(2), "M 10.00 10.00 L 20.00 10.00");
        assert_eq!(s.length(), 20.0);
    }
}
