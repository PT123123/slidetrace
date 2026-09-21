//! `--selftest`：用真实的应用状态机跑一遍 V1 核心闭环，并逐条断言。
//!
//! 这不是单元测试的替代品（`cargo test` 覆盖的是纯函数与存储层），而是
//! **端到端冒烟**：它真的创建 Slint 组件、真的把数据推进 Slint 模型、
//! 真的写 project.json 与 rehearsal.json、真的去 seek 音频时钟，
//! 因此能验证「UI ↔ 状态机 ↔ 存储 ↔ 时间轴」之间的接线没有断。
//!
//! 覆盖的链路（对应 SPEC §25）：
//! ```text
//! 进入演练 → 开始录制 → 快捷键 Reveal（写入时间戳）→ 鼠标画笔迹（逐点带时间）
//! → 空格触发下一个 → 翻页 → 停止录制（落盘为 Rehearsal）
//! → 回放：seek / 点击元素跳回讲话时间 / 点击笔迹跳回写字时间 / 点击讲稿跳转
//! → 演讲模式：不录音的触发
//! ```
//!
//! 借用约定：`AppState` 装在 `RefCell` 里，所以下面每一段都用**短生命周期的
//! 独立 borrow**，绝不在持有一个 borrow 的同时再借一次。

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use crate::app::{AppMode, AppState, AppWindow, Mode};
use crate::storage::{ProjectHandle, Store};
use crate::timeline::{nearest_stroke_time, reveal_time, TimelineEvent};

struct Check {
    passed: usize,
    failed: usize,
}

impl Check {
    fn new() -> Self {
        Self {
            passed: 0,
            failed: 0,
        }
    }

    fn ok(&mut self, cond: bool, label: &str, detail: impl std::fmt::Display) {
        if cond {
            self.passed += 1;
        } else {
            self.failed += 1;
        }
        println!("  [{}] {label}  ({detail})", if cond { "PASS" } else { "FAIL" });
    }

    fn near(&mut self, got: f64, want: f64, tol: f64, label: &str) {
        let good = (got - want).abs() <= tol;
        self.ok(good, label, format!("got={got:.3} want={want:.3}"));
    }
}

/// 某个元素中心的页面坐标。
fn element_center(s: &AppState, id: &str) -> (f64, f64) {
    s.project()
        .find_element(id)
        .map(|(_, e)| (e.x + e.w / 2.0, e.y + e.h / 2.0))
        .unwrap_or((0.0, 0.0))
}

/// 在画布上"画"一条笔迹：按下 → 若干次移动 → 抬起。
fn draw_stroke(state: &Rc<RefCell<AppState>>, pts: &[(f64, f64)]) {
    let mut s = state.borrow_mut();
    s.canvas_pointer("down", pts[0].0, pts[0].1);
    drop(s);
    for p in &pts[1..] {
        state.borrow_mut().canvas_pointer("move", p.0, p.1);
        std::thread::sleep(std::time::Duration::from_millis(14));
    }
    let last = *pts.last().unwrap();
    state.borrow_mut().canvas_pointer("up", last.0, last.1);
}

/// 模拟一次"点击"（按下与抬起在同一点 → 无位移 → 判定为点击）。
fn click(state: &Rc<RefCell<AppState>>, x: f64, y: f64) {
    let mut s = state.borrow_mut();
    s.canvas_pointer("down", x, y);
    s.canvas_pointer("up", x, y);
}

pub fn run(store: Store, handle: ProjectHandle) -> Result<(), String> {
    let ui = AppWindow::new().map_err(|e| e.to_string())?;
    let state = Rc::new(RefCell::new(AppState::new(
        ui.clone_strong(),
        store.clone(),
        handle,
    )));
    state.borrow().bind_models();
    crate::app::wire(&ui, &state);

    let mut c = Check::new();

    println!("== 1. 演示项目 ==");
    {
        let s = state.borrow();
        let p = s.project();
        c.ok(p.pages.len() == 3, "3 页", p.pages.len());
        c.ok(p.script.sections.len() == 5, "5 段讲稿", p.script.sections.len());
        c.ok(p.rehearsals.len() == 1, "自带 1 次演练", p.rehearsals.len());
        c.ok(
            p.pages.iter().all(|pg| (2..=3).contains(&pg.elements.len())),
            "每页 2-3 个元素",
            p.pages
                .iter()
                .map(|pg| pg.elements.len())
                .collect::<Vec<_>>()
                .len(),
        );
        c.ok(p.assets.len() == 2, "2 张占位图片", p.assets.len());
    }

    println!("== 2. 制作模式：新建文字 / 编辑 / 删除 ==");
    let created_id;
    {
        let mut s = state.borrow_mut();
        s.set_mode(Mode::Create);
        let (rows, _) = s.model_row_count();
        c.ok(rows == 3, "当前页 3 个元素全部可见", rows);
        s.toolbar("tool-text");
        s.canvas_pointer("down", 200.0, 600.0);
        created_id = s.ui().get_editing_id().to_string();
        c.ok(!created_id.is_empty(), "文字工具新建并进入编辑", &created_id);
        s.commit_edit(created_id.clone(), "自检写入的文字".into());
        let written = s
            .project()
            .find_element(&created_id)
            .and_then(|(_, e)| e.text_content().map(|t| t.to_string()))
            .unwrap_or_default();
        c.ok(written == "自检写入的文字", "文字写入模型", written);
        s.delete_selected();
        c.ok(
            s.project().find_element(&created_id).is_none(),
            "删除选中元素",
            "removed",
        );
    }

    println!("== 3. 演练：录制 + 快捷键触发 + 手写 ==");
    {
        let mut s = state.borrow_mut();
        s.set_mode(Mode::Rehearse);
        c.ok(
            s.ui().get_mode() == AppMode::Rehearse,
            "模式切到演练",
            "rehearse",
        );
        let (rows, strokes) = s.model_row_count();
        c.ok(rows == 3, "未出现的元素显示为带数字键的提示框", rows);
        c.ok(strokes == 0, "录制前没有笔迹", strokes);
        s.toggle_recording();
        c.ok(s.is_recording(), "进入录制状态", s.is_recording());
        println!("  [info] 音频：{}", s.audio_note());
    }

    // 1) 数字键 2 → 触发第 2 个元素
    {
        let mut s = state.borrow_mut();
        s.key_action("2", false, false);
    }
    std::thread::sleep(std::time::Duration::from_millis(40));
    {
        let s = state.borrow();
        let (rows, _) = s.model_row_count();
        c.ok(rows == 3, "触发后元素数不变（提示框变成真内容）", rows);
    }

    // 2) 手写一条折线（逐点带时间戳）
    draw_stroke(
        &state,
        &[
            (100.0, 500.0),
            (160.0, 512.0),
            (220.0, 545.0),
            (280.0, 562.0),
            (340.0, 585.0),
            (400.0, 600.0),
        ],
    );
    {
        let s = state.borrow();
        let (_, strokes) = s.model_row_count();
        c.ok(strokes == 1, "实时笔迹已画到画布", strokes);
    }

    // 3) 空格 → 触发下一个隐藏元素
    {
        let mut s = state.borrow_mut();
        s.key_action("\u{20}", false, false);
    }
    std::thread::sleep(std::time::Duration::from_millis(30));

    // 4) 翻到第 2 页，触发一个元素，再画一条笔迹
    {
        let mut s = state.borrow_mut();
        s.key_action("\u{F703}", false, false); // →
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    {
        let mut s = state.borrow_mut();
        c.ok(s.project().current_page == 1, "→ 翻到第 2 页", s.project().current_page);
        s.key_action("2", false, false);
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    draw_stroke(&state, &[(300.0, 300.0), (340.0, 330.0), (380.0, 360.0), (420.0, 400.0)]);
    {
        let mut s = state.borrow_mut();
        s.key_action("\u{F702}", false, false); // ←
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    {
        let s = state.borrow();
        let (rows, strokes) = s.model_row_count();
        c.ok(s.project().current_page == 0, "← 回到第 1 页", s.project().current_page);
        c.ok(rows == 3, "第 1 页元素数正确", rows);
        c.ok(strokes == 1, "第 1 页的笔迹只显示属于本页的", strokes);
    }

    println!("== 4. 停止录制 → 落盘为 Rehearsal ==");
    let rehearsal_id;
    {
        let mut s = state.borrow_mut();
        s.toggle_recording();
        c.ok(!s.is_recording(), "已停止录制", "-");
        c.ok(
            s.ui().get_mode() == AppMode::Review,
            "停止后自动进入回放",
            "review",
        );
        let r = s
            .rehearsal()
            .cloned()
            .ok_or_else(|| "停止录制后没有加载到演练".to_string())?;
        rehearsal_id = r.id.clone();
        c.ok(r.events.len() >= 4, "事件数 >= 4", r.events.len());
        c.ok(r.strokes.len() == 2, "笔迹数 == 2", r.strokes.len());
        c.ok(
            r.strokes.iter().all(|st| st.points.len() >= 4),
            "每条笔迹都有多个带时间的点",
            r.strokes
                .iter()
                .map(|st| st.points.len())
                .collect::<Vec<_>>()
                .len(),
        );
        c.ok(
            r.strokes
                .iter()
                .all(|st| st.points.windows(2).all(|w| w[0].t <= w[1].t)),
            "笔迹点时间单调递增",
            "-",
        );
        c.ok(
            r.strokes.iter().any(|st| st.page_id == "p1") && r.strokes.iter().any(|st| st.page_id == "p2"),
            "笔迹记录了所属页面",
            "-",
        );
        c.ok(r.duration > 0.05, "演练时长 > 0.05s", format!("{:.3}", r.duration));
        let reloaded = s
            .load_rehearsal_by_id(&rehearsal_id)
            .ok_or_else(|| "无法从磁盘读回演练".to_string())?;
        c.ok(
            reloaded.events.len() == r.events.len() && reloaded.strokes.len() == r.strokes.len(),
            "rehearsal.json 存盘/读盘往返一致",
            format!(
                "{} events / {} strokes",
                reloaded.events.len(),
                reloaded.strokes.len()
            ),
        );
        let audio = match &reloaded.audio {
            Some(a) => format!("{}Hz/{}ch/{} 帧", a.sample_rate, a.channels, a.frames),
            None => "无".into(),
        };
        println!("  [info] 音频轨道：{audio}");
    }

    println!("== 5. 回放：时间轴 / 空间→时间反查 ==");
    {
        // (a) 拖动时间轴（自检里的录制很短，所以取实际时长的一半）
        let d = state.borrow().rehearsal().map(|r| r.duration).unwrap_or(0.0);
        let target = d * 0.5;
        state.borrow_mut().seek(target);
        let pos = state.borrow().player_position();
        c.near(pos, target, 1e-3, "拖动时间轴定位准确");
    }
    // (b) 点击元素 → 跳回它出现的时间
    let el2_want = {
        let s = state.borrow();
        let events: Vec<TimelineEvent> = s.rehearsal().map(|r| r.events.clone()).unwrap_or_default();
        let want = reveal_time(&events, "el2", f64::INFINITY);
        let (cx, cy) = element_center(&s, "el2");
        drop(s);
        if let Some(want) = want {
            // 先把播放头放到元素已出现之后，否则它不在可见集合里
            state.borrow_mut().seek(want + 0.5);
            click(&state, cx, cy);
            let got = state.borrow().player_position();
            c.near(got, want, 0.02, "点击元素 → 跳到它的 Reveal 时间");
            Some(want)
        } else {
            c.ok(false, "点击元素 → 找不到 Reveal 时间", "-");
            None
        }
    };
    let _ = el2_want;
    // (c) 点击笔迹 → 跳到"当时正在写这一笔"的时间
    {
        let s = state.borrow();
        let r = s.rehearsal().cloned().unwrap_or_else(|| {
            // 理论上不会发生
            crate::model::Rehearsal::new("none", "none", "p1")
        });
        drop(s);
        if let Some(st) = r.strokes.iter().find(|st| st.page_id == "p1").cloned() {
            let mid = st.points[st.points.len() / 2];
            let want_t = nearest_stroke_time(&st, mid.x, mid.y);
            let enter = st.start_time + (st.end_time - st.start_time) * 0.6;
            state.borrow_mut().seek(enter);
            click(&state, mid.x + 1.0, mid.y + 1.0);
            let got = state.borrow().player_position();
            c.near(got, want_t, 0.03, "点击笔迹 → 跳到几何最近点的时间戳");
        } else {
            c.ok(false, "找不到第 1 页的笔迹", "-");
        }
    }
    // (d) 播放 / 暂停 / 回到开头
    {
        let before = state.borrow().player_position();
        state.borrow_mut().play_toggle();
        c.ok(state.borrow().player_is_playing(), "播放已开始", "-");
        std::thread::sleep(std::time::Duration::from_millis(150));
        let after = state.borrow().player_position();
        c.ok(after > before, "播放头在推进", format!("{before:.3} → {after:.3}"));
        state.borrow_mut().play_toggle();
        c.ok(!state.borrow().player_is_playing(), "暂停生效", "-");
        state.borrow_mut().stop_playback();
        let p = state.borrow().player_position();
        c.near(p, 0.0, 1e-6, "回到开头");
    }

    println!("== 6. 讲稿：三档 + 按时间反查段落 ==");
    {
        let (s2_id, level0) = {
            let s = state.borrow();
            (s.project().script.sections[1].id.clone(), s.script_level())
        };
        let want = {
            let s = state.borrow();
            s.rehearsal().and_then(|r| {
                r.events.iter().find_map(|e| match e {
                    TimelineEvent::ScriptMark { section_id, t } if section_id == &s2_id => Some(*t),
                    _ => None,
                })
            })
        };
        {
            let mut s = state.borrow_mut();
            s.ui().set_script_level(2);
            s.script_row_click(1);
        }
        let pos = state.borrow().player_position();
        match want {
            Some(t) => c.near(pos, t, 0.02, "点击讲稿某句 → 跳到该句时间"),
            None => c.ok(false, "第 2 段讲稿应有自动锚点", "-"),
        }
        let lvl = state.borrow().script_level();
        c.ok(lvl == 2 && lvl != level0, "讲稿档位可切换", lvl);
    }

    println!("== 7. 演讲模式（不录音） ==");
    {
        state.borrow_mut().set_mode(Mode::Present);
        c.ok(!state.borrow().is_recording(), "演讲模式不录音", "-");
        let r0 = state.borrow().model_row_count().0;
        c.ok(r0 == 1, "只显示已出现的元素（无提示框）", r0);
        state.borrow_mut().key_action("\u{20}", false, false);
        let r1 = state.borrow().model_row_count().0;
        c.ok(r1 == 2, "空格让下一个元素出现", r1);
        state.borrow_mut().key_action("3", false, false);
        let r2 = state.borrow().model_row_count().0;
        c.ok(r2 == 3, "数字键让元素出现", r2);
        state.borrow_mut().next_page(1);
        c.ok(
            state.borrow().project().current_page == 1,
            "演讲模式可以翻页",
            state.borrow().project().current_page,
        );
    }

    println!("== 8. 保存 / 重新打开项目 ==");
    {
        state.borrow_mut().handle_save();
        let slug = state.borrow().project().id.clone();
        let reopened = store
            .open_project(&slug)
            .map_err(|e| format!("重新打开项目失败：{e}"))?;
        c.ok(
            reopened.project().rehearsals.len() == 2,
            "project.json 记录了 2 次演练",
            reopened.project().rehearsals.len(),
        );
        c.ok(
            reopened.project().script.sections[0].reveal_time.is_some(),
            "讲稿锚点已回写到 project.json",
            format!("{:?}", reopened.project().script.sections[0].reveal_time),
        );
        let latest = reopened
            .load_latest_rehearsal()
            .ok_or_else(|| "无法读回最新演练".to_string())?;
        c.ok(
            latest.id == rehearsal_id,
            "最新演练就是刚才录的那次",
            latest.id.clone(),
        );
    }

    println!("\n== 结果：{} 项通过 / {} 项失败 ==", c.passed, c.failed);
    if c.failed == 0 {
        Ok(())
    } else {
        Err(format!("自检有 {} 项失败", c.failed))
    }
}
