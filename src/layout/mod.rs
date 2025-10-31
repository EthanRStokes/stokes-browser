// Layout engine for computing element positions and sizes
pub(crate) mod box_model;
mod layout_tree;

pub use self::layout_tree::*;

use crate::css::{ComputedValues, StyleResolver, Stylesheet};
use crate::dom::{DomNode, ElementData, NodeData};
use std::cell::RefCell;
use std::rc::Rc;

use taffy::prelude::*;
use taffy::{Overflow, Point, TaffyTree, TextAlign};

/// Layout engine responsible for computing element positions and sizes
pub struct LayoutEngine {
    viewport_width: f32,
    viewport_height: f32,
    style_resolver: StyleResolver,
    // Taffy layout tree
    taffy: TaffyTree<usize>,
    // Store reference to DOM root for measure function access
    dom_root: Option<Rc<RefCell<DomNode>>>,
}

impl LayoutEngine {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            viewport_width,
            viewport_height,
            style_resolver: StyleResolver::new(),
            taffy: TaffyTree::new(),
            dom_root: None,
        }
    }

    /// Compute layout for a DOM tree
    pub fn compute_layout(&mut self, root: &Rc<RefCell<DomNode>>, scale_factor: f32) -> &TaffyTree<usize> {
        // Store DOM root for measure function
        self.dom_root = Some(root.clone());

        // Clear previous layout
        self.taffy.clear();

        // First pass: compute styles for all nodes
        self.compute_styles_recursive(root, None);

        // Second pass: build Taffy tree from DOM
        let root_taffy_node = self.build_taffy_tree(root, scale_factor);

        // Reserve space for browser UI at the top (address bar, tabs, etc.)
        let ui_height = 0.0;
        let available_height = self.viewport_height - ui_height;

        // Compute layout using Taffy with scaled viewport
        let available_space = taffy::Size {
            width: AvailableSpace::Definite(self.viewport_width * scale_factor),
            height: AvailableSpace::Definite(available_height * scale_factor),
        };

        // Clone dom_root for use in closure
        let dom_root_clone = self.dom_root.clone();

        self.taffy.compute_layout_with_measure(
            root_taffy_node,
            available_space,
            |known_dimensions, available_space, _node_id, node_context, _style| {
                // node_context is Option<&mut usize>, extract the value if present
                let context_id = node_context.map(|ctx| *ctx).unwrap_or(0);
                Self::measure_node_static(
                    &dom_root_clone,
                    known_dimensions,
                    available_space,
                    context_id,
                    scale_factor
                )
            }
        ).unwrap();

        // Set all child nodes final_layout
        fn set_final_layout_recursively(
            engine: &LayoutEngine,
            dom_node: &Rc<RefCell<DomNode>>,
        ) {
            // First, get the layout and update this node
            {
                let borrowed = dom_node.borrow();
                if let Some(taffy_node_id) = borrowed.layout_id {
                    let layout = engine.taffy.get_final_layout(taffy_node_id);
                    drop(borrowed); // Drop immutable borrow before getting mutable borrow
                    let mut mutable_borrowed = dom_node.borrow_mut();
                    mutable_borrowed.final_layout = layout;
                    // mutable_borrowed is dropped here at end of scope
                }
                // If no layout_id, borrowed is dropped here
            }

            // Now get children and recurse (all previous borrows are dropped)
            let child_ids: Vec<usize> = {
                let borrowed = dom_node.borrow();
                borrowed.children.clone()
            };

            for child_id in child_ids {
                let child_node = {
                    let borrowed = dom_node.borrow();
                    borrowed.get_node(child_id).clone()
                };
                set_final_layout_recursively(engine, &child_node);
            }
        }

        set_final_layout_recursively(self, &root);

        &self.taffy
    }

    /// Build Taffy tree from DOM tree
    fn build_taffy_tree(&mut self, dom_node: &Rc<RefCell<DomNode>>, scale_factor: f32) -> NodeId {
        let borrowed = dom_node.borrow();

        // Convert CSS styles to Taffy style
        let style = borrowed.style.clone();

        // Determine if this is a leaf node that needs a measure function
        let is_text_node = matches!(borrowed.data, NodeData::Text { .. });
        let is_image_node = matches!(borrowed.data, NodeData::Element(ElementData { ref name, .. }) if name.local.as_ref() == "img");

        // Create Taffy node based on node type
        let taffy_node = if is_text_node || is_image_node {
            // Text and image nodes are leaf nodes with measure function
            let node_id = borrowed.id;

            self.taffy.new_leaf_with_context(
                style,
                node_id, // Use node_id as context to identify the node
            ).unwrap()
        } else {
            // Process children for container nodes
            let mut child_nodes = Vec::new();
            for child in &borrowed.children {
                let child = borrowed.get_node(*child);
                let child_taffy_node = self.build_taffy_tree(child, scale_factor);

                let mut child = child.borrow_mut();
                child.layout_id = Some(child_taffy_node);
                drop(child);

                child_nodes.push(child_taffy_node);
            }

            // Regular container nodes
            self.taffy.new_with_children(style, &child_nodes).unwrap()
        };

        // Set the layout_id on the current node (important for leaf nodes!)
        drop(borrowed);
        let mut borrowed_mut = dom_node.borrow_mut();
        borrowed_mut.layout_id = Some(taffy_node);
        drop(borrowed_mut);

        taffy_node
    }

    /// Measure function for text and image nodes (static to avoid borrow issues)
    fn measure_node_static(
        dom_root: &Option<Rc<RefCell<DomNode>>>,
        known_dimensions: taffy::Size<Option<f32>>,
        available_space: taffy::Size<AvailableSpace>,
        node_id: usize,
        scale_factor: f32,
    ) -> taffy::Size<f32> {
        // Get the DOM node
        let dom_root = dom_root.as_ref().unwrap();
        let dom_root_borrowed = dom_root.borrow();
        let node = dom_root_borrowed.get_node(node_id);
        let node_borrowed = node.borrow();

        // If dimensions are already known, return them
        if let (Some(width), Some(height)) = (known_dimensions.width, known_dimensions.height) {
            return taffy::Size { width, height };
        }

        // Measure based on node type
        match &node_borrowed.data {
            NodeData::Text { contents } => {
                let text = contents.borrow();
                Self::measure_text(&text, known_dimensions, available_space, scale_factor)
            }
            NodeData::Image(image_data) => {
                let image_data = image_data.borrow();
                Self::measure_image(&image_data, known_dimensions, scale_factor)
            }
            _ => {
                // Default size for non-leaf nodes (shouldn't happen)
                taffy::Size { width: 0.0, height: 0.0 }
            }
        }
    }

    /// Measure text node dimensions
    fn measure_text(
        text: &str,
        known_dimensions: taffy::Size<Option<f32>>,
        available_space: taffy::Size<AvailableSpace>,
        scale_factor: f32,
    ) -> taffy::Size<f32> {
        // Character dimensions (scaled for high DPI)
        let char_width = 8.0 * scale_factor;
        let line_height = 16.0 * scale_factor;

        // Get available width for text wrapping
        let max_width = match available_space.width {
            AvailableSpace::Definite(w) => w,
            AvailableSpace::MinContent => {
                // For min-content, use the longest word width
                let longest_word = text.split_whitespace()
                    .max_by_key(|word| word.len())
                    .unwrap_or("");
                longest_word.len() as f32 * char_width
            }
            AvailableSpace::MaxContent => {
                // For max-content, don't wrap - use full text width
                text.lines()
                    .map(|line| line.len() as f32 * char_width)
                    .fold(0.0, f32::max)
            }
        };

        // Wrap text to fit within available width
        let wrapped_lines = Self::wrap_text(text, max_width, char_width);
        let num_lines = wrapped_lines.len().max(1);

        // Calculate width based on the longest wrapped line
        let max_line_width = wrapped_lines.iter()
            .map(|line| line.len() as f32 * char_width)
            .fold(0.0, f32::max)
            .min(max_width);

        let width = known_dimensions.width.unwrap_or(if text.trim().is_empty() { 0.0 } else { max_line_width });
        let height = known_dimensions.height.unwrap_or(num_lines as f32 * line_height);

        taffy::Size { width, height }
    }

    /// Helper function to wrap text into lines that fit within a given width
    fn wrap_text(text: &str, max_width: f32, char_width: f32) -> Vec<String> {
        let mut wrapped_lines = Vec::new();

        // Split by explicit newlines first
        let paragraphs: Vec<&str> = text.split('\n').collect();

        for paragraph in paragraphs {
            if paragraph.is_empty() {
                wrapped_lines.push(String::new());
                continue;
            }

            // Calculate max characters per line
            let max_chars = (max_width / char_width).floor() as usize;

            if max_chars == 0 {
                wrapped_lines.push(paragraph.to_string());
                continue;
            }

            // Split paragraph into words
            let words: Vec<&str> = paragraph.split_whitespace().collect();

            if words.is_empty() {
                wrapped_lines.push(String::new());
                continue;
            }

            let mut current_line = String::new();
            let mut current_char_count = 0;

            for word in words {
                let word_char_count = word.chars().count();

                let test_char_count = if current_line.is_empty() {
                    word_char_count
                } else {
                    current_char_count + 1 + word_char_count
                };

                if test_char_count <= max_chars {
                    if current_line.is_empty() {
                        current_line.push_str(word);
                        current_char_count = word_char_count;
                    } else {
                        current_line.push(' ');
                        current_line.push_str(word);
                        current_char_count = test_char_count;
                    }
                } else {
                    if !current_line.is_empty() {
                        wrapped_lines.push(current_line);
                        current_line = String::new();
                        current_char_count = 0;
                    }

                    if word_char_count > max_chars {
                        // Break long words
                        let chars: Vec<char> = word.chars().collect();
                        let mut start = 0;
                        while start < chars.len() {
                            let end = (start + max_chars).min(chars.len());
                            let chunk: String = chars[start..end].iter().collect();
                            wrapped_lines.push(chunk);
                            start = end;
                        }
                        current_line = String::new();
                        current_char_count = 0;
                    } else {
                        current_line.push_str(word);
                        current_char_count = word_char_count;
                    }
                }
            }

            if !current_line.is_empty() {
                wrapped_lines.push(current_line);
            }
        }

        if wrapped_lines.is_empty() {
            wrapped_lines.push(String::new());
        }

        wrapped_lines
    }

    /// Measure image node dimensions
    fn measure_image(
        image_data: &crate::dom::ImageData,
        known_dimensions: taffy::Size<Option<f32>>,
        scale_factor: f32,
    ) -> taffy::Size<f32> {
        // Default image dimensions
        let default_width = 150.0;
        let default_height = 100.0;

        // Use specified dimensions from HTML attributes if available
        let base_width = image_data.width.unwrap_or(default_width as u32) as f32;
        let base_height = image_data.height.unwrap_or(default_height as u32) as f32;

        // Apply scale factor and known dimensions
        let width = known_dimensions.width.unwrap_or(base_width * scale_factor);
        let height = known_dimensions.height.unwrap_or(base_height * scale_factor);

        taffy::Size { width, height }
    }

    /// Convert CSS computed values to Taffy style
    fn css_to_taffy_style(&self, css: &ComputedValues) -> Style {
        use taffy::prelude::*;

        let mut style = Style::default();

        // Display type
        style.display = match css.display {
            crate::css::computed::DisplayType::Block => Display::Block,
            crate::css::computed::DisplayType::Flex => Display::Flex,
            crate::css::computed::DisplayType::Inline => Display::Block, // Treat inline as block for now
            crate::css::computed::DisplayType::InlineBlock => Display::Block,
            crate::css::computed::DisplayType::None => Display::None,
        };

        style.box_sizing = match css.box_sizing {
            crate::css::BoxSizing::ContentBox => BoxSizing::ContentBox,
            crate::css::BoxSizing::BorderBox => BoxSizing::BorderBox,
        };

        let overflow_x = match css.overflow_x {
            crate::css::Overflow::Visible => Overflow::Visible,
            crate::css::Overflow::Hidden => Overflow::Hidden,
            crate::css::Overflow::Scroll => Overflow::Scroll,
            crate::css::Overflow::Auto => Overflow::Clip,
        };
        let overflow_y = match css.overflow_y {
            crate::css::Overflow::Visible => Overflow::Visible,
            crate::css::Overflow::Hidden => Overflow::Hidden,
            crate::css::Overflow::Scroll => Overflow::Scroll,
            crate::css::Overflow::Auto => Overflow::Clip,
        };
        style.overflow = Point { x: overflow_x, y: overflow_y };

        // Size
        if let Some(width) = &css.width {
            let w = width.to_px(css.font_size, self.viewport_width);
            style.size.width = Dimension::length(w);
        }
        if let Some(height) = &css.height {
            let h = height.to_px(css.font_size, self.viewport_height);
            style.size.height = Dimension::length(h);
        }

        // Min/Max size
        if let Some(min_width) = &css.min_width {
            let w = min_width.to_px(css.font_size, self.viewport_width);
            style.min_size.width = Dimension::length(w);
        }
        if let Some(max_width) = &css.max_width {
            let w = max_width.to_px(css.font_size, self.viewport_width);
            style.max_size.width = Dimension::length(w);
        }
        if let Some(min_height) = &css.min_height {
            let h = min_height.to_px(css.font_size, self.viewport_height);
            style.min_size.height = Dimension::length(h);
        }
        if let Some(max_height) = &css.max_height {
            let h = max_height.to_px(css.font_size, self.viewport_height);
            style.max_size.height = Dimension::length(h);
        }

        // Margin
        style.margin = Rect {
            left: LengthPercentageAuto::length(css.margin.left),
            right: LengthPercentageAuto::length(css.margin.right),
            top: LengthPercentageAuto::length(css.margin.top),
            bottom: LengthPercentageAuto::length(css.margin.bottom),
        };

        // Padding
        style.padding = Rect {
            left: LengthPercentage::length(css.padding.left),
            right: LengthPercentage::length(css.padding.right),
            top: LengthPercentage::length(css.padding.top),
            bottom: LengthPercentage::length(css.padding.bottom),
        };

        // Border
        style.border = Rect {
            left: LengthPercentage::length(css.border.left),
            right: LengthPercentage::length(css.border.right),
            top: LengthPercentage::length(css.border.top),
            bottom: LengthPercentage::length(css.border.bottom),
        };

        // Flex properties (if display is flex)
        if matches!(css.display, crate::css::computed::DisplayType::Flex) {
            style.flex_grow = css.flex_grow.0;
            style.flex_shrink = css.flex_shrink.0;

            style.flex_basis = match &css.flex_basis {
                crate::css::FlexBasis::Length(len) => {
                    let val = len.to_px(css.font_size, self.viewport_width);
                    Dimension::length(val)
                }
                crate::css::FlexBasis::Auto => Dimension::AUTO,
                crate::css::FlexBasis::Content => Dimension::AUTO, // Treat content as auto for now
            };

            // Gap
            style.gap.width = LengthPercentage::length(css.gap.column.to_px(css.font_size, self.viewport_width));
            style.gap.height = LengthPercentage::length(css.gap.row.to_px(css.font_size, self.viewport_height));
        }

        style
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
    fn compute_styles_recursive(&self, node: &Rc<RefCell<DomNode>>, parent_styles: Option<&ComputedValues>) {
        let mut borrowed = node.borrow_mut();
        // Compute styles for this node
        let computed_styles = self.style_resolver.resolve_styles(&*borrowed, parent_styles);
        borrowed.style = self.css_to_taffy_style(&computed_styles);

        borrowed.final_layout.order = computed_styles.z_index as u32;

        // Process children
        let children = &borrowed.children;
        for child in children {
            let child = borrowed.get_node(*child);
            self.compute_styles_recursive(child, Some(&computed_styles));
        }
    }
}
