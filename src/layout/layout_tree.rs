// Layout tree implementation
use super::box_model::{Dimensions, EdgeSizes};
use skia_safe::Rect;
use crate::dom::ImageData;

/// Type of layout box
#[derive(Debug, Clone, PartialEq)]
pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Text,
    Image(ImageData),
}

/// A box in the layout tree
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub box_type: BoxType,
    pub dimensions: Dimensions,
    pub children: Vec<LayoutBox>,
    pub node_id: usize,
    pub content: Option<String>, // For text nodes
    pub css_width: Option<crate::css::Length>, // CSS specified width
    pub css_height: Option<crate::css::Length>, // CSS specified height
}

impl LayoutBox {
    pub fn new(box_type: BoxType, node_id: usize) -> Self {
        Self {
            box_type,
            dimensions: Dimensions::new(),
            children: Vec::new(),
            node_id,
            content: None,
            css_width: None,
            css_height: None,
        }
    }

    /// Calculate layout
    pub fn layout(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        match &self.box_type {
            BoxType::Block => self.layout_block(container_width, container_height, offset_x, offset_y, scale_factor),
            BoxType::Inline => self.layout_inline(container_width, container_height, offset_x, offset_y, scale_factor),
            BoxType::InlineBlock => self.layout_inline_block(container_width, container_height, offset_x, offset_y, scale_factor),
            BoxType::Text => self.layout_text(container_width, container_height, offset_x, offset_y, scale_factor),
            BoxType::Image(data) => {
                self.layout_image(data.clone(), container_width, container_height, offset_x, offset_y, scale_factor)
            },
        }
    }

    /// Layout block elements with position offset (stack vertically)
    fn layout_block(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        // Scale margins, padding, and borders for high DPI
        self.scale_edge_sizes(scale_factor);

        // Calculate content area with proper offset positioning
        let content_x = offset_x + self.dimensions.margin.left + self.dimensions.border.left + self.dimensions.padding.left;
        let content_y = offset_y + self.dimensions.margin.top + self.dimensions.border.top + self.dimensions.padding.top;

        // Use CSS width if specified, otherwise use available container width
        let content_width = self.calculate_used_width(container_width, scale_factor);

        self.dimensions.content = Rect::from_xywh(content_x, content_y, content_width, 0.0);

        let mut current_y = content_y;
        let available_width = content_width;

        // Layout children vertically
        for child in &mut self.children {
            child.layout(available_width, container_height, content_x, current_y, scale_factor);
            current_y += child.dimensions.total_height();
        }

        // Update our content height based on children
        let content_height = if self.children.is_empty() {
            20.0 * scale_factor // Minimum height for empty blocks, scaled
        } else {
            current_y - content_y
        };

        self.dimensions.content = Rect::from_xywh(
            content_x,
            content_y,
            content_width,
            content_height
        );
    }

    /// Layout inline elements with position offset (flow horizontally)
    fn layout_inline(&mut self, container_width: f32, _container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        // Scale padding for high DPI
        self.dimensions.padding = EdgeSizes::uniform(2.0 * scale_factor);

        let content_x = offset_x + self.dimensions.padding.left;
        let content_y = offset_y + self.dimensions.padding.top;

        self.dimensions.content = Rect::from_xywh(
            content_x,
            content_y,
            container_width - self.dimensions.padding.left - self.dimensions.padding.right,
            20.0 * scale_factor // Scale line height
        );

        // Layout children horizontally
        let mut current_x = content_x;

        for child in &mut self.children {
            child.layout(100.0 * scale_factor, 20.0 * scale_factor, current_x, content_y, scale_factor);
            current_x += child.dimensions.total_width();
        }
    }

    /// Layout inline-block elements with position offset
    fn layout_inline_block(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        // Similar to block but flows inline - scale the max width
        self.layout_block((container_width).min(200.0 * scale_factor), container_height, offset_x, offset_y, scale_factor);
    }

    /// Layout text nodes with position offset
    fn layout_text(&mut self, container_width: f32, _container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        if let Some(text) = &self.content {
            // Handle newlines and calculate proper text dimensions - scale for high DPI
            let char_width = 8.0 * scale_factor; // Average character width, scaled
            let line_height = 16.0 * scale_factor; // Line height, scaled

            // Split text by newlines to handle line breaks properly
            let lines: Vec<&str> = text.split('\n').collect();
            let num_lines = lines.len().max(1);
            
            // Calculate width based on the longest line
            let max_line_width = lines.iter()
                .map(|line| line.len() as f32 * char_width)
                .fold(0.0, f32::max)
                .min(container_width);
            
            let text_width = if text.trim().is_empty() { 0.0 } else { max_line_width };
            let text_height = num_lines as f32 * line_height;

            self.dimensions.content = Rect::from_xywh(
                offset_x,
                offset_y,
                text_width,
                text_height
            );
        } else {
            self.dimensions.content = Rect::from_xywh(offset_x, offset_y, 0.0, 0.0);
        }
    }

    /// Layout image nodes with position offset
    fn layout_image(&mut self, data: ImageData, container_width: f32, _container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        // Default image dimensions
        let default_width = 150;
        let default_height = 100;

        // Use specified dimensions from HTML attributes if available
        let image_width = data.width.unwrap_or(default_width) as f32 * scale_factor;
        let image_height = data.height.unwrap_or(default_height) as f32 * scale_factor;

        // Set margins for inline-block behavior - scale for high DPI
        self.dimensions.margin = EdgeSizes::new(
            4.0 * scale_factor,
            4.0 * scale_factor,
            4.0 * scale_factor,
            4.0 * scale_factor
        );
        self.dimensions.padding = EdgeSizes::new(0.0, 0.0, 0.0, 0.0);

        // Calculate final position with margins
        let final_x = offset_x + self.dimensions.margin.left;
        let final_y = offset_y + self.dimensions.margin.top;

        self.dimensions.content = Rect::from_xywh(
            final_x,
            final_y,
            image_width,
            image_height
        );
    }

    /// Calculate the actual width this box should use, respecting CSS width values
    fn calculate_used_width(&self, container_width: f32, scale_factor: f32) -> f32 {
        if let Some(css_width) = &self.css_width {
            // Use the CSS-specified width, converting to pixels and scaling
            let parent_font_size = 16.0 * scale_factor; // Default font size scaled for length calculations
            css_width.to_px(parent_font_size, container_width)
        } else {
            // Use auto width (full container width minus margins, borders, padding)
            container_width - self.dimensions.padding.left - self.dimensions.padding.right
                - self.dimensions.border.left - self.dimensions.border.right
                - self.dimensions.margin.left - self.dimensions.margin.right
        }
    }

    /// Scale edge sizes (margins, padding, borders) for high DPI displays
    fn scale_edge_sizes(&mut self, scale_factor: f32) {
        // Only scale if not already scaled (to avoid double scaling)
        // We can check if any edge size is non-zero and not already scaled
        if self.dimensions.margin.top > 0.0 && self.dimensions.margin.top.fract() == 0.0 {
            self.dimensions.margin.top *= scale_factor;
            self.dimensions.margin.right *= scale_factor;
            self.dimensions.margin.bottom *= scale_factor;
            self.dimensions.margin.left *= scale_factor;
        }
        if self.dimensions.padding.top > 0.0 && self.dimensions.padding.top.fract() == 0.0 {
            self.dimensions.padding.top *= scale_factor;
            self.dimensions.padding.right *= scale_factor;
            self.dimensions.padding.bottom *= scale_factor;
            self.dimensions.padding.left *= scale_factor;
        }
        if self.dimensions.border.top > 0.0 && self.dimensions.border.top.fract() == 0.0 {
            self.dimensions.border.top *= scale_factor;
            self.dimensions.border.right *= scale_factor;
            self.dimensions.border.bottom *= scale_factor;
            self.dimensions.border.left *= scale_factor;
        }
    }

    /// Get all layout boxes in depth-first order
    pub fn get_all_boxes(&self) -> Vec<&LayoutBox> {
        let mut result = vec![self];
        for child in &self.children {
            result.extend(child.get_all_boxes());
        }
        result
    }

    /// Apply CSS styles to this layout box
    pub fn apply_styles(&mut self, styles: &crate::css::ComputedValues) {
        // Apply margin, padding, and border from computed styles
        self.dimensions.margin = styles.margin.clone();
        self.dimensions.padding = styles.padding.clone();
        self.dimensions.border = styles.border.clone();

        // Store CSS width and height values
        self.css_width = styles.width.clone();
        self.css_height = styles.height.clone();

        // Note: Other style properties like colors, fonts are handled in the renderer
        // Scale factor will be applied during layout phase
    }
}
