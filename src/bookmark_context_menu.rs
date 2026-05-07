use cosmic::widget;
use crate::cosmic_app::Message;

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

pub fn build_bookmark_context_menu(
    id: &str,
    has_clipboard: bool,
    is_folder: bool,
) -> Option<Vec<cosmic::widget::menu::Tree<Message>>> {
    use cosmic::widget::menu::{Tree, menu_button};

    let make_item = |label: &'static str, msg: Message| -> Tree<Message> {
        Tree::from(cosmic::Element::from(
            menu_button(vec![cosmic::Element::from(widget::text(label))]).on_press(msg),
        ))
    };

    let divider = || -> Tree<Message> {
        Tree::from(cosmic::Element::from(widget::divider::horizontal::light()))
    };

    let id = id.to_string();
    let mut items: Vec<Tree<Message>> = Vec::new();

    if !is_folder {
        items.push(make_item("Open in new tab",    Message::BookmarkOpenNewTab(id.clone())));
        items.push(make_item("Open in new window", Message::BookmarkOpenNewWindow(id.clone())));
        items.push(divider());
    }

    items.push(make_item("Edit",   Message::BookmarkEdit(id.clone())));
    items.push(divider());
    items.push(make_item("Cut",    Message::BookmarkCut(id.clone())));
    items.push(make_item("Copy",   Message::BookmarkCopy(id.clone())));

    if has_clipboard {
        items.push(make_item("Paste", Message::BookmarkPasteAfter(id.clone())));
    }

    items.push(divider());
    items.push(make_item("Delete", Message::BookmarkDelete(id)));

    Some(items)
}

pub fn compute_drag_insert_index(
    items: &[crate::bookmarks::BookmarkNode],
    current_x: f32,
) -> usize {
    let mut x = 0.0f32;
    for (i, node) in items.iter().enumerate() {
        let width = estimated_bookmark_width(node);
        if current_x < x + width / 2.0 {
            return i;
        }
        x += width + 2.0;
    }
    items.len()
}

pub fn find_bookmark_at_x(
    items: &[crate::bookmarks::BookmarkNode],
    x: f32,
) -> Option<String> {
    let mut cur_x = 0.0f32;
    for node in items {
        let width = estimated_bookmark_width(node);
        if x >= cur_x && x < cur_x + width {
            return Some(node.id.clone());
        }
        cur_x += width + 2.0;
    }
    None
}

fn estimated_bookmark_width(node: &crate::bookmarks::BookmarkNode) -> f32 {
    node.title.len() as f32 * 7.5 + 32.0
}