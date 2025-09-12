// Layout module - responsible for computing positions and dimensions of elements
use crate::renderer::style::{ComputedStyle, DisplayType};
use markup5ever_rcdom::{Handle, NodeData};
use std::collections::HashMap;

/// Represents a positioned and sized box in the layout
#[derive(Debug, Clone)]
pub struct LayoutBox {
    // Geometric properties
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // Reference to computed style
    pub style: ComputedStyle,

    // DOM node reference
    pub node: Handle,

    // Child layout boxes
    pub children: Vec<LayoutBox>,

    // Content type
    pub box_type: BoxType,
}

/// Different types of layout boxes
#[derive(Debug, Clone)]
pub enum BoxType {
    Block,
    Inline,
    Anonymous,
    Root,
}

/// The layout engine that positions elements
pub struct LayoutEngine {
    // Map from DOM nodes to computed styles
    style_map: HashMap<usize, ComputedStyle>,

    // Viewport dimensions
    viewport_width: f32,
    viewport_height: f32,
}

impl LayoutEngine {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            style_map: HashMap::new(),
            viewport_width,
            viewport_height,
        }
    }

    /// Set the style for a specific DOM node
    pub fn set_style(&mut self, node_ptr: usize, style: ComputedStyle) {
        self.style_map.insert(node_ptr, style);
    }

    /// Create a layout tree from a DOM tree
    pub fn create_layout(&self, root: &Handle) -> LayoutBox {
        // Start with a root box that represents the viewport
        let mut root_box = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: self.viewport_width,
            height: self.viewport_height,
            style: self.default_root_style(),
            node: root.clone(),
            children: Vec::new(),
            box_type: BoxType::Root,
        };

        // Recursively build the layout tree
        self.build_layout_tree(root, &mut root_box);

        // Calculate positions and dimensions
        self.perform_layout(&mut root_box);

        root_box
    }

    /// Recursively build the layout tree from the DOM
    fn build_layout_tree(&self, node: &Handle, parent: &mut LayoutBox) {
        // Skip non-element nodes for simplicity
        match &node.data {
            NodeData::Element { name, .. } => {
                let tag_name = name.local.to_string();

                // Get style for this node
                let node_addr = node as *const _ as usize;
                let style = self.style_map
                    .get(&node_addr)
                    .cloned()
                    .unwrap_or_else(|| self.default_element_style(&tag_name));

                // Skip display:none elements
                if matches!(style.display, DisplayType::None) {
                    return;
                }

                // Create box based on display type
                let box_type = match style.display {
                    DisplayType::Block | DisplayType::Flex | DisplayType::Grid => BoxType::Block,
                    DisplayType::Inline | DisplayType::InlineBlock => BoxType::Inline,
                    _ => BoxType::Block, // Default to block
                };

                let mut layout_box = LayoutBox {
                    x: 0.0, // Will be calculated in layout phase
                    y: 0.0, // Will be calculated in layout phase
                    width: 0.0, // Will be calculated in layout phase
                    height: 0.0, // Will be calculated in layout phase
                    style,
                    node: node.clone(),
                    children: Vec::new(),
                    box_type,
                };

                // Process children
                for child in node.children.borrow().iter() {
                    self.build_layout_tree(child, &mut layout_box);
                }

                // Add this box to parent's children
                parent.children.push(layout_box);
            },
            NodeData::Text { contents } => {
                // For text nodes, create an anonymous inline box
                let text = contents.borrow().to_string();
                if !text.trim().is_empty() {
                    let text_box = LayoutBox {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0, // Will calculate based on text content
                        height: 0.0, // Will calculate based on font size
                        style: self.default_text_style(),
                        node: node.clone(),
                        children: Vec::new(),
                        box_type: BoxType::Inline,
                    };

                    parent.children.push(text_box);
                }
            },
            _ => {
                // TODO this probably causes performance issues
                let mut new_parent = parent.clone();
                // Skip other node types
                for child in node.children.borrow().iter() {
                    self.build_layout_tree(child, &mut new_parent);
                }
            }
        }
    }

    /// Calculate positions and dimensions for all boxes in the layout tree
    fn perform_layout(&self, layout_box: &mut LayoutBox) {
        // Different layout algorithms based on display type
        match layout_box.box_type {
            BoxType::Block => self.layout_block(layout_box),
            BoxType::Inline => self.layout_inline(layout_box),
            BoxType::Root => self.layout_block(layout_box),
            BoxType::Anonymous => self.layout_inline(layout_box),
        }

        // Recursively layout children
        for child in &mut layout_box.children {
            self.perform_layout(child);
        }
    }

    /// Layout algorithm for block-level elements
    fn layout_block(&self, layout_box: &mut LayoutBox) {
        let parent_width = layout_box.width;

        // Calculate box dimensions
        self.calculate_block_width(layout_box, parent_width);
        self.calculate_block_position(layout_box);

        // Layout children
        self.layout_block_children(layout_box);

        // Calculate height based on children
        self.calculate_block_height(layout_box);
    }

    /// Calculate the width of a block element
    fn calculate_block_width(&self, layout_box: &mut LayoutBox, containing_width: f32) {
        // Get style properties
        let style = &layout_box.style;

        // Default width if not specified is 100% of containing block
        let mut width = match style.width {
            Some(crate::renderer::style::Dimension::Pixels(px)) => px,
            Some(crate::renderer::style::Dimension::Percentage(pct)) => containing_width * pct / 100.0,
            Some(crate::renderer::style::Dimension::Auto) | None => containing_width,
            _ => containing_width, // Default to 100% for other dimensions
        };

        // Apply min/max width constraints (would be in style in a real implementation)
        width = width.max(0.0).min(containing_width);

        layout_box.width = width;
    }

    /// Calculate the vertical position of a block element
    fn calculate_block_position(&self, layout_box: &mut LayoutBox) {
        // For simplicity, just stack blocks vertically
        if let Some(parent) = layout_box.node.parent.take() {
            // In a real implementation, we'd get the parent box and use its dimensions
            // For now, just set y to 0 (we'll position children later)
            layout_box.y = 0.0;
            layout_box.x = 0.0; // Align to left edge
        }
    }

    /// Layout children of a block element
    fn layout_block_children(&self, layout_box: &mut LayoutBox) {
        let mut current_y = 0.0; // Start at the top of the parent

        for child in &mut layout_box.children {
            // Position child relative to parent
            child.x = 0.0; // Align to left edge
            child.y = current_y;

            // Update current_y for next child
            current_y += child.height;
        }
    }

    /// Calculate the height of a block element based on its children
    fn calculate_block_height(&self, layout_box: &mut LayoutBox) {
        // Use explicit height if specified, otherwise sum of children
        let style = &layout_box.style;

        let height = match style.height {
            Some(crate::renderer::style::Dimension::Pixels(px)) => px,
            Some(crate::renderer::style::Dimension::Percentage(pct)) => {
                // In a real browser, this would be based on the containing block's height
                // For simplicity, we'll use viewport height
                self.viewport_height * pct / 100.0
            },
            Some(crate::renderer::style::Dimension::Auto) | None => {
                // Sum of children's heights
                layout_box.children.iter()
                    .map(|child| child.y + child.height)
                    .max_by(|a, b| a.partial_cmp(b).unwrap())
                    .unwrap_or(0.0)
            },
            _ => 0.0, // Default for other dimensions
        };

        layout_box.height = height;
    }

    /// Layout algorithm for inline elements
    fn layout_inline(&self, layout_box: &mut LayoutBox) {
        // In a real implementation, this would handle text layout and line breaking
        // For now, implement a simple inline layout

        // Set default dimensions for simplicity
        layout_box.width = 100.0;  // Default width
        layout_box.height = 20.0;  // Default height for text line

        // In a real browser, we would calculate the width based on text content and font metrics
        // For now, we'll use a simple approximation

        match &layout_box.node.data {
            markup5ever_rcdom::NodeData::Text { contents } => {
                let text = contents.borrow();
                if !text.is_empty() {
                    // Rough approximation: each character is ~10px wide
                    let text_width = text.len() as f32 * 10.0;
                    layout_box.width = text_width.min(layout_box.width);
                }
            },
            _ => {}
        }
    }

    /// Default style for the root box
    fn default_root_style(&self) -> ComputedStyle {
        use crate::renderer::style::*;

        ComputedStyle {
            color: [0.0, 0.0, 0.0],  // Black text
            background_color: [1.0, 1.0, 1.0],  // White background
            width: Some(Dimension::Pixels(self.viewport_width)),
            height: Some(Dimension::Pixels(self.viewport_height)),
            margin: BoxValues {
                top: Dimension::Pixels(0.0),
                right: Dimension::Pixels(0.0),
                bottom: Dimension::Pixels(0.0),
                left: Dimension::Pixels(0.0),
            },
            padding: BoxValues {
                top: Dimension::Pixels(0.0),
                right: Dimension::Pixels(0.0),
                bottom: Dimension::Pixels(0.0),
                left: Dimension::Pixels(0.0),
            },
            border: BoxValues {
                top: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                right: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                bottom: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                left: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
            },
            display: DisplayType::Block,
            position: PositionType::Static,
            top: None,
            right: None,
            bottom: None,
            left: None,
            font_size: Dimension::Pixels(16.0),
            font_weight: FontWeight::Normal,
            font_family: vec!["Arial".to_string(), "sans-serif".to_string()],
            text_align: TextAlign::Left,
        }
    }

    /// Default style based on element tag
    fn default_element_style(&self, tag_name: &str) -> ComputedStyle {
        use crate::renderer::style::*;

        let mut style = ComputedStyle {
            color: [0.0, 0.0, 0.0],  // Black text
            background_color: [1.0, 1.0, 1.0],  // White background (transparent)
            width: None,  // Auto width by default
            height: None, // Auto height by default
            margin: BoxValues {
                top: Dimension::Pixels(0.0),
                right: Dimension::Pixels(0.0),
                bottom: Dimension::Pixels(0.0),
                left: Dimension::Pixels(0.0),
            },
            padding: BoxValues {
                top: Dimension::Pixels(0.0),
                right: Dimension::Pixels(0.0),
                bottom: Dimension::Pixels(0.0),
                left: Dimension::Pixels(0.0),
            },
            border: BoxValues {
                top: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                right: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                bottom: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                left: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
            },
            display: DisplayType::Inline,  // Default display type
            position: PositionType::Static,
            top: None,
            right: None,
            bottom: None,
            left: None,
            font_size: Dimension::Pixels(16.0),
            font_weight: FontWeight::Normal,
            font_family: vec!["Arial".to_string(), "sans-serif".to_string()],
            text_align: TextAlign::Left,
        };

        // Adjust style based on element type
        match tag_name {
            "html" | "body" => {
                style.display = DisplayType::Block;
                style.width = Some(Dimension::Percentage(100.0));
                style.height = Some(Dimension::Percentage(100.0));
            },
            "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article" | "header" | "footer" | "nav" => {
                style.display = DisplayType::Block;
                style.margin.bottom = Dimension::Pixels(10.0);
            },
            "h1" => {
                style.font_size = Dimension::Pixels(32.0);
                style.font_weight = FontWeight::Bold;
                style.margin.bottom = Dimension::Pixels(16.0);
            },
            "h2" => {
                style.font_size = Dimension::Pixels(24.0);
                style.font_weight = FontWeight::Bold;
                style.margin.bottom = Dimension::Pixels(14.0);
            },
            "h3" => {
                style.font_size = Dimension::Pixels(20.0);
                style.font_weight = FontWeight::Bold;
                style.margin.bottom = Dimension::Pixels(12.0);
            },
            "a" => {
                style.color = [0.0, 0.0, 1.0]; // Blue for links
            },
            "span" | "a" | "strong" | "em" | "b" | "i" => {
                style.display = DisplayType::Inline;
            },
            "strong" | "b" => {
                style.font_weight = FontWeight::Bold;
            },
            // Add more element styles as needed
            _ => {} // Default style for unknown elements
        }

        style
    }

    /// Default style for text nodes
    fn default_text_style(&self) -> ComputedStyle {
        use crate::renderer::style::*;

        ComputedStyle {
            color: [0.0, 0.0, 0.0],  // Black text
            background_color: [1.0, 1.0, 1.0],  // Transparent background
            width: None,
            height: None,
            margin: BoxValues {
                top: Dimension::Pixels(0.0),
                right: Dimension::Pixels(0.0),
                bottom: Dimension::Pixels(0.0),
                left: Dimension::Pixels(0.0),
            },
            padding: BoxValues {
                top: Dimension::Pixels(0.0),
                right: Dimension::Pixels(0.0),
                bottom: Dimension::Pixels(0.0),
                left: Dimension::Pixels(0.0),
            },
            border: BoxValues {
                top: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                right: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                bottom: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
                left: Border { width: 0.0, style: BorderStyle::None, color: [0.0, 0.0, 0.0] },
            },
            display: DisplayType::Inline,
            position: PositionType::Static,
            top: None,
            right: None,
            bottom: None,
            left: None,
            font_size: Dimension::Pixels(16.0),
            font_weight: FontWeight::Normal,
            font_family: vec!["Arial".to_string(), "sans-serif".to_string()],
            text_align: TextAlign::Left,
        }
    }
}
