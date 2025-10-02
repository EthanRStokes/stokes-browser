use skia_safe::{Canvas, Paint, Color, Rect, Font, TextBlob, Vector, FontStyle};
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
        cursor_position: usize,
        selection_start: Option<usize>,
        selection_end: Option<usize>,
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
            position: [x_pos, 0.01], // Move to top of window
            size: [0.03, 0.025],     // Smaller buttons for top bar
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
            position: [0.15, 0.01], // Move to top of window
            size: [0.7, 0.025],     // Smaller height for top bar
            color: [1.0, 1.0, 1.0],
            border_color: [0.7, 0.7, 0.7],
            has_focus: false,
            cursor_position: 0,
            selection_start: None,
            selection_end: None,
        }
    }

    /// Create a tab button
    pub fn tab(id: &str, title: &str, index: usize, is_active: bool) -> Self {
        let x_pos = 0.05 + (index as f32 * 0.12); // Better spacing from left edge
        UiComponent::TabButton {
            id: id.to_string(),
            title: title.to_string(),
            position: [x_pos, 0.04], // Position in the chrome bar
            size: [0.11, 0.03],
            color: if is_active { [0.95, 0.95, 0.95] } else { [0.8, 0.8, 0.8] },
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
    pub components: Vec<UiComponent>,
    pub scale_factor: f64,
}

impl BrowserUI {
    pub fn new(_skia_context: &skia_safe::gpu::DirectContext, scale_factor: f64) -> Self {
        Self {
            components: vec![
                UiComponent::navigation_button("back", "<", 0.01),
                UiComponent::navigation_button("forward", ">", 0.05),
                UiComponent::navigation_button("refresh", "⟳", 0.09),
                UiComponent::navigation_button("new_tab", "+", 0.87), // Add new tab button
                UiComponent::address_bar("")
            ],
            scale_factor,
        }
    }

    /// Initialize rendering resources
    pub fn initialize_renderer(&mut self) {
        // No-op for Skia
    }

    /// Add a new tab
    pub fn add_tab(&mut self, id: &str, title: &str) {
        let tab_count = self.components.iter().filter(|c| matches!(c, UiComponent::TabButton { .. })).count();

        // Set all existing tabs to inactive
        for comp in &mut self.components {
            if let UiComponent::TabButton { is_active, color, .. } = comp {
                *is_active = false;
                *color = [0.8, 0.8, 0.8]; // Inactive color
            }
        }

        // Add the new tab as active
        let new_tab = UiComponent::tab(id, title, tab_count, true);
        self.components.push(new_tab);
    }

    /// Set active tab
    pub fn set_active_tab(&mut self, tab_id: &str) {
        for comp in &mut self.components {
            if let UiComponent::TabButton { id, is_active, color, .. } = comp {
                if id == tab_id {
                    *is_active = true;
                    *color = [0.95, 0.95, 0.95]; // Active color
                } else {
                    *is_active = false;
                    *color = [0.8, 0.8, 0.8]; // Inactive color
                }
            }
        }
    }

    /// Remove a tab
    pub fn remove_tab(&mut self, tab_id: &str) -> bool {
        let initial_count = self.components.len();
        self.components.retain(|comp| {
            if let UiComponent::TabButton { id, .. } = comp {
                id != tab_id
            } else {
                true
            }
        });

        // Reposition remaining tabs
        let mut tab_index = 0;
        for comp in &mut self.components {
            if let UiComponent::TabButton { position, .. } = comp {
                position[0] = 0.05 + (tab_index as f32 * 0.12);
                tab_index += 1;
            }
        }

        self.components.len() < initial_count
    }

    /// Get all tab IDs
    pub fn get_tab_ids(&self) -> Vec<String> {
        self.components.iter()
            .filter_map(|comp| {
                if let UiComponent::TabButton { id, .. } = comp {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get active tab ID
    pub fn get_active_tab_id(&self) -> Option<String> {
        for comp in &self.components {
            if let UiComponent::TabButton { id, is_active: true, .. } = comp {
                return Some(id.clone());
            }
        }
        None
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

    /// Set focus to a specific component
    pub fn set_focus(&mut self, component_id: &str) {
        for comp in &mut self.components {
            match comp {
                UiComponent::TextField { id, has_focus, cursor_position, text, .. } => {
                    if id == component_id {
                        *has_focus = true;
                        *cursor_position = text.len(); // Move cursor to end
                    } else {
                        *has_focus = false;
                    }
                }
                _ => {}
            }
        }
    }

    /// Handle text input for focused component
    pub fn handle_text_input(&mut self, text: &str) {
        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus: true, text: field_text, cursor_position, .. } = comp {
                // Insert text at cursor position
                field_text.insert_str(*cursor_position, text);
                *cursor_position += text.len();
                break;
            }
        }
    }

    /// Handle key input for text editing
    pub fn handle_key_input(&mut self, key: &str) -> Option<String> {
        for comp in &mut self.components {
            if let UiComponent::TextField {
                id,
                has_focus: true,
                text: field_text,
                cursor_position,
                ..
            } = comp {
                match key {
                    "Backspace" => {
                        if *cursor_position > 0 {
                            field_text.remove(*cursor_position - 1);
                            *cursor_position -= 1;
                        }
                    }
                    "Delete" => {
                        if *cursor_position < field_text.len() {
                            field_text.remove(*cursor_position);
                        }
                    }
                    "ArrowLeft" => {
                        if *cursor_position > 0 {
                            *cursor_position -= 1;
                        }
                    }
                    "ArrowRight" => {
                        if *cursor_position < field_text.len() {
                            *cursor_position += 1;
                        }
                    }
                    "Home" => {
                        *cursor_position = 0;
                    }
                    "End" => {
                        *cursor_position = field_text.len();
                    }
                    "Enter" => {
                        // Return the field content for navigation
                        if id == "address_bar" {
                            return Some(field_text.clone());
                        }
                    }
                    _ => {}
                }
                break;
            }
        }
        None
    }

    /// Get the current text of a text field
    pub fn get_text_field_content(&self, field_id: &str) -> Option<String> {
        for comp in &self.components {
            if let UiComponent::TextField { id, text, .. } = comp {
                if id == field_id {
                    return Some(text.clone());
                }
            }
        }
        None
    }

    /// Update scale factor for DPI changes
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// Clear focus from all components
    pub fn clear_focus(&mut self) {
        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus, .. } = comp {
                *has_focus = false;
            }
        }
    }

    /// Render the UI
    pub fn render(&self, canvas: &Canvas) {
        let canvas_width = canvas.image_info().width() as f32;
        let canvas_height = canvas.image_info().height() as f32;

        // Draw browser chrome background bar at the top
        let mut chrome_paint = Paint::default();
        chrome_paint.set_color(Color::from_rgb(240, 240, 240)); // Light gray background
        let chrome_rect = Rect::from_xywh(0.0, 0.0, canvas_width, canvas_height * 0.08);
        canvas.draw_rect(chrome_rect, &chrome_paint);

        // Draw a bottom border for the chrome
        chrome_paint.set_color(Color::from_rgb(200, 200, 200));
        let border_rect = Rect::from_xywh(0.0, canvas_height * 0.08 - 1.0, canvas_width, 1.0);
        canvas.draw_rect(border_rect, &chrome_paint);

        let mut paint = Paint::default();
        let font_mgr = skia_safe::FontMgr::new();
        let typeface = font_mgr.legacy_make_typeface(None, FontStyle::default())
            .expect("Failed to create default typeface");
        
        // Apply scale factor to font size for proper DPI scaling
        let base_font_size = 18.0;
        let scaled_font_size = base_font_size * self.scale_factor as f32;
        let font = Font::new(typeface, scaled_font_size);

        // Scale other text rendering properties
        let text_padding = 5.0 * self.scale_factor as f32;
        let button_text_padding = 3.0 * self.scale_factor as f32;
        let text_offset_from_bottom = 5.0 * self.scale_factor as f32;
        let cursor_margin = 3.0 * self.scale_factor as f32;
        let cursor_stroke_width = 1.0 * self.scale_factor as f32;

        for comp in &self.components {
            match comp {
                UiComponent::Button { label, position, size, color, .. } => {
                    let rect = Rect::from_xywh(
                        position[0] * canvas_width,
                        position[1] * canvas_height,
                        size[0] * canvas_width,
                        size[1] * canvas_height,
                    );
                    paint.set_color(Color::from_rgb(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                    ));
                    canvas.draw_rect(rect, &paint);

                    // Draw button text in black with scaled padding
                    paint.set_color(Color::BLACK);
                    if let Some(blob) = TextBlob::new(label, &font) {
                        canvas.draw_text_blob(&blob, (rect.left() + button_text_padding, rect.bottom() - text_offset_from_bottom), &paint);
                    }
                }
                UiComponent::TextField { text, position, size, color, border_color, has_focus, cursor_position, .. } => {
                    let rect = Rect::from_xywh(
                        position[0] * canvas_width,
                        position[1] * canvas_height,
                        size[0] * canvas_width,
                        size[1] * canvas_height,
                    );

                    // Draw field background (brighter when focused)
                    let bg_color = if *has_focus {
                        Color::WHITE
                    } else {
                        Color::from_rgb(250, 250, 250)
                    };
                    paint.set_color(bg_color);
                    canvas.draw_rect(rect, &paint);

                    // Draw field border (blue when focused) with scaled stroke width
                    let border_color = if *has_focus {
                        Color::from_rgb(100, 150, 255)
                    } else {
                        Color::from_rgb(
                            (border_color[0] * 255.0) as u8,
                            (border_color[1] * 255.0) as u8,
                            (border_color[2] * 255.0) as u8,
                        )
                    };
                    paint.set_color(border_color);
                    paint.set_stroke(true);
                    paint.set_stroke_width(if *has_focus { 2.0 * self.scale_factor as f32 } else { 1.0 * self.scale_factor as f32 });
                    canvas.draw_rect(rect, &paint);
                    paint.set_stroke(false);

                    // Draw text content with scaled padding
                    paint.set_color(Color::BLACK);
                    if let Some(blob) = TextBlob::new(text, &font) {
                        canvas.draw_text_blob(&blob, (rect.left() + text_padding, rect.bottom() - text_offset_from_bottom), &paint);
                    }

                    // Draw cursor if focused
                    if *has_focus {
                        // Calculate cursor position in pixels
                        let text_before_cursor = if *cursor_position > 0 {
                            &text[..*cursor_position.min(&text.len())]
                        } else {
                            ""
                        };
                        let cursor_x = if let Some(text_blob) = TextBlob::new(text_before_cursor, &font) {
                            rect.left() + text_padding + text_blob.bounds().width()
                        } else {
                            rect.left() + text_padding
                        };

                        // Draw cursor line with scaled stroke width and margins
                        paint.set_color(Color::BLACK);
                        paint.set_stroke(true);
                        paint.set_stroke_width(cursor_stroke_width);
                        canvas.draw_line(
                            (cursor_x, rect.top() + cursor_margin),
                            (cursor_x, rect.bottom() - cursor_margin),
                            &paint
                        );
                        paint.set_stroke(false);
                    }
                }
                UiComponent::TabButton { title, position, size, color, .. } => {
                    let rect = Rect::from_xywh(
                        position[0] * canvas_width,
                        position[1] * canvas_height,
                        size[0] * canvas_width,
                        size[1] * canvas_height,
                    );
                    paint.set_color(Color::from_rgb(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                    ));
                    canvas.draw_rect(rect, &paint);

                    // Draw tab text with scaled padding
                    paint.set_color(Color::BLACK);
                    if let Some(blob) = TextBlob::new(title, &font) {
                        canvas.draw_text_blob(&blob, (rect.left() + text_padding, rect.bottom() - text_offset_from_bottom), &paint);
                    }
                }
            }
        }
    }
}
