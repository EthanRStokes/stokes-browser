// HTML renderer for drawing web content
use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use skia_safe::{Canvas, Paint, Color, Rect, Font, TextBlob, FontStyle, Typeface};
use crate::dom::{DomNode, NodeType, ElementData, ImageData, ImageLoadingState};
use crate::layout::{LayoutBox, BoxType};
use crate::css::ComputedValues;

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

    /// Render a layout tree to the canvas with CSS styling support
    pub fn render_with_styles(
        &self,
        canvas: &Canvas,
        layout: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
        style_map: &HashMap<usize, ComputedValues>,
        scroll_x: f32,
        scroll_y: f32,
    ) {
        // Save the current canvas state
        canvas.save();

        // Apply scroll offset by translating the canvas
        canvas.translate((-scroll_x, -scroll_y));

        // Render the layout tree with styles
        self.render_box_with_styles(canvas, layout, node_map, style_map);

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

    /// Render a single layout box with CSS styles
    fn render_box_with_styles(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
        style_map: &HashMap<usize, ComputedValues>,
    ) {
        // Get computed styles for this node
        let computed_styles = style_map.get(&layout_box.node_id);

        // Get the DOM node for this layout box
        if let Some(dom_node_rc) = node_map.get(&layout_box.node_id) {
            let dom_node = dom_node_rc.borrow();

            // Check if this node should be skipped from rendering
            if self.should_skip_rendering(&dom_node) {
                return; // Skip rendering this node and its children
            }

            match &dom_node.node_type {
                NodeType::Element(element_data) => {
                    self.render_element_with_styles(canvas, layout_box, element_data, computed_styles);
                },
                NodeType::Text(_) => {
                    // Check if text node is inside a non-visual element
                    if !self.is_inside_non_visual_element(&dom_node) {
                        self.render_text_node_with_styles(canvas, layout_box, computed_styles);
                    }
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

        // Render children only if this node should be rendered
        if let Some(dom_node_rc) = node_map.get(&layout_box.node_id) {
            let dom_node = dom_node_rc.borrow();
            if !self.should_skip_rendering(&dom_node) {
                for child in &layout_box.children {
                    self.render_box_with_styles(canvas, child, node_map, style_map);
                }
            }
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

    /// Render an element with CSS styles applied
    fn render_element_with_styles(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        element_data: &ElementData,
        computed_styles: Option<&ComputedValues>,
    ) {
        let border_box = layout_box.dimensions.border_box();

        // Create background paint with CSS colors
        let mut bg_paint = Paint::default();
        if let Some(styles) = computed_styles {
            if let Some(bg_color) = &styles.background_color {
                bg_paint.set_color(bg_color.to_skia_color());
            } else {
                // Fallback to default background colors
                self.set_default_background_color(&mut bg_paint, &element_data.tag_name);
            }
        } else {
            self.set_default_background_color(&mut bg_paint, &element_data.tag_name);
        }

        // Draw background
        canvas.draw_rect(border_box, &bg_paint);

        // Draw border if specified in styles or default for certain elements
        let mut should_draw_border = false;
        let mut border_paint = self.border_paint.clone();

        if let Some(styles) = computed_styles {
            // Check if border is specified in styles
            if styles.border.top > 0.0 || styles.border.right > 0.0 || 
               styles.border.bottom > 0.0 || styles.border.left > 0.0 {
                should_draw_border = true;
                // Use average border width for simplicity
                let avg_border_width = (styles.border.top + styles.border.right + 
                                      styles.border.bottom + styles.border.left) / 4.0;
                border_paint.set_stroke_width(avg_border_width);
            }
        }

        // Default border for certain elements
        if !should_draw_border {
            match element_data.tag_name.as_str() {
                "div" | "section" | "article" | "header" | "footer" => {
                    should_draw_border = true;
                },
                _ => {}
            }
        }

        if should_draw_border {
            canvas.draw_rect(border_box, &border_paint);
        }

        // Add visual indicators for headings
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

    /// Render text with CSS styles applied
    fn render_text_node_with_styles(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        computed_styles: Option<&ComputedValues>,
    ) {
        if let Some(text) = &layout_box.content {
            let content_rect = layout_box.dimensions.content;

            // Skip empty or whitespace-only text
            let trimmed_text = text.trim();
            if trimmed_text.is_empty() {
                return;
            }

            // Create font with CSS font-size if available
            let font = if let Some(styles) = computed_styles {
                self.create_font_from_styles(styles)
            } else {
                self.default_font.clone()
            };

            // Create text paint with CSS color if available
            let mut text_paint = self.text_paint.clone();
            if let Some(styles) = computed_styles {
                if let Some(text_color) = &styles.color {
                    text_paint.set_color(text_color.to_skia_color());
                }
            }

            let line_height = font.size() * 1.2; // 120% line height

            // Split text by newlines and render each line separately
            let lines: Vec<&str> = text.split('\n').collect();

            for (line_index, line) in lines.iter().enumerate() {
                // Skip empty lines but still advance the Y position
                if line.trim().is_empty() && lines.len() > 1 {
                    continue;
                }

                // Create text blob for this line
                if let Some(text_blob) = TextBlob::new(line.trim(), &font) {
                    // Position text within content area, with proper line spacing
                    let x = content_rect.left;
                    let y = content_rect.top + font.size() + (line_index as f32 * line_height);

                    canvas.draw_text_blob(&text_blob, (x, y), &text_paint);
                }
            }
        }
    }

    /// Create a font from CSS computed styles
    fn create_font_from_styles(&self, styles: &ComputedValues) -> Font {
        let font_mgr = skia_safe::FontMgr::new();
        let typeface = font_mgr.legacy_make_typeface(None, FontStyle::default())
            .expect("Failed to create typeface");
        
        Font::new(typeface, styles.font_size)
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
        // Add debugging information
        println!("Attempting to decode image data: {} bytes", image_bytes.len());

        if image_bytes.is_empty() {
            println!("Error: Empty image data");
            return None;
        }

        // Check the first few bytes to identify the image format
        if image_bytes.len() >= 4 {
            let header = &image_bytes[0..4];
            match header {
                [0xFF, 0xD8, 0xFF, ..] => println!("Detected JPEG image format"),
                [0x89, 0x50, 0x4E, 0x47] => println!("Detected PNG image format"),
                [0x47, 0x49, 0x46, 0x38] => println!("Detected GIF image format"),
                [0x42, 0x4D, ..] => println!("Detected BMP image format"),
                [0x52, 0x49, 0x46, 0x46] => println!("Detected WebP image format"),
                _ => println!("Unknown image format, header: {:02X} {:02X} {:02X} {:02X}",
                            header[0], header[1], header[2], header[3]),
            }
        }

        // Try to create Skia Data object
        let skia_data = skia_safe::Data::new_copy(image_bytes);
        // write image to a random file in the user folder
        let folder = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("skia_debug");
        let file_path = folder.join("image_".to_owned() + &format!("{:?}", std::time::Instant::now()) + ".png");
        if let Err(e) = std::fs::write(&file_path, image_bytes)
        {
            println!("Warning: Failed to write debug image data to file: {}", e);
        } else {
            println!("Wrote debug image data to {}", file_path.display());
        }
        if skia_data.is_empty() {
            println!("Error: Failed to create Skia Data object");
            return None;
        }

        println!("Skia Data created successfully: {} bytes", skia_data.size());

        // Try to decode the image using the primary method
        match skia_safe::Image::from_encoded(skia_data.clone()) {
            Some(image) => {
                println!("Successfully decoded image: {}x{}", image.width(), image.height());
                Some(image)
            }
            None => {
                println!("Error: Skia failed to decode image data with from_encoded");
                None
            }
        }
    }

    /// Set default background color for elements
    fn set_default_background_color(&self, paint: &mut Paint, tag_name: &str) {
        match tag_name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                paint.set_color(Color::from_rgb(240, 240, 250));
            },
            "div" => {
                paint.set_color(Color::from_rgb(248, 248, 248));
            },
            "p" => {
                paint.set_color(Color::WHITE);
            },
            "a" => {
                paint.set_color(Color::from_rgb(230, 240, 255));
            },
            _ => {
                paint.set_color(Color::WHITE);
            }
        }
    }

    /// Determine if rendering should be skipped for this node (and its children)
    fn should_skip_rendering(&self, dom_node: &DomNode) -> bool {
        // Skip rendering for non-visual elements like <style>, <script>, etc.
        match dom_node.node_type {
            NodeType::Element(ref element_data) => {
                let tag = element_data.tag_name.as_str();
                // Skip if the tag is one of the non-visual elements
                tag == "style" || tag == "script" || tag == "head" || tag == "title"
            },
            _ => false
        }
    }

    /// Check if the current node is inside a non-visual element
    fn is_inside_non_visual_element(&self, dom_node: &DomNode) -> bool {
        // Simple approach: just check if this is a text node and skip the parent traversal
        // Text nodes inside style/script tags should be filtered out during DOM building
        // or layout phase, not during rendering
        match dom_node.node_type {
            NodeType::Text(_) => {
                // For now, we'll be conservative and not traverse parents to avoid infinite loops
                // The better approach is to filter these out during layout building
                false
            },
            _ => false
        }
    }
}
