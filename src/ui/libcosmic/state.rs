use std::collections::HashSet;
use std::time::Instant;

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
    pub source_folder_id: Option<String>,
    pub start_x: f32,
    pub start_y: f32,
    pub current_x: f32,
    pub current_y: f32,
    pub active: bool,
    pub over_folder_level: Option<usize>,
}

pub struct FolderLevel {
    pub folder_id: String,
    pub popup_x: f32,
    pub popup_y: f32,
    pub cursor_y: f32,
    pub cursor_over: bool,
    pub hovered_subfolder: Option<String>,
    pub hover_started: Option<Instant>,
}

pub struct FolderDropdownState {
    pub levels: Vec<FolderLevel>,
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
