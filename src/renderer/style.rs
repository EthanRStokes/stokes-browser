// Style module - responsible for handling CSS styling
use std::collections::HashMap;

/// Represents different CSS display types for elements
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayType {
    Block,
    Inline,
    None,
    Flex,
    Grid,
    InlineBlock,
}

/// A complete set of computed styles for an element
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    // Colors
    pub color: [f32; 3],
    pub background_color: [f32; 3],

    // Box model
    pub width: Option<Dimension>,
    pub height: Option<Dimension>,
    pub margin: BoxValues<Dimension>,
    pub padding: BoxValues<Dimension>,
    pub border: BoxValues<Border>,

    // Layout
    pub display: DisplayType,
    pub position: PositionType,
    pub top: Option<Dimension>,
    pub right: Option<Dimension>,
    pub bottom: Option<Dimension>,
    pub left: Option<Dimension>,

    // Typography
    pub font_size: Dimension,
    pub font_weight: FontWeight,
    pub font_family: Vec<String>,
    pub text_align: TextAlign,
}

/// Represents a CSS dimension (px, em, %, etc.)
#[derive(Debug, Clone, Copy)]
pub enum Dimension {
    Pixels(f32),
    Percentage(f32),
    Em(f32),
    Rem(f32),
    Auto,
}

/// Represents values for the four sides of an element
#[derive(Debug, Clone)]
pub struct BoxValues<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

/// Represents border properties
#[derive(Debug, Clone)]
pub struct Border {
    pub width: f32,
    pub style: BorderStyle,
    pub color: [f32; 3],
}

/// Different border styles
#[derive(Debug, Clone, Copy)]
pub enum BorderStyle {
    None,
    Solid,
    Dashed,
    Dotted,
}

/// Positioning types
#[derive(Debug, Clone, Copy)]
pub enum PositionType {
    Static,
    Relative,
    Absolute,
    Fixed,
}

/// Font weight options
#[derive(Debug, Clone, Copy)]
pub enum FontWeight {
    Normal,
    Bold,
    Value(f32),
}

/// Text alignment options
#[derive(Debug, Clone, Copy)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

/// Stylesheet with default element styles
pub struct StyleSheet {
    element_styles: HashMap<String, ComputedStyle>,
}

impl StyleSheet {
    pub fn new() -> Self {
        let mut element_styles = HashMap::new();

        // Add default styles for common elements
        // In a real browser, these would come from the user agent stylesheet

        Self { element_styles }
    }

    /// Retrieve style for a given element tag
    pub fn get_style(&self, tag_name: &str) -> Option<&ComputedStyle> {
        self.element_styles.get(tag_name)
    }

    /// Apply a style to an element type
    pub fn set_style(&mut self, tag_name: String, style: ComputedStyle) {
        self.element_styles.insert(tag_name, style);
    }
}

/// Style resolver to handle CSS cascade, inheritance, and specificity
pub struct StyleResolver {
    pub stylesheet: StyleSheet,
}

impl StyleResolver {
    pub fn new() -> Self {
        Self {
            stylesheet: StyleSheet::new(),
        }
    }

    /// Compute final styles for an element based on inherited styles,
    /// element type, classes, IDs, and inline styles
    pub fn compute_style_for_element(&self, element_tag: &str) -> ComputedStyle {
        // For now, just return a simple default style
        // In a real implementation, this would handle inheritance, cascade, etc.
        todo!("Implement style resolution")
    }
}
