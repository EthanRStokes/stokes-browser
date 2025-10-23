// Layout engine for computing element positions and sizes
pub(crate) mod box_model;
mod layout_tree;

pub use self::layout_tree::*;

use crate::css::{ComputedValues, StyleResolver, Stylesheet};
use crate::dom::{DomNode, NodeType};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Layout engine responsible for computing element positions and sizes
pub struct LayoutEngine {
    viewport_width: f32,
    viewport_height: f32,
    node_map: HashMap<usize, Rc<RefCell<DomNode>>>,
    style_map: HashMap<usize, ComputedValues>,
    next_node_id: usize,
    style_resolver: StyleResolver,
}

impl LayoutEngine {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            viewport_width,
            viewport_height,
            node_map: HashMap::new(),
            style_map: HashMap::new(),
            next_node_id: 0,
            style_resolver: StyleResolver::new(),
        }
    }

    /// Compute layout for a DOM tree
    pub fn compute_layout(&mut self, root: &Rc<RefCell<DomNode>>, scale_factor: f32) -> LayoutBox {
        // Clear previous layout
        self.node_map.clear();
        self.style_map.clear();
        self.next_node_id = 0;

        // First pass: compute styles for all nodes
        self.compute_styles_recursive(root, None);
        self.next_node_id = 0; // Reset for layout tree building

        // Second pass: build layout tree from DOM with styles applied
        let mut layout_root = self.build_layout_tree(root);

        // Reserve space for browser UI at the top (address bar, tabs, etc.)
        let ui_height = 0.0;
        let content_start_y = ui_height;
        let available_height = self.viewport_height - ui_height;

        // Compute layout dimensions with scaled viewport
        layout_root.layout(self.viewport_width, available_height, 0.0, content_start_y, scale_factor);

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
            NodeType::Image(data) => {
                // TODO can i make this better?
                LayoutBox::new(BoxType::Image(data.clone()), node_id)
            },
            _ => {
                // Skip other node types for now
                LayoutBox::new(BoxType::Block, node_id)
            }
        };

        // Apply CSS styles if available
        if let Some(computed_styles) = self.style_map.get(&node_id) {
            // Apply margin and padding from computed styles
            layout_box.dimensions.margin = computed_styles.margin.clone();
            layout_box.dimensions.padding = computed_styles.padding.clone();
            layout_box.dimensions.border = computed_styles.border.clone();

            // Apply box-sizing property
            layout_box.box_sizing = computed_styles.box_sizing.clone();

            // Apply float and clear properties
            layout_box.float = computed_styles.float.clone();
            layout_box.clear = computed_styles.clear.clone();

            // Apply width and height constraints
            if let Some(width) = &computed_styles.width {
                let parent_width = self.viewport_width; // Simplified parent width
                let computed_width = width.to_px(computed_styles.font_size, parent_width);
                layout_box.dimensions.content.right = layout_box.dimensions.content.left + computed_width;
            }

            if let Some(height) = &computed_styles.height {
                let parent_height = self.viewport_height; // Simplified parent height
                let computed_height = height.to_px(computed_styles.font_size, parent_height);
                layout_box.dimensions.content.bottom = layout_box.dimensions.content.top + computed_height;
            }

            layout_box.css_height = computed_styles.height.clone();
            layout_box.css_width = computed_styles.width.clone();

            // Set display type from computed styles
            layout_box.display_type = computed_styles.display.clone();

            // Override box type based on display property
            match computed_styles.display {
                crate::css::computed::DisplayType::Block => {
                    // TODO: reconsider this
                    // layout_box.box_type = BoxType::Block;
                },
                crate::css::computed::DisplayType::Inline => {
                    layout_box.box_type = BoxType::Inline;
                },
                crate::css::computed::DisplayType::InlineBlock => {
                    layout_box.box_type = BoxType::InlineBlock;
                },
                crate::css::computed::DisplayType::Flex => {
                    // Flex containers behave like block containers but layout children horizontally
                    // Keep the box_type as is, the display_type will control layout
                },
                crate::css::computed::DisplayType::None => {
                    // Elements with display: none should not be rendered
                    // We'll handle this by creating an empty block that takes no space
                    layout_box.box_type = BoxType::Block;
                    layout_box.dimensions.content.right = layout_box.dimensions.content.left;
                    layout_box.dimensions.content.bottom = layout_box.dimensions.content.top;
                }
            }
        }

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

    /// Add a stylesheet to the style resolver
    pub fn add_stylesheet(&mut self, stylesheet: Stylesheet) {
        self.style_resolver.add_stylesheet(stylesheet);
    }

    /// Recursively compute styles for DOM nodes
    fn compute_styles_recursive(&mut self, node: &Rc<RefCell<DomNode>>, parent_styles: Option<&ComputedValues>) {
        let borrowed = node.borrow();
        let node_id = self.next_node_id;
        self.next_node_id += 1;

        // Compute styles for this node
        let computed_styles = self.style_resolver.resolve_styles(&*borrowed, parent_styles);
        self.style_map.insert(node_id, computed_styles.clone());

        // Process children
        for child in &borrowed.children {
            self.compute_styles_recursive(child, Some(&computed_styles));
        }

        drop(borrowed);
    }

    /// Get computed styles for a node
    pub fn get_computed_styles(&self, node_id: usize) -> Option<&ComputedValues> {
        self.style_map.get(&node_id)
    }
}
