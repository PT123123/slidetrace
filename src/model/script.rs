use serde::{Deserialize, Serialize};

use super::PageId;

/// 讲稿段落 id。
pub type SectionId = String;

/// 一句话的三个层级（SPEC §19）：
/// - `full`：完整讲话内容（编辑时看）
/// - `prompt`：缩短成提示（练习时看）
/// - `minimal`：只有关键词（正式演讲时看）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptSection {
    pub id: SectionId,
    pub full: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub minimal: String,
    /// 可选：这一句大致对应的页面，用于 Create 模式联动。
    #[serde(default)]
    pub page_id: Option<PageId>,
    /// 最近一次录制得到的"这句话是在第几秒开始讲的"。
    ///
    /// 权威数据在 `Rehearsal.events` 的 `ScriptMark` 里；这个字段是给 Create 模式
    /// 和"还没有 Rehearsal 时"用的工作副本，会自动回写。
    #[serde(default)]
    pub reveal_time: Option<f64>,
}

impl ScriptSection {
    pub fn new(id: impl Into<String>, full: impl Into<String>) -> Self {
        let full = full.into();
        Self {
            id: id.into(),
            prompt: compact(&full, 12),
            minimal: keywords(&full, 3),
            full,
            page_id: None,
            reveal_time: None,
        }
    }

    /// 取某一档文本。空字符串视为"该档未填写"，回退到上一档，避免面板出现空白。
    pub fn text(&self, level: ScriptLevel) -> &str {
        match level {
            ScriptLevel::Full => &self.full,
            ScriptLevel::Prompt => {
                if self.prompt.trim().is_empty() {
                    &self.full
                } else {
                    &self.prompt
                }
            }
            ScriptLevel::Minimal => {
                if !self.minimal.trim().is_empty() {
                    &self.minimal
                } else if !self.prompt.trim().is_empty() {
                    &self.prompt
                } else {
                    &self.full
                }
            }
        }
    }
}

/// 讲稿三档。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptLevel {
    Full,
    Prompt,
    Minimal,
}

impl Default for ScriptLevel {
    fn default() -> Self {
        ScriptLevel::Full
    }
}

impl ScriptLevel {
    pub fn label(self) -> &'static str {
        match self {
            ScriptLevel::Full => "完整",
            ScriptLevel::Prompt => "提示",
            ScriptLevel::Minimal => "关键词",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ScriptLevel::Full => ScriptLevel::Prompt,
            ScriptLevel::Prompt => ScriptLevel::Minimal,
            ScriptLevel::Minimal => ScriptLevel::Full,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptDoc {
    #[serde(default)]
    pub sections: Vec<ScriptSection>,
}

impl ScriptDoc {
    pub fn section_index(&self, id: &str) -> Option<usize> {
        self.sections.iter().position(|s| s.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

/// 把长句压成短提示：按标点切成短句，取前 2 段拼接。
fn compact(full: &str, max_chars: usize) -> String {
    let parts = split_clauses(full);
    let mut out = String::new();
    for p in parts.iter().take(2) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(p);
        if out.chars().count() >= max_chars {
            break;
        }
    }
    truncate_chars(&out, max_chars)
}

/// 取关键词：按标点切分后取每段的前 4 个字，取前 N 段。
fn keywords(full: &str, n: usize) -> String {
    split_clauses(full)
        .iter()
        .take(n)
        .map(|p| truncate_chars(p, 6))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// 中英文标点都切。
fn split_clauses(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if matches!(ch, '。' | '，' | '；' | '！' | '？' | '、' | '.' | ',' | ';' | '!' | '?' | '\n') {
            let t = cur.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    if out.is_empty() {
        out.push(text.trim().to_string());
    }
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_prompt_and_minimal_are_generated() {
        let s = ScriptSection::new("s1", "首先我们来讨论整个数据库系统，然后看一下它的架构。");
        assert_eq!(s.prompt, "首先我们来讨论整个数据库…");
        assert_eq!(s.minimal, "首先我们来讨… · 然后看一下它…");
        // 三档都有值，永远不空白
        assert!(!s.text(ScriptLevel::Minimal).is_empty());
    }

    #[test]
    fn empty_level_falls_back() {
        let mut s = ScriptSection::new("s1", "完整内容");
        s.prompt = "  ".into();
        s.minimal = String::new();
        assert_eq!(s.text(ScriptLevel::Prompt), "完整内容");
        assert_eq!(s.text(ScriptLevel::Minimal), "完整内容");
    }

    #[test]
    fn level_cycles() {
        assert_eq!(ScriptLevel::Full.next(), ScriptLevel::Prompt);
        assert_eq!(ScriptLevel::Prompt.next(), ScriptLevel::Minimal);
        assert_eq!(ScriptLevel::Minimal.next(), ScriptLevel::Full);
    }
}
