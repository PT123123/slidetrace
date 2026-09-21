use serde::{Deserialize, Serialize};

use super::{format_unix_time, now_unix, stroke::Stroke, PageId, SectionId};
use crate::timeline::{sort_events, TimelineEvent};

/// 一次演练的音频引用。V1 只支持单轨麦克风录音。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioRef {
    /// 相对 `<rehearsal>/` 的文件名，固定为 `audio.wav`。
    pub file: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// 实际写入 WAV 的采样帧数（用于校验 duration，容忍浮点误差）。
    #[serde(default)]
    pub frames: u64,
    /// V1 恒为 `"mic"`。为将来的 `"system"` / 多轨预留（见 README 扩展点）。
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "mic".to_string()
}

impl AudioRef {
    pub fn mic(sample_rate: u32, channels: u16, frames: u64) -> Self {
        Self {
            file: "audio.wav".to_string(),
            sample_rate,
            channels,
            frames,
            source: default_source(),
        }
    }

    pub fn duration(&self) -> f64 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0.0;
        }
        self.frames as f64 / (self.sample_rate as f64 * self.channels as f64)
    }
}

/// 一次完整的演练（SPEC §13）。这是一等对象，不是一段视频。
///
/// 讲稿锚点（`ScriptMark`）直接存在 [`Rehearsal::events`] 里，通过
/// [`Rehearsal::section_reveal_time`] 反查，避免同一份数据存两遍。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rehearsal {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created_at_unix: u64,
    /// 录音总时长（秒）。没有音频时也要有值，时间轴依赖它。
    pub duration: f64,
    /// 录制开始时停留在哪一页。
    pub start_page_id: PageId,
    #[serde(default)]
    pub audio: Option<AudioRef>,
    /// 时间轴事件，按 `t` 升序。
    #[serde(default)]
    pub events: Vec<TimelineEvent>,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
}

impl Rehearsal {
    pub fn new(id: impl Into<String>, name: impl Into<String>, start_page_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            created_at_unix: now_unix(),
            duration: 0.0,
            start_page_id: start_page_id.into(),
            audio: None,
            events: Vec::new(),
            strokes: Vec::new(),
        }
    }

    /// 插入一个事件并保持按时间有序（允许乱序插入）。
    pub fn push_event(&mut self, event: TimelineEvent) {
        self.events.push(event);
        sort_events(&mut self.events);
    }

    /// 同一页面上的笔迹。
    pub fn strokes_on<'a>(&'a self, page_id: &'a str) -> impl Iterator<Item = &'a Stroke> + 'a {
        self.strokes.iter().filter(move |s| s.page_id == page_id)
    }

    /// 某个讲稿段落第一次被讲到的时刻。
    pub fn section_reveal_time(&self, section_id: &str) -> Option<f64> {
        self.events
            .iter()
            .filter_map(|e| match e {
                TimelineEvent::ScriptMark { section_id: sid, t } if sid == section_id => Some(*t),
                _ => None,
            })
            .next()
    }

    /// 元素第一次出现的时刻。
    pub fn element_reveal_time(&self, element_id: &str) -> Option<f64> {
        self.events
            .iter()
            .filter_map(|e| match e {
                TimelineEvent::Reveal { element_id: eid, t } if eid == element_id => Some(*t),
                _ => None,
            })
            .next()
    }

    /// 所有讲稿锚点（按时间）。
    pub fn script_anchors(&self) -> Vec<(SectionId, f64)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                TimelineEvent::ScriptMark { section_id, t } => Some((section_id.clone(), *t)),
                _ => None,
            })
            .collect()
    }

    pub fn created_at_display(&self) -> String {
        format_unix_time(self.created_at_unix)
    }

    /// 清空旧的讲稿锚点后写入新的（重新录制时用）。
    pub fn set_section_marks(&mut self, marks: &[(SectionId, f64)]) {
        self.events.retain(|e| !matches!(e, TimelineEvent::ScriptMark { .. }));
        for (id, t) in marks {
            self.events.push(TimelineEvent::ScriptMark {
                section_id: id.clone(),
                t: *t,
            });
        }
        sort_events(&mut self.events);
    }
}

/// `project.json` 里保存的演练**元信息**（不包含事件与笔迹，那些在
/// `rehearsals/<id>/rehearsal.json`）。这样项目文件不会随录制次数膨胀。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RehearsalMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created_at_unix: u64,
    pub duration: f64,
    #[serde(default)]
    pub has_audio: bool,
    #[serde(default)]
    pub event_count: usize,
    #[serde(default)]
    pub stroke_count: usize,
}

impl RehearsalMeta {
    pub fn from_rehearsal(r: &Rehearsal) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            created_at_unix: r.created_at_unix,
            duration: r.duration,
            has_audio: r.audio.is_some(),
            event_count: r.events.len(),
            stroke_count: r.strokes.len(),
        }
    }

    pub fn created_at_display(&self) -> String {
        format_unix_time(self.created_at_unix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Stroke;

    #[test]
    fn audio_duration_is_frames_over_rate_times_channels() {
        let a = AudioRef::mic(48_000, 2, 48_000 * 2 * 3);
        assert_eq!(a.duration(), 3.0);
        assert_eq!(AudioRef::mic(0, 0, 10).duration(), 0.0);
    }

    #[test]
    fn script_marks_are_derived_from_events() {
        let mut r = Rehearsal::new("r1", "第一次演练", "p1");
        r.push_event(TimelineEvent::ScriptMark {
            section_id: "s2".into(),
            t: 8.5,
        });
        r.push_event(TimelineEvent::Reveal {
            element_id: "el1".into(),
            t: 4.0,
        });
        r.push_event(TimelineEvent::ScriptMark {
            section_id: "s1".into(),
            t: 0.0,
        });
        // 事件已被排序
        assert_eq!(r.events[0].time(), 0.0);
        assert_eq!(r.section_reveal_time("s2"), Some(8.5));
        assert_eq!(r.section_reveal_time("s9"), None);
        assert_eq!(r.element_reveal_time("el1"), Some(4.0));
        assert_eq!(r.script_anchors().len(), 2);
        // 重写锚点会替换掉旧的
        r.set_section_marks(&[("s1".into(), 0.0), ("s3".into(), 12.0)]);
        assert_eq!(r.script_anchors().len(), 2);
        assert_eq!(r.section_reveal_time("s2"), None);
        assert_eq!(r.section_reveal_time("s3"), Some(12.0));
    }

    #[test]
    fn meta_mirrors_rehearsal() {
        let mut r = Rehearsal::new("r1", "演练 01", "p1");
        r.duration = 12.5;
        r.audio = Some(AudioRef::mic(16_000, 1, 16_000 * 12));
        r.strokes.push(Stroke::new("st1", "p1", "#000", 2.0));
        let m = RehearsalMeta::from_rehearsal(&r);
        assert!(m.has_audio);
        assert_eq!(m.stroke_count, 1);
        assert_eq!(m.duration, 12.5);
    }
}
