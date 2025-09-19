// Layout tree implementation
use super::box_model::{Dimensions, EdgeSizes};
use skia_safe::{Rect, Point, Size};

/// Type of layout box
#[derive(Debug, Clone, PartialEq)]
pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Text,
    Image,
}

/// A box in the layout tree
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub box_type: BoxType,
    pub dimensions: Dimensions,
    pub children: Vec<LayoutBox>,
    pub node_id: usize,
    pub content: Option<String>, // For text nodes
}

impl LayoutBox {
    pub fn new(box_type: BoxType, node_id: usize) -> Self {
        Self {
            box_type,
            dimensions: Dimensions::new(),
            children: Vec::new(),
            node_id,
            content: None,
        }
    }

    /// Calculate layout for this box and its children
    pub fn layout(&mut self, container_width: f32, container_height: f32) {
        self.layout_with_position(container_width, container_height, 0.0, 0.0);
    }

    /// Calculate layout with explicit position offset
    pub fn layout_with_position(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32) {
        match self.box_type {
            BoxType::Block => self.layout_block_with_position(container_width, container_height, offset_x, offset_y),
            BoxType::Inline => self.layout_inline_with_position(container_width, container_height, offset_x, offset_y),
            BoxType::InlineBlock => self.layout_inline_block_with_position(container_width, container_height, offset_x, offset_y),
            BoxType::Text => self.layout_text_with_position(container_width, container_height, offset_x, offset_y),
            BoxType::Image => self.layout_image_with_position(container_width, container_height, offset_x, offset_y),
        }
    }

    /// Layout block elements (stack vertically)
    fn layout_block(&mut self, container_width: f32, container_height: f32) {
        // Set default dimensions - start with proper positioning
        self.dimensions.padding = EdgeSizes::uniform(8.0); // Default padding
        self.dimensions.margin = EdgeSizes::new(8.0, 0.0, 8.0, 0.0); // Top/bottom margin

        // Calculate content area after accounting for padding and margin
        let content_x = self.dimensions.margin.left + self.dimensions.border.left + self.dimensions.padding.left;
        let content_y = self.dimensions.margin.top + self.dimensions.border.top + self.dimensions.padding.top;
        let content_width = container_width - self.dimensions.padding.left - self.dimensions.padding.right
            - self.dimensions.border.left - self.dimensions.border.right
            - self.dimensions.margin.left - self.dimensions.margin.right;

        self.dimensions.content = Rect::from_xywh(content_x, content_y, content_width, 0.0);

        let mut current_y = content_y;
        let available_width = content_width;

        // Layout children vertically
        for child in &mut self.children {
            // Position child at current Y position
            child.dimensions.content = Rect::from_xywh(content_x, current_y, available_width, 0.0);

            child.layout(available_width, container_height);

            // Move to next position based on child's total height
            current_y += child.dimensions.total_height();
        }

        // Update our content height based on children
        let content_height = if self.children.is_empty() {
            20.0 // Minimum height for empty blocks
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

    /// Layout inline elements (flow horizontally)
    fn layout_inline(&mut self, container_width: f32, _container_height: f32) {
        // Simplified inline layout
        self.dimensions.content = Rect::from_xywh(0.0, 0.0, container_width, 20.0);
        self.dimensions.padding = EdgeSizes::uniform(2.0);

        // Layout children horizontally (simplified)
        let mut current_x = self.dimensions.content.left + self.dimensions.padding.left;

        for child in &mut self.children {
            child.dimensions.content.offset((current_x, self.dimensions.content.top + self.dimensions.padding.top));
            child.layout(100.0, 20.0); // Fixed size for simplicity
            current_x += child.dimensions.total_width();
        }
    }

    /// Layout inline-block elements
    fn layout_inline_block(&mut self, container_width: f32, container_height: f32) {
        // Similar to block but flows inline
        self.layout_block(container_width.min(200.0), container_height);
    }

    /// Layout text nodes
    fn layout_text(&mut self, container_width: f32, _container_height: f32) {
        if let Some(text) = &self.content {
            // Handle newlines and calculate proper text dimensions
            let char_width = 8.0; // Average character width
            let line_height = 16.0;
            
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
                0.0,
                0.0,
                text_width,
                text_height
            );
        } else {
            self.dimensions.content = Rect::from_xywh(0.0, 0.0, 0.0, 0.0);
        }
    }

    /// Layout block elements with position offset (stack vertically)
    fn layout_block_with_position(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32) {
        // Set default dimensions
        self.dimensions.padding = EdgeSizes::uniform(8.0); // Default padding
        self.dimensions.margin = EdgeSizes::new(8.0, 0.0, 8.0, 0.0); // Top/bottom margin

        // Calculate content area with proper offset positioning
        let content_x = offset_x + self.dimensions.margin.left + self.dimensions.border.left + self.dimensions.padding.left;
        let content_y = offset_y + self.dimensions.margin.top + self.dimensions.border.top + self.dimensions.padding.top;
        let content_width = container_width - self.dimensions.padding.left - self.dimensions.padding.right
            - self.dimensions.border.left - self.dimensions.border.right
            - self.dimensions.margin.left - self.dimensions.margin.right;

        self.dimensions.content = Rect::from_xywh(content_x, content_y, content_width, 0.0);

        let mut current_y = content_y;
        let available_width = content_width;

        // Layout children vertically
        for child in &mut self.children {
            child.layout_with_position(available_width, container_height, content_x, current_y);
            current_y += child.dimensions.total_height();
        }

        // Update our content height based on children
        let content_height = if self.children.is_empty() {
            20.0 // Minimum height for empty blocks
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
    fn layout_inline_with_position(&mut self, container_width: f32, _container_height: f32, offset_x: f32, offset_y: f32) {
        self.dimensions.padding = EdgeSizes::uniform(2.0);

        let content_x = offset_x + self.dimensions.padding.left;
        let content_y = offset_y + self.dimensions.padding.top;

        self.dimensions.content = Rect::from_xywh(content_x, content_y, container_width - self.dimensions.padding.left - self.dimensions.padding.right, 20.0);

        // Layout children horizontally
        let mut current_x = content_x;

        for child in &mut self.children {
            child.layout_with_position(100.0, 20.0, current_x, content_y);
            current_x += child.dimensions.total_width();
        }
    }

    /// Layout inline-block elements with position offset
    fn layout_inline_block_with_position(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32) {
        // Similar to block but flows inline
        self.layout_block_with_position(container_width.min(200.0), container_height, offset_x, offset_y);
    }

    /// Layout text nodes with position offset
    fn layout_text_with_position(&mut self, container_width: f32, _container_height: f32, offset_x: f32, offset_y: f32) {
        if let Some(text) = &self.content {
            // Handle newlines and calculate proper text dimensions
            let char_width = 8.0; // Average character width
            let line_height = 16.0;
            
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
    fn layout_image_with_position(&mut self, container_width: f32, _container_height: f32, offset_x: f32, offset_y: f32) {
        // Default image dimensions
        let default_width = 150.0;
        let default_height = 100.0;

        // Use specified dimensions from HTML attributes if available
        let image_width = container_width.min(default_width);
        let image_height = default_height;

        // Set margins for inline-block behavior
        self.dimensions.margin = EdgeSizes::new(4.0, 4.0, 4.0, 4.0);
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
        
        // Note: Other style properties like colors, fonts are handled in the renderer
    }
}
