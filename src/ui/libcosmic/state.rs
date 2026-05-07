use std::collections::HashSet;

pub struct TabDragState {
    pub index: usize,
    pub start_x: f32,
    pub current_x: f32,
    pub active: bool,
}

pub struct BookmarkClipboardEntry {
    pub id: String,
    pub is_cut: bool,
}

pub struct BookmarkDragState {
    pub id: String,
    pub start_x: f32,
    pub current_x: f32,
    pub active: bool,
}

pub struct PendingFolder {
    pub temp_id: String,
    pub parent_id: Option<String>,
    pub name: String,
}

pub struct BookmarkEditState {
    pub id: String,
    pub title: String,
    pub url: String,
    pub is_folder: bool,
    pub selected_folder_id: Option<String>,
    pub expanded_folders: HashSet<String>,
    pub pending_folders: Vec<PendingFolder>,
    pub naming_folder_temp_id: Option<String>,
    pub next_temp_id: u32,
}
