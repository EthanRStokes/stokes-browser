use cosmic::{Element, widget};
use cosmic::iced::{Alignment, Background, Color, Length};
use cosmic::iced::widget::container::Style as ContainerStyle;
use cosmic::widget::mouse_area;
use crate::ui::bookmarks::BookmarkNode;
use crate::ui::libcosmic::app::CosmicBrowserApp;
use crate::ui::libcosmic::context_menus::{BookmarkContextAction, build_bookmark_context_menu};
use crate::ui::libcosmic::messages::Message;
use crate::ui::libcosmic::state::{BookmarkEditState, PendingFolder};

pub const POPUP_WIDTH: f32 = 220.0;
pub const FOLDER_ITEM_HEIGHT: f32 = 28.0;
pub const BAR_TOP_OFFSET: f32 = 114.0;
const MAX_POPUP_HEIGHT: f32 = 400.0;

pub fn bookmarks_bar_view(app: &CosmicBrowserApp) -> Element<'_, Message> {
    let drag_insert_idx = app.bookmark_drag.as_ref()
        .filter(|d| d.active)
        .map(|d| compute_drag_insert_index(app.bookmarks.items(), d.current_x));

    let mut row = widget::row::with_capacity(32);

    for (i, node) in app.bookmarks.items().iter().enumerate() {
        if drag_insert_idx == Some(i) {
            row = row.push(widget::divider::vertical::light());
        }

        let icon_elem: Element<'_, Message> = if node.is_folder() {
            Element::from(widget::icon::from_name("folder-symbolic").size(14).icon())
        } else if let Some(handle) = app.bookmark_favicon_handles.get(&node.id) {
            Element::from(
                widget::image::Image::new(handle.clone())
                    .width(Length::Fixed(14.0))
                    .height(Length::Fixed(14.0))
            )
        } else {
            Element::from(widget::space::horizontal().width(Length::Fixed(14.0)))
        };

        let bm_btn = widget::button::custom(
            widget::row![
                icon_elem,
                widget::text(&node.title).size(13),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
        )
        .on_press(Message::BookmarkMousePressed { id: node.id.clone() })
        .class(cosmic::theme::Button::Text)
        .padding([2, 6]);

        let item: Element<'_, Message> = Element::from(
            cosmic::widget::context_menu(
                bm_btn,
                build_bookmark_context_menu(
                    &node.id,
                    app.bookmark_clipboard.is_some(),
                    node.is_folder(),
                    |id, action| match action {
                        BookmarkContextAction::OpenNewTab    => Message::BookmarkOpenNewTab(id),
                        BookmarkContextAction::OpenNewWindow => Message::BookmarkOpenNewWindow(id),
                        BookmarkContextAction::Edit          => Message::BookmarkEdit(id),
                        BookmarkContextAction::Cut           => Message::BookmarkCut(id),
                        BookmarkContextAction::Copy          => Message::BookmarkCopy(id),
                        BookmarkContextAction::Paste         => Message::BookmarkPasteAfter(id),
                        BookmarkContextAction::Delete        => Message::BookmarkDelete(id),
                    },
                ),
            )
        );

        row = row.push(item);
    }

    if drag_insert_idx == Some(app.bookmarks.items().len()) {
        row = row.push(widget::divider::vertical::light());
    }

    Element::from(
        mouse_area(
            row.spacing(2)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .height(Length::Fixed(32.0)),
        )
        .on_move(|pos: cosmic::iced::Point| Message::BookmarkBarMouseMove { x: pos.x })
        .on_enter(Message::BookmarkBarEntered)
        .on_exit(Message::BookmarkBarLeft),
    )
}

pub fn bookmark_edit_dialog_view(app: &CosmicBrowserApp) -> Element<'_, Message> {
    let edit = match app.bookmark_edit.as_ref() {
        Some(e) => e,
        None => return widget::space::horizontal().into(),
    };

    let name_row: Element<'_, Message> = widget::row![
        widget::text("Name").width(Length::Fixed(50.0)),
        widget::text_input("Name", &edit.title)
            .on_input(Message::BookmarkEditTitleChanged)
            .width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into();

    let url_row: Option<Element<'_, Message>> = if !edit.is_folder {
        Some(
            widget::row![
                widget::text("URL").width(Length::Fixed(50.0)),
                widget::text_input("URL", &edit.url)
                    .on_input(Message::BookmarkEditUrlChanged)
                    .width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        )
    } else {
        None
    };

    let tree_col: cosmic::Element<'_, Message> = render_folder_tree(app, edit).into();
    let tree_scroll = widget::scrollable(tree_col)
        .height(Length::Fixed(220.0))
        .width(Length::Fill);

    let bottom_row: Element<'_, Message> = widget::row![
        widget::button::text("New Folder")
            .on_press(Message::BookmarkEditNewFolder),
        widget::space::horizontal(),
        widget::button::text("Cancel")
            .on_press(Message::BookmarkEditCancel),
        widget::button::text("Save")
            .on_press(Message::BookmarkEditCommit)
            .class(cosmic::theme::Button::Suggested),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into();

    let mut content = widget::column::with_capacity(6).spacing(10);
    content = content.push(widget::text("Edit Bookmark").size(16));
    content = content.push(name_row);
    if let Some(url) = url_row {
        content = content.push(url);
    }
    content = content.push(widget::text("Folder").size(13));
    content = content.push(
        widget::container(tree_scroll)
            .width(Length::Fill)
            .class(cosmic::theme::Container::Secondary)
    );
    content = content.push(bottom_row);

    let dialog_box = widget::container(content)
        .padding(20)
        .width(Length::Fixed(500.0))
        .class(cosmic::theme::Container::Primary);

    let scrim = widget::container(
        widget::column![dialog_box]
            .align_x(Alignment::Center)
    )
    .align_x(Alignment::Center)
    .align_y(Alignment::Start)
    .width(Length::Fill)
    .height(Length::Fill)
    .class(cosmic::theme::Container::custom(|_theme| ContainerStyle {
        background: Some(Background::Color(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 })),
        ..Default::default()
    }));

    scrim.into()
}

fn render_folder_tree<'a>(app: &'a CosmicBrowserApp, edit: &'a BookmarkEditState) -> cosmic::Element<'a, Message> {
    let root_selected = edit.selected_folder_id.is_none();
    let root_btn_style = if root_selected {
        cosmic::theme::Button::Suggested
    } else {
        cosmic::theme::Button::Text
    };

    let root_row: Element<'a, Message> = widget::button::custom(
        widget::row![
            widget::icon::from_name("folder-symbolic").size(14).icon(),
            widget::text("Bookmarks bar").size(13),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
    )
    .on_press(Message::BookmarkEditFolderSelected(None))
    .class(root_btn_style)
    .padding([3, 8])
    .into();

    let mut col = widget::column::with_capacity(32).spacing(2);
    col = col.push(root_row);

    let top_level_folders: Vec<&BookmarkNode> = app.bookmarks.items()
        .iter()
        .filter(|n| n.is_folder())
        .collect();

    let top_pending: Vec<&PendingFolder> = edit.pending_folders
        .iter()
        .filter(|p| p.parent_id.is_none())
        .collect();

    for elem in render_folder_level(
        &top_level_folders,
        &top_pending,
        edit,
        1,
    ) {
        col = col.push(elem);
    }

    col.into()
}

fn render_folder_level<'a>(
    real_nodes: &[&'a BookmarkNode],
    pending_nodes: &[&'a PendingFolder],
    edit: &'a BookmarkEditState,
    depth: usize,
) -> Vec<Element<'a, Message>> {
    let indent = depth as f32 * 16.0;
    let mut items: Vec<Element<'a, Message>> = Vec::new();

    for node in real_nodes {
        let id = node.id.clone();
        let is_selected = edit.selected_folder_id.as_deref() == Some(&id);
        let is_expanded = edit.expanded_folders.contains(&id);

        let child_folders: Vec<&BookmarkNode> = node.children
            .iter()
            .filter(|c| c.is_folder())
            .collect();
        let child_pending: Vec<&PendingFolder> = edit.pending_folders
            .iter()
            .filter(|p| p.parent_id.as_deref() == Some(&id))
            .collect();
        let has_children = !child_folders.is_empty() || !child_pending.is_empty();

        let chevron: Element<'a, Message> = if has_children {
            let chevron_name = if is_expanded {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            };
            widget::button::custom(
                widget::icon::from_name(chevron_name).size(12).icon()
            )
            .on_press(Message::BookmarkEditToggleFolder(id.clone()))
            .class(cosmic::theme::Button::Text)
            .padding([2, 4])
            .into()
        } else {
            widget::space::horizontal().width(Length::Fixed(20.0)).into()
        };

        let btn_style = if is_selected {
            cosmic::theme::Button::Suggested
        } else {
            cosmic::theme::Button::Text
        };

        let folder_btn: Element<'a, Message> = widget::button::custom(
            widget::row![
                widget::icon::from_name("folder-symbolic").size(14).icon(),
                widget::text(node.title.clone()).size(13),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
        )
        .on_press(Message::BookmarkEditFolderSelected(Some(id.clone())))
        .class(btn_style)
        .padding([3, 8])
        .into();

        let indent_space: Element<'a, Message> = widget::space::horizontal().width(Length::Fixed(indent)).into();
        let row: Element<'a, Message> = widget::row![
            indent_space,
            chevron,
            folder_btn,
        ]
        .align_y(Alignment::Center)
        .into();

        items.push(row);

        if is_expanded && has_children {
            let sub_items = render_folder_level(
                &child_folders,
                &child_pending,
                edit,
                depth + 1,
            );
            items.extend(sub_items);
        }
    }

    for pf in pending_nodes {
        let temp_id = pf.temp_id.clone();
        let is_selected = edit.selected_folder_id.as_deref() == Some(&temp_id);
        let is_naming = edit.naming_folder_temp_id.as_deref() == Some(&temp_id);

        let content: Element<'a, Message> = if is_naming {
            widget::row![
                widget::icon::from_name("folder-new-symbolic").size(14).icon(),
                widget::text_input("Folder name", &pf.name)
                    .on_input(Message::BookmarkEditNewFolderNameChanged)
                    .on_submit(|_| Message::BookmarkEditNewFolderConfirm)
                    .width(Length::Fill),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        } else {
            let btn_style = if is_selected {
                cosmic::theme::Button::Suggested
            } else {
                cosmic::theme::Button::Text
            };
            widget::button::custom(
                widget::row![
                    widget::icon::from_name("folder-new-symbolic").size(14).icon(),
                    widget::text(if pf.name.is_empty() { "(unnamed)" } else { &pf.name }).size(13),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
            )
            .on_press(Message::BookmarkEditFolderSelected(Some(temp_id.clone())))
            .class(btn_style)
            .padding([3, 8])
            .into()
        };

        let pf_indent: Element<'a, Message> = widget::space::horizontal().width(Length::Fixed(indent)).into();
        let pf_chevron_space: Element<'a, Message> = widget::space::horizontal().width(Length::Fixed(20.0)).into();
        let row: Element<'a, Message> = widget::row![
            pf_indent,
            pf_chevron_space,
            content,
        ]
        .align_y(Alignment::Center)
        .into();

        items.push(row);
    }

    items
}

pub fn compute_drag_insert_index(
    items: &[BookmarkNode],
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
    items: &[BookmarkNode],
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

pub fn bar_x_of_id(items: &[BookmarkNode], id: &str) -> f32 {
    let mut x = 0.0f32;
    for node in items {
        if node.id == id {
            return x;
        }
        x += estimated_bookmark_width(node) + 2.0;
    }
    x
}

fn estimated_bookmark_width(node: &BookmarkNode) -> f32 {
    node.title.len() as f32 * 7.5 + 32.0
}

pub fn folder_dropdown_overlay_view(app: &CosmicBrowserApp) -> Element<'_, Message> {
    let dd = match app.folder_dropdown.as_ref() {
        Some(d) => d,
        None => return widget::space::horizontal().into(),
    };

    // Bottom layer: transparent click-outside-to-close surface
    let close_bg: Element<'_, Message> = mouse_area(
        widget::container(widget::space::horizontal())
            .width(Length::Fill)
            .height(Length::Fill)
    )
    .on_press(Message::BookmarkFolderClose)
    .into();

    let mut stack_layers: Vec<Element<'_, Message>> = vec![close_bg];

    for (level_idx, level) in dd.levels.iter().enumerate() {
        let folder = match app.bookmarks.get(&level.folder_id) {
            Some(f) => f,
            None => continue,
        };

        let drag_insert_idx: Option<usize> = app.bookmark_drag.as_ref()
            .filter(|d| d.active && d.over_folder_level == Some(level_idx))
            .map(|d| (d.current_y / FOLDER_ITEM_HEIGHT) as usize);

        let mut popup_col = widget::column::with_capacity(folder.children.len() * 2);

        for (child_idx, child) in folder.children.iter().enumerate() {
            if drag_insert_idx == Some(child_idx) {
                popup_col = popup_col.push(widget::divider::horizontal::light());
            }

            let icon_elem: Element<'_, Message> = if child.is_folder() {
                Element::from(widget::icon::from_name("folder-symbolic").size(14).icon())
            } else if let Some(handle) = app.bookmark_favicon_handles.get(&child.id) {
                Element::from(
                    widget::image::Image::new(handle.clone())
                        .width(Length::Fixed(14.0))
                        .height(Length::Fixed(14.0))
                )
            } else {
                Element::from(widget::space::horizontal().width(Length::Fixed(14.0)))
            };

            let child_is_folder = child.is_folder();

            let btn = widget::button::custom(
                widget::row![
                    icon_elem,
                    widget::text(&child.title).size(13),
                ]
                .spacing(4)
                .align_y(Alignment::Center)
            )
            .width(Length::Fill)
            .class(cosmic::theme::Button::Text)
            .padding([3, 8]);

            let item: Element<'_, Message> = if child_is_folder {
                let subfolder_id = child.id.clone();
                mouse_area(btn.on_press(Message::FolderSubfolderHovered {
                    level: level_idx,
                    subfolder_id: subfolder_id.clone(),
                }))
                .on_enter(Message::FolderSubfolderHovered {
                    level: level_idx,
                    subfolder_id,
                })
                .on_exit(Message::FolderSubfolderLeft(level_idx))
                .into()
            } else if let Some(url) = child.url.clone() {
                btn.on_press(Message::OpenBookmark(url)).into()
            } else {
                btn.into()
            };

            popup_col = popup_col.push(item);
        }

        if drag_insert_idx == Some(folder.children.len()) {
            popup_col = popup_col.push(widget::divider::horizontal::light());
        }

        let popup_height = MAX_POPUP_HEIGHT.min(folder.children.len() as f32 * FOLDER_ITEM_HEIGHT + 8.0);

        let inner = widget::container(
            widget::scrollable(popup_col)
                .height(Length::Fixed(popup_height))
        )
        .padding(4)
        .width(Length::Fixed(POPUP_WIDTH))
        .class(cosmic::theme::Container::Primary);

        let tracked = mouse_area(inner)
            .on_move(move |pos: cosmic::iced::Point| Message::FolderLevelMouseMove {
                level: level_idx,
                x: pos.x,
                y: pos.y,
            })
            .on_enter(Message::FolderLevelEntered(level_idx))
            .on_exit(Message::FolderLevelLeft(level_idx));

        let clamped_x = level.popup_x.max(0.0).min(app.window_size.0 as f32 - POPUP_WIDTH - 4.0);

        // Each level is positioned independently from window top-left using spacers,
        // then placed in the stack so all levels are z-stacked correctly.
        let positioned: Element<'_, Message> = widget::container(
            widget::column![
                widget::space::vertical().height(Length::Fixed(level.popup_y)),
                widget::row![
                    widget::space::horizontal().width(Length::Fixed(clamped_x)),
                    tracked,
                ]
            ]
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        stack_layers.push(positioned);
    }

    cosmic::iced::widget::stack(stack_layers).into()
}
