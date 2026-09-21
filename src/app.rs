//! 应用状态机：Create / Rehearse / Review / Present（SPEC §11）。
//!
//! 这是唯一把「数据模型 / 时间轴 / 音频 / Slint」粘起来的地方。
//! 所有业务判断都尽量下沉到 `crate::timeline` 与 `crate::model`，
//! 本文件只负责：把 UI 事件翻译成模型操作，再把 `state_at(t)` 的结果推回 UI。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::audio::play::AudioPlayer;
use crate::audio::record::MicRecorder;
use crate::audio::Recorder;
use crate::model::{
    element::parse_hex_color, Element, ElementKind, Project, Rehearsal, Stroke, TextAlign,
};
use crate::model::script::ScriptLevel;
use crate::storage::{ProjectHandle, Store};
use crate::timeline::{
    format_time, hit_test_stroke, hit_test_visible_element, next_hidden_element, normalize_peaks,
    reveal_time, sort_events, waveform_peaks, PresentationState, TimelineEvent, TimelineInput,
};

slint::include_modules!();

/// 界面刷新频率（约 30 fps）。播放头与录制计时靠它平滑推进。
const FRAME: Duration = Duration::from_millis(33);
/// 波形降采样后的采样点数（同时也是 SVG 包络的点数）。
const WAVE_BINS: usize = 1000;
/// 点击判定阈值（页面坐标单位）：移动超过它就认为在拖动而不是点击。
const CLICK_SLOP: f64 = 6.0;
/// 右下角改尺寸的感应范围（页面坐标单位）。
const RESIZE_GRAB: f64 = 22.0;

const TRANSPARENT: slint::Color = slint::Color::from_argb_u8(0, 0, 0, 0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Create,
    Rehearse,
    Review,
    Present,
}

impl Mode {
    fn from_index(i: i32) -> Mode {
        match i {
            1 => Mode::Rehearse,
            2 => Mode::Review,
            3 => Mode::Present,
            _ => Mode::Create,
        }
    }
    fn to_slint(self) -> AppMode {
        match self {
            Mode::Create => AppMode::Create,
            Mode::Rehearse => AppMode::Rehearse,
            Mode::Review => AppMode::Review,
            Mode::Present => AppMode::Present,
        }
    }
}

/// 画布上正在进行的拖动操作。
#[derive(Clone, Debug, PartialEq)]
enum Drag {
    None,
    /// 移动元素：记录抓取点相对元素原点的偏移。
    Move { id: String, dx: f64, dy: f64 },
    /// 从右下角改尺寸。
    Resize { id: String },
    /// 手写：记录笔迹在 `live.strokes` 中的下标。
    Stroke { index: usize },
}

/// Rehearse / Present 共用的「实时状态」。
///
/// 与 `Rehearsal` 的区别：Rehearsal 是**落盘后**的完整记录，Live 是**正在发生**
/// 的过程。两者使用同一套 `TimelineEvent`，所以录制结束只是把 Live 的内容搬进
/// Rehearsal，不需要任何格式转换，也不可能漏字段。
struct Live {
    events: Vec<TimelineEvent>,
    strokes: Vec<Stroke>,
    start_page: usize,
    next_stroke_serial: u64,
}

impl Live {
    fn fresh(start_page: usize, page_id: &str, script_first: Option<&str>) -> Self {
        let mut events = vec![TimelineEvent::PageChange {
            page_id: page_id.to_string(),
            t: 0.0,
        }];
        // 第一段讲稿在 t=0 就算"开讲"，这样提示栏立刻有内容
        if let Some(sid) = script_first {
            events.push(TimelineEvent::ScriptMark {
                section_id: sid.to_string(),
                t: 0.0,
            });
        }
        Self {
            events,
            strokes: Vec::new(),
            start_page,
            next_stroke_serial: 1,
        }
    }
}

/// 画布按下动作的元信息，用来区分「点击」与「拖动」。
struct PressInfo {
    x: f64,
    y: f64,
    travel: f64,
    #[allow(dead_code)]
    at: Instant,
}

pub struct AppState {
    ui: AppWindow,
    store: Store,
    handle: ProjectHandle,

    mode: Mode,
    drag: Drag,
    press: Option<PressInfo>,
    last_pointer: (f64, f64),
    selected: Option<String>,
    editing: Option<String>,

    live: Live,
    /// 当前载入的演练（回放用；录制结束后也会填上）
    rehearsal: Option<Rehearsal>,
    rehearsal_index: usize,
    recorder: Option<Box<dyn Recorder>>,
    record_start: Instant,
    recording: bool,
    marked_sections: Vec<(String, f64)>,

    player: AudioPlayer,

    // ---- Slint 模型（保持 Rc 以便局部更新） ----
    elements_model: Rc<VecModel<ElementVisual>>,
    strokes_model: Rc<VecModel<StrokeVisual>>,
    ticks_model: Rc<VecModel<EventTick>>,
    script_model: Rc<VecModel<ScriptRow>>,
    pages_model: Rc<VecModel<PageRow>>,
    projects_model: Rc<VecModel<SharedString>>,

    // ---- 缓存 ----
    images: HashMap<String, slint::Image>,
    /// 上次推送给 UI 的渲染签名，用来避免每帧重建模型
    elem_sig: u64,
    stroke_sig: usize,
    pages_sig: (i32, i32),
    script_sig: (i32, i32, i32),
    status: String,
    last_position_label: String,
}

impl AppState {
    pub fn new(ui: AppWindow, store: Store, handle: ProjectHandle) -> Self {
        let mut state = Self {
            ui,
            store,
            handle,
            mode: Mode::Create,
            drag: Drag::None,
            press: None,
            last_pointer: (0.0, 0.0),
            selected: None,
            editing: None,
            live: Live::fresh(0, "", None),
            rehearsal: None,
            rehearsal_index: 0,
            recorder: None,
            record_start: Instant::now(),
            recording: false,
            marked_sections: Vec::new(),
            player: AudioPlayer::open(),
            elements_model: Rc::new(VecModel::default()),
            strokes_model: Rc::new(VecModel::default()),
            ticks_model: Rc::new(VecModel::default()),
            script_model: Rc::new(VecModel::default()),
            pages_model: Rc::new(VecModel::default()),
            projects_model: Rc::new(VecModel::default()),
            images: HashMap::new(),
            elem_sig: u64::MAX,
            stroke_sig: usize::MAX,
            pages_sig: (-1, -1),
            script_sig: (-1, -1, -1),
            status: String::new(),
            last_position_label: String::new(),
        };

        // 载入最近一次演练，让「回放」一打开就有东西可拖
        if let Some(r) = state.handle.load_latest_rehearsal() {
            state.rehearsal_index = state
                .handle
                .meta()
                .iter()
                .position(|m| m.id == r.id)
                .unwrap_or(0);
            state.load_rehearsal_into_player(r);
        }
        let mut msg = "提示：切到「演练」按 Record（Ctrl+R），一边讲话一边按 1-9 让元素出现".to_string();
        if let Some(note) = state.player.note() {
            msg = format!("{msg} · {note}");
        }
        state.set_status(msg);
        state.push_static_models();
        state.refresh(true);
        state
    }

    // ==================== UI 推送 ====================

    fn set_status(&mut self, s: String) {
        self.status = s;
        self.ui.set_status(SharedString::from(self.status.as_str()));
    }

    fn page_size(&self) -> (f64, f64) {
        (
            self.handle.project.page_size.w,
            self.handle.project.page_size.h,
        )
    }

    /// 推送「不随播放变化」的模型：页面列表、项目列表、画布尺寸。
    fn push_static_models(&mut self) {
        let project = self.handle.project();
        let (w, h) = self.page_size();
        self.ui.set_page_w(w as f32);
        self.ui.set_page_h(h as f32);
        self.ui.set_project_name(SharedString::from(project.name.as_str()));

        let rows: Vec<PageRow> = project
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| PageRow {
                name: p.name.clone().into(),
                index: i as i32,
                current: i == project.current_page,
                element_count: p.elements.len() as i32,
            })
            .collect();
        self.pages_model.set_vec(rows);
        self.ui.set_current_page(project.current_page as i32);
        self.pages_sig = (project.current_page as i32, project.pages.len() as i32);

        let names = self.store.list_projects();
        let list: Vec<SharedString> = names
            .iter()
            .map(|(_, n)| SharedString::from(n.as_str()))
            .collect();
        let idx = names
            .iter()
            .position(|(id, _)| id == &project.id)
            .map(|i| i as i32)
            .unwrap_or(-1);
        self.projects_model.set_vec(list);
        self.ui.set_project_index(idx);
        // 页面高亮的缓存也一并失效，下一次 refresh 会按当前页面重建
        self.pages_sig = (-1, -1);
    }

    /// 计算当前应展示的 `PresentationState`。
    ///
    /// 四个模式共用同一套算法：
    /// - Create：`t = ∞`，永远显示终态，方便排版；
    /// - Rehearse / Present：用 Live 的事件，同样 `t = ∞`（"已经发生的全部"）；
    /// - Review：用 `t = 播放位置`，这才是真正的"按时间回放"。
    fn current_state(&self) -> PresentationState {
        match self.mode {
            Mode::Create => {
                // 制作模式：显示**当前页面**的终态（所有元素都可见，方便排版）。
                // 刻意传入空事件/空笔迹：录制留下的 PageChange 与手写痕迹属于
                // 「某一次演练」，不应该干扰正在编辑的内容；默认隐藏的元素在
                // UI 上用虚线框提示它会由快捷键触发，而不是真的隐藏。
                let p = self.handle.project();
                let mut st = TimelineInput {
                    pages: &p.pages,
                    strokes: &[],
                    script: &p.script,
                    events: &[],
                    start_page_index: p.current_page,
                }
                .final_state();
                // 制作模式下"默认隐藏"只是录制行为，不是编辑时也要看不见：
                // 这里强制全部可见，隐藏语义只由画面上的四角括号提示。
                st.show_all();
                st
            }
            Mode::Review => {
                let t = self.player.position();
                compute_state(self.handle.project(), self.rehearsal.as_ref(), t)
            }
            Mode::Rehearse | Mode::Present => {
                let p = self.handle.project();
                TimelineInput {
                    pages: &p.pages,
                    strokes: &self.live.strokes,
                    script: &p.script,
                    events: &self.live.events,
                    start_page_index: self.live.start_page,
                }
                .state_at(f64::INFINITY)
            }
        }
    }

    fn refresh(&mut self, force: bool) {
        let st = self.current_state();
        self.refresh_elements(&st, force);
        self.refresh_strokes(&st, force);
        self.refresh_script(&st, force);
        self.refresh_canvas_flags();
        self.refresh_page_highlight(&st);
    }

    /// 让左侧页面栏的高亮跟随「当前真正显示的那一页」。
    ///
    /// 回放/演练时页面是被 `PageChange` 事件推动的，而不是被用户点选的，
    /// 所以这里不能用 `project.current_page`。
    fn refresh_page_highlight(&mut self, st: &PresentationState) {
        let sig = (st.page_index as i32, self.handle.project().pages.len() as i32);
        if sig == self.pages_sig {
            return;
        }
        self.pages_sig = sig;
        let project = self.handle.project();
        let rows: Vec<PageRow> = project
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| PageRow {
                name: SharedString::from(p.name.as_str()),
                index: i as i32,
                current: i == st.page_index,
                element_count: p.elements.len() as i32,
            })
            .collect();
        self.pages_model.set_vec(rows);
        self.ui.set_current_page(st.page_index as i32);
    }

    fn refresh_canvas_flags(&mut self) {
        let (can_draw, hint, interactive) = match self.mode {
            Mode::Create => (
                false,
                "画布是空的：用上方工具栏添加「文字」或「图片」".to_string(),
                true,
            ),
            Mode::Rehearse => (
                self.recording,
                if self.recording {
                    String::new()
                } else {
                    "按 Record（Ctrl+R）开始录制；录制时鼠标拖动 = 手写".to_string()
                },
                true,
            ),
            Mode::Review => (false, String::new(), true),
            Mode::Present => (false, String::new(), false),
        };
        self.ui.set_can_draw(can_draw);
        self.ui.set_canvas_hint(SharedString::from(hint.as_str()));
        self.ui.set_canvas_interactive(interactive);
    }

    /// 按需解码并缓存元素图片（在借用 project 之前先做完，避免借用冲突）。
    fn ensure_images(&mut self) {
        let names: Vec<String> = self
            .handle
            .project()
            .pages
            .iter()
            .flat_map(|p| p.elements.iter())
            .filter_map(|e| e.asset_name().map(|s| s.to_string()))
            .collect();
        for n in names {
            if !self.images.contains_key(&n) {
                let path = self.handle.resolve_asset(&n);
                let img = load_image(&path).unwrap_or_default();
                self.images.insert(n, img);
            }
        }
    }

    fn refresh_elements(&mut self, st: &PresentationState, force: bool) {
        self.ensure_images();
        let project = self.handle.project();
        let Some(page) = project.pages.get(st.page_index) else {
            return;
        };
        let order = page.draw_order();

        let mut hash: u64 = 1469598103934665603 ^ st.page_index as u64;
        for (i, &idx) in order.iter().enumerate() {
            let el = &page.elements[idx];
            let visible = st.is_visible(&el.id);
            hash = hash.wrapping_mul(1099511628211).wrapping_add(i as u64 + 1);
            if visible {
                hash = hash.wrapping_add(0x9E37);
            }
            if self.selected.as_deref() == Some(el.id.as_str()) {
                hash = hash.wrapping_add(0x51ED);
            }
            if self.mode == Mode::Create {
                hash ^= 0xABCD;
            }
        }
        if !force && hash == self.elem_sig {
            return;
        }
        self.elem_sig = hash;

        let mut rows: Vec<ElementVisual> = Vec::with_capacity(order.len());
        for (slot, &idx) in order.iter().enumerate() {
            let el = &page.elements[idx];
            let visible = st.is_visible(&el.id);
            // 演练模式下"还没出现"的元素：不画内容，但留一个占位框 + 数字键提示，
            // 让"按 1/2/3 会让什么出现"变成可见的。演讲模式保持干净，不做提示。
            let hint_only = !visible && self.mode == Mode::Rehearse;
            if !visible && !hint_only {
                continue;
            }
            // 1-9 对应页面内元素的**绘制顺序**（稳定映射，不随已出现的元素变化）
            let label = if hint_only && slot < 9 {
                format!("{}", slot + 1)
            } else {
                String::new()
            };
            let (kind, text) = if hint_only {
                ("hint", String::new())
            } else {
                match &el.kind {
                    ElementKind::Text { text } => ("text", text.clone()),
                    ElementKind::Image { .. } => ("image", String::new()),
                    ElementKind::Shape { .. } => ("shape", String::new()),
                }
            };
            let image = el
                .asset_name()
                .and_then(|n| self.images.get(n).cloned())
                .unwrap_or_default();
            rows.push(ElementVisual {
                id: SharedString::from(el.id.as_str()),
                kind: SharedString::from(kind),
                text: SharedString::from(text.as_str()),
                image,
                has_image: !hint_only && el.asset_name().is_some(),
                x: el.x as f32,
                y: el.y as f32,
                w: el.w as f32,
                h: el.h as f32,
                color: to_color(&el.style.color, slint::Color::from_rgb_u8(31, 41, 51)),
                background: el
                    .style
                    .background
                    .as_deref()
                    .map(|c| to_color(c, TRANSPARENT))
                    .unwrap_or(TRANSPARENT),
                has_background: el.style.background.is_some(),
                font_size: el.style.font_size as f32,
                bold: el.style.bold,
                align: match el.style.align {
                    TextAlign::Left => 0,
                    TextAlign::Center => 1,
                    TextAlign::Right => 2,
                },
                selected: self.selected.as_deref() == Some(el.id.as_str()),
                // 制作模式：默认隐藏的元素画四角括号，提示"它会在录制时被触发"
                ghost: self.mode == Mode::Create && el.hidden_by_default,
                trigger_label: SharedString::from(label.as_str()),
                hint_only,
            });
        }
        self.elements_model.set_vec(rows);
        self.ui.set_selected_id(SharedString::from(
            self.selected.as_deref().unwrap_or(""),
        ));
    }

    fn refresh_strokes(&mut self, st: &PresentationState, force: bool) {
        let page_id = st.page_id.clone();
        let counts: Option<Vec<usize>> = match self.mode {
            Mode::Rehearse | Mode::Present => None,
            _ => Some(st.stroke_point_counts.clone()),
        };
        let total: usize = match &counts {
            Some(c) => c.iter().sum(),
            None => self.live.strokes.iter().map(|s| s.points.len()).sum(),
        };
        if !force && total == self.stroke_sig {
            return;
        }
        self.stroke_sig = total;

        let mut rows: Vec<StrokeVisual> = Vec::new();
        {
            let (strokes, counts): (&[Stroke], Option<&Vec<usize>>) = match self.mode {
                Mode::Rehearse | Mode::Present => (&self.live.strokes, None),
                _ => (
                    self.rehearsal
                        .as_ref()
                        .map(|r| r.strokes.as_slice())
                        .unwrap_or(&[]),
                    counts.as_ref(),
                ),
            };
            for (i, s) in strokes.iter().enumerate() {
                if s.page_id != page_id {
                    continue;
                }
                let n = counts
                    .and_then(|c| c.get(i).copied())
                    .unwrap_or(s.points.len());
                if n == 0 {
                    continue;
                }
                rows.push(StrokeVisual {
                    id: SharedString::from(s.id.as_str()),
                    d: SharedString::from(s.to_svg_path(n).as_str()),
                    color: to_color(&s.color, slint::Color::from_rgb_u8(225, 29, 72)),
                    width: s.width as f32,
                });
            }
        }
        self.strokes_model.set_vec(rows);
    }

    fn refresh_script(&mut self, st: &PresentationState, force: bool) {
        let level = self.ui.get_script_level();
        let sig = (
            level,
            st.section_index.map(|i| i as i32).unwrap_or(-1),
            self.mode as i32,
        );
        if !force && sig == self.script_sig {
            return;
        }
        self.script_sig = sig;

        let level = match level {
            1 => ScriptLevel::Prompt,
            2 => ScriptLevel::Minimal,
            _ => ScriptLevel::Full,
        };
        let times = self.section_times();
        let mut rows = Vec::new();
        {
            let project = self.handle.project();
            for (i, s) in project.script.sections.iter().enumerate() {
                let t = times.get(i).copied().flatten();
                let page_label = s
                    .page_id
                    .as_deref()
                    .and_then(|pid| project.page(pid))
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                rows.push(ScriptRow {
                    text: SharedString::from(s.text(level)),
                    time_label: SharedString::from(t.map(format_time).unwrap_or_default().as_str()),
                    page_label: SharedString::from(page_label.as_str()),
                    has_time: t.is_some(),
                    current: st.section_index == Some(i),
                    markable: self.mode == Mode::Rehearse,
                });
            }
        }
        self.script_model.set_vec(rows);
    }

    /// 每个讲稿段落的实际讲话时间：优先用当前演练的锚点，其次用项目里的工作副本。
    fn section_times(&self) -> Vec<Option<f64>> {
        let project = self.handle.project();
        project
            .script
            .sections
            .iter()
            .map(|s| {
                self.rehearsal
                    .as_ref()
                    .and_then(|r| r.section_reveal_time(&s.id))
                    .or(s.reveal_time)
            })
            .collect()
    }

    // ==================== 模式切换 ====================

    pub fn set_mode(&mut self, mode: Mode) {
        if self.recording && mode != Mode::Rehearse {
            self.stop_recording();
        }
        self.mode = mode;
        self.drag = Drag::None;
        self.press = None;
        self.selected = None;
        self.editing = None;
        self.ui.set_editing_id(SharedString::default());
        self.ui.set_mode(mode.to_slint());
        self.ui.set_playing(false);
        self.ui.set_recording(self.recording);

        match mode {
            Mode::Create => {
                self.player.pause();
                self.set_status("制作模式：添加文字 / 图片、拖动排版、双击文字改内容".into());
            }
            Mode::Rehearse => {
                self.player.pause();
                let project = self.handle.project();
                let page_id = project.current_page_id();
                let first = project.script.sections.first().map(|s| s.id.clone());
                self.live = Live::fresh(project.current_page, &page_id, first.as_deref());
                self.rehearsal = None;
                self.ui.set_rehearsal_name(SharedString::default());
                self.ui.set_rehearsal_count(0);
                self.waveform_reset();
                self.set_status(
                    "演练模式：按 Record（Ctrl+R）开始；1-9 触发元素 · 空格下一个 · 鼠标拖动 = 手写".into(),
                );
            }
            Mode::Review => {
                if self.rehearsal.is_none() {
                    if let Some(r) = self.handle.load_latest_rehearsal() {
                        self.load_rehearsal_into_player(r);
                    }
                }
                if self.rehearsal.is_some() {
                    self.player.seek(0.0);
                    self.ui.set_position(0.0);
                    self.set_status(
                        "回放模式：拖时间轴同步画面/笔迹/讲稿；点元素或笔迹跳回当时的话".into(),
                    );
                } else {
                    self.set_status("还没有演练记录：先去「演练」录一次".into());
                }
            }
            Mode::Present => {
                self.player.pause();
                let project = self.handle.project();
                let page_id = project.current_page_id();
                self.live = Live::fresh(project.current_page, &page_id, None);
                self.set_status("演讲模式：←/→ 翻页，1-9 / 空格 触发元素出现（不录音）".into());
            }
        }
        self.refresh(true);
    }

    fn waveform_reset(&mut self) {
        self.player.set_duration(0.0);
        self.ui.set_duration(0.0);
        self.ui.set_position(0.0);
        self.ui.set_playing(false);
        self.ticks_model.set_vec(Vec::new());
        self.ui.set_wave_path(SharedString::default());
        self.ui.set_has_audio(false);
        self.ui.set_duration_label(SharedString::from("00:00.0"));
    }

    // ==================== 录制 ====================

    fn rec_time(&self) -> f64 {
        match &self.recorder {
            // 时间戳 = 已经采集到的帧数 / 采样率，与写进 WAV 的内容严格对齐，
            // 不会因为驱动缓冲区而系统性漂移。
            Some(r) => r.elapsed().as_secs_f64(),
            None => self.record_start.elapsed().as_secs_f64(),
        }
    }

    pub fn toggle_recording(&mut self) {
        if self.recording {
            self.stop_recording();
        } else {
            self.start_recording();
        }
    }

    fn start_recording(&mut self) {
        if self.mode != Mode::Rehearse {
            self.set_mode(Mode::Rehearse);
        }
        let project = self.handle.project();
        let page_id = project.current_page_id();
        let first = project.script.sections.first().map(|s| s.id.clone());
        self.live = Live::fresh(project.current_page, &page_id, first.as_deref());
        self.marked_sections.clear();
        if let Some(sid) = &first {
            self.marked_sections.push((sid.clone(), 0.0));
        }
        self.rehearsal = None;
        self.record_start = Instant::now();
        self.recording = true;

        match MicRecorder::start() {
            Ok(rec) => {
                let kind = rec.name().to_string();
                let dev = rec.device_name().to_string();
                let rate = rec.sample_rate();
                let ch = rec.channels();
                self.recorder = Some(Box::new(rec));
                self.set_status(format!(
                    "● 录音中（{kind}: {dev} @ {rate}Hz×{ch}ch）· 1-9 触发元素 · 空格下一个 · 拖动鼠标手写"
                ));
            }
            Err(e) => {
                self.recorder = None;
                self.set_status(format!("录音不可用：{e} → 继续无声演练（时间轴照常工作）"));
            }
        }
        self.ui.set_recording(true);
        self.ui.set_playing(false);
        self.refresh(true);
    }

    fn stop_recording(&mut self) {
        if !self.recording {
            return;
        }
        self.recording = false;
        self.ui.set_recording(false);
        self.drag = Drag::None;

        let elapsed = self.rec_time();
        let start_page_id = self
            .handle
            .project()
            .pages
            .get(self.live.start_page)
            .map(|p| p.id.clone())
            .unwrap_or_default();
        let rid = self.handle.project_mut().next_id("r");
        let rname = format!("演练 {}", self.handle.project().rehearsals.len() + 1);

        let mut rehearsal = Rehearsal::new(rid, rname, start_page_id);
        rehearsal.duration = elapsed;
        rehearsal.events = std::mem::take(&mut self.live.events);
        rehearsal.strokes = std::mem::take(&mut self.live.strokes);

        // 音频落盘
        let mut audio_error: Option<String> = None;
        if let Some(rec) = self.recorder.take() {
            let stream_error = rec.take_error();
            let dir = self.handle.rehearsal_dir(&rehearsal.id);
            match rec.finish(&dir) {
                Ok(fin) => {
                    rehearsal.duration = rehearsal.duration.max(fin.duration());
                    let mut aref =
                        crate::model::AudioRef::mic(fin.sample_rate, fin.channels, fin.frames);
                    // 用录制器真实写出的文件名，而不是硬编码
                    aref.file = fin.file_name.clone();
                    rehearsal.audio = Some(aref);
                }
                Err(e) => audio_error = Some(e),
            }
            if let Some(e) = stream_error {
                audio_error = Some(e);
            }
        }
        // 时间轴长度至少覆盖最后一个事件 / 笔迹点，
        // 这样即使录音被驱动截断，尾部内容也不会落在时间轴之外。
        {
            let p = self.handle.project();
            let input = TimelineInput {
                pages: &p.pages,
                strokes: &rehearsal.strokes,
                script: &p.script,
                events: &rehearsal.events,
                start_page_index: 0,
            };
            rehearsal.duration = rehearsal.duration.max(input.duration());
        }

        // 讲稿锚点：既写进本次演练，也回写到 project.json 的工作副本
        let marks = std::mem::take(&mut self.marked_sections);
        rehearsal.set_section_marks(&marks);
        {
            let project = self.handle.project_mut();
            for (sid, t) in &marks {
                if let Some(idx) = project.script.section_index(sid) {
                    project.script.sections[idx].reveal_time = Some(*t);
                }
            }
            project.current_page = self.live.start_page;
        }

        let id = rehearsal.id.clone();
        let summary = format!(
            "{:.1}s · {} 个事件 · {} 条笔迹 · {}",
            rehearsal.duration,
            rehearsal.events.len(),
            rehearsal.strokes.len(),
            if rehearsal.audio.is_some() {
                "含音频"
            } else {
                "无音频"
            }
        );
        match self.handle.save_rehearsal(&rehearsal) {
            Ok(()) => self.set_status(format!("已保存演练 {id}：{summary}")),
            Err(e) => self.set_status(format!("保存演练失败：{e}")),
        }
        if let Some(e) = audio_error {
            self.set_status(format!("音频保存失败：{e}（演练数据仍然保存）"));
        }
        let _ = self.handle.save();
        self.rehearsal_index = self.handle.meta().len().saturating_sub(1);

        // 录完直接进入回放，形成「录 → 看 → 改 → 再录」闭环
        self.mode = Mode::Review;
        self.ui.set_mode(Mode::Review.to_slint());
        if let Ok(r) = self.handle.load_rehearsal(&id) {
            self.load_rehearsal_into_player(r);
        }
        self.push_static_models();
        self.refresh(true);
    }

    fn load_rehearsal_into_player(&mut self, r: Rehearsal) {
        let audio = r.audio.clone();
        let duration = r.duration;
        self.ticks_model.set_vec(ticks_for(&r));
        self.ui.set_rehearsal_count(self.handle.meta().len() as i32);
        let title = format!("{} · {}", r.name, r.created_at_display());
        self.ui.set_rehearsal_name(SharedString::from(title.as_str()));
        self.ui.set_position(0.0);
        self.ui.set_playing(false);

        let mut effective = duration;
        match &audio {
            Some(a) => {
                let path = self.handle.rehearsal_dir(&r.id).join(&a.file);
                self.player.load(&path, duration);
                // 以播放器解析出的 WAV 真实时长为准，时间轴刻度才不会偏
                effective = self.player.duration().max(duration);
                self.ui.set_has_audio(self.player.has_audio());
                self.rebuild_waveform(&path);
            }
            None => {
                self.player.stop_and_clear();
                self.player.set_duration(duration);
                self.ui.set_has_audio(false);
                self.ui.set_wave_path(SharedString::default());
            }
        }
        self.ui
            .set_duration_label(SharedString::from(format_time(effective).as_str()));
        self.ui.set_duration(effective as f32);
        self.rehearsal = Some(r);
        self.stroke_sig = usize::MAX;
        self.script_sig = (-1, -1, -1);
    }

    fn rebuild_waveform(&mut self, path: &Path) {
        match crate::audio::read_wav_mono(path) {
            Ok((samples, _rate)) => {
                let mut peaks = waveform_peaks(&samples, WAVE_BINS);
                normalize_peaks(&mut peaks);
                self.ui
                    .set_wave_path(SharedString::from(build_wave_path(&peaks).as_str()));
            }
            Err(e) => {
                self.ui.set_wave_path(SharedString::default());
                self.set_status(format!("读取波形失败：{e}"));
            }
        }
    }

    // ==================== 触发（Rehearse / Present 的核心） ====================

    fn reveal_element(&mut self, id: String) {
        let t = self.rec_time();
        self.live.events.push(TimelineEvent::Reveal {
            element_id: id,
            t,
        });
        sort_events(&mut self.live.events);
        if self.recording {
            self.auto_mark_section(t);
        }
        self.refresh(true);
    }

    fn hide_element(&mut self, id: String) {
        let t = self.rec_time();
        self.live.events.push(TimelineEvent::Hide { element_id: id, t });
        sort_events(&mut self.live.events);
        self.refresh(true);
    }

    /// 快捷键 1-9：触发当前页面上第 N 个元素（按绘制顺序）。
    fn trigger_slot(&mut self, slot: usize) {
        let st = self.current_state();
        let id = {
            let project = self.handle.project();
            let Some(page) = project.pages.get(st.page_index) else {
                return;
            };
            let order = page.draw_order();
            let Some(&idx) = order.get(slot) else {
                self.set_status(format!("这一页没有第 {} 个元素", slot + 1));
                return;
            };
            page.elements[idx].id.clone()
        };
        if st.is_visible(&id) {
            self.set_status(format!("元素 {id} 已经出现过了"));
            return;
        }
        let t = self.rec_time();
        let tail = self.progress_note();
        if self.recording {
            self.set_status(format!("▶ {id} @ {}{tail}", format_time(t)));
        } else {
            self.set_status(format!("▶ {id}{tail}"));
        }
        self.reveal_element(id);
    }

    /// 空格：出现「下一个还没出现的元素」。
    ///
    /// 直接复用 timeline 的纯函数 [`next_hidden_element`]，
    /// 保证"下一个该出现的元素"在全应用只有一处定义。
    fn trigger_next(&mut self) {
        let st = self.current_state();
        let next = {
            let project = self.handle.project();
            project
                .pages
                .get(st.page_index)
                .and_then(|p| next_hidden_element(p, &st))
        };
        match next {
            Some(id) => {
                let t = self.rec_time();
                let tail = self.progress_note();
                if self.recording {
                    self.set_status(format!("▶ {id} @ {}{tail}", format_time(t)));
                } else {
                    self.set_status(format!("▶ {id}{tail}"));
                }
                self.reveal_element(id);
            }
            None => self.set_status("这一页的元素都已经出现了（← / → 翻页）".into()),
        }
    }

    /// 「本页已出现 x/y 个元素」的进度提示。
    fn progress_note(&self) -> String {
        let st = self.current_state();
        let project = self.handle.project();
        match project.pages.get(st.page_index) {
            Some(page) => format!("（本页 {}/{}）", st.visible_count(page), page.elements.len()),
            None => String::new(),
        }
    }

    /// 录制时：新触发的元素所在页面若正好有讲稿段落标注，且该段还没锚点，
    /// 就自动记成"从这一刻开始讲这一段"。这让讲稿时间轴**零操作**就能建立。
    fn auto_mark_section(&mut self, t: f64) {
        let page_id = self.current_state().page_id;
        let marked: Vec<String> = self.marked_sections.iter().map(|(id, _)| id.clone()).collect();
        let candidate = self
            .handle
            .project()
            .script
            .sections
            .iter()
            // 注意："属于本页" 与 "尚未打锚点" 必须写在同一个 find 条件里。
            // 先 find 再 filter 会停在第一个同页段落上，永远选不到后面的段落。
            .find(|s| {
                s.page_id.as_deref() == Some(page_id.as_str()) && !marked.contains(&s.id)
            })
            .map(|s| s.id.clone());
        if let Some(sid) = candidate {
            self.mark_section(&sid, t);
        }
    }

    fn mark_section(&mut self, section_id: &str, t: f64) {
        match self
            .marked_sections
            .iter_mut()
            .find(|(id, _)| id == section_id)
        {
            Some(slot) => slot.1 = t,
            None => self.marked_sections.push((section_id.to_string(), t)),
        }
        self.live.events.push(TimelineEvent::ScriptMark {
            section_id: section_id.to_string(),
            t,
        });
        sort_events(&mut self.live.events);
        self.refresh(true);
    }

    // ==================== 画布交互 ====================

    pub fn canvas_pointer(&mut self, kind: &str, x: f64, y: f64) {
        self.last_pointer = (x, y);
        match kind {
            "down" => self.canvas_down(x, y),
            "move" => self.canvas_move(x, y),
            "up" => self.canvas_up(x, y),
            _ => {}
        }
    }

    fn canvas_down(&mut self, x: f64, y: f64) {
        self.press = Some(PressInfo {
            x,
            y,
            travel: 0.0,
            at: Instant::now(),
        });
        match self.mode {
            Mode::Create => self.create_mode_down(x, y),
            Mode::Rehearse => {
                if self.recording {
                    // 按下即开始一条笔迹：单击得到点，拖动得到轨迹
                    let t = self.rec_time();
                    let page_id = self.current_state().page_id;
                    let id = format!("st{}", self.live.next_stroke_serial);
                    self.live.next_stroke_serial += 1;
                    let mut s = Stroke::new(id, page_id, "#e11d48", 3.0);
                    s.push(x, y, t);
                    self.live.strokes.push(s);
                    self.drag = Drag::Stroke {
                        index: self.live.strokes.len() - 1,
                    };
                    self.refresh(true);
                }
            }
            Mode::Review | Mode::Present => self.drag = Drag::None,
        }
    }

    fn create_mode_down(&mut self, x: f64, y: f64) {
        let tool = self.ui.get_tool();
        if tool == CanvasTool::Text {
            self.create_text_at(x, y);
            return;
        }
        if tool == CanvasTool::Image {
            self.insert_first_asset(x, y);
            return;
        }
        let st = self.current_state();
        let hit = {
            let project = self.handle.project();
            project
                .pages
                .get(st.page_index)
                .and_then(|p| hit_test_visible_element(p, &st, x, y))
        };
        match hit {
            Some(id) => {
                let (ox, oy, w, h) = self.element_rect(&id).unwrap_or((0.0, 0.0, 0.0, 0.0));
                let near_corner =
                    (x - (ox + w)).abs() <= RESIZE_GRAB && (y - (oy + h)).abs() <= RESIZE_GRAB;
                self.selected = Some(id.clone());
                self.drag = if near_corner {
                    Drag::Resize { id }
                } else {
                    Drag::Move {
                        id,
                        dx: x - ox,
                        dy: y - oy,
                    }
                };
                self.refresh(true);
            }
            None => {
                self.selected = None;
                self.drag = Drag::None;
                self.refresh(true);
            }
        }
    }

    fn canvas_move(&mut self, x: f64, y: f64) {
        if let Some(p) = &mut self.press {
            let d = ((x - p.x).powi(2) + (y - p.y).powi(2)).sqrt();
            p.travel = p.travel.max(d);
        }
        match self.drag.clone() {
            Drag::Move { id, dx, dy } => {
                if let Some(page) = self.handle.project_mut().current_page_mut() {
                    if let Some(el) = page.element_mut(&id) {
                        el.x = (x - dx).clamp(-500.0, 4000.0);
                        el.y = (y - dy).clamp(-500.0, 4000.0);
                    }
                }
                self.refresh(true);
            }
            Drag::Resize { id } => {
                let (ox, oy) = self
                    .element_rect(&id)
                    .map(|(a, b, _, _)| (a, b))
                    .unwrap_or((0.0, 0.0));
                if let Some(page) = self.handle.project_mut().current_page_mut() {
                    if let Some(el) = page.element_mut(&id) {
                        el.w = (x - ox).max(24.0);
                        el.h = (y - oy).max(18.0);
                    }
                }
                self.refresh(true);
            }
            Drag::Stroke { index } => {
                let t = self.rec_time();
                if let Some(s) = self.live.strokes.get_mut(index) {
                    // 过滤抖动点，避免 30fps 采样产生大量冗余坐标
                    let far_enough = s
                        .points
                        .last()
                        .map(|p| (p.x - x).hypot(p.y - y) > 1.5)
                        .unwrap_or(true);
                    if far_enough {
                        s.push(x, y, t);
                    }
                }
                self.refresh(true);
            }
            Drag::None => {}
        }
    }

    fn canvas_up(&mut self, x: f64, y: f64) {
        let travel = self.press.take().map(|p| p.travel).unwrap_or(0.0);
        let was_drag = std::mem::replace(&mut self.drag, Drag::None);

        match was_drag {
            Drag::Move { .. } | Drag::Resize { .. } => {
                // 排版改动落盘
                self.handle.project.touch();
                let _ = self.handle.save();
                self.refresh(true);
                return;
            }
            Drag::Stroke { .. } => {
                self.refresh(true);
                return;
            }
            Drag::None => {}
        }
        if travel > CLICK_SLOP {
            return; // 是拖动而不是点击
        }
        if self.mode == Mode::Review {
            self.click_to_seek(x, y);
        }
    }

    /// 点击画布 → 跳回当时讲话的时间点（SPEC §6 的核心差异化能力）。
    fn click_to_seek(&mut self, x: f64, y: f64) {
        if self.rehearsal.is_none() {
            return;
        }
        let t_now = self.player.position();
        let st = self.current_state();
        let (hit_el, hit_stroke) = {
            let project = self.handle.project();
            let Some(page) = project.pages.get(st.page_index) else {
                return;
            };
            let el = hit_test_visible_element(page, &st, x, y);
            let stroke = self
                .rehearsal
                .as_ref()
                .and_then(|r| hit_test_stroke(&r.strokes, &page.id, x, y, 14.0));
            (el, stroke)
        };

        // 笔迹优先于元素：笔迹画在元素之上，点击也更精确
        if let Some((idx, t)) = hit_stroke {
            self.player.seek(t);
            self.selected = None;
            self.set_status(format!("点击笔迹 #{} → 跳到 {}", idx + 1, format_time(t)));
            self.refresh(true);
            return;
        }
        if let Some(el_id) = hit_el {
            let events: &[TimelineEvent] = self
                .rehearsal
                .as_ref()
                .map(|r| r.events.as_slice())
                .unwrap_or(&[]);
            match reveal_time(events, &el_id, t_now) {
                Some(t) => {
                    self.player.seek(t);
                    self.selected = Some(el_id.clone());
                    self.set_status(format!("点击元素 {el_id} → 跳到 {}", format_time(t)));
                }
                None => {
                    self.selected = Some(el_id.clone());
                    self.set_status(format!("元素 {el_id} 在这次演练里没有 Reveal 事件"));
                }
            }
            self.refresh(true);
            return;
        }
        self.set_status("这里没有可点击的元素或笔迹".into());
    }

    fn element_rect(&self, id: &str) -> Option<(f64, f64, f64, f64)> {
        self.handle
            .project()
            .find_element(id)
            .map(|(_, e)| e.rect())
    }

    /// 双击：文字元素进入编辑；空白处则就地新建文字；回放模式下等价于单击跳转。
    pub fn canvas_double_click(&mut self) {
        let (x, y) = self.last_pointer;
        if self.mode == Mode::Review {
            self.click_to_seek(x, y);
            return;
        }
        if self.mode != Mode::Create {
            return;
        }
        let st = self.current_state();
        let hit = {
            let project = self.handle.project();
            project
                .pages
                .get(st.page_index)
                .and_then(|p| hit_test_visible_element(p, &st, x, y))
        };
        if let Some(id) = hit {
            let is_text = self
                .handle
                .project()
                .find_element(&id)
                .map(|(_, e)| matches!(e.kind, ElementKind::Text { .. }))
                .unwrap_or(false);
            if is_text {
                self.begin_edit(&id);
            } else {
                self.set_status("只有文字元素可以直接编辑内容".into());
            }
        } else {
            self.create_text_at(x, y);
        }
    }

    pub fn begin_edit(&mut self, id: &str) {
        let Some((_, el)) = self.handle.project().find_element(id) else {
            return;
        };
        let (x, y, w, h) = el.rect();
        let text = el.text_content().unwrap_or("").to_string();
        self.selected = Some(id.to_string());
        self.editing = Some(id.to_string());
        self.ui.set_editing_id(SharedString::from(id));
        self.ui.set_editing_text(SharedString::from(text.as_str()));
        self.ui.set_edit_x(x as f32);
        self.ui.set_edit_y(y as f32);
        self.ui.set_edit_w(w as f32);
        self.ui.set_edit_h(h as f32);
        self.set_status("编辑文字：改完按 Enter 或点「完成」".into());
        self.refresh(true);
    }

    pub fn commit_edit(&mut self, id: String, text: String) {
        let trimmed = text.trim().to_string();
        if let Some(page) = self.handle.project_mut().current_page_mut() {
            if let Some(el) = page.element_mut(&id) {
                el.set_text(if trimmed.is_empty() {
                    "（空文字）".to_string()
                } else {
                    trimmed
                });
            }
        }
        self.handle.project.touch();
        let _ = self.handle.save();
        self.end_edit();
        self.set_status("文字已更新".into());
        self.refresh(true);
    }

    pub fn end_edit(&mut self) {
        self.editing = None;
        self.ui.set_editing_id(SharedString::default());
        self.ui.set_editing_text(SharedString::default());
    }

    fn create_text_at(&mut self, x: f64, y: f64) {
        let (pw, _ph) = self.page_size();
        let id = self.handle.project_mut().next_id("el");
        let w = (pw * 0.45).min(pw);
        let bx = x.clamp(0.0, (pw - w).max(0.0));
        let mut el = Element::text(&id, bx, y.clamp(0.0, 5000.0), w, 60.0, "新文字");
        el.style.font_size = 32.0;
        el.hidden_by_default = true;
        el.z = self
            .handle
            .project()
            .current_page_ref()
            .map(|p| p.next_z())
            .unwrap_or(1);
        if let Some(page) = self.handle.project_mut().current_page_mut() {
            page.elements.push(el);
        }
        self.ui.set_tool(CanvasTool::Select);
        self.handle.project.touch();
        let _ = self.handle.save();
        self.selected = Some(id.clone());
        self.push_static_models();
        self.begin_edit(&id);
    }

    fn insert_first_asset(&mut self, x: f64, y: f64) {
        match self.handle.project().assets.first().cloned() {
            Some(a) => self.insert_asset(&a.name, x, y),
            None => {
                self.set_status("项目里还没有图片：先用「插入图片文件…」导入一张".into());
                self.import_image_dialog();
            }
        }
    }

    fn insert_asset(&mut self, name: &str, x: f64, y: f64) {
        let (pw, ph) = self.page_size();
        let aspect = self
            .handle
            .project()
            .asset(name)
            .map(|a| {
                if a.height > 0 {
                    a.width as f64 / a.height as f64
                } else {
                    1.5
                }
            })
            .unwrap_or(1.5);
        let w = (pw * 0.42).min(520.0);
        let h = (w / aspect.max(0.2)).min(ph * 0.7);
        let id = self.handle.project_mut().next_id("el");
        let mut el = Element::image(
            &id,
            (x - w / 2.0).clamp(0.0, (pw - w).max(0.0)),
            (y - h / 2.0).clamp(0.0, (ph - h).max(0.0)),
            w,
            h,
            name,
        );
        el.hidden_by_default = true;
        el.z = self
            .handle
            .project()
            .current_page_ref()
            .map(|p| p.next_z())
            .unwrap_or(1);
        if let Some(page) = self.handle.project_mut().current_page_mut() {
            page.elements.push(el);
        }
        self.selected = Some(id);
        self.handle.project.touch();
        let _ = self.handle.save();
        self.ui.set_tool(CanvasTool::Select);
        self.set_status(format!("已插入图片 {name}"));
        self.push_static_models();
        self.refresh(true);
    }

    pub fn import_image_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("选择要插入的图片")
            .add_filter("图片", &["png", "jpg", "jpeg", "bmp", "gif"])
            .pick_file();
        let Some(path) = picked else {
            return;
        };
        match self.handle.import_asset(&path) {
            Ok(asset) => {
                let (x, y) = self.last_pointer;
                self.images.remove(&asset.name);
                self.insert_asset(&asset.name.clone(), x, y);
            }
            Err(e) => self.set_status(format!("导入图片失败：{e}")),
        }
    }

    pub fn toggle_script_panel(&mut self) {
        let v = !self.ui.get_script_open();
        self.ui.set_script_open(v);
    }

    pub fn toggle_inspector(&mut self) {
        let v = !self.ui.get_inspector_open();
        self.ui.set_inspector_open(v);
        if v {
            let audio = if self.player.device_available() {
                format!(
                    "音频输出正常（rodio 位置 {:.1}s）",
                    self.player.device_position()
                )
            } else {
                self.player.note().unwrap_or("无音频输出设备").to_string()
            };
            self.set_status(audio);
        }
    }

    pub fn delete_selected(&mut self) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if let Some(page) = self.handle.project_mut().current_page_mut() {
            page.elements.retain(|e| e.id != id);
        }
        self.selected = None;
        self.handle.project.touch();
        let _ = self.handle.save();
        self.push_static_models();
        self.set_status(format!("已删除 {id}"));
        self.refresh(true);
    }

    // ==================== 工具栏 / 页面 / 讲稿 ====================

    pub fn toolbar(&mut self, cmd: &str) {
        match cmd {
            "tool-select" => {
                self.ui.set_tool(CanvasTool::Select);
                self.set_status("选择工具：拖动移动，拖右下角改尺寸".into());
            }
            "tool-text" => {
                self.ui.set_tool(CanvasTool::Text);
                self.set_status("文字工具：在画布上点一下即可新建文字".into());
            }
            "tool-image" => {
                self.ui.set_tool(CanvasTool::Image);
                self.set_status("图片工具：在画布上点一下插入项目里的图片".into());
            }
            "import-image" => self.import_image_dialog(),
            "add-page" => {
                let id = self.handle.project_mut().add_page();
                let n = self.handle.project().pages.len();
                self.handle.project.current_page = n - 1;
                self.handle.project.touch();
                let _ = self.handle.save();
                self.push_static_models();
                self.set_status(format!("已添加页面 {id}"));
                self.refresh(true);
            }
            "del-page" => {
                let idx = self.handle.project().current_page;
                if self.handle.project_mut().remove_page(idx) {
                    let n = self.handle.project().pages.len();
                    self.handle.project.current_page = n - 1;
                    self.handle.project.touch();
                    let _ = self.handle.save();
                    self.push_static_models();
                    self.refresh(true);
                    self.set_status("已删除页面".into());
                } else {
                    self.set_status("至少要保留一页".into());
                }
            }
            other => self.set_status(format!("未知命令：{other}")),
        }
    }

    pub fn select_page(&mut self, index: i32) {
        let n = self.handle.project().pages.len() as i32;
        if index < 0 || index >= n {
            return;
        }
        self.handle.project.current_page = index as usize;
        self.selected = None;
        self.end_edit();
        if self.mode == Mode::Rehearse || self.mode == Mode::Present {
            let page_id = self.handle.project().current_page_id();
            let t = self.rec_time();
            self.live.events.push(TimelineEvent::PageChange { page_id, t });
            sort_events(&mut self.live.events);
        }
        let name = self
            .handle
            .project()
            .pages
            .get(index as usize)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let _ = self.handle.save();
        self.push_static_models();
        self.set_status(format!("当前页面：{name}"));
        self.refresh(true);
    }

    pub fn next_page(&mut self, delta: i32) {
        let n = self.handle.project().pages.len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.handle.project().current_page as i32;
        let next = (cur + delta).clamp(0, n - 1);
        if next == cur {
            self.set_status(if delta > 0 {
                "已经是最后一页".into()
            } else {
                "已经是第一页".into()
            });
        } else {
            self.select_page(next);
        }
    }

    pub fn script_row_click(&mut self, index: i32) {
        let idx = index as usize;
        let (section_id, page_id) = {
            let project = self.handle.project();
            let Some(s) = project.script.sections.get(idx) else {
                return;
            };
            (s.id.clone(), s.page_id.clone())
        };

        // 录制中：点击讲稿 = 给这一段打时间锚点
        if self.mode == Mode::Rehearse && self.recording {
            let t = self.rec_time();
            self.mark_section(&section_id, t);
            self.set_status(format!("已标记 {section_id} @ {}", format_time(t)));
            return;
        }

        // 演练 / 演讲中：点击讲稿 = 跳到对应页面
        if self.mode == Mode::Rehearse || self.mode == Mode::Present {
            if let Some(pid) = page_id {
                if let Some(i) = self.handle.project().page_index(&pid) {
                    self.select_page(i as i32);
                    self.set_status(format!("跳到讲稿 {section_id} 对应的页面"));
                }
            } else {
                self.set_status(format!("{section_id} 没有标注页面，先看页面栏的快捷键提示"));
            }
            return;
        }

        // 回放：seek 到这句话真正被讲到的时间（SPEC §20 的反向映射）
        match self.section_times().get(idx).copied().flatten() {
            Some(t) => {
                self.player.seek(t);
                self.ui.set_position(t as f32);
                self.set_status(format!("讲稿 {section_id} → 跳到 {}", format_time(t)));
                self.refresh(true);
            }
            None => {
                self.set_status(format!("{section_id} 还没有时间锚点（录制时点一下它即可建立）"));
                if let Some(pid) = page_id {
                    if let Some(i) = self.handle.project().page_index(&pid) {
                        self.select_page(i as i32);
                    }
                }
            }
        }
    }

    // ==================== 播放 ====================

    pub fn play_toggle(&mut self) {
        if self.mode != Mode::Review {
            self.set_mode(Mode::Review);
        }
        if self.rehearsal.is_none() {
            self.set_status("还没有演练记录可以播放".into());
            return;
        }
        self.player.toggle();
        let playing = self.player.is_playing();
        self.ui.set_playing(playing);
        self.set_status(if playing {
            "▶ 播放中：画面、笔迹、讲稿会随时间轴同步".into()
        } else {
            "⏸ 已暂停".into()
        });
        self.refresh(true);
    }

    pub fn seek(&mut self, t: f64) {
        if self.mode != Mode::Review {
            return;
        }
        self.player.seek_throttled(t);
        let pos = self.player.position();
        self.ui.set_position(pos as f32);
        let label = format_time(pos);
        self.last_position_label = label.clone();
        self.ui.set_position_label(SharedString::from(label.as_str()));
        self.refresh(false);
    }

    pub fn stop_playback(&mut self) {
        self.player.pause();
        self.player.seek(0.0);
        self.ui.set_playing(false);
        self.ui.set_position(0.0);
        self.ui.set_position_label(SharedString::from("00:00.0"));
        self.refresh(true);
    }

    pub fn select_rehearsal(&mut self, index: i32) {
        let list = self.handle.meta().to_vec();
        let n = list.len() as i32;
        if n == 0 {
            return;
        }
        let i = index.clamp(0, n - 1);
        if i as usize == self.rehearsal_index && self.rehearsal.is_some() {
            return;
        }
        let id = list[i as usize].id.clone();
        match self.handle.load_rehearsal(&id) {
            Ok(r) => {
                self.rehearsal_index = i as usize;
                self.load_rehearsal_into_player(r);
                self.set_status(format!("已载入演练 {id}"));
                self.refresh(true);
            }
            Err(e) => self.set_status(format!("载入演练失败：{e}")),
        }
    }

    // ==================== 项目 ====================

    fn reset_for_new_project(&mut self) {
        self.rehearsal = None;
        self.rehearsal_index = 0;
        self.images.clear();
        self.ticks_model.set_vec(Vec::new());
        self.ui.set_rehearsal_count(0);
        self.ui.set_rehearsal_name(SharedString::default());
        self.waveform_reset();
        self.stroke_sig = usize::MAX;
        self.script_sig = (-1, -1, -1);
        self.elem_sig = u64::MAX;
        self.selected = None;
    }

    pub fn new_project(&mut self) {
        let name = format!("未命名项目 {}", self.store.list_projects().len() + 1);
        match self.store.create_project(&name) {
            Ok(h) => {
                self.handle = h;
                self.reset_for_new_project();
                self.push_static_models();
                self.set_mode(Mode::Create);
                self.set_status(format!("已新建项目「{name}」，开始添加页面与讲稿吧"));
            }
            Err(e) => self.set_status(format!("新建项目失败：{e}")),
        }
    }

    pub fn open_project_dialog(&mut self) {
        let root = self.store.projects_dir();
        let picked = rfd::FileDialog::new()
            .set_title("选择项目里的 project.json")
            .set_directory(&root)
            .add_filter("slidetrace 项目", &["json"])
            .pick_file();
        let Some(path) = picked else {
            return;
        };
        let slug = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string());
        match slug {
            Some(s) => self.open_project(&s),
            None => self.set_status("无法识别项目目录".into()),
        }
    }

    pub fn open_project(&mut self, slug: &str) {
        match self.store.open_project(slug) {
            Ok(h) => {
                self.handle = h;
                self.reset_for_new_project();
                if let Some(r) = self.handle.load_latest_rehearsal() {
                    self.rehearsal_index = self.handle.meta().len().saturating_sub(1);
                    self.load_rehearsal_into_player(r);
                }
                let name = self.handle.project().name.clone();
                self.push_static_models();
                self.refresh(true);
                self.set_status(format!("已打开项目「{name}」"));
            }
            Err(e) => self.set_status(format!("打开项目失败：{e}")),
        }
    }

    pub fn select_project(&mut self, index: i32) {
        let list = self.store.list_projects();
        if let Some((slug, _)) = list.get(index.max(0) as usize) {
            let slug = slug.clone();
            if slug != self.handle.project().id {
                self.open_project(&slug);
            }
        }
    }

    // ==================== 快捷键 ====================

    pub fn key_action(&mut self, text: &str, ctrl: bool, shift: bool) {
        if self.editing.is_some() {
            return;
        }
        if ctrl {
            match text.to_ascii_lowercase().as_str() {
                "r" => self.toggle_recording(),
                "p" => self.play_toggle(),
                "n" => self.new_project(),
                "o" => self.open_project_dialog(),
                "i" => {
                    let v = !self.ui.get_inspector_open();
                    self.ui.set_inspector_open(v);
                }
                "s" => {
                    let _ = self.handle.save();
                    self.set_status("已保存到 project.json".into());
                }
                _ => {}
            }
            return;
        }
        match text {
            // Esc：停止录制 / 退出演讲
            "\u{1b}" => {
                if self.recording {
                    self.stop_recording();
                } else {
                    self.set_mode(Mode::Create);
                }
                return;
            }
            // 空格：出现下一个元素；非演练模式则播放/暂停
            "\u{20}" => {
                if self.mode == Mode::Rehearse || self.mode == Mode::Present {
                    self.trigger_next();
                } else {
                    self.play_toggle();
                }
                return;
            }
            // → / ↓ 下一页，← / ↑ 上一页
            "\u{F703}" | "\u{F701}" => {
                self.next_page(1);
                return;
            }
            "\u{F702}" | "\u{F700}" => {
                self.next_page(-1);
                return;
            }
            _ => {}
        }
        if !shift {
            if let Some(d) = text.chars().next().and_then(|c| c.to_digit(10)) {
                if (1..=9).contains(&d) {
                    self.trigger_slot(d as usize - 1);
                    return;
                }
            }
        }
        match text.to_ascii_lowercase().as_str() {
            "m" if self.mode == Mode::Rehearse && self.recording => {
                let t = self.rec_time();
                let label = format!("标记 {}", self.marker_count() + 1);
                self.live.events.push(TimelineEvent::Marker { label, t });
                sort_events(&mut self.live.events);
                self.set_status(format!("已打标记 @ {}", format_time(t)));
                self.refresh(true);
            }
            "h" if self.mode == Mode::Rehearse || self.mode == Mode::Present => {
                let last = {
                    let st = self.current_state();
                    let project = self.handle.project();
                    project.pages.get(st.page_index).and_then(|p| {
                        p.draw_order()
                            .into_iter()
                            .rev()
                            .filter_map(|i| p.elements.get(i))
                            .find(|e| st.is_visible(&e.id))
                            .map(|e| e.id.clone())
                    })
                };
                if let Some(id) = last {
                    self.hide_element(id);
                }
            }
            _ => {}
        }
    }

    fn marker_count(&self) -> usize {
        self.live
            .events
            .iter()
            .filter(|e| matches!(e, TimelineEvent::Marker { .. }))
            .count()
    }

    // ==================== 每帧刷新 ====================

    pub fn tick(&mut self) {
        let record_label = if self.recording {
            format!("■ 停止录制   {}", format_time(self.rec_time()))
        } else if self.mode == Mode::Present {
            "● 开始录制".to_string()
        } else {
            "● 开始录制（Ctrl+R）".to_string()
        };
        if self.ui.get_record_label().as_str() != record_label {
            self.ui.set_record_label(SharedString::from(record_label.as_str()));
        }

        if self.mode == Mode::Review && self.player.is_playing() {
            let pos = self.player.position();
            self.ui.set_position(pos as f32);
            let label = format_time(pos);
            if label != self.last_position_label {
                self.last_position_label = label.clone();
                self.ui.set_position_label(SharedString::from(label.as_str()));
            }
            self.refresh(false);
        }
        self.player.tick();
        if self.mode == Mode::Review && !self.player.is_playing() && self.ui.get_playing() {
            self.ui.set_playing(false);
            self.refresh(false);
        }
    }

    /// 把模型句柄挂到 UI 上（必须在 `AppWindow::new()` 之后调用）。
    pub fn bind_models(&self) {
        self.ui.set_elements(ModelRc::from(self.elements_model.clone()));
        self.ui.set_strokes(ModelRc::from(self.strokes_model.clone()));
        self.ui.set_ticks(ModelRc::from(self.ticks_model.clone()));
        self.ui.set_script_rows(ModelRc::from(self.script_model.clone()));
        self.ui.set_pages(ModelRc::from(self.pages_model.clone()));
        self.ui.set_project_names(ModelRc::from(self.projects_model.clone()));
    }

    /// 组件句柄（自检与截图流程需要访问它）。
    pub fn ui(&self) -> &AppWindow {
        &self.ui
    }

    /// 当前推送到 UI 的 (元素行数, 笔迹行数)，用于验证渲染模型确实更新了。
    pub fn model_row_count(&self) -> (usize, usize) {
        (
            self.elements_model.row_count(),
            self.strokes_model.row_count(),
        )
    }

    // ==================== 供 --selftest 使用的只读访问器 ====================
    // （UI 自己不需要这些访问器，但端到端自检需要从外部观察状态机的内部状态）

    pub fn project(&self) -> &Project {
        self.handle.project()
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    pub fn rehearsal(&self) -> Option<&Rehearsal> {
        self.rehearsal.as_ref()
    }

    /// 从磁盘按 id 读回一次演练，用于验证真的落盘了。
    pub fn load_rehearsal_by_id(&self, id: &str) -> Option<Rehearsal> {
        self.handle.load_rehearsal(id).ok()
    }

    pub fn player_position(&self) -> f64 {
        self.player.position()
    }

    pub fn player_is_playing(&self) -> bool {
        self.player.is_playing()
    }

    /// 音频能力的可读描述（没有麦克风时也要如实说明）。
    pub fn audio_note(&self) -> String {
        let out = if self.player.device_available() {
            "输出正常"
        } else {
            self.player.note().unwrap_or("无输出设备")
        };
        match &self.recorder {
            Some(r) => format!("录音中（{}）· 输出：{out}", r.name()),
            None => format!("未录音 · 输出：{out}"),
        }
    }

    pub fn handle_save(&mut self) {
        let _ = self.handle.save();
    }

    pub fn script_level(&self) -> i32 {
        self.ui.get_script_level()
    }

    /// 供冒烟测试/截图使用的自检信息。
    pub fn snapshot_info(&self) -> String {
        format!(
            "mode={:?} elements={} strokes={} script={} pages={} status={}",
            self.mode,
            self.elements_model.row_count(),
            self.strokes_model.row_count(),
            self.script_model.row_count(),
            self.pages_model.row_count(),
            self.status
        )
    }
}

// ======================= 纯函数工具 =======================

/// 从项目 + 演练算出某一时刻的完整状态。
pub fn compute_state(project: &Project, rehearsal: Option<&Rehearsal>, t: f64) -> PresentationState {
    TimelineInput::new(project, rehearsal).state_at(t)
}

/// 时间轴事件刻度。
fn ticks_for(r: &Rehearsal) -> Vec<EventTick> {
    let d = r.duration.max(1e-6);
    r.events
        .iter()
        .map(|e| EventTick {
            x: (e.time() / d) as f32,
            kind: SharedString::from(e.kind_str()),
        })
        .collect()
}

/// 生成波形填充包络的 SVG path（viewbox 固定为 1000 × 100）。
///
/// 用「填充多边形」而不是「描边折线」是刻意的：Path 的 viewbox 是各向异性缩放，
/// 描边的线宽会被拉伸得很难看，而填充不受影响。
fn build_wave_path(peaks: &[f32]) -> String {
    let n = peaks.len();
    if n == 0 {
        return String::new();
    }
    let x_at = |i: usize| -> f64 {
        if n <= 1 {
            0.0
        } else {
            i as f64 / (n - 1) as f64 * 1000.0
        }
    };
    let top = |p: f32| 50.0 - (p.clamp(0.0, 1.0) as f64) * 46.0;
    let bottom = |p: f32| 50.0 + (p.clamp(0.0, 1.0) as f64) * 46.0;
    let mut d = String::with_capacity(n * 26 + 32);
    for (i, p) in peaks.iter().enumerate() {
        let cmd = if i == 0 { "M" } else { "L" };
        d.push_str(&format!("{cmd} {:.2} {:.2} ", x_at(i), top(*p)));
    }
    for (i, p) in peaks.iter().enumerate().rev() {
        d.push_str(&format!("L {:.2} {:.2} ", x_at(i), bottom(*p)));
    }
    d.push('Z');
    d
}

/// `#RRGGBB` → `slint::Color`。
fn to_color(s: &str, fallback: slint::Color) -> slint::Color {
    match parse_hex_color(s) {
        Some((r, g, b, a)) => slint::Color::from_argb_u8(a, r, g, b),
        None => fallback,
    }
}

/// 用 `image` crate 解码图片并转成 Slint 纹理。
///
/// 刻意不走 Slint 自己的 `Image::load_from_path`：这样解码特性完全由我们自己的
/// 依赖控制，也让"图片加载失败"变成一个可处理的 `None` 而不是运行期报错。
fn load_image(path: &Path) -> Option<slint::Image> {
    let img = image::open(path).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    // 超大图先缩放，避免一次性把几十 MB 传给渲染后端
    let (w, h, data) = if w > 2048 {
        let nh = ((h as f64 * 2048.0 / w as f64).round() as u32).max(1);
        let small =
            image::imageops::resize(&rgba, 2048, nh, image::imageops::FilterType::Triangle);
        (small.width(), small.height(), small.into_raw())
    } else {
        (w, h, rgba.into_raw())
    };
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&data, w, h);
    Some(slint::Image::from_rgba8(buffer))
}

// ======================= 回调接线 =======================

/// 把 Slint 的回调接到状态机上。用 `Weak` 避免 UI 反过来持有状态造成引用循环。
pub fn wire(ui: &AppWindow, state: &Rc<RefCell<AppState>>) {
    macro_rules! simple {
        ($setter:ident, |$s:ident| $body:expr) => {{
            let weak = Rc::downgrade(state);
            ui.$setter(move || {
                if let Some(rc) = weak.upgrade() {
                    if let Ok(mut $s) = rc.try_borrow_mut() {
                        $body;
                    }
                }
            });
        }};
    }
    macro_rules! with_args {
        ($setter:ident, |$s:ident, $($arg:ident : $ty:ty),+| $body:expr) => {{
            let weak = Rc::downgrade(state);
            ui.$setter(move |$($arg : $ty),+| {
                if let Some(rc) = weak.upgrade() {
                    if let Ok(mut $s) = rc.try_borrow_mut() {
                        $body;
                    }
                }
            });
        }};
    }

    with_args!(on_mode_request, |s, i: i32| s.set_mode(Mode::from_index(i)));
    simple!(on_record_toggle, |s| s.toggle_recording());
    simple!(on_play_toggle, |s| s.play_toggle());
    simple!(on_stop_playback, |s| s.stop_playback());
    simple!(on_canvas_double_click, |s| s.canvas_double_click());
    simple!(on_edit_cancel, |s| {
        s.end_edit();
        s.set_status_public("已取消编辑".into());
    });
    simple!(on_script_toggle, |s| s.toggle_script_panel());
    simple!(on_project_new, |s| s.new_project());
    simple!(on_project_open, |s| s.open_project_dialog());
    simple!(on_inspector_toggle, |s| s.toggle_inspector());
    simple!(on_delete_selected, |s| s.delete_selected());
    simple!(on_rehearsal_prev, |s| {
        let i = s.rehearsal_index as i32 - 1;
        s.select_rehearsal(i);
    });
    simple!(on_rehearsal_next, |s| {
        let i = s.rehearsal_index as i32 + 1;
        s.select_rehearsal(i);
    });

    with_args!(on_seek, |s, t: f32| s.seek(t as f64));
    with_args!(on_page_select, |s, i: i32| s.select_page(i));
    with_args!(on_script_row_click, |s, i: i32| s.script_row_click(i));
    with_args!(on_script_level_set, |s, l: i32| {
        s.ui.set_script_level(l);
        s.refresh_script_public();
    });
    with_args!(on_project_select, |s, i: i32| s.select_project(i));
    with_args!(on_toolbar, |s, cmd: SharedString| s.toolbar(cmd.as_str()));
    with_args!(on_canvas_pointer, |s, kind: SharedString, x: f32, y: f32| {
        s.canvas_pointer(kind.as_str(), x as f64, y as f64)
    });
    with_args!(on_edit_commit, |s, id: SharedString, text: SharedString| {
        s.commit_edit(id.to_string(), text.to_string())
    });
    with_args!(on_key_action, |s, text: SharedString, ctrl: bool, shift: bool| {
        s.key_action(text.as_str(), ctrl, shift)
    });
}

/// 宏在同模块内展开，私有方法可以直接调用；这两个小包装只是为了让
/// `simple!` / `with_args!` 里的表达式读起来更清楚。
impl AppState {
    pub fn set_status_public(&mut self, s: String) {
        self.set_status(s);
    }
    pub fn refresh_script_public(&mut self) {
        let st = self.current_state();
        self.refresh_script(&st, true);
    }
}

// ======================= 启动 =======================

fn make_state(ui: &AppWindow, store: Store, handle: ProjectHandle) -> Rc<RefCell<AppState>> {
    let state = Rc::new(RefCell::new(AppState::new(
        ui.clone_strong(),
        store,
        handle,
    )));
    state.borrow().bind_models();
    wire(ui, &state);
    state
}

/// 每帧驱动。timer 由调用方持有，`run()` 阻塞期间一直存活。
fn start_frame_timer(state: &Rc<RefCell<AppState>>) -> slint::Timer {
    let weak = Rc::downgrade(state);
    let t = slint::Timer::default();
    t.start(slint::TimerMode::Repeated, FRAME, move || {
        if let Some(rc) = weak.upgrade() {
            if let Ok(mut s) = rc.try_borrow_mut() {
                s.tick();
            }
        }
    });
    t
}

/// 启动应用（阻塞直到窗口关闭）。
pub fn run_app(store: Store, handle: ProjectHandle) -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let state = make_state(&ui, store, handle);
    let _timer = start_frame_timer(&state);
    ui.run()
}

/// 存 PNG；`crop` 为 `(x, y, w, h)` 时只导出感兴趣的区域，
/// 方便自动化冒烟测试核对某个面板的具体像素。
fn save_shot(
    img: &image::RgbaImage,
    out: &Path,
    crop: Option<(u32, u32, u32, u32)>,
) -> image::ImageResult<()> {
    match crop {
        Some((x, y, w, h)) => {
            let x = x.min(img.width().saturating_sub(1));
            let y = y.min(img.height().saturating_sub(1));
            let w = w.min(img.width() - x).max(1);
            let h = h.min(img.height() - y).max(1);
            image::imageops::crop_imm(img, x, y, w, h).to_image().save(out)
        }
        None => img.save(out),
    }
}

/// `--screenshot <path>` 用的单帧路径：显示 → 等若干帧 → 截图 → 退出。
///
/// 这是给自动化冒烟测试用的开发参数，不参与正常交互流程。
pub fn run_screenshot(
    store: Store,
    handle: ProjectHandle,
    mode: Mode,
    out: PathBuf,
    delay: Duration,
    crop: Option<(u32, u32, u32, u32)>,
) -> Result<(), String> {
    let ui = AppWindow::new().map_err(|e| e.to_string())?;
    let state = make_state(&ui, store, handle);
    let _timer = start_frame_timer(&state);

    state.borrow_mut().set_mode(mode);
    if mode == Mode::Review {
        let dur = state.borrow().rehearsal.as_ref().map(|r| r.duration);
        if let Some(d) = dur {
            state.borrow_mut().seek(d * 0.5);
        }
    }
    println!("[截图] {}", state.borrow().snapshot_info());
    ui.show().map_err(|e| e.to_string())?;

    let ok = Rc::new(std::cell::Cell::new(false));
    let _shot = {
        let weak = ui.as_weak();
        let ok = ok.clone();
        let t = slint::Timer::default();
        t.start(slint::TimerMode::SingleShot, delay, move || {
            if let Some(ui) = weak.upgrade() {
                match ui.window().take_snapshot() {
                    Ok(buf) => {
                        match image::RgbaImage::from_raw(
                            buf.width(),
                            buf.height(),
                            buf.as_bytes().to_vec(),
                        ) {
                            Some(img) => match save_shot(&img, &out, crop) {
                                Ok(()) => {
                                    println!(
                                        "[截图] 已写入 {} ({}×{})",
                                        out.display(),
                                        buf.width(),
                                        buf.height()
                                    );
                                    ok.set(true);
                                }
                                Err(e) => eprintln!("[截图] 保存失败：{e}"),
                            },
                            None => eprintln!("[截图] 像素缓冲异常"),
                        }
                    }
                    Err(e) => eprintln!("[截图] take_snapshot 失败：{e}"),
                }
            }
            slint::quit_event_loop().ok();
        });
        t
    };

    slint::run_event_loop().map_err(|e| e.to_string())?;
    if ok.get() {
        Ok(())
    } else {
        Err("截图流程未成功（见上面的错误输出）".into())
    }
}
