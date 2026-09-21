//! 项目目录读写。
//!
//! ```text
//! <root>\projects\<slug>\
//! ├── project.json
//! ├── assets\figure-01.png ...
//! └── rehearsals\r1\
//!     ├── audio.wav
//!     └── rehearsal.json
//! ```
//!
//! 存储层**支持注入根路径**（[`Store::new`]），因此单测可以用 tempdir
//! 完整跑一遍「建项目 → 存 → 读 → 比对」，不碰用户真实目录。
// 本模块是刻意保留的「核心 API 层」：它被单元测试完整覆盖，但并不要求每个
// 访问器都被当前 UI 调用（例如 `Rehearsal::push_event` / `Stroke::length` 这类
// 查询函数，未来接入时间轴编辑、摄像头轨道时会直接用上）。
// 这里统一放行 dead_code，避免 `cargo build` 输出被几十条"未被 UI 调用"淹没；
// UI 层（src/app.rs、src/main.rs）**不加**这个放行，真正的死代码仍会被指出。
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{Project, Rehearsal, RehearsalMeta, SCHEMA_VERSION};
use crate::timeline::sort_events;

#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    Json(serde_json::Error),
    NotFound(String),
    Version(u32),
    Message(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Io(e) => write!(f, "文件读写失败：{e}"),
            StorageError::Json(e) => write!(f, "JSON 解析失败：{e}"),
            StorageError::NotFound(s) => write!(f, "找不到：{s}"),
            StorageError::Version(v) => write!(f, "项目文件版本 {v} 高于本程序支持的 {SCHEMA_VERSION}"),
            StorageError::Message(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        StorageError::Io(e)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// 项目仓库。`root` 之下是 `projects/<slug>/`。
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

/// 当前打开的项目句柄。
#[derive(Clone, Debug)]
pub struct ProjectHandle {
    store: Store,
    dir: PathBuf,
    pub project: Project,
}

impl Store {
    /// 注入根路径（单测用 tempdir，正式运行用 `%APPDATA%\slidetrace`）。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 默认根路径：`%APPDATA%\slidetrace`（非 Windows 时退回 `dirs::data_dir()`）。
    pub fn default_root() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("slidetrace")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }

    /// 列出磁盘上已有项目（按目录名排序，只认含 `project.json` 的目录）。
    pub fn list_projects(&self) -> Vec<(String, String)> {
        let dir = self.projects_dir();
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&dir) else {
            return out;
        };
        for e in entries.flatten() {
            let path = e.path().join("project.json");
            if !path.is_file() {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(p) = serde_json::from_str::<Project>(&text) {
                    out.push((p.id, p.name));
                }
            }
        }
        out.sort();
        out
    }

    /// 保证根目录存在。
    pub fn ensure_root(&self) -> Result<()> {
        fs::create_dir_all(self.projects_dir())?;
        Ok(())
    }

    fn project_dir(&self, slug: &str) -> PathBuf {
        self.projects_dir().join(slug)
    }

    /// 新建项目：分配不冲突的 slug、建目录、写 `project.json`。
    pub fn create_project(&self, name: &str) -> Result<ProjectHandle> {
        self.ensure_root()?;
        let mut slug = crate::model::slugify(name);
        let mut n = 2;
        while self.project_dir(&slug).exists() {
            slug = format!("{}-{}", crate::model::slugify(name), n);
            n += 1;
        }
        let mut project = Project::new(name);
        project.id = slug.clone();

        let dir = self.project_dir(&slug);
        fs::create_dir_all(dir.join("assets"))?;
        fs::create_dir_all(dir.join("rehearsals"))?;

        let handle = ProjectHandle {
            store: self.clone(),
            dir,
            project,
        };
        handle.save()?;
        Ok(handle)
    }

    /// 打开已有项目。
    pub fn open_project(&self, slug: &str) -> Result<ProjectHandle> {
        let dir = self.project_dir(slug);
        let file = dir.join("project.json");
        if !file.is_file() {
            return Err(StorageError::NotFound(file.display().to_string()));
        }
        let text = fs::read_to_string(&file)?;
        let mut project: Project = serde_json::from_str(&text)?;
        if project.schema_version > SCHEMA_VERSION {
            return Err(StorageError::Version(project.schema_version));
        }
        // project.json 里没有事件（事件在 rehearsal.json），但演练元信息必须一致。
        project.rehearsals.sort_by(|a, b| a.created_at_unix.cmp(&b.created_at_unix));
        Ok(ProjectHandle {
            store: self.clone(),
            dir,
            project,
        })
    }

    /// 只在磁盘上已有项目时打开，否则新建一个（首次启动的兜底逻辑）。
    pub fn open_or_create(&self, name: &str) -> Result<ProjectHandle> {
        let existing = self.list_projects();
        if let Some((slug, _)) = existing.first() {
            return self.open_project(slug);
        }
        self.create_project(name)
    }
}

impl ProjectHandle {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.dir.join("assets")
    }

    pub fn rehearsals_dir(&self) -> PathBuf {
        self.dir.join("rehearsals")
    }

    pub fn rehearsal_dir(&self, id: &str) -> PathBuf {
        self.rehearsals_dir().join(id)
    }

    pub fn project_json_path(&self) -> PathBuf {
        self.dir.join("project.json")
    }

    /// 把 `asset_name` 解析成磁盘绝对路径。
    ///
    /// 支持三种写法：`assets/` 内的相对文件名、项目目录内的相对路径、绝对路径。
    pub fn resolve_asset(&self, asset_name: &str) -> PathBuf {
        let p = Path::new(asset_name);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        let in_assets = self.assets_dir().join(asset_name);
        if in_assets.is_file() {
            return in_assets;
        }
        self.dir.join(asset_name)
    }

    // ---------------- 保存 ----------------

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(self.dir.join("assets"))?;
        fs::create_dir_all(self.dir.join("rehearsals"))?;
        let text = serde_json::to_string_pretty(&self.project)?;
        // 先写临时文件再改名：避免写一半崩溃导致 project.json 损坏。
        let tmp = self.dir.join("project.json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, self.project_json_path())?;
        Ok(())
    }

    /// 保存一次演练：`rehearsals/<id>/{audio.wav, rehearsal.json}`，同时刷新
    /// `project.json` 里的元信息列表。
    pub fn save_rehearsal(&mut self, rehearsal: &Rehearsal) -> Result<()> {
        let dir = self.rehearsal_dir(&rehearsal.id);
        fs::create_dir_all(&dir)?;
        // 事件保证按时间有序后再落盘，读盘方就不必再排序。
        let mut r = rehearsal.clone();
        sort_events(&mut r.events);
        let text = serde_json::to_string_pretty(&r)?;
        let tmp = dir.join("rehearsal.json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, dir.join("rehearsal.json"))?;

        let meta = RehearsalMeta::from_rehearsal(&r);
        match self.project.rehearsals.iter_mut().find(|m| m.id == meta.id) {
            Some(slot) => *slot = meta,
            None => self.project.rehearsals.push(meta),
        }
        self.project.rehearsals.sort_by(|a, b| {
            a.created_at_unix
                .cmp(&b.created_at_unix)
                .then_with(|| a.id.cmp(&b.id))
        });
        self.project.touch();
        self.save()
    }

    /// 音频文件路径（录音时由 `audio::record` 写入）。
    pub fn audio_path(&self, rehearsal_id: &str) -> PathBuf {
        self.rehearsal_dir(rehearsal_id).join("audio.wav")
    }

    // ---------------- 读取 ----------------

    pub fn load_rehearsal(&self, id: &str) -> Result<Rehearsal> {
        let file = self.rehearsal_dir(id).join("rehearsal.json");
        if !file.is_file() {
            return Err(StorageError::NotFound(file.display().to_string()));
        }
        let text = fs::read_to_string(&file)?;
        let mut r: Rehearsal = serde_json::from_str(&text)?;
        sort_events(&mut r.events);
        for s in r.strokes.iter_mut() {
            s.refresh_bounds();
        }
        // 音频文件可能被手工删掉：以磁盘为准，避免回放时才发现。
        if let Some(audio) = &r.audio {
            if !self.rehearsal_dir(id).join(&audio.file).is_file() {
                r.audio = None;
            }
        }
        Ok(r)
    }

    /// 最近一次演练（按创建时间）。
    pub fn latest_rehearsal_id(&self) -> Option<String> {
        self.project
            .rehearsals
            .last()
            .map(|m| m.id.clone())
    }

    pub fn load_latest_rehearsal(&self) -> Option<Rehearsal> {
        let id = self.latest_rehearsal_id()?;
        self.load_rehearsal(&id).ok()
    }

    /// 复制一个外部文件到 `assets/`，返回新的 `AssetRef`。
    pub fn import_asset(&mut self, src: &Path) -> Result<crate::model::AssetRef> {
        let name = src
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .ok_or_else(|| StorageError::Message("无效的文件名".into()))?;
        // 处理重名：figure.png → figure-2.png
        let mut target_name = name.clone();
        let mut n = 2;
        while self.assets_dir().join(&target_name).exists() {
            let stem = Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "asset".into());
            let ext = Path::new(&name)
                .extension()
                .map(|s| format!(".{}", s.to_string_lossy()))
                .unwrap_or_default();
            target_name = format!("{stem}-{n}{ext}");
            n += 1;
        }
        fs::create_dir_all(self.assets_dir())?;
        fs::copy(src, self.assets_dir().join(&target_name))?;

        let (w, h) = image::image_dimensions(src).unwrap_or((0, 0));
        let mut asset = crate::model::AssetRef::new(target_name, w, h);
        asset.original_path = Some(src.display().to_string());
        self.project.assets.push(asset.clone());
        self.project.touch();
        self.save()?;
        Ok(asset)
    }

    /// 修改项目名（同时改 slug 目录？V1 不移动目录，只改显示名）。
    pub fn rename_project(&mut self, name: &str) -> Result<()> {
        self.project.name = name.to_string();
        self.project.touch();
        self.save()
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn project_mut(&mut self) -> &mut Project {
        &mut self.project
    }

    pub fn meta(&self) -> &[RehearsalMeta] {
        &self.project.rehearsals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AudioRef, Element, Stroke};
    use crate::timeline::TimelineEvent;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        (dir, store)
    }

    // ---------- 验收标准 2.5：项目 JSON 存盘 / 读盘往返一致 ----------

    #[test]
    fn project_json_roundtrip_is_lossless() {
        let (_tmp, store) = temp_store();
        let mut handle = store.create_project("往返测试").unwrap();

        // 构造一个有内容的项目
        {
            let p = handle.project_mut();
            p.page_size = crate::model::Size::new(1280.0, 720.0);
            let el = Element::text("el1", 12.5, 30.0, 400.0, 80.0, "第一段文字");
            p.pages[0].elements.push(el);
            let mut img = Element::image("el2", 600.0, 200.0, 300.0, 200.0, "figure-01.png");
            img.hidden_by_default = false;
            img.style.background = Some("#eeeeee".into());
            p.pages[0].elements.push(img);
            let pid = p.add_page();
            p.page_mut(pid.as_str())
                .unwrap()
                .elements
                .push(Element::text("el3", 0.0, 0.0, 100.0, 40.0, "第二页"));
            p.script.sections.push(crate::model::ScriptSection::new(
                "s1",
                "首先我们来讨论整个数据库系统，然后看架构。",
            ));
            p.script.sections[0].reveal_time = Some(3.25);
            p.assets.push(crate::model::AssetRef::new("figure-01.png", 800, 600));
        }
        handle.save().unwrap();

        let before = handle.project().clone();
        let reopened = store.open_project(&before.id).unwrap();
        assert_eq!(&before, reopened.project());
        // 逐字段抽查，避免 PartialEq 被将来新增的 #[serde(skip)] 字段悄悄绕过
        assert_eq!(reopened.project().pages.len(), 2);
        assert_eq!(reopened.project().pages[0].elements.len(), 2);
        assert_eq!(
            reopened.project().pages[0].elements[0].text_content(),
            Some("第一段文字")
        );
        assert_eq!(
            reopened.project().script.sections[0].reveal_time,
            Some(3.25)
        );
        assert_eq!(reopened.project().assets[0].width, 800);
    }

    #[test]
    fn rehearsal_json_roundtrip_includes_events_strokes_and_audio() {
        let (_tmp, store) = temp_store();
        let mut handle = store.create_project("演练往返").unwrap();

        let mut r = Rehearsal::new("r1", "第 1 次演练", "p1");
        r.duration = 42.5;
        r.audio = Some(AudioRef::mic(16_000, 1, 16_000 * 42));
        r.push_event(TimelineEvent::Reveal {
            element_id: "el1".into(),
            t: 8.42,
        });
        r.push_event(TimelineEvent::PageChange {
            page_id: "p1".into(),
            t: 0.0,
        });
        r.push_event(TimelineEvent::ScriptMark {
            section_id: "s1".into(),
            t: 0.0,
        });
        let mut stroke = Stroke::new("st1", "p1", "#e11d48", 3.0);
        stroke.push(10.0, 10.0, 12.01);
        stroke.push(20.0, 15.0, 12.08);
        r.strokes.push(stroke);

        // 写一个假音频文件，保证 load 时不会被判为"音频丢失"
        fs::create_dir_all(handle.rehearsal_dir("r1")).unwrap();
        fs::write(handle.audio_path("r1"), b"RIFF____WAVEfake").unwrap();

        handle.save_rehearsal(&r).unwrap();

        let loaded = handle.load_rehearsal("r1").unwrap();
        assert_eq!(loaded.duration, 42.5);
        assert_eq!(loaded.strokes.len(), 1);
        assert_eq!(loaded.strokes[0].points.len(), 2);
        assert_eq!(loaded.strokes[0].end_time, 12.08);
        // 事件在落盘前被排序
        assert!(crate::timeline::is_sorted(&loaded.events));
        assert_eq!(loaded.events[0].time(), 0.0);
        assert_eq!(loaded.audio.as_ref().unwrap().sample_rate, 16_000);
        assert_eq!(loaded.element_reveal_time("el1"), Some(8.42));

        // project.json 里的元信息也被刷新
        let reopened = store.open_project(handle.project().id.as_str()).unwrap();
        assert_eq!(reopened.meta().len(), 1);
        assert!(reopened.meta()[0].has_audio);
        assert_eq!(reopened.meta()[0].event_count, 3);
        assert_eq!(reopened.meta()[0].stroke_count, 1);
    }

    #[test]
    fn missing_audio_file_is_reported_as_none() {
        let (_tmp, store) = temp_store();
        let mut handle = store.create_project("缺音频").unwrap();
        let mut r = Rehearsal::new("r1", "演练", "p1");
        r.duration = 5.0;
        r.audio = Some(AudioRef::mic(44_100, 2, 44_100 * 2 * 5));
        // 故意不写 audio.wav
        handle.save_rehearsal(&r).unwrap();
        let loaded = handle.load_rehearsal("r1").unwrap();
        assert!(loaded.audio.is_none());
    }

    #[test]
    fn listing_projects_reads_disk() {
        let (_tmp, store) = temp_store();
        store.create_project("Alpha Talk").unwrap();
        store.create_project("Beta Talk").unwrap();
        let list = store.list_projects();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|(_, n)| n == "Alpha Talk"));
    }

    #[test]
    fn create_project_avoids_slug_collision() {
        let (_tmp, store) = temp_store();
        let a = store.create_project("Same Name").unwrap();
        let b = store.create_project("Same Name").unwrap();
        assert_ne!(a.project().id, b.project().id);
        assert_eq!(a.project().id, "same-name");
        assert_eq!(b.project().id, "same-name-2");
    }

    #[test]
    fn open_missing_project_errors() {
        let (_tmp, store) = temp_store();
        assert!(matches!(
            store.open_project("nope"),
            Err(StorageError::NotFound(_))
        ));
    }

    #[test]
    fn future_schema_version_is_rejected() {
        let (_tmp, store) = temp_store();
        let handle = store.create_project("未来版本").unwrap();
        let path = handle.project_json_path();
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(SCHEMA_VERSION + 5);
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
        assert!(matches!(
            store.open_project(&handle.project().id),
            Err(StorageError::Version(_))
        ));
    }

    #[test]
    fn resolve_asset_prefers_assets_dir() {
        let (_tmp, store) = temp_store();
        let handle = store.create_project("资源").unwrap();
        fs::write(handle.assets_dir().join("a.png"), b"x").unwrap();
        assert_eq!(handle.resolve_asset("a.png"), handle.assets_dir().join("a.png"));
        // 不存在时退回项目目录（不 panic）
        assert!(handle.resolve_asset("missing.png").ends_with("missing.png"));
    }

    #[test]
    fn open_or_create_reuses_existing() {
        let (_tmp, store) = temp_store();
        let first = store.create_project("第一个").unwrap();
        let again = store.open_or_create("随便").unwrap();
        assert_eq!(again.project().id, first.project().id);
    }
}
