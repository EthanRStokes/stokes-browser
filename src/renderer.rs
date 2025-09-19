// HTML renderer for drawing web content
use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use skia_safe::{Canvas, Paint, Color, Rect, Font, TextBlob, FontStyle, Typeface};
use crate::dom::{DomNode, NodeType, ElementData, ImageData, ImageLoadingState};
use crate::layout::{LayoutBox, BoxType};

/// HTML renderer that draws layout boxes to a canvas
pub struct HtmlRenderer {
    default_font: Font,
    heading_font: Font,
    text_paint: Paint,
    background_paint: Paint,
    border_paint: Paint,
}

impl HtmlRenderer {
    pub fn new() -> Self {
        // Create default fonts - using a simpler approach that works with Skia 0.88.0
        let font_mgr = skia_safe::FontMgr::new();
        let typeface = font_mgr.legacy_make_typeface(None, FontStyle::default())
            .expect("Failed to create default typeface");
        let default_font = Font::new(typeface.clone(), 14.0);
        let heading_font = Font::new(typeface, 18.0);

        // Create paints
        let mut text_paint = Paint::default();
        text_paint.set_color(Color::BLACK);
        text_paint.set_anti_alias(true);

        let mut background_paint = Paint::default();
        background_paint.set_color(Color::WHITE);

        let mut border_paint = Paint::default();
        border_paint.set_color(Color::from_rgb(200, 200, 200));
        border_paint.set_stroke(true);
        border_paint.set_stroke_width(1.0);

        Self {
            default_font,
            heading_font,
            text_paint,
            background_paint,
            border_paint,
        }
    }

    /// Render a layout tree to the canvas
    pub fn render(
        &self,
        canvas: &Canvas,
        layout: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
    ) {
        self.render_with_scroll(canvas, layout, node_map, 0.0, 0.0);
    }

    /// Render a layout tree to the canvas with scroll offset
    pub fn render_with_scroll(
        &self,
        canvas: &Canvas,
        layout: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
        scroll_x: f32,
        scroll_y: f32,
    ) {
        // Save the current canvas state
        canvas.save();

        // Apply scroll offset by translating the canvas
        canvas.translate((-scroll_x, -scroll_y));

        // Render the layout tree
        self.render_box(canvas, layout, node_map);

        // Restore the canvas state
        canvas.restore();
    }

    /// Render a single layout box
    fn render_box(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
    ) {
        // Get the DOM node for this layout box
        if let Some(dom_node_rc) = node_map.get(&layout_box.node_id) {
            let dom_node = dom_node_rc.borrow();

            match &dom_node.node_type {
                NodeType::Element(element_data) => {
                    self.render_element(canvas, layout_box, element_data);
                },
                NodeType::Text(_) => {
                    self.render_text_node(canvas, layout_box);
                },
                NodeType::Image(image_data) => {
                    self.render_image_node(canvas, layout_box, image_data);
                },
                NodeType::Document => {
                    // Just render children for document
                },
                _ => {
                    // Skip other node types
                }
            }
        }

        // Render children
        for child in &layout_box.children {
            self.render_box(canvas, child, node_map);
        }
    }

    /// Render an element (with background, border, etc.)
    fn render_element(&self, canvas: &Canvas, layout_box: &LayoutBox, element_data: &ElementData) {
        let border_box = layout_box.dimensions.border_box();

        // Render background based on element type
        let mut bg_paint = self.background_paint.clone();
        match element_data.tag_name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                bg_paint.set_color(Color::from_rgb(240, 240, 250));
            },
            "div" => {
                bg_paint.set_color(Color::from_rgb(248, 248, 248));
            },
            "p" => {
                bg_paint.set_color(Color::WHITE);
            },
            "a" => {
                bg_paint.set_color(Color::from_rgb(230, 240, 255));
            },
            _ => {
                bg_paint.set_color(Color::WHITE);
            }
        }

        // Draw background
        canvas.draw_rect(border_box, &bg_paint);

        // Draw border for certain elements
        match element_data.tag_name.as_str() {
            "div" | "section" | "article" | "header" | "footer" => {
                canvas.draw_rect(border_box, &self.border_paint);
            },
            _ => {}
        }

        // Add visual indicators for different elements
        if element_data.tag_name.starts_with('h') {
            let mut heading_paint = Paint::default();
            heading_paint.set_color(Color::from_rgb(50, 50, 150));
            heading_paint.set_stroke(true);
            heading_paint.set_stroke_width(2.0);
            canvas.draw_rect(border_box, &heading_paint);
        }
    }

    /// Render text content
    fn render_text_node(&self, canvas: &Canvas, layout_box: &LayoutBox) {
        if let Some(text) = &layout_box.content {
            let content_rect = layout_box.dimensions.content;

            // Skip empty or whitespace-only text
            let trimmed_text = text.trim();
            if trimmed_text.is_empty() {
                return;
            }

            // Choose font based on parent context (simplified)
            let font = &self.default_font;
            let line_height = 16.0;

            // Split text by newlines and render each line separately
            let lines: Vec<&str> = text.split('\n').collect();

            for (line_index, line) in lines.iter().enumerate() {
                // Skip empty lines but still advance the Y position
                if line.trim().is_empty() && lines.len() > 1 {
                    continue;
                }

                // Create text blob for this line
                if let Some(text_blob) = TextBlob::new(line.trim(), font) {
                    // Position text within content area, with proper line spacing
                    let x = content_rect.left;
                    let y = content_rect.top + font.size() + (line_index as f32 * line_height);

                    canvas.draw_text_blob(&text_blob, (x, y), &self.text_paint);
                }
            }
        }
    }

    /// Update colors based on element attributes (simplified styling)
    fn get_element_colors(&self, element_data: &ElementData) -> (Color, Color) {
        let mut bg_color = Color::WHITE;
        let mut text_color = Color::BLACK;

        // Simple attribute-based styling
        if let Some(style) = element_data.attributes.get("style") {
            if style.contains("background-color:") {
                // Very simplified CSS parsing
                bg_color = Color::from_rgb(240, 240, 240);
            }
        }

        // Default colors based on tag
        match element_data.tag_name.as_str() {
            "a" => {
                text_color = Color::BLUE;
            },
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                text_color = Color::from_rgb(50, 50, 100);
                bg_color = Color::from_rgb(250, 250, 255);
            },
            _ => {}
        }

        (bg_color, text_color)
    }

    /// Render image content
    fn render_image_node(&self, canvas: &Canvas, layout_box: &LayoutBox, image_data: &ImageData) {
        let content_rect = layout_box.dimensions.content;

        match &image_data.loading_state {
            ImageLoadingState::Loaded(image_bytes) => {
                // Try to decode and render the actual image
                if let Some(skia_image) = self.decode_image_data(image_bytes) {
                    let mut paint = Paint::default();
                    paint.set_anti_alias(true);
                    
                    // Draw the image scaled to fit the content rect
                    canvas.draw_image_rect(
                        &skia_image,
                        None, // Use entire source image
                        content_rect,
                        &paint
                    );
                } else {
                    // Failed to decode, show placeholder
                    self.render_image_placeholder(canvas, &content_rect, "Failed to decode image");
                }
            },
            ImageLoadingState::Loading => {
                // Show loading placeholder
                self.render_image_placeholder(canvas, &content_rect, "Loading...");
            },
            ImageLoadingState::Failed(error) => {
                // Show error placeholder
                self.render_image_placeholder(canvas, &content_rect, &format!("Error: {}", error));
            },
            ImageLoadingState::NotLoaded => {
                // Show placeholder with alt text or src
                let placeholder_text = if !image_data.alt.is_empty() {
                    &image_data.alt
                } else {
                    &image_data.src
                };
                self.render_image_placeholder(canvas, &content_rect, placeholder_text);
            }
        }
    }

    /// Render a placeholder for images (when not loaded, loading, or failed)
    fn render_image_placeholder(&self, canvas: &Canvas, rect: &Rect, text: &str) {
        // Draw a light gray background
        let mut bg_paint = Paint::default();
        bg_paint.set_color(Color::from_rgb(240, 240, 240));
        canvas.draw_rect(*rect, &bg_paint);

        // Draw a border
        let mut border_paint = Paint::default();
        border_paint.set_color(Color::from_rgb(180, 180, 180));
        border_paint.set_stroke(true);
        border_paint.set_stroke_width(1.0);
        canvas.draw_rect(*rect, &border_paint);

        // Draw placeholder text
        if !text.is_empty() && rect.width() > 20.0 && rect.height() > 20.0 {
            let mut text_paint = Paint::default();
            text_paint.set_color(Color::from_rgb(100, 100, 100));
            text_paint.set_anti_alias(true);

            // Truncate text if it's too long
            let display_text = if text.len() > 20 {
                format!("{}...", &text[..17])
            } else {
                text.to_string()
            };

            if let Some(text_blob) = TextBlob::new(&display_text, &self.default_font) {
                let text_bounds = text_blob.bounds();
                
                // Center the text in the placeholder
                let text_x = rect.left + (rect.width() - text_bounds.width()) / 2.0;
                let text_y = rect.top + (rect.height() + text_bounds.height()) / 2.0;
                
                canvas.draw_text_blob(&text_blob, (text_x, text_y), &text_paint);
            }
        }

        // Draw a simple "broken image" icon if there's space
        if rect.width() > 40.0 && rect.height() > 40.0 {
            let mut icon_paint = Paint::default();
            icon_paint.set_color(Color::from_rgb(150, 150, 150));
            icon_paint.set_stroke(true);
            icon_paint.set_stroke_width(2.0);

            let icon_size = 16.0;
            let icon_x = rect.left + (rect.width() - icon_size) / 2.0;
            let icon_y = rect.top + 8.0;
            let icon_rect = Rect::from_xywh(icon_x, icon_y, icon_size, icon_size);
            
            // Draw a simple square with an X
            canvas.draw_rect(icon_rect, &icon_paint);
            canvas.draw_line(
                (icon_rect.left, icon_rect.top),
                (icon_rect.right, icon_rect.bottom),
                &icon_paint
            );
            canvas.draw_line(
                (icon_rect.right, icon_rect.top),
                (icon_rect.left, icon_rect.bottom),
                &icon_paint
            );
        }
    }

    /// Decode image data into a Skia image
    fn decode_image_data(&self, image_bytes: &[u8]) -> Option<skia_safe::Image> {
        // Try to decode the image using Skia's built-in decoders
        skia_safe::Image::from_encoded(skia_safe::Data::new_copy(image_bytes))
    }
}
