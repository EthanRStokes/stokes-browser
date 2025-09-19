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
        match self.box_type {
            BoxType::Block => self.layout_block(container_width, container_height),
            BoxType::Inline => self.layout_inline(container_width, container_height),
            BoxType::InlineBlock => self.layout_inline_block(container_width, container_height),
            BoxType::Text => self.layout_text(container_width, container_height),
        }
    }

    /// Layout block elements (stack vertically)
    fn layout_block(&mut self, container_width: f32, container_height: f32) {
        // Set default dimensions
        self.dimensions.content = Rect::from_xywh(0.0, 0.0, container_width, 0.0);
        self.dimensions.padding = EdgeSizes::uniform(8.0); // Default padding
        self.dimensions.margin = EdgeSizes::new(8.0, 0.0, 8.0, 0.0); // Top/bottom margin

        let mut current_y = self.dimensions.content.top + self.dimensions.padding.top;
        let available_width = container_width - self.dimensions.padding.left - self.dimensions.padding.right;

        // Layout children vertically
        for child in &mut self.children {
            child.dimensions.content.offset((
                self.dimensions.content.left + self.dimensions.padding.left,
                current_y
            ));

            child.layout(available_width, container_height);
            current_y += child.dimensions.total_height();
        }

        // Update our height based on children
        let content_height = if self.children.is_empty() {
            20.0 // Minimum height for empty blocks
        } else {
            current_y - (self.dimensions.content.top + self.dimensions.padding.top)
        };

        self.dimensions.content = Rect::from_xywh(
            self.dimensions.content.left,
            self.dimensions.content.top,
            self.dimensions.content.width(),
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
            // Estimate text dimensions (simplified)
            let char_width = 8.0; // Average character width
            let line_height = 16.0;
            let chars_per_line = (container_width / char_width) as usize;
            let lines = (text.len() / chars_per_line.max(1)) + 1;

            self.dimensions.content = Rect::from_xywh(
                0.0,
                0.0,
                container_width.min(text.len() as f32 * char_width),
                lines as f32 * line_height
            );
        } else {
            self.dimensions.content = Rect::from_xywh(0.0, 0.0, 0.0, 0.0);
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
}
