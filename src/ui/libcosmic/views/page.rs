use cosmic::{Element, widget};
use cosmic::iced::Length;
use cosmic::iced::Alignment;
use cosmic::iced::widget::shader::Shader;
use crate::browser_frame_primitive::BrowserFrameProgram;
use crate::ui::libcosmic::app::CosmicBrowserApp;
use crate::ui::libcosmic::messages::{CosmicMouseButton, Message};
use cosmic::widget::mouse_area;

pub fn page_content_view(app: &CosmicBrowserApp) -> Element<'_, Message> {
    let image_widget: Element<'_, Message> = if let Some(primitive) = &app.current_frame {
        let program = BrowserFrameProgram { current: Some(primitive.clone()) };
        Element::from(
            Shader::new(program)
                .width(Length::Fill)
                .height(Length::Fill)
        )
    } else {
        Element::from(
            widget::container(widget::text("Loading..."))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
        )
    };

    Element::from(
        mouse_area(image_widget)
            .on_press(Message::PageClick)
            .on_release(Message::PageButtonReleased)
            .on_middle_press(Message::PagePointerPressed { button: CosmicMouseButton::Middle })
            .on_middle_release(Message::PagePointerReleased { button: CosmicMouseButton::Middle })
            .on_move(|pos: cosmic::iced::Point| Message::PageMouseMove { x: pos.x, y: pos.y })
            .on_scroll(|delta| {
                use cosmic::iced::mouse::ScrollDelta;
                let (dx, dy) = match delta {
                    ScrollDelta::Lines { x, y } => (x, y),
                    ScrollDelta::Pixels { x, y } => (x, y),
                };
                Message::PageScroll { delta_x: dx, delta_y: dy }
            })
    )
}
