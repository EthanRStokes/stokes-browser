use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const STORAGE_VERSION: u32 = 1;
const BOOKMARKS_FILE: &str = "bookmarks.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BookmarkNode {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    #[serde(default)]
    pub children: Vec<BookmarkNode>,
}

impl BookmarkNode {
    pub fn is_folder(&self) -> bool {
        self.url.is_none()
    }

    pub fn bookmark(id: String, title: String, url: String) -> Self {
        Self {
            id,
            title,
            url: Some(url),
            children: Vec::new(),
        }
    }

    pub fn folder(id: String, title: String) -> Self {
        Self {
            id,
            title,
            url: None,
            children: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedBookmarks {
    #[serde(default = "default_storage_version")]
    version: u32,
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    items: Vec<BookmarkNode>,
}

const fn default_storage_version() -> u32 {
    STORAGE_VERSION
}

#[derive(Debug, Clone)]
pub struct BookmarkStore {
    next_id: u64,
    items: Vec<BookmarkNode>,
    path: PathBuf,
}

impl Default for BookmarkStore {
    fn default() -> Self {
        Self {
            next_id: 1,
            items: Vec::new(),
            path: bookmarks_file_path(),
        }
    }
}

impl BookmarkStore {
    pub fn load_from_disk() -> Self {
        let path = bookmarks_file_path();
        let mut store = Self {
            path,
            ..Self::default()
        };

        if let Ok(contents) = std::fs::read_to_string(&store.path) {
            if let Ok(persisted) = serde_json::from_str::<PersistedBookmarks>(&contents) {
                store.items = persisted.items;
                store.next_id = persisted.next_id.max(1);
                store.repair_next_id();
            }
        }

        store
    }

    pub fn save_to_disk(&self) {
        let payload = PersistedBookmarks {
            version: STORAGE_VERSION,
            next_id: self.next_id,
            items: self.items.clone(),
        };

        let Ok(json) = serde_json::to_string_pretty(&payload) else {
            return;
        };

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }

    pub fn items(&self) -> &[BookmarkNode] {
        &self.items
    }

    pub fn add_bookmark(&mut self, title: String, url: String, parent_id: Option<&str>) -> Result<String, String> {
        let id = self.allocate_id();
        let node = BookmarkNode::bookmark(id.clone(), title, url);

        match parent_id {
            Some(parent_id) => {
                let Some(parent) = Self::find_node_mut(&mut self.items, parent_id) else {
                    return Err("Folder not found".to_string());
                };
                if !parent.is_folder() {
                    return Err("Selected parent is not a folder".to_string());
                }
                parent.children.push(node);
            }
            None => {
                self.items.push(node);
            }
        }

        Ok(id)
    }

    pub fn add_folder(&mut self, title: String, parent_id: Option<&str>) -> Result<String, String> {
        let id = self.allocate_id();
        let node = BookmarkNode::folder(id.clone(), title);

        match parent_id {
            Some(parent_id) => {
                let Some(parent) = Self::find_node_mut(&mut self.items, parent_id) else {
                    return Err("Folder not found".to_string());
                };
                if !parent.is_folder() {
                    return Err("Selected parent is not a folder".to_string());
                }
                parent.children.push(node);
            }
            None => {
                self.items.push(node);
            }
        }

        Ok(id)
    }

    pub fn rename(&mut self, id: &str, new_title: String) -> Result<(), String> {
        let Some(node) = Self::find_node_mut(&mut self.items, id) else {
            return Err("Bookmark not found".to_string());
        };
        node.title = new_title;
        Ok(())
    }

    pub fn update_url(&mut self, id: &str, new_url: String) -> Result<(), String> {
        let Some(node) = Self::find_node_mut(&mut self.items, id) else {
            return Err("Bookmark not found".to_string());
        };
        if node.is_folder() {
            return Err("Folders do not have URLs".to_string());
        }
        node.url = Some(new_url);
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        if Self::remove_by_id(&mut self.items, id) {
            Ok(())
        } else {
            Err("Bookmark not found".to_string())
        }
    }

    pub fn get(&self, id: &str) -> Option<&BookmarkNode> {
        Self::find_node(&self.items, id)
    }

    fn allocate_id(&mut self) -> String {
        let id = format!("bm{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn repair_next_id(&mut self) {
        let max_id = Self::max_numeric_id(&self.items);
        self.next_id = self.next_id.max(max_id.saturating_add(1));
    }

    fn max_numeric_id(nodes: &[BookmarkNode]) -> u64 {
        let mut max_id = 0;
        for node in nodes {
            if let Some(num) = node.id.strip_prefix("bm").and_then(|v| v.parse::<u64>().ok()) {
                max_id = max_id.max(num);
            }
            max_id = max_id.max(Self::max_numeric_id(&node.children));
        }
        max_id
    }

    fn find_node<'a>(nodes: &'a [BookmarkNode], id: &str) -> Option<&'a BookmarkNode> {
        for node in nodes {
            if node.id == id {
                return Some(node);
            }
            if let Some(found) = Self::find_node(&node.children, id) {
                return Some(found);
            }
        }
        None
    }

    fn find_node_mut<'a>(nodes: &'a mut [BookmarkNode], id: &str) -> Option<&'a mut BookmarkNode> {
        for node in nodes {
            if node.id == id {
                return Some(node);
            }
            if let Some(found) = Self::find_node_mut(&mut node.children, id) {
                return Some(found);
            }
        }
        None
    }

    fn remove_by_id(nodes: &mut Vec<BookmarkNode>, id: &str) -> bool {
        if let Some(idx) = nodes.iter().position(|node| node.id == id) {
            nodes.remove(idx);
            return true;
        }

        for node in nodes.iter_mut() {
            if Self::remove_by_id(&mut node.children, id) {
                return true;
            }
        }

        false
    }
}

fn bookmarks_file_path() -> PathBuf {
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("stokes-browser");
    base.join(BOOKMARKS_FILE)
}

#[cfg(test)]
mod tests {
    use super::{BookmarkNode, BookmarkStore};

    #[test]
    fn add_and_delete_bookmark() {
        let mut store = BookmarkStore::default();
        let id = store
            .add_bookmark("Rust".to_string(), "https://www.rust-lang.org".to_string(), None)
            .expect("bookmark add should succeed");

        assert_eq!(store.items().len(), 1);
        assert_eq!(store.get(&id).and_then(|b| b.url.clone()), Some("https://www.rust-lang.org".to_string()));

        store.delete(&id).expect("delete should succeed");
        assert!(store.items().is_empty());
    }

    #[test]
    fn add_to_folder_and_rename() {
        let mut store = BookmarkStore::default();
        let folder_id = store.add_folder("Work".to_string(), None).expect("folder add should succeed");
        let bm_id = store
            .add_bookmark("Mail".to_string(), "https://mail.example.com".to_string(), Some(&folder_id))
            .expect("bookmark add should succeed");

        store.rename(&bm_id, "Inbox".to_string()).expect("rename should succeed");
        let folder = store.get(&folder_id).expect("folder should exist");
        assert_eq!(folder.children.len(), 1);
        assert_eq!(folder.children[0].title, "Inbox");
    }

    #[test]
    fn update_url_rejects_folders() {
        let mut store = BookmarkStore::default();
        let folder_id = store.add_folder("Folder".to_string(), None).expect("folder add should succeed");

        let result = store.update_url(&folder_id, "https://example.com".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn max_id_repair_keeps_allocations_unique() {
        let mut store = BookmarkStore::default();
        store.add_bookmark("One".to_string(), "https://1.example".to_string(), None).unwrap();

        // Simulate loaded state with an explicit higher id.
        let mut custom = BookmarkStore::default();
        custom.add_folder("F".to_string(), None).unwrap();
        let tree = vec![BookmarkNode::bookmark("bm42".to_string(), "X".to_string(), "https://x.example".to_string())];
        custom.next_id = 2;
        custom.items = tree;
        custom.repair_next_id();

        let new_id = custom.add_bookmark("Y".to_string(), "https://y.example".to_string(), None).unwrap();
        assert_eq!(new_id, "bm43");
    }
}

