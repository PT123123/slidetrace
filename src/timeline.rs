//! 时间轴：把「时间 t」映射成「完整演讲状态」（SPEC §2 / §26）。
//!
//! 本模块是**纯函数 + 纯数据**，不依赖 Slint / cpal / rodio，也不碰文件系统，
//! 因此可以被完整单测覆盖。所有 UI 与音频只负责把 `t` 传进来、把结果画出来。
// 本模块是刻意保留的「核心 API 层」：它被单元测试完整覆盖，但并不要求每个
// 访问器都被当前 UI 调用（例如 `Rehearsal::push_event` / `Stroke::length` 这类
// 查询函数，未来接入时间轴编辑、摄像头轨道时会直接用上）。
// 这里统一放行 dead_code，避免 `cargo build` 输出被几十条"未被 UI 调用"淹没；
// UI 层（src/app.rs、src/main.rs）**不加**这个放行，真正的死代码仍会被指出。
#![allow(dead_code)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{Element, ElementId, Page, PageId, ScriptDoc, SectionId, Stroke};

/// 时间轴上的一个事件。**一切"什么时候发生了什么"都用它表达**（SPEC §17）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineEvent {
    /// 元素出现（按快捷键触发的核心事件）。
    Reveal { element_id: ElementId, t: f64 },
    /// 元素消失。
    Hide { element_id: ElementId, t: f64 },
    /// 切页。
    PageChange { page_id: PageId, t: f64 },
    /// 用户手动打的标记点。
    Marker { label: String, t: f64 },
    /// 讲稿锚点：第 t 秒开始讲这一段。
    ScriptMark { section_id: SectionId, t: f64 },
}

impl TimelineEvent {
    pub fn time(&self) -> f64 {
        match self {
            TimelineEvent::Reveal { t, .. }
            | TimelineEvent::Hide { t, .. }
            | TimelineEvent::PageChange { t, .. }
            | TimelineEvent::Marker { t, .. }
            | TimelineEvent::ScriptMark { t, .. } => *t,
        }
    }

    pub fn set_time(&mut self, new_t: f64) {
        match self {
            TimelineEvent::Reveal { t, .. }
            | TimelineEvent::Hide { t, .. }
            | TimelineEvent::PageChange { t, .. }
            | TimelineEvent::Marker { t, .. }
            | TimelineEvent::ScriptMark { t, .. } => *t = new_t,
        }
    }

    /// 用于 UI 分组/着色/提示的稳定短标识。
    pub fn kind_str(&self) -> &'static str {
        match self {
            TimelineEvent::Reveal { .. } => "reveal",
            TimelineEvent::Hide { .. } => "hide",
            TimelineEvent::PageChange { .. } => "page",
            TimelineEvent::Marker { .. } => "marker",
            TimelineEvent::ScriptMark { .. } => "script",
        }
    }

    /// 中文标签，时间轴上鼠标悬停/列表里显示。
    pub fn label(&self) -> String {
        match self {
            TimelineEvent::Reveal { element_id, .. } => format!("出现 {element_id}"),
            TimelineEvent::Hide { element_id, .. } => format!("隐藏 {element_id}"),
            TimelineEvent::PageChange { page_id, .. } => format!("切页 {page_id}"),
            TimelineEvent::Marker { label, .. } => format!("标记 {label}"),
            TimelineEvent::ScriptMark { section_id, .. } => format!("讲稿 {section_id}"),
        }
    }

    /// 同一时刻的排序优先级：切页 → 讲稿 → 出现 → 消失 → 标记。
    ///
    /// 之所以需要它：`t = 0` 时往往同时存在 `PageChange` 和 `ScriptMark`，
    /// 固定次序才能保证回放与单测结果可复现。
    fn tie_break_rank(&self) -> u8 {
        match self {
            TimelineEvent::PageChange { .. } => 0,
            TimelineEvent::ScriptMark { .. } => 1,
            TimelineEvent::Reveal { .. } => 2,
            TimelineEvent::Hide { .. } => 3,
            TimelineEvent::Marker { .. } => 4,
        }
    }
}

/// 按时间排序（稳定：同一时刻保持原有相对顺序，并再按类型次序打破平局）。
///
/// 乱序插入后调用它即可恢复正确顺序。
pub fn sort_events(events: &mut [TimelineEvent]) {
    // 先记录原始下标，保证同 t 同类型的事件也保持插入顺序（稳定排序 + 显式 key）。
    let mut keyed: Vec<(usize, TimelineEvent)> = events.iter().cloned().enumerate().collect();
    keyed.sort_by(|(ia, a), (ib, b)| {
        a.time()
            .partial_cmp(&b.time())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.tie_break_rank().cmp(&b.tie_break_rank()))
            .then_with(|| ia.cmp(ib))
    });
    for (slot, (_, ev)) in events.iter_mut().zip(keyed) {
        *slot = ev;
    }
}

/// 事件排序是否已经正确（供调试与单测使用，避免每次都排序）。
pub fn is_sorted(events: &[TimelineEvent]) -> bool {
    events.windows(2).all(|w| {
        w[0].time() < w[1].time()
            || (w[0].time() == w[1].time() && w[0].tie_break_rank() <= w[1].tie_break_rank())
    })
}

/// 计算 `state_at(t)` 所需的全部输入。
///
/// 之所以用一个 struct 而不是直接传 `&Project`：V1 的 Create 模式还没有
/// Rehearsal，Review 模式只有 Rehearsal，Present 模式两者都可能用到；
/// 把它们解耦后这个函数就是纯粹的时间函数，单测不需要构造完整项目。
pub struct TimelineInput<'a> {
    pub pages: &'a [Page],
    pub strokes: &'a [Stroke],
    pub script: &'a ScriptDoc,
    pub events: &'a [TimelineEvent],
    /// 录制/回放开始时停留在哪一页（没有 `PageChange` 事件时的初值）。
    pub start_page_index: usize,
}

impl<'a> TimelineInput<'a> {
    /// 从项目 + 可选演练构造输入。
    pub fn new(project: &'a crate::model::Project, rehearsal: Option<&'a crate::model::Rehearsal>) -> Self {
        let start_page_index = rehearsal
            .and_then(|r| project.page_index(&r.start_page_id))
            .unwrap_or(project.current_page)
            .min(project.pages.len().saturating_sub(1));
        Self {
            pages: &project.pages,
            strokes: rehearsal.map(|r| r.strokes.as_slice()).unwrap_or(&[]),
            script: &project.script,
            events: rehearsal.map(|r| r.events.as_slice()).unwrap_or(&[]),
            start_page_index,
        }
    }

    /// `PresentationState::state_at(t)` —— 本产品的核心纯函数。
    pub fn state_at(&self, t: f64) -> PresentationState {
        PresentationState::state_at(self, t)
    }

    /// "所有内容都已出现"的状态。
    ///
    /// Create / Present 模式以及"跳到最后"都用它，避免写两份可见性逻辑。
    pub fn final_state(&self) -> PresentationState {
        self.state_at(f64::INFINITY)
    }

    pub fn duration(&self) -> f64 {
        self.events
            .iter()
            .map(|e| e.time())
            .fold(0.0f64, f64::max)
            .max(
                self.strokes
                    .iter()
                    .map(|s| s.end_time)
                    .fold(0.0f64, f64::max),
            )
    }
}

/// 时间 `t` 所对应的完整演讲状态（SPEC §2）。
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationState {
    pub t: f64,
    /// 当前页面下标（一定落在 `pages` 范围内，前提是 `pages` 非空）。
    pub page_index: usize,
    pub page_id: PageId,
    /// 每个元素在该时刻是否可见。按元素 id 索引。
    visible: HashMap<ElementId, bool>,
    /// 每条笔迹（与 `strokes` 同序）在该时刻应绘制的前 N 个点。
    pub stroke_point_counts: Vec<usize>,
    /// 当前讲稿段落下标。
    pub section_index: Option<usize>,
    /// 当前（最近一次）标记点标签。
    pub active_marker: Option<String>,
}

impl PresentationState {
    /// **核心纯函数**：给定 `t`，返回当前页面、每个元素的可见性、
    /// 每条笔迹应画的前 N 个点、当前讲稿段落。
    ///
    /// 复杂度 O(events + strokes × log points)，与画布上元素数量无关。
    pub fn state_at(input: &TimelineInput, t: f64) -> Self {
        let page_count = input.pages.len();

        // ---- 1. 当前页面：最后一个 t_ev <= t 的 PageChange ----
        let mut page_index = input.start_page_index.min(page_count.saturating_sub(1));
        for ev in input.events {
            if ev.time() > t {
                break;
            }
            if let TimelineEvent::PageChange { page_id, .. } = ev {
                if let Some(i) = input.pages.iter().position(|p| &p.id == page_id) {
                    page_index = i;
                }
            }
        }
        let page_id = input
            .pages
            .get(page_index)
            .map(|p| p.id.clone())
            .unwrap_or_default();

        // ---- 2. 元素可见性 ----
        // 初值：hidden_by_default 决定；然后按时间顺序应用 Reveal / Hide。
        let mut visible = HashMap::new();
        for page in input.pages {
            for el in &page.elements {
                visible.insert(el.id.clone(), !el.hidden_by_default);
            }
        }
        for ev in input.events {
            if ev.time() > t {
                break;
            }
            match ev {
                TimelineEvent::Reveal { element_id, .. } => {
                    if let Some(v) = visible.get_mut(element_id) {
                        *v = true;
                    }
                }
                TimelineEvent::Hide { element_id, .. } => {
                    if let Some(v) = visible.get_mut(element_id) {
                        *v = false;
                    }
                }
                _ => {}
            }
        }

        // ---- 3. 笔迹绘制进度 ----
        let stroke_point_counts = input
            .strokes
            .iter()
            .map(|s| s.point_count_at(t))
            .collect();

        // ---- 4. 当前讲稿段落：最后一个 t_ev <= t 的 ScriptMark ----
        let mut section_index = None;
        let mut active_marker = None;
        for ev in input.events {
            if ev.time() > t {
                break;
            }
            match ev {
                TimelineEvent::ScriptMark { section_id, .. } => {
                    if let Some(i) = input.script.section_index(section_id) {
                        section_index = Some(i);
                    }
                }
                TimelineEvent::Marker { label, .. } => {
                    active_marker = Some(label.clone());
                }
                _ => {}
            }
        }
        // 还没有任何锚点：默认停在第一段，让提示栏不至于空着。
        if section_index.is_none() && !input.script.sections.is_empty() {
            section_index = Some(0);
        }

        PresentationState {
            t,
            page_index,
            page_id,
            visible,
            stroke_point_counts,
            section_index,
            active_marker,
        }
    }

    /// 强制把可见性设为"全部可见"。
    ///
    /// 制作模式用它表达"所有元素都已出现"（方便排版），这样 Create 模式与
    /// `state_at` 的可见性规则就只有一套语义，不会出现"UI 以为可见、
    /// state 以为隐藏"的分裂。
    pub fn show_all(&mut self) {
        for v in self.visible.values_mut() {
            *v = true;
        }
    }

    pub fn is_visible(&self, element_id: &str) -> bool {
        self.visible.get(element_id).copied().unwrap_or(false)
    }

    /// 当前页面里按绘制顺序排列、且在 `t` 时刻可见的元素下标。
    pub fn visible_elements<'a>(&'a self, page: &'a Page) -> impl Iterator<Item = &'a Element> + 'a {
        page.draw_order()
            .into_iter()
            .filter_map(move |i| page.elements.get(i))
            .filter(move |el| self.is_visible(&el.id))
    }

    pub fn stroke_points(&self, stroke_index: usize) -> usize {
        self.stroke_point_counts.get(stroke_index).copied().unwrap_or(0)
    }

    /// 当前时刻可见元素的数量（用于"还有几个没出现"的提示）。
    pub fn visible_count(&self, page: &Page) -> usize {
        self.visible_elements(page).count()
    }
}

/// 空间 → 元素 反查：返回点 `(x, y)` 命中的**最上层**元素（页面坐标系）。
///
/// 纯几何函数：不考虑可见性，是否可见由调用方结合 [`PresentationState`] 判断。
pub fn hit_test_element(page: &Page, x: f64, y: f64) -> Option<ElementId> {
    // 逆着绘制顺序找，第一个命中的就是最上层的。
    page.draw_order()
        .into_iter()
        .rev()
        .find(|&i| page.elements[i].contains(x, y))
        .map(|i| page.elements[i].id.clone())
}

/// 同上，但只在 `state` 认为可见的元素里找。
pub fn hit_test_visible_element(
    page: &Page,
    state: &PresentationState,
    x: f64,
    y: f64,
) -> Option<ElementId> {
    page.draw_order()
        .into_iter()
        .rev()
        .find(|&i| {
            let el = &page.elements[i];
            state.is_visible(&el.id) && el.contains(x, y)
        })
        .map(|i| page.elements[i].id.clone())
}

/// 点到线段的最近点参数 `u ∈ [0, 1]`，以及距离平方。
fn project_on_segment(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> (f64, f64) {
    let dx = bx - ax;
    let dy = by - ay;
    let len_sq = dx * dx + dy * dy;
    if len_sq <= f64::EPSILON {
        let ddx = px - ax;
        let ddy = py - ay;
        return (0.0, ddx * ddx + ddy * ddy);
    }
    let u = (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0);
    let cx = ax + u * dx;
    let cy = ay + u * dy;
    let ddx = px - cx;
    let ddy = py - cy;
    (u, ddx * ddx + ddy * ddy)
}

/// 空间 → 时间 反查：给定画布上的点，返回这条笔迹上**几何最近点**的时间戳。
///
/// 这是本产品区别于普通笔记软件的核心能力（SPEC §6 / §18）：
/// 点击手写轨迹上的某一笔，就能回到"当时正在写这个字"的说话时间。
///
/// 注意这里做的是**线段上的插值**而不是取最近的采样点：采样点之间可能相隔
/// 几十毫秒，插值能让 seek 的精度提升一个量级，代价可以忽略。
pub fn nearest_stroke_time(stroke: &Stroke, x: f64, y: f64) -> f64 {
    match stroke.points.len() {
        0 => 0.0,
        1 => stroke.points[0].t,
        _ => {
            let mut best = (f64::MAX, stroke.points[0].t);
            for w in stroke.points.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (u, dist_sq) = project_on_segment(x, y, a.x, a.y, b.x, b.y);
                if dist_sq < best.0 {
                    best = (dist_sq, a.t + u * (b.t - a.t));
                }
            }
            best.1
        }
    }
}

/// 点到笔迹的最短距离（页面坐标系）。用于命中判定。
pub fn stroke_distance(stroke: &Stroke, x: f64, y: f64) -> f64 {
    match stroke.points.len() {
        0 => f64::MAX,
        1 => {
            let dx = x - stroke.points[0].x;
            let dy = y - stroke.points[0].y;
            (dx * dx + dy * dy).sqrt()
        }
        _ => stroke
            .points
            .windows(2)
            .map(|w| project_on_segment(x, y, w[0].x, w[0].y, w[1].x, w[1].y).1)
            .fold(f64::MAX, f64::min)
            .sqrt(),
    }
}

/// 命中一条笔迹：在所有页面内笔迹中取最近的一条，超过 `tolerance` 视为没点到。
///
/// 返回 `(笔迹下标, 该点对应的时间戳)`。
pub fn hit_test_stroke(
    strokes: &[Stroke],
    page_id: &str,
    x: f64,
    y: f64,
    tolerance: f64,
) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64, f64)> = None; // (idx, dist, t)
    for (i, s) in strokes.iter().enumerate() {
        if s.page_id != page_id || s.points.is_empty() {
            continue;
        }
        let d = stroke_distance(s, x, y);
        if d > tolerance {
            continue;
        }
        // 距离可视作容差内的"置信度"，取最近的；距离相同则取后画的（更上面）。
        match best {
            Some((_, bd, _)) if bd <= d => {}
            _ => best = Some((i, d, nearest_stroke_time(s, x, y))),
        }
    }
    best.map(|(i, _, t)| (i, t))
}

/// 元素 → 时间的反查：返回在 `t_hint` 时刻（或之前）最近一次 Reveal 的时间。
///
/// 若该时刻之前没有 Reveal（比如用户把播放头拖到了元素出现之前），
/// 则回退到第一次 Reveal 的时间。都没有则返回 `None`。
///
/// 用于「点击画布上的元素 → 音频 seek 回当时的讲话位置」（SPEC §6）。
pub fn reveal_time(events: &[TimelineEvent], element_id: &str, t_hint: f64) -> Option<f64> {
    let mut first: Option<f64> = None;
    let mut last_before: Option<f64> = None;
    for ev in events {
        if let TimelineEvent::Reveal { element_id: eid, t } = ev {
            if eid != element_id {
                continue;
            }
            if first.is_none() {
                first = Some(*t);
            }
            if *t <= t_hint {
                last_before = Some(*t);
            }
        }
    }
    last_before.or(first)
}

/// 该元素是否是"在 `t` 时刻已经出现但本来默认隐藏"的（即由录制时的快捷键触发）。
pub fn was_triggered(events: &[TimelineEvent], element_id: &str, t: f64) -> bool {
    events
        .iter()
        .any(|e| matches!(e, TimelineEvent::Reveal { element_id: eid, t: et } if eid == element_id && *et <= t))
}

/// 取当前页面上"下一个还没出现的元素"（按绘制顺序），用于 `Space` 快捷键。
pub fn next_hidden_element(page: &Page, state: &PresentationState) -> Option<ElementId> {
    page.draw_order()
        .into_iter()
        .filter_map(|i| page.elements.get(i))
        .find(|el| !state.is_visible(&el.id))
        .map(|el| el.id.clone())
}

/// 把波形（-1.0 ~ 1.0 的样本）降采样成 `bins` 个峰值，供时间轴绘制。
///
/// 直接取每段的最大绝对值即可（SPEC §9-A 只需要"看得出停顿和快慢"，不需要精确）。
pub fn waveform_peaks(samples: &[f32], bins: usize) -> Vec<f32> {
    if samples.is_empty() || bins == 0 {
        return Vec::new();
    }
    let chunk = samples.len().div_ceil(bins).max(1);
    let mut out = Vec::with_capacity(bins);
    for c in samples.chunks(chunk) {
        let peak = c.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        out.push(peak.min(1.0));
    }
    out
}

/// 把波形峰值归一化到 0..1，避免小声录音看起来是一条直线。
pub fn normalize_peaks(peaks: &mut [f32]) {
    let max = peaks.iter().cloned().fold(0.0f32, f32::max);
    if max > 1e-4 {
        for p in peaks.iter_mut() {
            *p = (*p / max).min(1.0);
        }
    }
}

/// 秒 → `mm:ss.d` 显示。
pub fn format_time(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    format!("{:02}:{:04.1}", m, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Element, Page, ScriptDoc, ScriptSection, Stroke};

    /// 构造一个 3 页项目：p1 上 el1(默认隐藏)/el2(默认显示)，p2 上 el3(默认隐藏)。
    fn fixture() -> (Vec<Page>, ScriptDoc) {
        let mut p1 = Page::new("p1", "第 1 页");
        let mut el1 = Element::text("el1", 0.0, 0.0, 100.0, 40.0, "A");
        el1.hidden_by_default = true;
        let mut el2 = Element::text("el2", 0.0, 50.0, 100.0, 40.0, "B");
        el2.hidden_by_default = false;
        p1.elements = vec![el1, el2];

        let mut p2 = Page::new("p2", "第 2 页");
        let mut el3 = Element::text("el3", 0.0, 0.0, 100.0, 40.0, "C");
        el3.hidden_by_default = true;
        p2.elements = vec![el3];

        let p3 = Page::new("p3", "第 3 页");

        let script = ScriptDoc {
            sections: vec![
                ScriptSection::new("s1", "第一段"),
                ScriptSection::new("s2", "第二段"),
                ScriptSection::new("s3", "第三段"),
            ],
        };
        (vec![p1, p2, p3], script)
    }

    fn fixture_strokes() -> Vec<Stroke> {
        let mut s = Stroke::new("st1", "p1", "#e11d48", 3.0);
        s.push(0.0, 0.0, 10.0);
        s.push(10.0, 0.0, 11.0);
        s.push(20.0, 0.0, 12.0);
        vec![s]
    }

    fn events() -> Vec<TimelineEvent> {
        vec![
            TimelineEvent::PageChange {
                page_id: "p1".into(),
                t: 0.0,
            },
            TimelineEvent::ScriptMark {
                section_id: "s1".into(),
                t: 0.0,
            },
            TimelineEvent::Reveal {
                element_id: "el1".into(),
                t: 5.0,
            },
            TimelineEvent::ScriptMark {
                section_id: "s2".into(),
                t: 8.0,
            },
            TimelineEvent::PageChange {
                page_id: "p2".into(),
                t: 20.0,
            },
            TimelineEvent::Reveal {
                element_id: "el3".into(),
                t: 21.0,
            },
            TimelineEvent::Hide {
                element_id: "el1".into(),
                t: 30.0,
            },
        ]
    }

    // ---------- 验收标准 2.1：state_at(t) 的可见性 / 笔迹点数 ----------

    #[test]
    fn state_at_visibility_over_time() {
        let (pages, script) = fixture();
        let strokes = fixture_strokes();
        let evs = events();
        let input = TimelineInput {
            pages: &pages,
            strokes: &strokes,
            script: &script,
            events: &evs,
            start_page_index: 0,
        };

        // t = 0：el1 还没出现，el2 默认可见
        let s0 = input.state_at(0.0);
        assert_eq!(s0.page_index, 0);
        assert_eq!(s0.page_id, "p1");
        assert!(!s0.is_visible("el1"));
        assert!(s0.is_visible("el2"));
        assert!(!s0.is_visible("el3"));
        assert_eq!(s0.section_index, Some(0));

        // t = 6：el1 已出现
        let s6 = input.state_at(6.0);
        assert!(s6.is_visible("el1"));
        assert!(s6.is_visible("el2"));

        // t = 9：讲稿推进到第 2 段
        assert_eq!(input.state_at(9.0).section_index, Some(1));

        // t = 20 / 22：切页 + el3 出现
        let s20 = input.state_at(20.0);
        assert_eq!(s20.page_index, 1);
        assert_eq!(s20.page_id, "p2");
        assert!(!s20.is_visible("el3"));
        let s22 = input.state_at(22.0);
        assert!(s22.is_visible("el3"));

        // t = 31：el1 被 Hide 掉
        assert!(!input.state_at(31.0).is_visible("el1"));

        // 边界：t 极大 = 终态
        let sf = input.final_state();
        assert!(sf.is_visible("el3"));
        assert_eq!(sf.page_index, 1);
    }

    #[test]
    fn state_at_stroke_point_counts() {
        let (pages, script) = fixture();
        let strokes = fixture_strokes();
        let evs = events();
        let input = TimelineInput {
            pages: &pages,
            strokes: &strokes,
            script: &script,
            events: &evs,
            start_page_index: 0,
        };

        assert_eq!(input.state_at(9.9).stroke_points(0), 0);
        assert_eq!(input.state_at(10.0).stroke_points(0), 1);
        assert_eq!(input.state_at(10.5).stroke_points(0), 1);
        assert_eq!(input.state_at(11.0).stroke_points(0), 2);
        assert_eq!(input.state_at(12.0).stroke_points(0), 3);
        assert_eq!(input.state_at(100.0).stroke_points(0), 3);
        // 越界索引安全
        assert_eq!(input.state_at(100.0).stroke_points(42), 0);
    }

    #[test]
    fn show_all_makes_every_element_visible() {
        let (pages, script) = fixture();
        let input = TimelineInput {
            pages: &pages,
            strokes: &[],
            script: &script,
            events: &[],
            start_page_index: 0,
        };
        // 没有任何事件时，hidden_by_default 的元素是隐藏的
        let mut st = input.state_at(f64::INFINITY);
        assert!(!st.is_visible("el1"));
        assert!(!st.is_visible("el3"));
        st.show_all();
        assert!(st.is_visible("el1"));
        assert!(st.is_visible("el2"));
        assert!(st.is_visible("el3"));
    }

    #[test]
    fn state_at_before_start_page_change() {
        // 录制从第 2 页开始，且 t=0 没有 PageChange 时，应停在 start_page_index。
        let (pages, script) = fixture();
        let input = TimelineInput {
            pages: &pages,
            strokes: &[],
            script: &script,
            events: &[],
            start_page_index: 2,
        };
        assert_eq!(input.state_at(0.0).page_index, 2);
        assert_eq!(input.state_at(-5.0).page_index, 2);
    }

    #[test]
    fn visible_elements_filters_and_orders_by_z() {
        let (mut pages, script) = fixture();
        let mut top = Element::text("el_top", 0.0, 0.0, 10.0, 10.0, "top");
        top.z = 9;
        top.hidden_by_default = false;
        pages[0].elements.push(top);
        let input = TimelineInput {
            pages: &pages,
            strokes: &[],
            script: &script,
            events: &[],
            start_page_index: 0,
        };
        let s = input.state_at(0.0);
        // z 大的排后面（后绘制）
        let ids: Vec<&str> = s
            .visible_elements(&pages[0])
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, vec!["el2", "el_top"]);
        assert_eq!(s.visible_count(&pages[0]), 2);
    }

    // ---------- 验收标准 2.2：Reveal 事件排序 / 乱序插入 ----------

    #[test]
    fn events_are_sorted_after_out_of_order_insert() {
        let mut evs = vec![
            TimelineEvent::Reveal {
                element_id: "b".into(),
                t: 14.21,
            },
            TimelineEvent::Reveal {
                element_id: "d".into(),
                t: 28.32,
            },
            TimelineEvent::Reveal {
                element_id: "a".into(),
                t: 8.42,
            },
            TimelineEvent::Reveal {
                element_id: "c".into(),
                t: 21.75,
            },
        ];
        sort_events(&mut evs);
        assert!(is_sorted(&evs));
        let order: Vec<(String, f64)> = evs
            .iter()
            .map(|e| match e {
                TimelineEvent::Reveal { element_id, t } => (element_id.clone(), *t),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            order,
            vec![
                ("a".to_string(), 8.42),
                ("b".to_string(), 14.21),
                ("c".to_string(), 21.75),
                ("d".to_string(), 28.32),
            ]
        );
    }

    #[test]
    fn same_timestamp_uses_kind_priority_then_insertion_order() {
        let mut evs = vec![
            TimelineEvent::Marker {
                label: "m".into(),
                t: 0.0,
            },
            TimelineEvent::Reveal {
                element_id: "e1".into(),
                t: 0.0,
            },
            TimelineEvent::ScriptMark {
                section_id: "s1".into(),
                t: 0.0,
            },
            TimelineEvent::PageChange {
                page_id: "p1".into(),
                t: 0.0,
            },
            TimelineEvent::Reveal {
                element_id: "e2".into(),
                t: 0.0,
            },
        ];
        sort_events(&mut evs);
        let kinds: Vec<&str> = evs.iter().map(|e| e.kind_str()).collect();
        assert_eq!(kinds, vec!["page", "script", "reveal", "reveal", "marker"]);
        // 同类型内部保持插入顺序
        match (&evs[2], &evs[3]) {
            (
                TimelineEvent::Reveal { element_id: a, .. },
                TimelineEvent::Reveal { element_id: b, .. },
            ) => {
                assert_eq!((a.as_str(), b.as_str()), ("e1", "e2"));
            }
            _ => panic!("unexpected order"),
        }
    }

    #[test]
    fn sorting_is_stable_for_many_equal_events() {
        let mut evs: Vec<TimelineEvent> = (0..50)
            .map(|i| TimelineEvent::Reveal {
                element_id: format!("e{i}"),
                t: 3.0,
            })
            .collect();
        sort_events(&mut evs);
        for (i, e) in evs.iter().enumerate() {
            match e {
                TimelineEvent::Reveal { element_id, .. } => assert_eq!(element_id, &format!("e{i}")),
                _ => unreachable!(),
            }
        }
    }

    // ---------- 验收标准 2.3：nearest_stroke_time 空间→时间 ----------

    #[test]
    fn nearest_stroke_time_interpolates_along_segment() {
        let strokes = fixture_strokes();
        let s = &strokes[0];
        // 起点
        assert!((nearest_stroke_time(s, 0.0, 0.0) - 10.0).abs() < 1e-9);
        // 终点
        assert!((nearest_stroke_time(s, 20.0, 0.0) - 12.0).abs() < 1e-9);
        // 正中点：x=5 → 线段 1 的中点 → t = 10.5
        assert!((nearest_stroke_time(s, 5.0, 0.0) - 10.5).abs() < 1e-9);
        // x=15 → 第二段中点 → t = 11.5
        assert!((nearest_stroke_time(s, 15.0, 0.0) - 11.5).abs() < 1e-9);
        // 垂直方向偏移不影响参数 u
        assert!((nearest_stroke_time(s, 5.0, 100.0) - 10.5).abs() < 1e-9);
        // 超出两端会被 clamp
        assert!((nearest_stroke_time(s, -50.0, 0.0) - 10.0).abs() < 1e-9);
        assert!((nearest_stroke_time(s, 500.0, 0.0) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn nearest_stroke_time_degenerate_cases() {
        let empty = Stroke::new("e", "p1", "#000", 1.0);
        assert_eq!(nearest_stroke_time(&empty, 1.0, 1.0), 0.0);

        let mut single = Stroke::new("s", "p1", "#000", 1.0);
        single.push(5.0, 5.0, 7.25);
        assert_eq!(nearest_stroke_time(&single, 100.0, 100.0), 7.25);

        // 重复点（原地停顿）不应产生 NaN
        let mut dup = Stroke::new("d", "p1", "#000", 1.0);
        dup.push(1.0, 1.0, 1.0);
        dup.push(1.0, 1.0, 2.0);
        let t = nearest_stroke_time(&dup, 1.0, 1.0);
        assert!(t.is_finite());
        assert!((1.0..=2.0).contains(&t));
    }

    #[test]
    fn nearest_stroke_time_picks_geometrically_closest_segment() {
        // Z 字形：距离查询点更近的那一段应胜出，而不是时间上更早的那段
        let mut s = Stroke::new("z", "p1", "#000", 1.0);
        s.push(0.0, 0.0, 1.0);
        s.push(100.0, 0.0, 2.0);
        s.push(100.0, 100.0, 3.0);
        s.push(0.0, 100.0, 4.0);
        // 点在最后一段附近 → 时间应接近 4.0
        let t = nearest_stroke_time(&s, 50.0, 100.0);
        assert!((t - 3.5).abs() < 1e-6, "got {t}");
    }

    #[test]
    fn hit_test_prefers_topmost_element_and_respects_visibility() {
        let mut p = Page::new("p1", "第 1 页");
        let mut under = Element::text("under", 0.0, 0.0, 100.0, 100.0, "under");
        under.hidden_by_default = false;
        let mut over = Element::text("over", 0.0, 0.0, 100.0, 100.0, "over");
        over.z = 5;
        over.hidden_by_default = true;
        p.elements = vec![under, over];

        // 纯几何：命中 z 更大的 over
        assert_eq!(hit_test_element(&p, 50.0, 50.0).as_deref(), Some("over"));
        assert_eq!(hit_test_element(&p, 500.0, 50.0), None);

        // 考虑可见性：t=0 时 over 还没出现 → 命中 under
        let script = ScriptDoc::default();
        let input = TimelineInput {
            pages: std::slice::from_ref(&p),
            strokes: &[],
            script: &script,
            events: &[],
            start_page_index: 0,
        };
        let s = input.state_at(0.0);
        assert_eq!(
            hit_test_visible_element(&p, &s, 50.0, 50.0).as_deref(),
            Some("under")
        );
        assert_eq!(next_hidden_element(&p, &s).as_deref(), Some("over"));
    }

    #[test]
    fn hit_test_stroke_returns_time_of_nearest_point() {
        let strokes = fixture_strokes();
        let hit = hit_test_stroke(&strokes, "p1", 5.0, 2.0, 5.0);
        assert!(hit.is_some());
        let (idx, t) = hit.unwrap();
        assert_eq!(idx, 0);
        assert!((t - 10.5).abs() < 1e-9);

        // 容差之外点不中
        assert!(hit_test_stroke(&strokes, "p1", 5.0, 500.0, 5.0).is_none());
        // 页面不匹配也不命中
        assert!(hit_test_stroke(&strokes, "p2", 5.0, 0.0, 5.0).is_none());
    }

    // ---------- 验收标准 2.4：按时间反查讲稿段落 ----------

    #[test]
    fn script_section_lookup_by_time() {
        let (pages, script) = fixture();
        let evs = events();
        let input = TimelineInput {
            pages: &pages,
            strokes: &[],
            script: &script,
            events: &evs,
            start_page_index: 0,
        };
        assert_eq!(input.state_at(0.0).section_index, Some(0));
        assert_eq!(input.state_at(7.99).section_index, Some(0));
        assert_eq!(input.state_at(8.0).section_index, Some(1));
        assert_eq!(input.state_at(19.0).section_index, Some(1));
        assert_eq!(input.state_at(999.0).section_index, Some(1));
    }

    #[test]
    fn script_defaults_to_first_section_when_no_marks() {
        let (pages, script) = fixture();
        let input = TimelineInput {
            pages: &pages,
            strokes: &[],
            script: &script,
            events: &[],
            start_page_index: 0,
        };
        assert_eq!(input.state_at(3.0).section_index, Some(0));

        // 空讲稿不应 panic
        let empty = ScriptDoc::default();
        let input2 = TimelineInput {
            pages: &pages,
            strokes: &[],
            script: &empty,
            events: &[],
            start_page_index: 0,
        };
        assert_eq!(input2.state_at(3.0).section_index, None);
    }

    #[test]
    fn reveal_time_navigation() {
        let evs = events();
        // 元素在 t=5 出现
        assert_eq!(reveal_time(&evs, "el1", 100.0), Some(5.0));
        assert_eq!(reveal_time(&evs, "el3", 100.0), Some(21.0));
        // 播放头在出现之前 → 回退到首次出现时间
        assert_eq!(reveal_time(&evs, "el1", 1.0), Some(5.0));
        // 不存在的元素
        assert_eq!(reveal_time(&evs, "nope", 100.0), None);
        assert!(was_triggered(&evs, "el1", 6.0));
        assert!(!was_triggered(&evs, "el1", 4.0));
    }

    // ---------- 波形与时间格式化 ----------

    #[test]
    fn waveform_downsampling() {
        let samples: Vec<f32> = (0..1000).map(|i| if i % 100 == 0 { 1.0 } else { 0.01 }).collect();
        let peaks = waveform_peaks(&samples, 10);
        assert_eq!(peaks.len(), 10);
        assert!(peaks.iter().all(|p| (0.0..=1.0).contains(p)));
        assert!(peaks.iter().any(|p| *p > 0.9));

        let mut norm = vec![0.1f32, 0.05, 0.2];
        normalize_peaks(&mut norm);
        assert!((norm[2] - 1.0).abs() < 1e-6);

        assert!(waveform_peaks(&[], 10).is_empty());
        assert!(waveform_peaks(&[1.0], 0).is_empty());
    }

    #[test]
    fn time_formatting() {
        assert_eq!(format_time(0.0), "00:00.0");
        assert_eq!(format_time(8.42), "00:08.4");
        assert_eq!(format_time(75.5), "01:15.5");
        assert_eq!(format_time(-1.0), "00:00.0");
    }

    #[test]
    fn duration_is_max_of_events_and_strokes() {
        let (pages, script) = fixture();
        let strokes = fixture_strokes();
        let evs = events();
        let input = TimelineInput {
            pages: &pages,
            strokes: &strokes,
            script: &script,
            events: &evs,
            start_page_index: 0,
        };
        // 事件最大 t = 30，笔迹最大 t = 12
        assert_eq!(input.duration(), 30.0);
    }
}
