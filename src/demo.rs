//! 演示项目：首次启动时自动创建，保证"一打开就有东西可看可点"。
//!
//! 为了在**没有麦克风**的环境下也能立刻体验 Review / Replay / 点击跳转，
//! 演示项目自带一次合成音频的演练（`r1`）。音频是程序生成的占位波形，
//! 不是真人录音——这样可以在 CI / 无声卡机器上完整打通闭环。
//!
//! [`build_demo_project`] 是纯函数（不碰磁盘），方便单测；
//! [`install`] 负责把资源与演练写到磁盘。
// 本模块是刻意保留的「核心 API 层」：它被单元测试完整覆盖，但并不要求每个
// 访问器都被当前 UI 调用（例如 `Rehearsal::push_event` / `Stroke::length` 这类
// 查询函数，未来接入时间轴编辑、摄像头轨道时会直接用上）。
// 这里统一放行 dead_code，避免 `cargo build` 输出被几十条"未被 UI 调用"淹没；
// UI 层（src/app.rs、src/main.rs）**不加**这个放行，真正的死代码仍会被指出。
#![allow(dead_code)]

use std::path::Path;

use crate::model::{
    AssetRef, Element, Page, Project, Rehearsal, ScriptSection, Size, Stroke, DEFAULT_PAGE_HEIGHT,
    DEFAULT_PAGE_WIDTH,
};
use crate::timeline::TimelineEvent;

pub const DEMO_REHEARSAL_ID: &str = "r1";
pub const DEMO_ASSET_TIMELINE: &str = "figure-timeline.png";
pub const DEMO_ASSET_CANVAS: &str = "figure-canvas.png";

/// 合成演练的总时长（秒）。
pub const DEMO_DURATION: f64 = 36.5;
/// 合成音频参数：16 kHz 单声道，和典型麦克风录音一致，文件也不大。
const DEMO_SAMPLE_RATE: u32 = 16_000;

/// 构造演示项目的**数据部分**（3 页 / 8 个元素 / 5 段讲稿）。
pub fn build_demo_project() -> Project {
    let mut p = Project::new("示例演讲：以讲话时间为主轴");
    p.page_size = Size::new(DEFAULT_PAGE_WIDTH, DEFAULT_PAGE_HEIGHT);
    p.pages.clear();

    // ---------------- 第 1 页 ----------------
    let mut page1 = Page::new("p1", "第 1 页 · 开场");
    page1.note = "标题页：先讲定位，再让要点依次出现".into();
    let mut title = Element::text(
        "el1",
        96.0,
        140.0,
        1088.0,
        120.0,
        "以讲话时间为主轴",
    );
    title.hidden_by_default = false;
    title.style.font_size = 64.0;
    title.style.bold = true;
    title.style.align = crate::model::TextAlign::Center;
    let mut sub = Element::text(
        "el2",
        96.0,
        286.0,
        1088.0,
        64.0,
        "讲稿 · 画面 · 手写 · 音频，全部绑定到同一条时间轴",
    );
    sub.style.font_size = 30.0;
    sub.style.align = crate::model::TextAlign::Center;
    sub.hidden_by_default = true;
    let mut fig = Element::image("el3", 340.0, 380.0, 600.0, 280.0, DEMO_ASSET_TIMELINE);
    fig.hidden_by_default = true;
    page1.elements = vec![title, sub, fig];

    // ---------------- 第 2 页 ----------------
    let mut page2 = Page::new("p2", "第 2 页 · 核心模型");
    page2.note = "核心页：这页会有手写标注".into();
    let mut head = Element::text("el4", 96.0, 80.0, 1088.0, 72.0, "Presentation State(t)");
    head.hidden_by_default = false;
    head.style.font_size = 46.0;
    head.style.bold = true;
    let mut body = Element::text(
        "el5",
        96.0,
        470.0,
        1088.0,
        56.0,
        "state_at(t) → 当前页面 / 元素可见性 / 笔迹进度 / 讲稿段落",
    );
    body.style.font_size = 28.0;
    body.hidden_by_default = true;
    let mut fig2 = Element::image("el6", 200.0, 180.0, 880.0, 260.0, DEMO_ASSET_CANVAS);
    fig2.hidden_by_default = true;
    page2.elements = vec![head, fig2, body];

    // ---------------- 第 3 页 ----------------
    let mut page3 = Page::new("p3", "第 3 页 · 下一步");
    page3.note = "收尾页".into();
    let mut a = Element::text("el7", 96.0, 160.0, 1088.0, 80.0, "按下 Record，一边讲话一边让内容出现");
    a.hidden_by_default = false;
    a.style.font_size = 40.0;
    let mut b = Element::text(
        "el8",
        96.0,
        300.0,
        1088.0,
        80.0,
        "鼠标在画布上拖动 = 手写，笔迹逐点带时间戳",
    );
    b.hidden_by_default = true;
    b.style.font_size = 34.0;
    page3.elements = vec![a, b];

    p.pages = vec![page1, page2, page3];

    // ---------------- 讲稿（5 段，三档文本） ----------------
    let sections: Vec<(&str, &str, &str, &str, &str)> = vec![
        (
            "s1",
            "大家好，今天我用十分钟讲清楚这套工具的定位：它不是带录音的 PPT。",
            "不是带录音的 PPT",
            "定位",
            "p1",
        ),
        (
            "s2",
            "它最核心的一点是：页面只是空间容器，真正的主轴是讲话时间。",
            "页面只是容器，主轴是时间",
            "时间主轴",
            "p1",
        ),
        (
            "s3",
            "在任意时刻 t，系统都知道画面是什么状态、手写写到了哪里、我讲到了哪一句。",
            "任意 t 都知道完整状态",
            "Presentation State(t)",
            "p2",
        ),
        (
            "s4",
            "所以点击任何一个元素、或者一段笔迹，都能跳回我当时说话的位置。",
            "点击内容 → 跳回当时的话",
            "空间反查时间",
            "p2",
        ),
        (
            "s5",
            "最后请记住一句话：先讲，再让内容跟着出现。",
            "先讲，再让内容出现",
            "先说再做",
            "p3",
        ),
    ];
    for (id, full, prompt, minimal, page) in sections {
        let mut s = ScriptSection::new(id, full);
        s.prompt = prompt.to_string();
        s.minimal = minimal.to_string();
        s.page_id = Some(page.to_string());
        p.script.sections.push(s);
    }

    p.assets = vec![
        AssetRef::new(DEMO_ASSET_TIMELINE, 960, 448),
        AssetRef::new(DEMO_ASSET_CANVAS, 960, 284),
    ];
    p.serial = 100;
    p.current_page = 0;
    p
}

/// 演示演练的合成音频：用包络模拟"说话 / 停顿"的节奏，方便看波形。
pub fn demo_audio_samples() -> Vec<f32> {
    let n = (DEMO_DURATION * DEMO_SAMPLE_RATE as f64) as usize;
    let mut out = Vec::with_capacity(n);
    let mut rng = Xorshift::new(0x5EED_1234);
    // 每个"音节"约 0.18 秒，音节之间随机插入停顿，整体听感接近讲话节奏。
    let syllable = (0.18 * DEMO_SAMPLE_RATE as f64) as usize;
    let mut remaining_in_syllable = 0usize;
    let mut gap = 0usize;
    let mut env = 0.0f32;
    let mut phase = 0.0f32;

    for i in 0..n {
        let t = i as f64 / DEMO_SAMPLE_RATE as f64;
        if remaining_in_syllable == 0 && gap == 0 {
            // 超过 6 秒后进入一次明显的长停顿，让波形上能看到"停顿"
            if rng.next_f32() < 0.02 && t > 6.0 {
                gap = (0.7 * DEMO_SAMPLE_RATE as f64) as usize;
            } else {
                remaining_in_syllable = syllable;
            }
        }
        if gap > 0 {
            gap -= 1;
            env *= 0.90;
        } else {
            remaining_in_syllable = remaining_in_syllable.saturating_sub(1);
            let target = 0.25 + 0.6 * rng.next_f32();
            env = env * 0.85 + target * 0.15;
        }
        // 基频 110~190 Hz + 少量噪声，纯占位音，不追求可听性
        let f0 = 110.0 + 40.0 * env;
        phase += 2.0 * std::f32::consts::PI * f0 / DEMO_SAMPLE_RATE as f32;
        if phase > std::f32::consts::TAU {
            phase -= std::f32::consts::TAU;
        }
        let tone = phase.sin() * 0.6 + (phase * 2.0).sin() * 0.25;
        let noise = rng.next_f32() * 2.0 - 1.0;
        out.push(((tone * 0.7 + noise * 0.3) * env * 0.5).clamp(-1.0, 1.0));
    }
    out
}

/// 演示演练的事件序列（时间轴与讲稿锚点一一对应）。
pub fn demo_events() -> Vec<TimelineEvent> {
    use TimelineEvent::*;
    let mut evs = vec![
        PageChange { page_id: "p1".into(), t: 0.0 },
        ScriptMark { section_id: "s1".into(), t: 0.0 },
        Reveal { element_id: "el2".into(), t: 4.2 },
        Reveal { element_id: "el3".into(), t: 9.1 },
        ScriptMark { section_id: "s2".into(), t: 13.5 },
        PageChange { page_id: "p2".into(), t: 17.0 },
        ScriptMark { section_id: "s3".into(), t: 17.0 },
        Reveal { element_id: "el6".into(), t: 21.4 },
        Reveal { element_id: "el5".into(), t: 25.2 },
        ScriptMark { section_id: "s4".into(), t: 29.0 },
        PageChange { page_id: "p3".into(), t: 31.5 },
        ScriptMark { section_id: "s5".into(), t: 31.5 },
        Reveal { element_id: "el8".into(), t: 34.0 },
        Marker { label: "结束".into(), t: 36.2 },
    ];
    crate::timeline::sort_events(&mut evs);
    evs
}

/// 演示演练的笔迹：第 1 页画一条下划线，第 2 页给标题画圈、给示意图上打勾。
pub fn demo_strokes() -> Vec<Stroke> {
    let mut out = Vec::new();

    // 第 1 页：在标题下面画一条横线（t 10.5 → 12.0）
    let mut s1 = Stroke::new("st1", "p1", "#e11d48", 4.0);
    let n = 24;
    for i in 0..=n {
        let u = i as f64 / n as f64;
        // 轻微的手抖感
        let wobble = (u * 18.0).sin() * 2.5;
        s1.push(
            360.0 + u * 560.0,
            352.0 + wobble,
            10.5 + u * 1.5,
        );
    }
    out.push(s1);

    // 第 2 页：给标题画一个椭圆圈（t 18.0 → 20.4）
    let mut s2 = Stroke::new("st2", "p2", "#2563eb", 4.0);
    let n = 48;
    for i in 0..=n {
        let a = i as f64 / n as f64 * std::f64::consts::TAU * 1.05;
        s2.push(
            640.0 + 300.0 * a.cos(),
            116.0 + 46.0 * a.sin(),
            18.0 + (i as f64 / n as f64) * 2.4,
        );
    }
    out.push(s2);

    // 第 2 页：在示意图上画一个勾（t 22.5 → 23.6）
    let mut s3 = Stroke::new("st3", "p2", "#16a34a", 6.0);
    let pts = [
        (620.0, 330.0, 22.5),
        (668.0, 392.0, 22.9),
        (760.0, 250.0, 23.6),
    ];
    for (x, y, t) in pts {
        s3.push(x, y, t);
    }
    out.push(s3);

    // 第 3 页：给"先讲，再让内容出现"画框（t 35.0 → 36.4）
    let mut s4 = Stroke::new("st4", "p3", "#f59e0b", 4.0);
    let corners = [
        (88.0, 288.0),
        (1184.0, 288.0),
        (1184.0, 392.0),
        (88.0, 392.0),
        (88.0, 288.0),
    ];
    for (i, (x, y)) in corners.iter().enumerate() {
        s4.push(*x, *y, 35.0 + i as f64 * 0.35);
    }
    out.push(s4);

    out
}

/// 组装演示演练对象（音频引用需要调用方提供帧数）。
pub fn demo_rehearsal(frames: u64) -> Rehearsal {
    let mut r = Rehearsal::new(DEMO_REHEARSAL_ID, "示例演练（合成音频）", "p1");
    r.duration = DEMO_DURATION;
    r.audio = Some(crate::model::AudioRef::mic(
        DEMO_SAMPLE_RATE,
        1,
        frames,
    ));
    r.events = demo_events();
    r.strokes = demo_strokes();
    r
}

/// 把演示项目完整写到磁盘：project.json + 占位图 + 演示演练（含 WAV）。
///
/// 返回创建好的项目句柄；若根目录不可写则返回 Err，由调用方决定是否降级。
pub fn install(store: &crate::storage::Store) -> Result<crate::storage::ProjectHandle, String> {
    let mut handle = store
        .create_project("示例演讲：以讲话时间为主轴")
        .map_err(|e| e.to_string())?;

    // 用真实数据覆盖 create_project 生成的默认内容
    let demo = {
        let mut d = build_demo_project();
        d.id = handle.project().id.clone();
        d.created_at_unix = handle.project().created_at_unix;
        // 刻意保留 build_demo_project 里的 serial（100），
        // 不要用空白项目的 serial 覆盖它 —— 否则后续 next_id 会从 2 开始，
        // 生成 el2 撞上演示项目里已有的 el2。
        d
    };
    *handle.project_mut() = demo;
    handle.save().map_err(|e| e.to_string())?;

    // 占位图片
    write_placeholder_png(
        &handle.assets_dir().join(DEMO_ASSET_TIMELINE),
        960,
        448,
        [37, 99, 235],
        [124, 58, 237],
    )
    .map_err(|e| e.to_string())?;
    write_placeholder_png(
        &handle.assets_dir().join(DEMO_ASSET_CANVAS),
        960,
        284,
        [13, 148, 136],
        [22, 163, 74],
    )
    .map_err(|e| e.to_string())?;

    // 合成音频 + 演练数据
    let samples = demo_audio_samples();
    std::fs::create_dir_all(handle.rehearsal_dir(DEMO_REHEARSAL_ID)).map_err(|e| e.to_string())?;
    write_wav_mono_i16(
        &handle.audio_path(DEMO_REHEARSAL_ID),
        DEMO_SAMPLE_RATE,
        &samples,
    )
    .map_err(|e| e.to_string())?;

    let rehearsal = demo_rehearsal(samples.len() as u64);
    handle
        .save_rehearsal(&rehearsal)
        .map_err(|e| e.to_string())?;

    Ok(handle)
}

/// 写一个占位 PNG（渐变底 + 边框 + 对角线），不需要任何字体资源。
pub fn write_placeholder_png(
    path: &Path,
    w: u32,
    h: u32,
    from: [u8; 3],
    to: [u8; 3],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let u = x as f32 / w.max(1) as f32;
        let v = y as f32 / h.max(1) as f32;
        let m = (u * 0.6 + v * 0.4).clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * m) as u8;
        let border = x < 3 || y < 3 || x >= w - 3 || y >= h - 3;
        let diag = ((x as i32 - y as i32).rem_euclid(48)) < 2;
        let c = if border {
            [255u8, 255, 255, 220]
        } else if diag {
            [255, 255, 255, 60]
        } else {
            [lerp(from[0], to[0]), lerp(from[1], to[1]), lerp(from[2], to[2]), 255]
        };
        *px = image::Rgba(c);
    }
    img.save(path)?;
    Ok(())
}

/// 写 16-bit PCM 单声道 WAV。演示音频与单元测试共用。
pub fn write_wav_mono_i16(
    path: &Path,
    sample_rate: u32,
    samples: &[f32],
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for s in samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}

/// 极小的确定性伪随机数发生器：避免为演示数据引入 rand 依赖。
struct Xorshift(u32);

impl Xorshift {
    fn new(seed: u32) -> Self {
        Self(seed | 1)
    }
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_project_shape_matches_spec() {
        let p = build_demo_project();
        assert_eq!(p.pages.len(), 3, "演示项目应有 3 页");
        assert_eq!(p.script.sections.len(), 5, "演示项目应有 5 段讲稿");
        for page in &p.pages {
            assert!(
                (2..=3).contains(&page.elements.len()),
                "每页应有 2-3 个元素，实际 {}",
                page.elements.len()
            );
        }
        assert_eq!(p.assets.len(), 2);
        // 三档文本都不能为空（面板切到 Minimal 也要有东西显示）
        for s in &p.script.sections {
            for lvl in [
                crate::model::script::ScriptLevel::Full,
                crate::model::script::ScriptLevel::Prompt,
                crate::model::script::ScriptLevel::Minimal,
            ] {
                assert!(!s.text(lvl).trim().is_empty(), "段落 {} 的 {:?} 为空", s.id, lvl);
            }
        }
    }

    #[test]
    fn demo_project_ids_are_unique() {
        let p = build_demo_project();
        let mut ids: Vec<&str> = p
            .pages
            .iter()
            .flat_map(|pg| pg.elements.iter().map(|e| e.id.as_str()))
            .collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }

    #[test]
    fn demo_events_only_reference_existing_ids() {
        let p = build_demo_project();
        for ev in demo_events() {
            match &ev {
                TimelineEvent::Reveal { element_id, .. }
                | TimelineEvent::Hide { element_id, .. } => {
                    assert!(p.find_element(element_id).is_some(), "{element_id} 不存在");
                }
                TimelineEvent::PageChange { page_id, .. } => {
                    assert!(p.page(page_id).is_some(), "{page_id} 不存在");
                }
                TimelineEvent::ScriptMark { section_id, .. } => {
                    assert!(p.script.section_index(section_id).is_some(), "{section_id} 不存在");
                }
                TimelineEvent::Marker { .. } => {}
            }
        }
    }

    #[test]
    fn demo_events_are_sorted_and_within_duration() {
        let evs = demo_events();
        assert!(crate::timeline::is_sorted(&evs));
        for e in &evs {
            assert!(e.time() >= 0.0 && e.time() <= DEMO_DURATION, "{:?}", e);
        }
    }

    #[test]
    fn demo_strokes_times_are_monotonic_and_within_duration() {
        for s in demo_strokes() {
            assert!(s.points.len() >= 2);
            for w in s.points.windows(2) {
                assert!(w[0].t <= w[1].t, "笔迹 {} 时间非单调", s.id);
            }
            assert!(s.end_time <= DEMO_DURATION);
            assert!(s.start_time >= 0.0);
        }
    }

    #[test]
    fn demo_audio_length_and_range() {
        let s = demo_audio_samples();
        assert_eq!(s.len(), (DEMO_DURATION * DEMO_SAMPLE_RATE as f64) as usize);
        assert!(s.iter().all(|v| (-1.0..=1.0).contains(v)));
        // 波形必须有起伏，否则时间轴上看不到东西
        let peaks = crate::timeline::waveform_peaks(&s, 120);
        let max = peaks.iter().cloned().fold(0.0f32, f32::max);
        let min = peaks.iter().cloned().fold(f32::MAX, f32::min);
        assert!(max > 0.15, "峰值太小：{max}");
        assert!(min < max * 0.5, "缺少停顿造成的低谷");
    }

    #[test]
    fn demo_state_at_end_shows_everything_revealed() {
        let p = build_demo_project();
        let r = demo_rehearsal(0);
        let input = crate::timeline::TimelineInput::new(&p, Some(&r));
        let last = input.state_at(DEMO_DURATION);
        assert_eq!(last.page_id, "p3");
        assert!(last.is_visible("el7"));
        assert!(last.is_visible("el8"));
        assert_eq!(last.section_index, Some(4));

        let first = input.state_at(0.0);
        assert_eq!(first.page_id, "p1");
        assert!(first.is_visible("el1"));
        assert!(!first.is_visible("el2"));
    }
}
