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

pub struct BookmarkEditState {
    pub id: String,
    pub title: String,
    pub url: String,
    pub is_folder: bool,
}
