use serde::{Deserialize, Serialize};

use super::{
    now_unix, slugify, AssetRef, Page, PageId, RehearsalMeta, ScriptDoc, Size, DEFAULT_PAGE_HEIGHT,
    DEFAULT_PAGE_WIDTH, SCHEMA_VERSION,
};

/// 项目 = Presentation Project（SPEC §12），不是 PPT 文件。
///
/// 落盘结构：
/// ```text
/// %APPDATA%\slidetrace\projects\<slug>\
/// ├── project.json
/// ├── assets\
/// │   ├── figure-01.png
/// │   └── logo.png
/// └── rehearsals\
///     └── r1\
///         ├── audio.wav
///         └── rehearsal.json
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "default_schema")]
    pub schema_version: u32,
    /// 目录名（slug），同时作为项目在磁盘上的唯一标识。
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub page_size: Size,
    #[serde(default)]
    pub pages: Vec<Page>,
    #[serde(default)]
    pub script: ScriptDoc,
    #[serde(default)]
    pub assets: Vec<AssetRef>,
    #[serde(default)]
    pub rehearsals: Vec<RehearsalMeta>,
    /// 上次停留的页面下标。
    #[serde(default)]
    pub current_page: usize,
    /// 单调递增的 id 分配器，保证 `el1` / `st1` 在项目内唯一。
    #[serde(default)]
    pub serial: u64,
    #[serde(default)]
    pub created_at_unix: u64,
    #[serde(default)]
    pub updated_at_unix: u64,
}

fn default_schema() -> u32 {
    SCHEMA_VERSION
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let id = slugify(&name);
        let now = now_unix();
        Self {
            schema_version: SCHEMA_VERSION,
            id,
            name,
            page_size: Size::new(DEFAULT_PAGE_WIDTH, DEFAULT_PAGE_HEIGHT),
            pages: vec![Page::new("p1", "第 1 页")],
            script: ScriptDoc::default(),
            assets: Vec::new(),
            rehearsals: Vec::new(),
            current_page: 0,
            serial: 1,
            created_at_unix: now,
            updated_at_unix: now,
        }
    }

    /// 分配一个项目内唯一的 id，例如 `el7`。
    ///
    /// 这里会真正检查 id 是否已被占用，而不是只依赖 `serial` 单调递增：
    /// 手工编辑过 project.json、或从模板导入过数据的项目，serial 与
    /// 实际 id 可能对不上，一旦撞 id，删除/命中测试就会误伤别的元素。
    pub fn next_id(&mut self, prefix: &str) -> String {
        loop {
            self.serial += 1;
            let id = format!("{}{}", prefix, self.serial);
            if !self.id_in_use(&id) {
                return id;
            }
        }
    }

    /// 这个 id 是否已经被页面元素或演练占用。
    pub fn id_in_use(&self, id: &str) -> bool {
        self.pages
            .iter()
            .any(|p| p.id == id || p.elements.iter().any(|e| e.id == id))
            || self.rehearsals.iter().any(|r| r.id == id)
    }

    pub fn touch(&mut self) {
        self.updated_at_unix = now_unix();
    }

    pub fn page(&self, id: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    pub fn page_mut(&mut self, id: &str) -> Option<&mut Page> {
        self.pages.iter_mut().find(|p| p.id == id)
    }

    pub fn page_index(&self, id: &str) -> Option<usize> {
        self.pages.iter().position(|p| p.id == id)
    }

    pub fn current_page_ref(&self) -> Option<&Page> {
        self.pages.get(self.current_page)
    }

    pub fn current_page_mut(&mut self) -> Option<&mut Page> {
        let i = self.current_page;
        self.pages.get_mut(i)
    }

    pub fn current_page_id(&self) -> PageId {
        self.pages
            .get(self.current_page)
            .map(|p| p.id.clone())
            .unwrap_or_default()
    }

    pub fn current_page_size(&self) -> Size {
        self.page_size
    }

    /// 在末尾追加一页并返回新页 id。
    pub fn add_page(&mut self) -> PageId {
        let id = self.next_id("p");
        let name = format!("第 {} 页", self.pages.len() + 1);
        self.pages.push(Page::new(&id, name));
        id
    }

    /// 删除一页；永远保留至少一页。返回是否真的删除了。
    pub fn remove_page(&mut self, index: usize) -> bool {
        if self.pages.len() <= 1 || index >= self.pages.len() {
            return false;
        }
        self.pages.remove(index);
        if self.current_page >= self.pages.len() {
            self.current_page = self.pages.len() - 1;
        }
        true
    }

    /// 全项目范围内查找元素，返回 (页码, 元素)。
    pub fn find_element(&self, element_id: &str) -> Option<(usize, &super::Element)> {
        self.pages.iter().enumerate().find_map(|(pi, p)| {
            p.element(element_id).map(|e| (pi, e))
        })
    }

    pub fn asset(&self, name: &str) -> Option<&AssetRef> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// 演示项目 / 冒烟测试用的固定构造。
    pub fn demo() -> Self {
        crate::demo::build_demo_project()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Element;

    #[test]
    fn ids_are_unique_and_monotonic() {
        let mut p = Project::new("Demo");
        let a = p.next_id("el");
        let b = p.next_id("el");
        assert_ne!(a, b);
        let c = p.next_id("st");
        assert_ne!(a, c);
    }

    #[test]
    fn cannot_delete_last_page() {
        let mut p = Project::new("Demo");
        assert!(!p.remove_page(0));
        let second = p.add_page();
        assert_eq!(p.pages.len(), 2);
        assert!(p.remove_page(1));
        assert_eq!(p.pages.len(), 1);
        assert_eq!(p.page_index(&second), None);
    }

    #[test]
    fn next_id_skips_ids_already_in_use() {
        let mut p = Project::new("Demo");
        p.pages[0].elements.push(Element::text("el2", 0.0, 0.0, 1.0, 1.0, "x"));
        // serial 从 1 开始：el2 已存在时必须跳到 el3，而不是撞上 el2
        assert_eq!(p.next_id("el"), "el3");
        // 页面 id 同样会被避让
        let pid = p.add_page();
        assert_ne!(pid, "el3");
        let fresh = p.next_id("el");
        assert!(!p.id_in_use(&fresh));
    }

    #[test]
    fn slug_comes_from_name() {
        assert_eq!(Project::new("My Talk").id, "my-talk");
    }
}
