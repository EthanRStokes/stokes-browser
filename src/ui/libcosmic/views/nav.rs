use cosmic::{Element, widget};
use cosmic::iced::{Alignment, Length};
use crate::ui::libcosmic::app::CosmicBrowserApp;
use crate::ui::libcosmic::messages::Message;

pub fn nav_bar_view(app: &CosmicBrowserApp) -> Element<'_, Message> {
    let back_btn = widget::button::icon(
        widget::icon::from_name("go-previous-symbolic").size(16)
    )
    .on_press(Message::GoBack)
    .padding(6);

    let fwd_btn = widget::button::icon(
        widget::icon::from_name("go-next-symbolic").size(16)
    )
    .on_press(Message::GoForward)
    .padding(6);

    let refresh_btn = widget::button::icon(
        widget::icon::from_name("view-refresh-symbolic").size(16)
    )
    .on_press(Message::Refresh)
    .padding(6);

    let home_btn = widget::button::icon(
        widget::icon::from_name("go-home-symbolic").size(16)
    )
    .on_press(Message::Home)
    .padding(6);

    let url_input = widget::text_input("Enter URL...", &app.url_input)
        .on_input(Message::UrlChanged)
        .on_submit(|_| Message::UrlSubmit)
        .width(Length::Fill);

    let bookmark_btn = widget::button::icon(
        widget::icon::from_name("user-bookmarks-symbolic").size(16)
    )
    .on_press(Message::AddBookmark)
    .padding(6);

    let settings_btn = widget::button::icon(
        widget::icon::from_name("emblem-system-symbolic").size(16)
    )
    .on_press(Message::ToggleSettings)
    .padding(6);

    Element::from(
        widget::row![
            back_btn,
            fwd_btn,
            refresh_btn,
            home_btn,
            url_input,
            bookmark_btn,
            settings_btn,
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fixed(40.0))
    )
}
