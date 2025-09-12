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
        // In a real implementation, this would handle the CSS box model
        // including margin, border, padding, and content dimensions
        todo!("Implement block layout")
    }

    /// Layout algorithm for inline elements
    fn layout_inline(&self, layout_box: &mut LayoutBox) {
        // In a real implementation, this would handle inline layout flow
        // including text measurement and line breaking
        todo!("Implement inline layout")
    }

    /// Default style for the root box
    fn default_root_style(&self) -> ComputedStyle {
        todo!("Create default root style")
    }

    /// Default style based on element tag
    fn default_element_style(&self, tag_name: &str) -> ComputedStyle {
        todo!("Create default element style")
    }

    /// Default style for text nodes
    fn default_text_style(&self) -> ComputedStyle {
        todo!("Create default text style")
    }
}
