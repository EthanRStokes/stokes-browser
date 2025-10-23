// Layout tree implementation
use super::box_model::{Dimensions, EdgeSizes};
use crate::dom::ImageData;
use skia_safe::Rect;
use std::cell::RefCell;

/// Tracks active floats in a formatting context
#[derive(Debug, Clone)]
struct FloatContext {
    /// Left-floated boxes with their positions
    left_floats: Vec<(f32, f32, f32, f32)>, // (x, y, width, height)
    /// Right-floated boxes with their positions
    right_floats: Vec<(f32, f32, f32, f32)>, // (x, y, width, height)
}

impl FloatContext {
    fn new() -> Self {
        Self {
            left_floats: Vec::new(),
            right_floats: Vec::new(),
        }
    }

    /// Get the available width at a given Y position, considering floats
    fn get_available_width(&self, y: f32, container_width: f32, container_x: f32) -> (f32, f32) {
        let mut left_edge = container_x;
        let mut right_edge = container_x + container_width;

        // Check left floats
        for (fx, fy, fw, fh) in &self.left_floats {
            if y >= *fy && y < fy + fh {
                left_edge = left_edge.max(fx + fw);
            }
        }

        // Check right floats
        for (fx, fy, fw, fh) in &self.right_floats {
            if y >= *fy && y < fy + fh {
                right_edge = right_edge.min(*fx);
            }
        }

        (left_edge, right_edge - left_edge)
    }

    /// Get the Y position to clear floats
    fn get_clear_y(&self, clear_type: &crate::css::Clear, current_y: f32) -> f32 {
        use crate::css::Clear;

        let mut clear_y = current_y;

        match clear_type {
            Clear::Left => {
                for (_, fy, _, fh) in &self.left_floats {
                    clear_y = clear_y.max(fy + fh);
                }
            }
            Clear::Right => {
                for (_, fy, _, fh) in &self.right_floats {
                    clear_y = clear_y.max(fy + fh);
                }
            }
            Clear::Both => {
                for (_, fy, _, fh) in &self.left_floats {
                    clear_y = clear_y.max(fy + fh);
                }
                for (_, fy, _, fh) in &self.right_floats {
                    clear_y = clear_y.max(fy + fh);
                }
            }
            Clear::None => {}
        }

        clear_y
    }

    /// Find the next Y position where content can fit with given width
    fn find_y_for_width(&self, start_y: f32, required_width: f32, container_width: f32, container_x: f32) -> f32 {
        let mut y = start_y;
        let step = 1.0; // Step size for searching

        // Try up to 100 steps to find space
        for _ in 0..100 {
            let (_, available_width) = self.get_available_width(y, container_width, container_x);
            if available_width >= required_width {
                return y;
            }
            y += step;
        }

        y
    }
}

/// Type of layout box
#[derive(Debug, Clone, PartialEq)]
pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Text,
    Image(RefCell<ImageData>),
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
    pub css_max_width: Option<crate::css::Length>, // CSS specified max-width
    pub css_min_width: Option<crate::css::Length>, // CSS specified min-width
    pub css_max_height: Option<crate::css::Length>, // CSS specified max-height
    pub css_min_height: Option<crate::css::Length>, // CSS specified min-height
    pub box_sizing: crate::css::BoxSizing, // CSS box-sizing property
    pub flex_grow: crate::css::FlexGrow, // CSS flex-grow property
    pub flex_shrink: crate::css::FlexShrink, // CSS flex-shrink property
    pub flex_basis: crate::css::FlexBasis, // CSS flex-basis property
    pub gap: crate::css::Gap, // CSS gap property (row-gap and column-gap)
    pub display_type: crate::css::computed::DisplayType, // CSS display property
    pub float: crate::css::Float, // CSS float property
    pub clear: crate::css::Clear, // CSS clear property
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
            css_max_width: None,
            css_min_width: None,
            css_max_height: None,
            css_min_height: None,
            box_sizing: crate::css::BoxSizing::ContentBox, // Default value
            flex_grow: crate::css::FlexGrow::default(), // Default value
            flex_shrink: crate::css::FlexShrink::default(), // Default value
            flex_basis: crate::css::FlexBasis::Auto, // Default value
            gap: crate::css::Gap::default(), // Default value
            display_type: crate::css::computed::DisplayType::Block, // Default value
            float: crate::css::Float::None, // Default value
            clear: crate::css::Clear::None, // Default value
        }
    }

    /// Calculate layout
    pub fn layout(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        // Check if this is a flex container
        if self.display_type == crate::css::computed::DisplayType::Flex {
            self.layout_flex(container_width, container_height, offset_x, offset_y, scale_factor);
            return;
        }

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

        // Use CSS width if specified, otherwise use available container width
        let content_width = self.calculate_used_width(container_width, scale_factor);

        // Calculate content area with proper offset positioning and centering
        let base_content_x = offset_x + self.dimensions.margin.left + self.dimensions.border.left + self.dimensions.padding.left;
        let content_y = offset_y + self.dimensions.margin.top + self.dimensions.border.top + self.dimensions.padding.top;

        // Center horizontally if CSS width is specified
        let content_x = if self.css_width.is_some() {
            let available_width = container_width - self.dimensions.margin.left - self.dimensions.margin.right
                - self.dimensions.border.left - self.dimensions.border.right
                - self.dimensions.padding.left - self.dimensions.padding.right;
            let centering_offset = (available_width - content_width) / 2.0;
            base_content_x + centering_offset.max(0.0)
        } else {
            base_content_x
        };

        self.dimensions.content = Rect::from_xywh(content_x, content_y, content_width, 0.0);

        let mut current_y = content_y;
        let available_width = content_width;

        // Track floats in this formatting context
        let mut float_context = FloatContext::new();

        // Calculate gap spacing (row-gap for vertical stacking)
        let row_gap = self.gap.row.to_px(16.0, container_width) * scale_factor;

        // Layout children vertically, handling floats and gap
        for (i, child) in self.children.iter_mut().enumerate() {
            // Add row-gap before each child except the first
            if i > 0 && row_gap > 0.0 {
                current_y += row_gap;
            }

            // Handle clear property - move past floats if needed
            if child.clear != crate::css::Clear::None {
                current_y = float_context.get_clear_y(&child.clear, current_y);
            }

            // Check if this child is floated
            match child.float {
                crate::css::Float::Left => {
                    // Layout the child first to get its dimensions
                    child.layout(available_width, container_height, content_x, current_y, scale_factor);

                    // Find appropriate Y position for the float if current position doesn't have space
                    let float_width = child.dimensions.total_width();
                    let float_y = float_context.find_y_for_width(current_y, float_width, available_width, content_x);

                    // Get the left edge position considering other floats
                    let (left_edge, _) = float_context.get_available_width(float_y, available_width, content_x);

                    // Position the float at the left edge
                    let float_x = left_edge;
                    child.layout(available_width, container_height, float_x - child.dimensions.margin.left, float_y - child.dimensions.margin.top, scale_factor);

                    // Register this float in the context
                    let float_height = child.dimensions.total_height();
                    float_context.left_floats.push((float_x, float_y, float_width, float_height));

                    // Floats don't affect current_y for normal flow (they're taken out of flow)
                }
                crate::css::Float::Right => {
                    // Layout the child first to get its dimensions
                    child.layout(available_width, container_height, content_x, current_y, scale_factor);

                    // Find appropriate Y position for the float if current position doesn't have space
                    let float_width = child.dimensions.total_width();
                    let float_y = float_context.find_y_for_width(current_y, float_width, available_width, content_x);

                    // Get the right edge position considering other floats
                    let (_, avail_width) = float_context.get_available_width(float_y, available_width, content_x);

                    // Position the float at the right edge
                    let float_x = content_x + avail_width - float_width;
                    child.layout(available_width, container_height, float_x - child.dimensions.margin.left, float_y - child.dimensions.margin.top, scale_factor);

                    // Register this float in the context
                    let float_height = child.dimensions.total_height();
                    float_context.right_floats.push((float_x, float_y, float_width, float_height));

                    // Floats don't affect current_y for normal flow (they're taken out of flow)
                }
                crate::css::Float::None => {
                    // Normal flow element - position it considering active floats
                    // Get available width at current Y position
                    let (_left_edge, avail_width) = float_context.get_available_width(current_y, available_width, content_x);

                    // If available width is too small, move down to find space
                    let required_width = child.dimensions.margin.left + child.dimensions.margin.right + 10.0; // Minimum required
                    let actual_y = if avail_width < required_width {
                        float_context.find_y_for_width(current_y, required_width, available_width, content_x)
                    } else {
                        current_y
                    };

                    // Get the final available space at the chosen Y position
                    let (final_left_edge, final_avail_width) = float_context.get_available_width(actual_y, available_width, content_x);

                    // Layout the child with the available width, positioned at the left edge
                    child.layout(final_avail_width, container_height, final_left_edge, actual_y, scale_factor);

                    // Advance current_y for the next normal flow element
                    current_y = actual_y + child.dimensions.total_height();
                }
            }
        }

        // Calculate auto content height based on children and floats
        let auto_content_height = if self.children.is_empty() {
            0.0
        } else {
            // Start with normal flow height
            let mut max_height = current_y - content_y;

            // Check if any floats extend beyond normal flow
            for (_, fy, _, fh) in &float_context.left_floats {
                max_height = max_height.max(fy + fh - content_y);
            }
            for (_, fy, _, fh) in &float_context.right_floats {
                max_height = max_height.max(fy + fh - content_y);
            }

            max_height
        };

        // Use CSS height if specified, otherwise use auto height
        let final_content_height = self.calculate_used_height(container_height, scale_factor, auto_content_height);

        // Center vertically if CSS height is specified
        let final_content_y = if self.css_height.is_some() {
            let available_height = container_height - self.dimensions.margin.top - self.dimensions.margin.bottom
                - self.dimensions.border.top - self.dimensions.border.bottom
                - self.dimensions.padding.top - self.dimensions.padding.bottom;
            let centering_offset = (available_height - final_content_height) / 2.0;
            content_y + centering_offset.max(0.0)
        } else {
            content_y
        };

        // Update our content dimensions with the final height and position
        self.dimensions.content = Rect::from_xywh(
            content_x,
            final_content_y,
            content_width,
            final_content_height
        );
    }

    /// Layout inline elements with position offset (flow horizontally)
    fn layout_inline(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        // Scale padding for high DPI
        self.dimensions.padding = EdgeSizes::uniform(2.0 * scale_factor);

        let base_content_x = offset_x + self.dimensions.padding.left;
        let base_content_y = offset_y + self.dimensions.padding.top;

        // Calculate default inline height and width
        let default_height = 20.0 * scale_factor; // Scale line height
        let default_width = container_width - self.dimensions.padding.left - self.dimensions.padding.right;

        // Use CSS dimensions if specified
        let content_width = if let Some(css_width) = &self.css_width {
            css_width.to_px(16.0, container_width) * scale_factor
        } else {
            default_width
        };

        let final_height = self.calculate_used_height(container_height, scale_factor, default_height);

        // Center horizontally if CSS width is specified
        let content_x = if self.css_width.is_some() {
            let centering_offset = (default_width - content_width) / 2.0;
            base_content_x + centering_offset.max(0.0)
        } else {
            base_content_x
        };

        // Center vertically if CSS height is specified
        let content_y = if self.css_height.is_some() {
            let available_height = container_height - self.dimensions.padding.top - self.dimensions.padding.bottom;
            let centering_offset = (available_height - final_height) / 2.0;
            base_content_y + centering_offset.max(0.0)
        } else {
            base_content_y
        };

        self.dimensions.content = Rect::from_xywh(
            content_x,
            content_y,
            content_width,
            final_height
        );

        // Calculate gap spacing (column-gap for horizontal flow)
        let column_gap = self.gap.column.to_px(16.0, container_width) * scale_factor;

        // Layout children horizontally with column-gap
        let mut current_x = content_x;
        let child_count = self.children.len().max(1);
        let child_width = content_width / child_count as f32;

        for (i, child) in self.children.iter_mut().enumerate() {
            // Add column-gap before each child except the first
            if i > 0 && column_gap > 0.0 {
                current_x += column_gap;
            }

            child.layout(child_width, final_height, current_x, content_y, scale_factor);
            current_x += child.dimensions.total_width();
        }
    }

    /// Layout inline-block elements with position offset
    fn layout_inline_block(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        // Similar to block but flows inline - scale the max width
        self.layout_block((container_width).min(200.0 * scale_factor), container_height, offset_x, offset_y, scale_factor);
    }

    /// Layout text nodes with position offset
    fn layout_text(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        if let Some(text) = &self.content {
            // Handle newlines and calculate proper text dimensions - scale for high DPI
            let char_width = 8.0 * scale_factor; // Average character width, scaled
            let line_height = 16.0 * scale_factor; // Line height, scaled

            // Wrap text to fit within container width
            let wrapped_lines = self.wrap_text(text, container_width, char_width);
            let num_lines = wrapped_lines.len().max(1);

            // Calculate width based on the longest wrapped line
            let max_line_width = wrapped_lines.iter()
                .map(|line| line.len() as f32 * char_width)
                .fold(0.0, f32::max)
                .min(container_width);
            
            let auto_text_width = if text.trim().is_empty() { 0.0 } else { max_line_width };
            let auto_text_height = num_lines as f32 * line_height;

            // Use CSS dimensions if specified, otherwise use calculated dimensions
            let final_text_width = if let Some(css_width) = &self.css_width {
                css_width.to_px(16.0, container_width) * scale_factor
            } else {
                auto_text_width
            };

            let final_text_height = self.calculate_used_height(container_height, scale_factor, auto_text_height);

            // Center horizontally if CSS width is specified
            let final_x = if self.css_width.is_some() {
                let centering_offset = (container_width - final_text_width) / 2.0;
                offset_x + centering_offset.max(0.0)
            } else {
                offset_x
            };

            // Center vertically if CSS height is specified
            let final_y = if self.css_height.is_some() {
                let centering_offset = (container_height - final_text_height) / 2.0;
                offset_y + centering_offset.max(0.0)
            } else {
                offset_y
            };

            self.dimensions.content = Rect::from_xywh(
                final_x,
                final_y,
                final_text_width,
                final_text_height
            );
        } else {
            // Empty text node - use CSS dimensions if specified
            let final_width = if let Some(css_width) = &self.css_width {
                css_width.to_px(16.0, container_width) * scale_factor
            } else {
                0.0
            };
            let final_height = self.calculate_used_height(container_height, scale_factor, 0.0);

            // Center if CSS dimensions are specified
            let final_x = if self.css_width.is_some() {
                let centering_offset = (container_width - final_width) / 2.0;
                offset_x + centering_offset.max(0.0)
            } else {
                offset_x
            };

            let final_y = if self.css_height.is_some() {
                let centering_offset = (container_height - final_height) / 2.0;
                offset_y + centering_offset.max(0.0)
            } else {
                offset_y
            };

            self.dimensions.content = Rect::from_xywh(final_x, final_y, final_width, final_height);
        }
    }

    /// Helper function to wrap text into lines that fit within a given width
    fn wrap_text(&self, text: &str, max_width: f32, char_width: f32) -> Vec<String> {
        let mut wrapped_lines = Vec::new();

        // Split by explicit newlines first
        let paragraphs: Vec<&str> = text.split('\n').collect();

        for paragraph in paragraphs {
            if paragraph.is_empty() {
                wrapped_lines.push(String::new());
                continue;
            }

            // Calculate max characters per line based on actual character count
            let max_chars = (max_width / char_width).floor() as usize;

            if max_chars == 0 {
                // If width is too small, just add the paragraph as-is
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

                // Calculate the character count if we add this word
                let test_char_count = if current_line.is_empty() {
                    word_char_count
                } else {
                    current_char_count + 1 + word_char_count // +1 for space
                };

                if test_char_count <= max_chars {
                    // Add word to current line
                    if current_line.is_empty() {
                        current_line.push_str(word);
                        current_char_count = word_char_count;
                    } else {
                        current_line.push(' ');
                        current_line.push_str(word);
                        current_char_count = test_char_count;
                    }
                } else {
                    // Word doesn't fit on current line
                    if !current_line.is_empty() {
                        // Save current line and start new line with this word
                        wrapped_lines.push(current_line);
                        current_line = String::new();
                        current_char_count = 0;
                    }

                    // Check if word itself is too long and needs to be broken
                    if word_char_count > max_chars {
                        // Break the word across multiple lines
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
                        // Word fits within max_chars, start new line with it
                        current_line.push_str(word);
                        current_char_count = word_char_count;
                    }
                }
            }

            // Add the last line if it's not empty
            if !current_line.is_empty() {
                wrapped_lines.push(current_line);
            }
        }

        // Return at least one empty line if everything was empty
        if wrapped_lines.is_empty() {
            wrapped_lines.push(String::new());
        }

        wrapped_lines
    }

    /// Layout image nodes with position offset
    fn layout_image(&mut self, data: RefCell<ImageData>, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        let data = data.borrow();
        // Default image dimensions
        let default_width = 150;
        let default_height = 100;

        // Use specified dimensions from HTML attributes if available, or CSS if specified
        let base_image_width = data.width.unwrap_or(default_width) as f32;
        let base_image_height = data.height.unwrap_or(default_height) as f32;

        // Apply CSS dimensions if specified, otherwise use HTML attributes or defaults
        let final_image_width = if let Some(css_width) = &self.css_width {
            css_width.to_px(16.0, container_width) * scale_factor
        } else {
            base_image_width * scale_factor
        };

        let final_image_height = if let Some(css_height) = &self.css_height {
            css_height.to_px(16.0, container_height) * scale_factor
        } else {
            base_image_height * scale_factor
        };

        // Set margins for inline-block behavior - scale for high DPI
        self.dimensions.margin = EdgeSizes::new(
            4.0 * scale_factor,
            4.0 * scale_factor,
            4.0 * scale_factor,
            4.0 * scale_factor
        );
        self.dimensions.padding = EdgeSizes::new(0.0, 0.0, 0.0, 0.0);

        // Calculate base position with margins
        let base_x = offset_x + self.dimensions.margin.left;
        let base_y = offset_y + self.dimensions.margin.top;

        // Center horizontally if CSS width is specified
        let final_x = if self.css_width.is_some() {
            let available_width = container_width - self.dimensions.margin.left - self.dimensions.margin.right;
            let centering_offset = (available_width - final_image_width) / 2.0;
            base_x + centering_offset.max(0.0)
        } else {
            base_x
        };

        // Center vertically if CSS height is specified
        let final_y = if self.css_height.is_some() {
            let available_height = container_height - self.dimensions.margin.top - self.dimensions.margin.bottom;
            let centering_offset = (available_height - final_image_height) / 2.0;
            base_y + centering_offset.max(0.0)
        } else {
            base_y
        };

        self.dimensions.content = Rect::from_xywh(
            final_x,
            final_y,
            final_image_width,
            final_image_height
        );
    }

    /// Calculate the actual width this box should use, respecting CSS width values, flex-basis, and box-sizing
    fn calculate_used_width(&self, container_width: f32, scale_factor: f32) -> f32 {
        // Determine the base width: prioritize flex-basis, then width, then auto
        let mut width = match &self.flex_basis {
            crate::css::FlexBasis::Length(length) => {
                // flex-basis with explicit length takes precedence
                let specified_width = length.to_px(16.0, container_width) * scale_factor;

                // Apply box-sizing logic
                match self.box_sizing {
                    crate::css::BoxSizing::ContentBox => specified_width,
                    crate::css::BoxSizing::BorderBox => {
                        specified_width
                            - self.dimensions.padding.left - self.dimensions.padding.right
                            - self.dimensions.border.left - self.dimensions.border.right
                    }
                }
            }
            crate::css::FlexBasis::Auto => {
                // When flex-basis is auto, fall back to width property
                if let Some(css_width) = &self.css_width {
                    let specified_width = css_width.to_px(16.0, container_width) * scale_factor;

                    // Apply box-sizing logic
                    match self.box_sizing {
                        crate::css::BoxSizing::ContentBox => specified_width,
                        crate::css::BoxSizing::BorderBox => {
                            specified_width
                                - self.dimensions.padding.left - self.dimensions.padding.right
                                - self.dimensions.border.left - self.dimensions.border.right
                        }
                    }
                } else {
                    // Use auto width (full container width minus margins, borders, padding)
                    container_width - self.dimensions.padding.left - self.dimensions.padding.right
                        - self.dimensions.border.left - self.dimensions.border.right
                        - self.dimensions.margin.left - self.dimensions.margin.right
                }
            }
            crate::css::FlexBasis::Content => {
                // Content-based sizing - use intrinsic content size
                // For now, fallback to width or auto (future: calculate from content)
                if let Some(css_width) = &self.css_width {
                    let specified_width = css_width.to_px(16.0, container_width) * scale_factor;
                    match self.box_sizing {
                        crate::css::BoxSizing::ContentBox => specified_width,
                        crate::css::BoxSizing::BorderBox => {
                            specified_width
                                - self.dimensions.padding.left - self.dimensions.padding.right
                                - self.dimensions.border.left - self.dimensions.border.right
                        }
                    }
                } else {
                    // Use container width for content-based sizing as fallback
                    container_width - self.dimensions.padding.left - self.dimensions.padding.right
                        - self.dimensions.border.left - self.dimensions.border.right
                        - self.dimensions.margin.left - self.dimensions.margin.right
                }
            }
        };

        // Apply max-width constraint if specified
        if let Some(css_max_width) = &self.css_max_width {
            let max_width = css_max_width.to_px(16.0, container_width) * scale_factor;
            let max_width_content = match self.box_sizing {
                crate::css::BoxSizing::ContentBox => max_width,
                crate::css::BoxSizing::BorderBox => {
                    max_width
                        - self.dimensions.padding.left - self.dimensions.padding.right
                        - self.dimensions.border.left - self.dimensions.border.right
                }
            };
            width = width.min(max_width_content);
        }

        // Apply min-width constraint if specified
        if let Some(css_min_width) = &self.css_min_width {
            let min_width = css_min_width.to_px(16.0, container_width) * scale_factor;
            let min_width_content = match self.box_sizing {
                crate::css::BoxSizing::ContentBox => min_width,
                crate::css::BoxSizing::BorderBox => {
                    min_width
                        - self.dimensions.padding.left - self.dimensions.padding.right
                        - self.dimensions.border.left - self.dimensions.border.right
                }
            };
            width = width.max(min_width_content);
        }

        width
    }

    /// Calculate the actual height this box should use, respecting CSS height values, flex-basis, and box-sizing
    fn calculate_used_height(&self, container_height: f32, scale_factor: f32, content_height: f32) -> f32 {
        // Note: flex-basis primarily affects the main axis in flex containers
        // For vertical flex containers, flex-basis would control height
        // For now, we support it but height property takes precedence in non-flex contexts

        let mut height = if let Some(css_height) = &self.css_height {
            // Use the CSS-specified height, converting to pixels and scaling
            let specified_height = css_height.to_px(16.0, container_height) * scale_factor;

            // Apply box-sizing logic
            match self.box_sizing {
                crate::css::BoxSizing::ContentBox => {
                    // Default behavior: height applies to content box only
                    specified_height
                }
                crate::css::BoxSizing::BorderBox => {
                    // Height includes padding and border, so subtract them to get content height
                    specified_height
                        - self.dimensions.padding.top - self.dimensions.padding.bottom
                        - self.dimensions.border.top - self.dimensions.border.bottom
                }
            }
        } else {
            // Use auto height (content-based height or minimum height for empty blocks)
            if content_height > 0.0 {
                content_height
            } else {
                20.0 * scale_factor // Minimum height for empty blocks, scaled
            }
        };

        // Apply max-height constraint if specified
        if let Some(css_max_height) = &self.css_max_height {
            let max_height = css_max_height.to_px(16.0, container_height) * scale_factor;
            let max_height_content = match self.box_sizing {
                crate::css::BoxSizing::ContentBox => max_height,
                crate::css::BoxSizing::BorderBox => {
                    max_height
                        - self.dimensions.padding.top - self.dimensions.padding.bottom
                        - self.dimensions.border.top - self.dimensions.border.bottom
                }
            };
            height = height.min(max_height_content);
        }

        // Apply min-height constraint if specified
        if let Some(css_min_height) = &self.css_min_height {
            let min_height = css_min_height.to_px(16.0, container_height) * scale_factor;
            let min_height_content = match self.box_sizing {
                crate::css::BoxSizing::ContentBox => min_height,
                crate::css::BoxSizing::BorderBox => {
                    min_height
                        - self.dimensions.padding.top - self.dimensions.padding.bottom
                        - self.dimensions.border.top - self.dimensions.border.bottom
                }
            };
            height = height.max(min_height_content);
        }

        height
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

    /// Layout flex container - arranges children horizontally
    fn layout_flex(&mut self, container_width: f32, container_height: f32, offset_x: f32, offset_y: f32, scale_factor: f32) {
        // Scale margins, padding, and borders for high DPI
        self.scale_edge_sizes(scale_factor);

        // Use CSS width if specified, otherwise use available container width
        let content_width = self.calculate_used_width(container_width, scale_factor);

        // Calculate content area with proper offset positioning
        let content_x = offset_x + self.dimensions.margin.left + self.dimensions.border.left + self.dimensions.padding.left;
        let content_y = offset_y + self.dimensions.margin.top + self.dimensions.border.top + self.dimensions.padding.top;

        self.dimensions.content = Rect::from_xywh(content_x, content_y, content_width, 0.0);

        // Calculate gap spacing (column-gap for horizontal flex layout)
        let column_gap = self.gap.column.to_px(16.0, container_width) * scale_factor;

        // Calculate total gap space
        let total_gap = if self.children.len() > 1 {
            column_gap * (self.children.len() - 1) as f32
        } else {
            0.0
        };

        // Available space for flex items (excluding gaps)
        let available_width = content_width - total_gap;

        // Step 1: Calculate the hypothetical main size (base size) for each flex item
        let mut flex_items: Vec<(f32, f32, f32)> = Vec::new(); // (base_size, flex_grow, flex_shrink)

        for child in &self.children {
            // Determine the flex base size according to flex-basis
            let base_size = match &child.flex_basis {
                crate::css::FlexBasis::Length(length) => {
                    // Explicit length value (including 0px from "flex: 1")
                    length.to_px(16.0, content_width) * scale_factor
                }
                crate::css::FlexBasis::Auto => {
                    // If flex-basis is auto, use the item's main size (width) if specified
                    if let Some(css_width) = &child.css_width {
                        css_width.to_px(16.0, content_width) * scale_factor
                    } else {
                        // Use content size - for now, we'll use a minimum size
                        // In a full implementation, this would calculate intrinsic content size
                        // For flex items without width, use a small default that will grow
                        50.0 * scale_factor // Minimum content size
                    }
                }
                crate::css::FlexBasis::Content => {
                    // Content-based sizing - use intrinsic content size
                    // For now, use a minimum default
                    50.0 * scale_factor
                }
            };

            flex_items.push((base_size, child.flex_grow.0, child.flex_shrink.0));
        }

        // Step 2: Calculate total base size
        let total_base_size: f32 = flex_items.iter().map(|(size, _, _)| size).sum();

        // Step 3: Determine if we need to grow or shrink
        let remaining_space = available_width - total_base_size;

        let mut final_widths = Vec::new();

        if remaining_space > 0.0 {
            // We have extra space - distribute using flex-grow
            let total_grow: f32 = flex_items.iter().map(|(_, grow, _)| grow).sum();

            if total_grow > 0.0 {
                // Distribute extra space proportionally based on flex-grow
                for (base_size, grow, _) in &flex_items {
                    let extra = (grow / total_grow) * remaining_space;
                    final_widths.push(base_size + extra);
                }
            } else {
                // No flex-grow, items keep their base size
                for (base_size, _, _) in &flex_items {
                    final_widths.push(*base_size);
                }
            }
        } else if remaining_space < 0.0 {
            // We need to shrink - distribute using flex-shrink
            let total_shrink: f32 = flex_items.iter().map(|(base, _, shrink)| base * shrink).sum();

            if total_shrink > 0.0 {
                // Shrink items proportionally based on flex-shrink weighted by base size
                for (base_size, _, shrink) in &flex_items {
                    let shrink_amount = (base_size * shrink / total_shrink) * remaining_space.abs();
                    final_widths.push((base_size - shrink_amount).max(0.0));
                }
            } else {
                // No flex-shrink, items keep their base size (may overflow)
                for (base_size, _, _) in &flex_items {
                    final_widths.push(*base_size);
                }
            }
        } else {
            // Perfect fit - no growth or shrinkage needed
            for (base_size, _, _) in &flex_items {
                final_widths.push(*base_size);
            }
        }

        // Step 4: Layout children horizontally with calculated widths
        let mut current_x = content_x;
        let mut max_child_height = 0.0f32;

        for (i, child) in self.children.iter_mut().enumerate() {
            // Add column-gap before each child except the first
            if i > 0 {
                current_x += column_gap;
            }

            let child_width = final_widths[i];

            // Layout the child with the calculated width
            child.layout(child_width, container_height, current_x, content_y, scale_factor);

            // Track maximum child height for container height
            max_child_height = max_child_height.max(child.dimensions.total_height());

            current_x += child_width;
        }

        // Calculate auto content height based on children
        let auto_content_height = if self.children.is_empty() {
            0.0
        } else {
            max_child_height
        };

        // Use CSS height if specified, otherwise use auto height
        let final_content_height = self.calculate_used_height(container_height, scale_factor, auto_content_height);

        // Update our content dimensions with the final height
        self.dimensions.content = Rect::from_xywh(
            content_x,
            content_y,
            content_width,
            final_content_height
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

        // Store CSS width and height values
        self.css_width = styles.width.clone();
        self.css_height = styles.height.clone();
        self.css_max_width = styles.max_width.clone();
        self.css_min_width = styles.min_width.clone();
        self.css_max_height = styles.max_height.clone();
        self.css_min_height = styles.min_height.clone();

        // Store box-sizing value
        self.box_sizing = styles.box_sizing.clone();

        // Store flex properties
        self.flex_grow = styles.flex_grow.clone();
        self.flex_shrink = styles.flex_shrink.clone();
        self.flex_basis = styles.flex_basis.clone();

        // Store gap value
        self.gap = styles.gap.clone();

        // Store float and clear properties
        self.float = styles.float.clone();
        self.clear = styles.clear.clone();

        // Note: Other style properties like colors, fonts are handled in the renderer
        // Scale factor will be applied during layout phase
    }
}
