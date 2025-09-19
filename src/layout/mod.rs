// Layout engine for computing element positions and sizes
mod box_model;
mod layout_tree;

pub use self::box_model::*;
pub use self::layout_tree::*;

use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::cell::RefCell;
use skia_safe::{Rect, Size};
use crate::dom::{DomNode, NodeType, ElementData};

/// Layout engine responsible for computing element positions and sizes
pub struct LayoutEngine {
    viewport_width: f32,
    viewport_height: f32,
    node_map: HashMap<usize, Rc<RefCell<DomNode>>>,
    next_node_id: usize,
}

impl LayoutEngine {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            viewport_width,
            viewport_height,
            node_map: HashMap::new(),
            next_node_id: 0,
        }
    }

    /// Compute layout for a DOM tree
    pub fn compute_layout(&mut self, root: &Rc<RefCell<DomNode>>) -> LayoutBox {
        // Clear previous layout
        self.node_map.clear();
        self.next_node_id = 0;

        // Build layout tree from DOM
        let mut layout_root = self.build_layout_tree(root);

        // Compute layout dimensions
        layout_root.layout(self.viewport_width, self.viewport_height);

        layout_root
    }

    /// Build layout tree from DOM tree
    fn build_layout_tree(&mut self, dom_node: &Rc<RefCell<DomNode>>) -> LayoutBox {
        let borrowed = dom_node.borrow();
        let node_id = self.next_node_id;
        self.next_node_id += 1;

        // Store reference for renderer
        self.node_map.insert(node_id, Rc::clone(dom_node));

        let mut layout_box = match &borrowed.node_type {
            NodeType::Document => {
                LayoutBox::new(BoxType::Block, node_id)
            },
            NodeType::Element(data) => {
                let box_type = self.determine_box_type(&data.tag_name);
                LayoutBox::new(box_type, node_id)
            },
            NodeType::Text(content) => {
                let mut text_box = LayoutBox::new(BoxType::Text, node_id);
                text_box.content = Some(content.clone());
                text_box
            },
            _ => {
                // Skip other node types for now
                LayoutBox::new(BoxType::Block, node_id)
            }
        };

        // Process children
        for child in &borrowed.children {
            let child_layout = self.build_layout_tree(child);
            layout_box.children.push(child_layout);
        }

        drop(borrowed); // Release borrow
        layout_box
    }

    /// Determine the box type for an element
    fn determine_box_type(&self, tag_name: &str) -> BoxType {
        match tag_name {
            // Block elements
            "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" |
            "section" | "article" | "header" | "footer" | "main" |
            "nav" | "aside" | "blockquote" | "ul" | "ol" | "li" => BoxType::Block,

            // Inline elements
            "span" | "a" | "em" | "strong" | "b" | "i" | "u" |
            "small" | "code" | "kbd" | "var" | "samp" => BoxType::Inline,

            // Special elements
            "img" => BoxType::InlineBlock,

            // Default to block for unknown elements
            _ => BoxType::Block,
        }
    }

    /// Get the node map for renderers
    pub fn get_node_map(&self) -> &HashMap<usize, Rc<RefCell<DomNode>>> {
        &self.node_map
    }

    /// Update viewport size
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
    }
}
