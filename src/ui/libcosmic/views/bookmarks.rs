use cosmic::{Element, widget};
use cosmic::iced::{Alignment, Length};
use cosmic::widget::mouse_area;
use crate::ui::libcosmic::app::CosmicBrowserApp;
use crate::ui::libcosmic::context_menus::{BookmarkContextAction, build_bookmark_context_menu};
use crate::ui::libcosmic::messages::Message;

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

        let cm = cosmic::widget::context_menu(
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
        );

        let item: Element<'_, Message> =
            if app.bookmark_edit.as_ref().map_or(false, |e| e.id == node.id) {
                let edit = app.bookmark_edit.as_ref().unwrap();
                let mut form_col = widget::column::with_capacity(3).spacing(4);
                form_col = form_col.push(
                    widget::text_input("Title", &edit.title)
                        .on_input(Message::BookmarkEditTitleChanged),
                );
                if !edit.is_folder {
                    form_col = form_col.push(
                        widget::text_input("URL", &edit.url)
                            .on_input(Message::BookmarkEditUrlChanged),
                    );
                }
                form_col = form_col.push(
                    widget::row![
                        widget::button::text("Save").on_press(Message::BookmarkEditCommit),
                        widget::button::text("Cancel").on_press(Message::BookmarkEditCancel),
                    ]
                    .spacing(4),
                );
                Element::from(
                    cosmic::widget::popover(cm)
                        .popup(widget::container(form_col).padding(8))
                        .position(cosmic::widget::popover::Position::Bottom)
                        .on_close(Message::BookmarkEditCancel),
                )
            } else {
                Element::from(cm)
            };

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

pub fn compute_drag_insert_index(
    items: &[crate::ui::bookmarks::BookmarkNode],
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
    items: &[crate::ui::bookmarks::BookmarkNode],
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

fn estimated_bookmark_width(node: &crate::ui::bookmarks::BookmarkNode) -> f32 {
    node.title.len() as f32 * 7.5 + 32.0
}
