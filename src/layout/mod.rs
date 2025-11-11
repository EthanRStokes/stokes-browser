// Layout engine for computing element positions and sizes
pub(crate) mod box_model;
mod layout_tree;

pub use self::layout_tree::*;

use crate::css::{ComputedValues, StyleResolver, Stylesheet};
use crate::dom::{DomNode, NodeData};
use slab::Slab;
use std::cell::RefCell;
use std::ops::Deref;

// Add taffy imports
use taffy::prelude::*;

/// Layout engine responsible for computing element positions and sizes
pub struct LayoutEngine {
    viewport_width: f32,
    viewport_height: f32,
    next_node_id: usize,
    style_resolver: StyleResolver,
}

impl LayoutEngine {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            viewport_width,
            viewport_height,
            next_node_id: 0,
            style_resolver: StyleResolver::new(),
        }
    }

    /// Compute layout for a DOM tree
    pub fn compute_layout(&mut self, root: &mut DomNode, scale_factor: f32) -> LayoutBox {
        self.next_node_id = 0;

        // First pass: compute styles for all nodes
        let root_cell = RefCell::new(root);
        self.compute_styles_recursive(&root_cell, None);
        self.next_node_id = 0; // Reset for layout tree building

        // Second pass: build layout tree from DOM with styles applied
        let mut layout_root = self.build_layout_tree(root_cell);

        // Reserve space for browser UI at the top (address bar, tabs, etc.)
        let ui_height = 0.0;
        let content_start_y = ui_height;
        let available_height = self.viewport_height - ui_height;

        // Compute layout dimensions with scaled viewport
        layout_root.layout(self.viewport_width, available_height, 0.0, content_start_y, scale_factor);

        layout_root
    }

    /// Build layout tree from DOM tree
    fn build_layout_tree(&mut self, dom_node: RefCell<&mut DomNode>) -> LayoutBox {
        let borrowed = dom_node.borrow();
        let node_id = self.next_node_id;
        self.next_node_id += 1;

        let stylo = borrowed.style_arc();
        let mut layout_box = match &borrowed.data {
            NodeData::Document => {
                LayoutBox::new(BoxType::Block, node_id, stylo.clone())
            },
            NodeData::Element(data) => {
                let box_type = self.determine_box_type(&data.name.local);
                LayoutBox::new(box_type, node_id, stylo.clone())
            },
            NodeData::Text { contents } => {
                let mut text_box = LayoutBox::new(BoxType::Text, node_id, stylo.clone());
                text_box.content = Some(LayoutContent::Text { content: contents.borrow().to_string() });
                text_box
            },
            NodeData::Image(data) => {
                // TODO can i make this better?
                LayoutBox::new(BoxType::Image(data.clone()), node_id, stylo.clone())
            },
            _ => {
                // Skip other node types for now
                LayoutBox::new(BoxType::Block, node_id, stylo.clone())
            }
        };

        // Apply CSS styles if available
        let computed_styles = &borrowed.style;
        let position = stylo.get_position();


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

        // Process children
        let children = borrowed.children.clone();
        drop(borrowed);
        for child in children {
            let tree_ptr = {
                let borrowed = dom_node.borrow();
                borrowed.tree() as *const Slab<DomNode>
            };

            let child = unsafe {
                let tree_mut_ptr = tree_ptr as *mut Slab<DomNode>;
                (*tree_mut_ptr).get_mut(child).unwrap()
            };

            let child_layout = self.build_layout_tree(RefCell::new(child));
            layout_box.children.push(child_layout);
        }

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

    /// Update viewport size
    #[inline]
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
    }

    /// Add a stylesheet to the style resolver
    #[inline]
    pub fn add_stylesheet(&mut self, stylesheet: Stylesheet) {
        self.style_resolver.add_stylesheet(stylesheet);
    }

    /// Recursively compute styles for DOM nodes
    fn compute_styles_recursive(&self, node: &RefCell<&mut DomNode>, parent_styles: Option<&ComputedValues>) {
        let computed_styles = {
            let mut borrowed = node.borrow_mut();
            // Compute styles for this node
            let computed_styles = self.style_resolver.resolve_styles(borrowed.deref(), parent_styles);
            borrowed.style = computed_styles.clone();
            computed_styles
        };

        // Collect children IDs first
        let children_ids: Vec<usize> = {
            let borrowed = node.borrow();
            borrowed.children.clone()
        };

        // Process children - must ensure borrow is dropped before recursive call
        for child_id in children_ids {
            // We need to get the tree pointer, drop the parent borrow, then access the child.
            // This is safe because:
            // 1) Each node in the slab is at a different memory location
            // 2) We're not modifying the parent during the child's recursion
            // 3) The tree pointer remains valid for the lifetime of this operation
            let tree_ptr = {
                let borrowed = node.borrow();
                // Get the raw pointer to the tree from the immutable reference
                // The tree is internally stored as a raw pointer in DomNode
                borrowed.tree() as *const Slab<DomNode>
            }; // parent borrow is dropped here

            // SAFETY: We know that:
            // - The tree pointer is valid (it comes from a valid DomNode)
            // - We're accessing a different entry in the slab (child_id != parent id)
            // - No other code can invalidate the slab during this operation
            // - The tree is fundamentally stored as a raw *mut pointer in DomNode
            // - We immediately wrap it in a RefCell for borrowing tracking
            let child = unsafe {
                let tree_mut_ptr = tree_ptr as *mut Slab<DomNode>;
                (*tree_mut_ptr).get_mut(child_id).unwrap()
            };
            let child_cell = RefCell::new(child);
            self.compute_styles_recursive(&child_cell, Some(&computed_styles));
        }
    }
}
