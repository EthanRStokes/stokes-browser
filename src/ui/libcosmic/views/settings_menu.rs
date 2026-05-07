use cosmic::{Element, widget};
use cosmic::iced::{Alignment, Background, Color, Length};
use cosmic::iced::widget::container::Style as ContainerStyle;
use cosmic::widget::mouse_area;
use crate::ui::libcosmic::app::{CosmicBrowserApp, VERSION};
use crate::ui::libcosmic::messages::Message;

const MENU_WIDTH: f32 = 240.0;
const MENU_TOP_OFFSET: f32 = 80.0;
const MENU_RIGHT_MARGIN: f32 = 8.0;

fn menu_item(label: &str, msg: Message) -> Element<'_, Message> {
    widget::button::custom(
        widget::text(label)
            .width(Length::Fill)
    )
    .width(Length::Fill)
    .class(cosmic::theme::Button::Text)
    .padding([4, 12])
    .on_press(msg)
    .into()
}

fn menu_item_with_shortcut<'a>(label: &'a str, shortcut: &'a str, msg: Message) -> Element<'a, Message> {
    widget::button::custom(
        widget::row![
            widget::text(label).width(Length::Fill),
            widget::text(shortcut)
                .size(12)
                .class(cosmic::theme::Text::Default),
        ]
        .align_y(Alignment::Center)
    )
    .width(Length::Fill)
    .class(cosmic::theme::Button::Text)
    .padding([4, 12])
    .on_press(msg)
    .into()
}

pub fn settings_dropdown_view(app: &CosmicBrowserApp) -> Element<'_, Message> {
    let close_bg: Element<'_, Message> = mouse_area(
        widget::container(widget::space::horizontal())
            .width(Length::Fill)
            .height(Length::Fill)
    )
    .on_press(Message::ToggleSettings)
    .into();

    let zoom_row: Element<'_, Message> = widget::row![
        widget::text("Zoom").width(Length::Fill),
        widget::button::icon(
            widget::icon::from_name("list-remove-symbolic").size(14)
        )
        .on_press(Message::ZoomOut)
        .padding([2, 6]),
        widget::button::custom(
            widget::text(format!("{}%", (app.zoom_level * 100.0).round() as u32)).size(13)
        )
            .on_press(Message::ZoomReset)
            .class(cosmic::theme::Button::Text)
            .padding([2, 6]),
        widget::button::icon(
            widget::icon::from_name("list-add-symbolic").size(14)
        )
        .on_press(Message::ZoomIn)
        .padding([2, 6]),
    ]
    .spacing(2)
    .align_y(Alignment::Center)
    .padding([4, 12])
    .into();

    let menu_content = widget::column![
        menu_item_with_shortcut("New tab", "Ctrl+T", Message::NewTab),
        menu_item_with_shortcut("New window", "Ctrl+N", Message::NewWindow),
        widget::divider::horizontal::light(),
        zoom_row,
        widget::divider::horizontal::light(),
        menu_item("About", Message::ShowAbout),
        menu_item("Settings", Message::ShowSettingsPage),
        widget::divider::horizontal::light(),
        menu_item("Exit", Message::Exit),
    ]
    .spacing(2);

    let menu_box = widget::container(menu_content)
        .padding([4, 0])
        .width(Length::Fixed(MENU_WIDTH))
        .class(cosmic::theme::Container::Primary);

    let positioned: Element<'_, Message> = widget::container(
        widget::column![
            widget::space::vertical().height(Length::Fixed(MENU_TOP_OFFSET)),
            widget::row![
                widget::space::horizontal().width(Length::Fill),
                menu_box,
                widget::space::horizontal().width(Length::Fixed(MENU_RIGHT_MARGIN)),
            ],
        ]
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into();

    cosmic::iced::widget::stack([close_bg, positioned]).into()
}

pub fn about_overlay_view(_app: &CosmicBrowserApp) -> Element<'_, Message> {
    let icon_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/com.ethanstokes.stokes-browser.png");

    let icon_elem: Element<'_, Message> = if icon_path.exists() {
        widget::image::Image::new(widget::image::Handle::from_path(icon_path))
            .width(Length::Fixed(64.0))
            .height(Length::Fixed(64.0))
            .into()
    } else {
        widget::icon::from_name("web-browser").size(64).icon().into()
    };

    let content = widget::column![
        widget::container(icon_elem).align_x(Alignment::Center).width(Length::Fill),
        widget::text("Stokes Browser").size(22),
        widget::text(format!("Version {}", VERSION)).size(14),
        widget::space::vertical().height(Length::Fixed(16.0)),
        widget::button::text("Close")
            .on_press(Message::CloseAbout),
    ]
    .spacing(8)
    .align_x(Alignment::Center);

    let dialog = widget::container(content)
        .padding(24)
        .width(Length::Fixed(320.0))
        .class(cosmic::theme::Container::Primary);

    let scrim = widget::container(
        widget::column![dialog].align_x(Alignment::Center)
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

pub fn settings_page_overlay_view(_app: &CosmicBrowserApp) -> Element<'_, Message> {
    let content = widget::column![
        widget::text("Settings").size(18),
        widget::divider::horizontal::light(),
        widget::space::vertical().height(Length::Fixed(8.0)),
        widget::button::text("Set as Default Browser")
            .on_press(Message::SetDefaultBrowser),
        widget::space::vertical().height(Length::Fixed(16.0)),
        widget::button::text("Close")
            .on_press(Message::CloseSettingsPage),
    ]
    .spacing(12)
    .align_x(Alignment::Start);

    let dialog = widget::container(content)
        .padding(24)
        .width(Length::Fixed(360.0))
        .class(cosmic::theme::Container::Primary);

    let scrim = widget::container(
        widget::column![dialog].align_x(Alignment::Center)
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
