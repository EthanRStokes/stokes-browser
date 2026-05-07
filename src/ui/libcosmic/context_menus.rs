use cosmic::widget;

pub enum BookmarkContextAction {
    OpenNewTab,
    OpenNewWindow,
    Edit,
    Cut,
    Copy,
    Paste,
    Delete,
}

pub fn build_bookmark_context_menu<M: Clone + 'static>(
    id: &str,
    has_clipboard: bool,
    is_folder: bool,
    map: impl Fn(String, BookmarkContextAction) -> M + Clone + 'static,
) -> Option<Vec<cosmic::widget::menu::Tree<M>>> {
    use cosmic::widget::menu::{Tree, menu_button};

    let make_item = |label: &'static str, msg: M| -> Tree<M> {
        Tree::from(cosmic::Element::from(
            menu_button(vec![cosmic::Element::from(widget::text(label))]).on_press(msg),
        ))
    };

    let divider = || -> Tree<M> {
        Tree::from(cosmic::Element::from(widget::divider::horizontal::light()))
    };

    let id = id.to_string();
    let mut items: Vec<Tree<M>> = Vec::new();

    if !is_folder {
        items.push(make_item("Open in new tab",    map(id.clone(), BookmarkContextAction::OpenNewTab)));
        items.push(make_item("Open in new window", map(id.clone(), BookmarkContextAction::OpenNewWindow)));
        items.push(divider());
    }

    items.push(make_item("Edit",   map(id.clone(), BookmarkContextAction::Edit)));
    items.push(divider());
    items.push(make_item("Cut",    map(id.clone(), BookmarkContextAction::Cut)));
    items.push(make_item("Copy",   map(id.clone(), BookmarkContextAction::Copy)));

    if has_clipboard {
        items.push(make_item("Paste", map(id.clone(), BookmarkContextAction::Paste)));
    }

    items.push(divider());
    items.push(make_item("Delete", map(id, BookmarkContextAction::Delete)));

    Some(items)
}
