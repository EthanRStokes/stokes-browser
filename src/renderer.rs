// HTML renderer for drawing web content
use std::collections::HashMap;
use std::rc::Rc;
use std::cell::{RefCell, Cell};
use skia_safe::{Canvas, Paint, Color, Rect, Font, TextBlob, FontStyle, Typeface};
use crate::dom::{DomNode, NodeType, ElementData, ImageData, ImageLoadingState};
use crate::layout::{LayoutBox, BoxType};
use crate::css::{ComputedValues, BorderRadiusPx, TextDecoration};

/// HTML renderer that draws layout boxes to a canvas
pub struct HtmlRenderer {
    default_font: Font,
    heading_font: Font,
    text_paint: Paint,
    background_paint: Paint,
    border_paint: Paint,
    // Add font cache for different sizes - wrapped in RefCell for interior mutability
    font_cache: RefCell<HashMap<u32, Font>>, // key is font size as u32 (rounded)
    typeface: Typeface,
    // Cache for loaded background images
    background_image_cache: RefCell<HashMap<String, Option<skia_safe::Image>>>,
}

impl HtmlRenderer {
    pub fn new() -> Self {
        // Create default fonts - using a simpler approach that works with Skia 0.88.0
        let font_mgr = skia_safe::FontMgr::new();
        let typeface = font_mgr.legacy_make_typeface(None, FontStyle::default())
            .expect("Failed to create default typeface");
        let default_font = Font::new(typeface.clone(), 14.0);
        let heading_font = Font::new(typeface.clone(), 18.0);

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
            font_cache: RefCell::new(HashMap::new()),
            typeface,
            background_image_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Render a layout tree to the canvas
    pub fn render(
        &self,
        canvas: &Canvas,
        layout: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
        style_map: &HashMap<usize, ComputedValues>,
        scroll_x: f32,
        scroll_y: f32,
        scale_factor: f64,
    ) {
        // Save the current canvas state
        canvas.save();

        // Apply scroll offset by translating the canvas
        canvas.translate((-scroll_x, -scroll_y));

        // Render the layout tree with styles and scale factor
        self.render_box(canvas, layout, node_map, style_map, scale_factor);

        // Restore the canvas state
        canvas.restore();
    }

    /// Render a single layout box with CSS styles and scale factor
    fn render_box(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
        style_map: &HashMap<usize, ComputedValues>,
        scale_factor: f64,
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
                    self.render_element(canvas, layout_box, element_data, computed_styles, scale_factor);
                },
                NodeType::Text(_) => {
                    // Check if text node is inside a non-visual element
                    if !self.is_inside_non_visual_element(&dom_node) {
                        self.render_text_node(canvas, layout_box, computed_styles, scale_factor);
                    }
                },
                NodeType::Image(image_data) => {
                    self.render_image_node(canvas, layout_box, image_data, scale_factor);
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
                    self.render_box(canvas, child, node_map, style_map, scale_factor);
                }
            }
        }
    }

    /// Render an element with CSS styles applied
    fn render_element(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        element_data: &ElementData,
        computed_styles: Option<&ComputedValues>,
        scale_factor: f64,
    ) {
        let border_box = layout_box.dimensions.border_box();

        // Render box shadows first (behind the element)
        if let Some(styles) = computed_styles {
            self.render_box_shadows(canvas, &border_box, styles, scale_factor);
        }

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

        // Render background image if specified
        if let Some(styles) = computed_styles {
            self.render_background_image(canvas, &border_box, styles, scale_factor);
        }

        // Draw border if specified in styles or default for certain elements
        let mut should_draw_border = false;
        let mut border_paint = self.border_paint.clone();

        if let Some(styles) = computed_styles {
            // Check if border is specified in styles
            if styles.border.top > 0.0 || styles.border.right > 0.0 || 
               styles.border.bottom > 0.0 || styles.border.left > 0.0 {
                should_draw_border = true;
                // Use average border width for simplicity and apply scale factor
                let avg_border_width = (styles.border.top + styles.border.right +
                                      styles.border.bottom + styles.border.left) / 4.0;
                let scaled_border_width = avg_border_width * scale_factor as f32;
                border_paint.set_stroke_width(scaled_border_width);
            }
        }

        // Default border for certain elements with scaling
        if !should_draw_border {
            match element_data.tag_name.as_str() {
                "div" | "section" | "article" | "header" | "footer" => {
                    should_draw_border = true;
                    let scaled_border_width = 1.0 * scale_factor as f32;
                    border_paint.set_stroke_width(scaled_border_width);
                },
                _ => {}
            }
        }

        if should_draw_border {
            canvas.draw_rect(border_box, &border_paint);
        }

        // Add visual indicators for headings with scaled border width
        if element_data.tag_name.starts_with('h') {
            let mut heading_paint = Paint::default();
            heading_paint.set_color(Color::from_rgb(50, 50, 150));
            heading_paint.set_stroke(true);
            let scaled_heading_border = 2.0 * scale_factor as f32;
            heading_paint.set_stroke_width(scaled_heading_border);
            canvas.draw_rect(border_box, &heading_paint);
        }

        // Render rounded corners if border radius is specified
        if let Some(styles) = computed_styles {
            let border_radius_px = styles.border_radius.to_px(styles.font_size, 400.0);
            if border_radius_px.has_radius() {
                self.render_rounded_element(canvas, border_box, &border_radius_px, &bg_paint, if should_draw_border { Some(&border_paint) } else { None }, scale_factor);
                return; // Skip the regular rectangle drawing since we drew rounded shapes
            }
        }

        // Draw background (only if no rounded corners)
        canvas.draw_rect(border_box, &bg_paint);

        if should_draw_border {
            canvas.draw_rect(border_box, &border_paint);
        }
    }

    /// Render an element with rounded corners
    fn render_rounded_element(
        &self,
        canvas: &Canvas,
        rect: Rect,
        border_radius_px: &BorderRadiusPx,
        bg_paint: &Paint,
        border_paint: Option<&Paint>,
        scale_factor: f64,
    ) {
        // Apply scale factor to border radius values
        let scaled_top_left = border_radius_px.top_left * scale_factor as f32;
        let scaled_top_right = border_radius_px.top_right * scale_factor as f32;
        let scaled_bottom_right = border_radius_px.bottom_right * scale_factor as f32;
        let scaled_bottom_left = border_radius_px.bottom_left * scale_factor as f32;

        // For now, use uniform radius (average of all corners) for simplicity
        // Skia's add_round_rect method expects a tuple for radius
        let avg_radius = (scaled_top_left + scaled_top_right + scaled_bottom_right + scaled_bottom_left) / 4.0;

        // Create a path with rounded rectangle for background
        let mut bg_path = skia_safe::Path::new();
        bg_path.add_round_rect(rect, (avg_radius, avg_radius), None);

        // Draw the background with rounded corners
        canvas.draw_path(&bg_path, bg_paint);

        // Draw border if specified
        if let Some(border_paint) = border_paint {
            let mut border_path = skia_safe::Path::new();
            border_path.add_round_rect(rect, (avg_radius, avg_radius), None);
            canvas.draw_path(&border_path, border_paint);
        }
    }

    /// Render rounded corners for an element
    fn render_rounded_corners(
        &self,
        canvas: &Canvas,
        rect: Rect,
        border_radius: f32,
        paint: &Paint,
    ) {
        // Create a path with rounded rectangle
        let mut path = skia_safe::Path::new();
        path.add_round_rect(rect, (border_radius, border_radius), None);

        // Draw the path with the specified paint
        canvas.draw_path(&path, paint);
    }

    /// Render text with CSS styles applied and DPI scale factor
    fn render_text_node(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        computed_styles: Option<&ComputedValues>,
        scale_factor: f64,
    ) {
        if let Some(text) = &layout_box.content {
            let content_rect = layout_box.dimensions.content;

            // Create text paint with CSS colors and font properties
            let mut text_paint = self.text_paint.clone();
            let mut font_size = 14.0; // Default font size
            let mut text_align = crate::css::TextAlign::Left; // Default alignment

            if let Some(styles) = computed_styles {
                // Apply CSS color
                if let Some(text_color) = &styles.color {
                    text_paint.set_color(text_color.to_skia_color());
                }

                // Apply CSS font size
                font_size = styles.font_size;

                // Apply CSS text-align
                text_align = styles.text_align.clone();
            }

            // Apply DPI scaling to font size
            let scaled_font_size = font_size * scale_factor as f32;
            let line_height = scaled_font_size * 1.0; // 1.2x line height for better readability

            // Get or create font with the scaled size
            let font = self.get_font_for_size(scaled_font_size);

            // Split text by newlines to handle line breaks properly
            let lines: Vec<&str> = text.split('\n')
                .map(|line| line.trim_start()) // Remove leading whitespace from each line
                .collect();

            // Position text within the content area with scaled padding
            let scaled_padding = 2.0 * scale_factor as f32;
            let mut current_y = content_rect.top + scaled_font_size; // Start at baseline position

            // Render each line separately
            for line in lines {
                if let Some(text_blob) = TextBlob::new(line, &font) {
                    let text_bounds = text_blob.bounds();
                    let text_width = text_bounds.width();

                    // Calculate x position based on text-align
                    let start_x = match text_align {
                        crate::css::TextAlign::Left => content_rect.left + scaled_padding,
                        crate::css::TextAlign::Right => content_rect.right - text_width - scaled_padding,
                        crate::css::TextAlign::Center => content_rect.left + (content_rect.width() - text_width) / 2.0,
                        crate::css::TextAlign::Justify => {
                            // For now, justify is treated as left-align
                            // Full justify implementation would require word spacing adjustments
                            content_rect.left + scaled_padding
                        }
                    };

                    canvas.draw_text_blob(&text_blob, (start_x, current_y), &text_paint);

                    // Render text decorations if specified
                    if let Some(styles) = computed_styles {
                        self.render_text_decorations(
                            canvas,
                            &text_blob,
                            (start_x, current_y),
                            &styles.text_decoration,
                            &text_paint,
                            scaled_font_size,
                            scale_factor,
                        );
                    }
                }
                current_y += line_height; // Move to next line
            }
        }
    }

    /// Render text decorations (underline, overline, line-through)
    fn render_text_decorations(
        &self,
        canvas: &Canvas,
        text_blob: &TextBlob,
        text_position: (f32, f32),
        text_decoration: &TextDecoration,
        text_paint: &Paint,
        font_size: f32,
        scale_factor: f64,
    ) {
        // Skip if no decorations
        if matches!(text_decoration, TextDecoration::None) {
            return;
        }

        let text_bounds = text_blob.bounds();
        let text_width = text_bounds.width();
        let (text_x, text_y) = text_position;

        // Create decoration paint based on text paint
        let mut decoration_paint = text_paint.clone();
        decoration_paint.set_stroke(true);
        let decoration_thickness = (font_size / 16.0).max(1.0) * scale_factor as f32;
        decoration_paint.set_stroke_width(decoration_thickness);

        // Render underline
        if text_decoration.has_underline() {
            let underline_y = text_y + font_size * 0.1; // Position below baseline
            canvas.draw_line(
                (text_x, underline_y),
                (text_x + text_width, underline_y),
                &decoration_paint,
            );
        }

        // Render overline
        if text_decoration.has_overline() {
            let overline_y = text_y - font_size * 0.8; // Position above text
            canvas.draw_line(
                (text_x, overline_y),
                (text_x + text_width, overline_y),
                &decoration_paint,
            );
        }

        // Render line-through (strikethrough)
        if text_decoration.has_line_through() {
            let line_through_y = text_y - font_size * 0.3; // Position through middle of text
            canvas.draw_line(
                (text_x, line_through_y),
                (text_x + text_width, line_through_y),
                &decoration_paint,
            );
        }
    }

    /// Get or create a font for the specified size
    fn get_font_for_size(&self, size: f32) -> Font {
        let size_key = size.round() as u32;

        // Check cache first
        {
            let cache = self.font_cache.borrow();
            if let Some(font) = cache.get(&size_key) {
                return font.clone();
            }
        }

        // Create new font and cache it
        let font = Font::new(self.typeface.clone(), size);
        self.font_cache.borrow_mut().insert(size_key, font.clone());
        font
    }

    /// Render image content
    fn render_image_node(&self, canvas: &Canvas, layout_box: &LayoutBox, image_data: &ImageData, scale_factor: f64) {
        let content_rect = layout_box.dimensions.content;

        match &image_data.loading_state {
            ImageLoadingState::Loaded(_) => {
                // Try to get the cached decoded image
                // We need to work around the borrowing issue by cloning the image if available
                if let Some(cached_image) = &image_data.cached_image {
                    let mut paint = Paint::default();
                    paint.set_anti_alias(true);

                    // Draw the cached image scaled to fit the content rect
                    canvas.draw_image_rect(
                        cached_image,
                        None, // Use entire source image
                        content_rect,
                        &paint
                    );
                } else {
                    // No cached image available, show placeholder indicating decoding issue
                    self.render_image_placeholder(canvas, &content_rect, "Image decoding failed", scale_factor);
                }
            },
            ImageLoadingState::Loading => {
                // Show loading placeholder
                self.render_image_placeholder(canvas, &content_rect, "Loading...", scale_factor);
            },
            ImageLoadingState::Failed(error) => {
                // Show error placeholder
                self.render_image_placeholder(canvas, &content_rect, &format!("Error: {}", error), scale_factor);
            },
            ImageLoadingState::NotLoaded => {
                // Show placeholder with alt text or src
                let placeholder_text = if !image_data.alt.is_empty() {
                    &image_data.alt
                } else {
                    &image_data.src
                };
                self.render_image_placeholder(canvas, &content_rect, placeholder_text, scale_factor);
            }
        }
    }

    /// Render a placeholder for images (when not loaded, loading, or failed)
    fn render_image_placeholder(&self, canvas: &Canvas, rect: &Rect, text: &str, scale_factor: f64) {
        // Draw a light gray background
        let mut bg_paint = Paint::default();
        bg_paint.set_color(Color::from_rgb(240, 240, 240));
        canvas.draw_rect(*rect, &bg_paint);

        // Draw a border with scaled width
        let mut border_paint = Paint::default();
        border_paint.set_color(Color::from_rgb(180, 180, 180));
        border_paint.set_stroke(true);
        let scaled_border_width = 1.0 * scale_factor as f32;
        border_paint.set_stroke_width(scaled_border_width);
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

            // Scale the font size for placeholder text
            let scaled_font_size = 12.0 * scale_factor as f32;
            let placeholder_font = self.get_font_for_size(scaled_font_size);

            if let Some(text_blob) = TextBlob::new(&display_text, &placeholder_font) {
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
            let scaled_icon_stroke = 2.0 * scale_factor as f32;
            icon_paint.set_stroke_width(scaled_icon_stroke);

            let scaled_icon_size = 16.0 * scale_factor as f32;
            let icon_x = rect.left + (rect.width() - scaled_icon_size) / 2.0;
            let icon_y = rect.top + 8.0 * scale_factor as f32;
            let icon_rect = Rect::from_xywh(icon_x, icon_y, scaled_icon_size, scaled_icon_size);

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

    /// Render box shadows for an element
    fn render_box_shadows(
        &self,
        canvas: &Canvas,
        rect: &Rect,
        styles: &ComputedValues,
        scale_factor: f64,
    ) {
        // Render each box shadow
        for shadow in &styles.box_shadow {
            // Convert shadow to pixel values
            let shadow_px = shadow.to_px(styles.font_size, 400.0);

            // Skip if no visible shadow
            if !shadow_px.has_shadow() {
                continue;
            }

            // Apply scale factor to shadow properties
            let scaled_offset_x = shadow_px.offset_x * scale_factor as f32;
            let scaled_offset_y = shadow_px.offset_y * scale_factor as f32;
            let scaled_blur_radius = shadow_px.blur_radius * scale_factor as f32;
            let scaled_spread_radius = shadow_px.spread_radius * scale_factor as f32;

            // Create shadow paint
            let mut shadow_paint = Paint::default();
            shadow_paint.set_color(shadow_px.color.to_skia_color());
            shadow_paint.set_anti_alias(true);

            // Calculate shadow rectangle
            let shadow_rect = if shadow_px.inset {
                // Inset shadows are drawn inside the element
                Rect::from_xywh(
                    rect.left + scaled_offset_x + scaled_spread_radius,
                    rect.top + scaled_offset_y + scaled_spread_radius,
                    rect.width() - 2.0 * scaled_spread_radius,
                    rect.height() - 2.0 * scaled_spread_radius,
                )
            } else {
                // Outset shadows are drawn outside the element
                Rect::from_xywh(
                    rect.left + scaled_offset_x - scaled_spread_radius,
                    rect.top + scaled_offset_y - scaled_spread_radius,
                    rect.width() + 2.0 * scaled_spread_radius,
                    rect.height() + 2.0 * scaled_spread_radius,
                )
            };

            // For now, render a simple shadow without proper blur
            // In a full implementation, you'd use image filters for blur
            if scaled_blur_radius > 0.0 {
                // Simulate blur by drawing multiple offset rectangles with reduced opacity
                let blur_steps = (scaled_blur_radius / 2.0).max(1.0) as i32;
                let step_alpha = 1.0 / blur_steps as f32;

                for i in 0..blur_steps {
                    let blur_offset = i as f32 * 2.0;
                    let blur_rect = Rect::from_xywh(
                        shadow_rect.left - blur_offset,
                        shadow_rect.top - blur_offset,
                        shadow_rect.width() + 2.0 * blur_offset,
                        shadow_rect.height() + 2.0 * blur_offset,
                    );

                    // Reduce alpha for blur effect
                    let mut blur_paint = shadow_paint.clone();
                    let original_color = shadow_px.color.to_skia_color();
                    let alpha = (original_color.a() as f32 * step_alpha * 0.3) as u8;
                    blur_paint.set_color(skia_safe::Color::from_argb(
                        alpha,
                        original_color.r(),
                        original_color.g(),
                        original_color.b(),
                    ));

                    canvas.draw_rect(blur_rect, &blur_paint);
                }
            } else {
                // No blur, just draw the shadow directly
                canvas.draw_rect(shadow_rect, &shadow_paint);
            }
        }
    }

    /// Render background image for an element
    fn render_background_image(
        &self,
        canvas: &Canvas,
        rect: &Rect,
        styles: &ComputedValues,
        scale_factor: f64,
    ) {
        use crate::css::BackgroundImage;

        // Check if background-image is specified
        match &styles.background_image {
            BackgroundImage::None => {
                // No background image, nothing to do
                return;
            }
            BackgroundImage::Url(url) => {
                // Try to load and render the background image
                let image_opt = self.load_background_image(url);

                if let Some(image) = image_opt {
                    let mut paint = Paint::default();
                    paint.set_anti_alias(true);

                    // Draw the background image to cover the entire rect
                    // For now, we'll use a simple "cover" behavior
                    canvas.draw_image_rect(
                        &image,
                        None, // Use entire source image
                        *rect,
                        &paint
                    );
                }
            }
        }
    }

    /// Load a background image from URL (with caching)
    fn load_background_image(&self, url: &str) -> Option<skia_safe::Image> {
        // Check cache first
        {
            let cache = self.background_image_cache.borrow();
            if let Some(cached) = cache.get(url) {
                return cached.clone();
            }
        }

        // Try to load the image from file system
        let image_opt = self.load_image_from_path(url);

        // Cache the result (even if None)
        self.background_image_cache.borrow_mut().insert(url.to_string(), image_opt.clone());

        image_opt
    }

    /// Load an image from a file path
    fn load_image_from_path(&self, path: &str) -> Option<skia_safe::Image> {
        use std::fs;

        // Try to read the file
        match fs::read(path) {
            Ok(data) => {
                // Try to decode the image data
                match skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&data)) {
                    Some(image) => Some(image),
                    None => {
                        println!("Failed to decode background image: {}", path);
                        None
                    }
                }
            }
            Err(e) => {
                println!("Failed to load background image {}: {}", path, e);
                None
            }
        }
    }
}
