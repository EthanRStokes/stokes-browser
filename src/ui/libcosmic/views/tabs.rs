use cosmic::{Element, widget};
use cosmic::iced::{Alignment, Length};
use cosmic::iced::widget::text::Wrapping;
use cosmic::widget::mouse_area;
use crate::ui::libcosmic::app::CosmicBrowserApp;
use crate::ui::libcosmic::messages::Message;

const TAB_SPACING: f32 = 2.0;
const NEW_TAB_BTN_WIDTH: f32 = 44.0;
const TAB_MAX_WIDTH: f32 = 200.0;
const TAB_MIN_WIDTH: f32 = 80.0;

pub fn compute_tab_width(window_width: f32, tab_count: usize) -> f32 {
    if tab_count == 0 {
        return TAB_MAX_WIDTH;
    }
    let available = (window_width - NEW_TAB_BTN_WIDTH - TAB_SPACING * tab_count as f32).max(0.0);
    (available / tab_count as f32).min(TAB_MAX_WIDTH).max(TAB_MIN_WIDTH)
}

pub fn find_tab_at_x(tab_count: usize, tab_width: f32, x: f32) -> Option<usize> {
    for i in 0..tab_count {
        let start = i as f32 * (tab_width + TAB_SPACING);
        if x >= start && x < start + tab_width {
            return Some(i);
        }
    }
    None
}

pub fn tab_drag_insert_index(tab_count: usize, tab_width: f32, x: f32) -> usize {
    for i in 0..tab_count {
        let mid = i as f32 * (tab_width + TAB_SPACING) + (tab_width + TAB_SPACING) / 2.0;
        if x < mid {
            return i;
        }
    }
    tab_count
}

pub fn tab_bar_view(app: &CosmicBrowserApp) -> Element<'_, Message> {
    let tab_count = app.tab_order.len();
    let tab_width = compute_tab_width(app.window_size.0 as f32, tab_count);

    let drag_insert_idx = app.tab_drag.as_ref()
        .filter(|d| d.active)
        .map(|d| tab_drag_insert_index(tab_count, tab_width, d.current_x));

    let mut row = widget::row::with_capacity(tab_count + 1);

    for (i, tab_id) in app.tab_order.iter().enumerate() {
        if drag_insert_idx == Some(i) {
            row = row.push(widget::divider::vertical::light());
        }

        let tab = app.tab_manager.get_tab(tab_id);
        let title = tab.map(|t| t.title.as_str()).unwrap_or("New Tab");
        let is_loading = tab.map(|t| t.is_loading).unwrap_or(false);
        let is_active = i == app.active_tab_index;

        // overhead: 16px icon + 24px close btn + 4*2 spacing + 16 padding ≈ 60px
        let max_chars = ((tab_width - 60.0) / 7.5).max(3.0) as usize;
        let display_title = if title.chars().count() > max_chars {
            let byte_end = title.char_indices()
                .nth(max_chars)
                .map(|(i, _)| i)
                .unwrap_or(title.len());
            format!("{}…", &title[..byte_end])
        } else {
            title.to_string()
        };

        let loading_indicator: Element<'_, Message> = if is_loading {
            Element::from(widget::text("⟳").size(14))
        } else if let Some(handle) = app.tab_favicon_handles.get(tab_id) {
            Element::from(
                widget::image::Image::new(handle.clone())
                    .width(Length::Fixed(16.0))
                    .height(Length::Fixed(16.0))
            )
        } else {
            Element::from(widget::space::horizontal().width(Length::Fixed(16.0)))
        };

        let close_btn = widget::button::icon(
            widget::icon::from_name("window-close-symbolic").size(12)
        )
        .on_press(Message::CloseTab(tab_id.clone()))
        .padding(2);

        let tab_content = widget::row![
            widget::row![
                loading_indicator,
                widget::text(display_title)
                    .width(Length::Fill)
                    .wrapping(Wrapping::None),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .width(Length::Fill),
            close_btn,
        ]
        .spacing(4)
        .align_y(Alignment::Center);

        let tab_btn = if is_active {
            widget::button::custom(tab_content)
                .on_press(Message::SwitchTab(i))
                .class(cosmic::theme::Button::Suggested)
                .width(Length::Fixed(tab_width))
        } else {
            widget::button::custom(tab_content)
                .on_press(Message::SwitchTab(i))
                .class(cosmic::theme::Button::Text)
                .width(Length::Fixed(tab_width))
        };

        row = row.push(tab_btn);
    }

    if drag_insert_idx == Some(tab_count) {
        row = row.push(widget::divider::vertical::light());
    }

    let new_tab_btn = widget::button::icon(
        widget::icon::from_name("list-add-symbolic").size(16)
    )
    .on_press(Message::NewTab)
    .padding(4);

    row = row.push(new_tab_btn);

    Element::from(
        mouse_area(
            row.spacing(TAB_SPACING)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .height(Length::Fixed(40.0)),
        )
        .on_move(|pos: cosmic::iced::Point| Message::TabBarMouseMove { x: pos.x })
        .on_enter(Message::TabBarEntered)
        .on_exit(Message::TabBarLeft),
    )
}
