use skia_safe::{Canvas, Paint, Color, Rect, Font, TextBlob, Vector};
use winit::window::Window;
use std::sync::Arc;

/// Represents a UI component in the browser chrome
#[derive(Debug, Clone)]
pub enum UiComponent {
    Button {
        id: String,
        label: String,
        position: [f32; 2],
        size: [f32; 2],
        color: [f32; 3],
        hover_color: [f32; 3],
        is_hover: bool,
        is_active: bool,
    },
    TextField {
        id: String,
        text: String,
        position: [f32; 2],
        size: [f32; 2],
        color: [f32; 3],
        border_color: [f32; 3],
        has_focus: bool,
    },
    TabButton {
        id: String,
        title: String,
        position: [f32; 2],
        size: [f32; 2],
        color: [f32; 3],
        is_active: bool,
    }
}

impl UiComponent {
    /// Create a navigation button (back, forward, refresh)
    pub fn navigation_button(id: &str, label: &str, x_pos: f32) -> Self {
        UiComponent::Button {
            id: id.to_string(),
            label: label.to_string(),
            position: [x_pos, 0.9],
            size: [0.05, 0.05],
            color: [0.8, 0.8, 0.8],
            hover_color: [0.9, 0.9, 1.0],
            is_hover: false,
            is_active: false,
        }
    }

    /// Create an address bar
    pub fn address_bar(url: &str) -> Self {
        UiComponent::TextField {
            id: "address_bar".to_string(),
            text: url.to_string(),
            position: [0.15, 0.9],
            size: [0.7, 0.05],
            color: [1.0, 1.0, 1.0],
            border_color: [0.7, 0.7, 0.7],
            has_focus: false,
        }
    }

    /// Create a tab button
    pub fn tab(id: &str, title: &str, index: usize, is_active: bool) -> Self {
        let x_pos = -0.95 + (index as f32 * 0.15);
        UiComponent::TabButton {
            id: id.to_string(),
            title: title.to_string(),
            position: [x_pos, 0.95],
            size: [0.14, 0.04],
            color: if is_active { [0.9, 0.9, 0.9] } else { [0.7, 0.7, 0.7] },
            is_active,
        }
    }

    /// Check if a point is inside this component
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        match self {
            UiComponent::Button { position, size, .. } |
            UiComponent::TextField { position, size, .. } |
            UiComponent::TabButton { position, size, .. } => {
                x >= position[0] - size[0] / 2.0 &&
                x <= position[0] + size[0] / 2.0 &&
                y >= position[1] - size[1] / 2.0 &&
                y <= position[1] + size[1] / 2.0
            }
        }
    }

    /// Get component ID
    pub fn id(&self) -> &str {
        match self {
            UiComponent::Button { id, .. } |
            UiComponent::TextField { id, .. } |
            UiComponent::TabButton { id, .. } => id,
        }
    }
}

/// Represents the browser UI (chrome)
pub struct BrowserUI {
    components: Vec<UiComponent>,
}

impl BrowserUI {
    pub fn new(_skia_context: &skia_safe::gpu::DirectContext) -> Self {
        Self {
            components: vec![
                UiComponent::navigation_button("back", "<", 0.01),
                UiComponent::navigation_button("forward", ">", 0.07),
                UiComponent::navigation_button("refresh", "⟳", 0.13),
                UiComponent::address_bar("")
            ],
        }
    }

    /// Initialize rendering resources
    pub fn initialize_renderer(&mut self) {
        // No-op for Skia
    }

    /// Add a new tab
    pub fn add_tab(&mut self, id: &str, title: &str) {
        self.components.push(UiComponent::TabButton {
            id: id.to_string(),
            title: title.to_string(),
            position: [0.2 + 0.1 * (self.components.len() as f32), 0.8],
            size: [0.09, 0.07],
            color: [0.7, 0.7, 0.9],
            is_active: false,
        });
    }

    /// Update the address bar with a new URL
    pub fn update_address_bar(&mut self, url: &str) {
        for comp in &mut self.components {
            if let UiComponent::TextField { id, text, .. } = comp {
                if id == "address_bar" {
                    *text = url.to_string();
                }
            }
        }
    }

    /// Update tab title
    pub fn update_tab_title(&mut self, tab_id: &str, title: &str) {
        for comp in &mut self.components {
            if let UiComponent::TabButton { id, title: tab_title, .. } = comp {
                if id == tab_id {
                    *tab_title = title.to_string();
                }
            }
        }
    }

    /// Handle mouse click
    pub fn handle_click(&mut self, x: f32, y: f32) -> Option<String> {
        for comp in &self.components {
            match comp {
                UiComponent::Button { id, position, size, .. } |
                UiComponent::TabButton { id, position, size, .. } => {
                    let px = position[0];
                    let py = position[1];
                    let sx = size[0];
                    let sy = size[1];
                    if x >= px && x <= px + sx && y >= py && y <= py + sy {
                        return Some(id.clone());
                    }
                }
                UiComponent::TextField { id, position, size, .. } => {
                    let px = position[0];
                    let py = position[1];
                    let sx = size[0];
                    let sy = size[1];
                    if x >= px && x <= px + sx && y >= py && y <= py + sy {
                        return Some(id.clone());
                    }
                }
            }
        }
        None
    }

    /// Render the UI
    pub fn render(&self, canvas: &Canvas) {
        let mut paint = Paint::default();
        let font = Font::default();
        for comp in &self.components {
            match comp {
                UiComponent::Button { label, position, size, color, is_hover, .. } => {
                    let rect = Rect::from_xywh(
                        position[0] * canvas.image_info().width() as f32,
                        position[1] * canvas.image_info().height() as f32,
                        size[0] * canvas.image_info().width() as f32,
                        size[1] * canvas.image_info().height() as f32,
                    );
                    paint.set_color(Color::from_rgb(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                    ));
                    canvas.draw_rect(rect, &paint);
                    let blob = TextBlob::new(label, &font).unwrap();
                    canvas.draw_text_blob(&blob, (rect.left() + 5.0, rect.top() + 20.0), &paint);
                }
                UiComponent::TextField { text, position, size, color, border_color, .. } => {
                    let rect = Rect::from_xywh(
                        position[0] * canvas.image_info().width() as f32,
                        position[1] * canvas.image_info().height() as f32,
                        size[0] * canvas.image_info().width() as f32,
                        size[1] * canvas.image_info().height() as f32,
                    );
                    paint.set_color(Color::from_rgb(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                    ));
                    canvas.draw_rect(rect, &paint);
                    paint.set_color(Color::from_rgb(
                        (border_color[0] * 255.0) as u8,
                        (border_color[1] * 255.0) as u8,
                        (border_color[2] * 255.0) as u8,
                    ));
                    canvas.draw_rect(rect, &paint);
                    let blob = TextBlob::new(text, &font).unwrap();
                    canvas.draw_text_blob(&blob, (rect.left() + 5.0, rect.top() + 20.0), &paint);
                }
                UiComponent::TabButton { title, position, size, color, .. } => {
                    let rect = Rect::from_xywh(
                        position[0] * canvas.image_info().width() as f32,
                        position[1] * canvas.image_info().height() as f32,
                        size[0] * canvas.image_info().width() as f32,
                        size[1] * canvas.image_info().height() as f32,
                    );
                    paint.set_color(Color::from_rgb(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                    ));
                    canvas.draw_rect(rect, &paint);
                    let blob = TextBlob::new(title, &font).unwrap();
                    canvas.draw_text_blob(&blob, (rect.left() + 5.0, rect.top() + 20.0), &paint);
                }
            }
        }
    }
}
