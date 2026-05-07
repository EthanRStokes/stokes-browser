use cosmic::{Element, widget};
use cosmic::iced::{Alignment, Length};
use crate::ui::libcosmic::app::CosmicBrowserApp;
use crate::ui::libcosmic::messages::Message;

pub fn tab_bar_view(app: &CosmicBrowserApp) -> Element<'_, Message> {
    let mut row = widget::row::with_capacity(app.tab_order.len() + 1);

    for (i, tab_id) in app.tab_order.iter().enumerate() {
        let tab = app.tab_manager.get_tab(tab_id);
        let title = tab.map(|t| t.title.as_str()).unwrap_or("New Tab");
        let is_loading = tab.map(|t| t.is_loading).unwrap_or(false);
        let is_active = i == app.active_tab_index;

        let title_text = widget::text(if title.len() > 20 {
            format!("{}…", &title[..20])
        } else {
            title.to_string()
        });

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
            loading_indicator,
            title_text,
            close_btn,
        ]
        .spacing(4)
        .align_y(Alignment::Center);

        let tab_btn = if is_active {
            widget::button::custom(tab_content)
                .on_press(Message::SwitchTab(i))
                .class(cosmic::theme::Button::Suggested)
        } else {
            widget::button::custom(tab_content)
                .on_press(Message::SwitchTab(i))
                .class(cosmic::theme::Button::Text)
        };

        row = row.push(tab_btn);
    }

    let new_tab_btn = widget::button::icon(
        widget::icon::from_name("list-add-symbolic").size(16)
    )
    .on_press(Message::NewTab)
    .padding(4);

    row = row.push(new_tab_btn);

    Element::from(
        row.spacing(2)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fixed(40.0))
    )
}
