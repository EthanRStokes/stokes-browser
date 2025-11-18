// Layout engine for computing element positions and sizes
pub(crate) mod box_model;
mod layout_tree;
mod taffy;
mod inline;
pub(crate) mod table;
mod replaced;

pub use self::layout_tree::*;

use crate::dom::{Dom, DomNode, NodeData};
use slab::Slab;
use std::cell::RefCell;
use std::ops::Deref;
use ::taffy::{compute_root_layout, round_layout, AvailableSpace, Display, NodeId};
use style::context::{RegisteredSpeculativePainter, RegisteredSpeculativePainters, SharedStyleContext};
use style::global_style_data::GLOBAL_STYLE_DATA;
use style::properties::longhands;
use style::shared_lock::StylesheetGuards;
use style::traversal::DomTraversal;
use style::traversal_flags::TraversalFlags;
use style::values::computed::{Au, Size};
use stylo_atoms::Atom;
use crate::css::stylo::RecalcStyle;

/// Layout engine responsible for computing element positions and sizes
pub struct LayoutEngine {
    viewport_width: f32,
    viewport_height: f32,
    next_node_id: usize,
}

impl LayoutEngine {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            viewport_width,
            viewport_height,
            next_node_id: 0,
        }
    }

    /// Compute layout for a DOM tree
    pub fn compute_layout(&mut self, dom: &mut Dom, scale_factor: f32) -> LayoutBox {
        self.next_node_id = 0;

        // First pass: compute styles for all nodes
        self.compute_styles_recursive(dom);
        self.next_node_id = 0; // Reset for layout tree building

        // Second pass: build layout tree from DOM with styles applied
        let root_cell = RefCell::new(dom.root_node_mut());
        let mut layout_root = self.build_layout_tree(root_cell);

        // Reserve space for browser UI at the top (address bar, tabs, etc.)
        let ui_height = 0.0;
        let content_start_y = ui_height;
        let available_height = self.viewport_height - ui_height;

        // Compute layout dimensions with scaled viewport
        layout_root.layout(self.viewport_width, available_height, 0.0, content_start_y, scale_factor);

        let root_element_id = NodeId::from(dom.root_element().id);
        compute_root_layout(dom, root_element_id, taffy::Size {
            width: AvailableSpace::Definite(self.viewport_width),
            height: AvailableSpace::Definite(available_height),
        });
        round_layout(dom, root_element_id);

        layout_root
    }

    /// Build layout tree from DOM tree
    fn build_layout_tree(&mut self, dom_node: RefCell<&mut DomNode>) -> LayoutBox {
        let borrowed = dom_node.borrow();
        let node_id = self.next_node_id;
        self.next_node_id += 1;

        let stylo = borrowed.style_arc();
        let style = &borrowed.taffy_style;
        let mut layout_box = match &borrowed.data {
            NodeData::Document => {
                LayoutBox::new(BoxType::Block, node_id, style.clone())
            },
            NodeData::Element(data) => {
                let box_type = self.determine_box_type(&data.name.local);
                LayoutBox::new(box_type, node_id, style.clone())
            },
            NodeData::Text { contents } => {
                let mut text_box = LayoutBox::new(BoxType::Text, node_id, style.clone());
                text_box.content = Some(LayoutContent::Text { content: contents.borrow().to_string() });
                text_box
            },
            _ => {
                // Skip other node types for now
                LayoutBox::new(BoxType::Block, node_id, style.clone())
            }
        };

        // Apply CSS styles if available
        let position = stylo.get_position();

        let width = &position.width;
        let height = &position.height;

        match width {
            Size::LengthPercentage(percent) => {
                let parent_width = self.viewport_width; // Simplified parent width
                let computed_width = percent.0.to_pixel_length(Au(parent_width as i32)).px();
                layout_box.dimensions.content.right = layout_box.dimensions.content.left + computed_width;
            }
            Size::Auto => {}
            Size::MaxContent => {}
            Size::MinContent => {}
            Size::FitContent => {}
            Size::WebkitFillAvailable => {
                let parent_width = self.viewport_width; // Simplified parent width
                layout_box.dimensions.content.right = layout_box.dimensions.content.left + parent_width;
            }
            Size::Stretch => {
                let parent_width = self.viewport_width; // Simplified parent width
                layout_box.dimensions.content.right = layout_box.dimensions.content.left + parent_width;
            }
            Size::FitContentFunction(percent) => {
                let parent_width = self.viewport_width; // Simplified parent width
                let computed_width = percent.0.to_pixel_length(Au(parent_width as i32)).px();
                layout_box.dimensions.content.right = layout_box.dimensions.content.left + computed_width;
            }
            Size::AnchorSizeFunction(anchor) => {}
            Size::AnchorContainingCalcFunction(anchor) => {}
        }

        match height {
            Size::LengthPercentage(percent) => {
                let parent_height = self.viewport_height; // Simplified parent height
                let computed_height = percent.0.to_pixel_length(Au(parent_height as i32)).px();
                layout_box.dimensions.content.bottom = layout_box.dimensions.content.top + computed_height;
            }
            Size::Auto => {}
            Size::MaxContent => {}
            Size::MinContent => {}
            Size::FitContent => {}
            Size::WebkitFillAvailable => {
                let parent_height = self.viewport_height; // Simplified parent height
                layout_box.dimensions.content.bottom = layout_box.dimensions.content.top + parent_height;
            }
            Size::Stretch => {
                let parent_height = self.viewport_height; // Simplified parent height
                layout_box.dimensions.content.bottom = layout_box.dimensions.content.top + parent_height;
            }
            Size::FitContentFunction(percent) => {
                let parent_height = self.viewport_height; // Simplified parent height
                let computed_height = percent.0.to_pixel_length(Au(parent_height as i32)).px();
                layout_box.dimensions.content.bottom = layout_box.dimensions.content.top + computed_height;
            }
            Size::AnchorSizeFunction(anchor) => {}
            Size::AnchorContainingCalcFunction(anchor) => {}
        }

        // Override box type based on display property
        match &style.display {
            Display::Block => {
                layout_box.box_type = BoxType::Block;
            }
            Display::Grid => {
                layout_box.box_type = BoxType::Inline;
            }
            Display::Flex => {
            }
            Display::None => {
                // Elements with display: none should not be rendered
                // We'll handle this by creating an empty block that takes no space
                layout_box.box_type = BoxType::Block;
                layout_box.dimensions.content.right = layout_box.dimensions.content.left;
                layout_box.dimensions.content.bottom = layout_box.dimensions.content.top;
            }
            _ => {
                println!("Unknown display type for node id {}: {:?}", node_id, style.display);
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

    /// Recursively compute styles for DOM nodes
    fn compute_styles_recursive(&self, dom: &mut Dom) {
        let lock = &dom.lock;
        let author = lock.read();
        let ua_or_user = lock.read();
        let guards = StylesheetGuards {
            author: &author,
            ua_or_user: &ua_or_user,
        };

        // Flush the stylist with all loaded stylesheets
        {
            let root = &dom.nodes[0];
            dom.stylist.flush(&guards, Some(root), Some(&dom.snapshots));
        }

        struct Painters;
        impl RegisteredSpeculativePainters for Painters {
            fn get(&self, name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
                None
            }
        }

        // Perform style traversal to compute styles for all elements
        {
            let context = SharedStyleContext {
                stylist: &dom.stylist,
                visited_styles_enabled: false,
                options: GLOBAL_STYLE_DATA.options.clone(),
                guards: guards,
                current_time_for_animations: 0.0, // TODO animations
                traversal_flags: TraversalFlags::empty(),
                snapshot_map: &dom.snapshots,
                animations: Default::default(),
                registered_speculative_painters: &Painters,
            };

            let root = dom.root_element();
            let token = RecalcStyle::pre_traverse(root, &context);

            if token.should_traverse() {
                let traverser = RecalcStyle::new(context);
                style::driver::traverse_dom(&traverser, token, None);
            }
        }
        drop(author);
        drop(ua_or_user);
    }
}
