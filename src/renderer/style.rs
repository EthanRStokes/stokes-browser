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
        // For a basic implementation, just return default styles based on element type
        // In a full browser, this would handle CSS cascading and specificity

        match self.stylesheet.get_style(element_tag) {
            Some(style) => style.clone(),
            None => {
                // Create a default style based on element type
                let mut style = ComputedStyle {
                    color: [0.0, 0.0, 0.0], // Black text
                    background_color: [1.0, 1.0, 1.0], // White background
                    width: None,
                    height: None,
                    margin: crate::renderer::style::BoxValues {
                        top: crate::renderer::style::Dimension::Pixels(0.0),
                        right: crate::renderer::style::Dimension::Pixels(0.0),
                        bottom: crate::renderer::style::Dimension::Pixels(0.0),
                        left: crate::renderer::style::Dimension::Pixels(0.0),
                    },
                    padding: crate::renderer::style::BoxValues {
                        top: crate::renderer::style::Dimension::Pixels(0.0),
                        right: crate::renderer::style::Dimension::Pixels(0.0),
                        bottom: crate::renderer::style::Dimension::Pixels(0.0),
                        left: crate::renderer::style::Dimension::Pixels(0.0),
                    },
                    border: crate::renderer::style::BoxValues {
                        top: crate::renderer::style::Border {
                            width: 0.0,
                            style: crate::renderer::style::BorderStyle::None,
                            color: [0.0, 0.0, 0.0],
                        },
                        right: crate::renderer::style::Border {
                            width: 0.0,
                            style: crate::renderer::style::BorderStyle::None,
                            color: [0.0, 0.0, 0.0],
                        },
                        bottom: crate::renderer::style::Border {
                            width: 0.0,
                            style: crate::renderer::style::BorderStyle::None,
                            color: [0.0, 0.0, 0.0],
                        },
                        left: crate::renderer::style::Border {
                            width: 0.0,
                            style: crate::renderer::style::BorderStyle::None,
                            color: [0.0, 0.0, 0.0],
                        },
                    },
                    display: crate::renderer::style::DisplayType::Inline,
                    position: crate::renderer::style::PositionType::Static,
                    top: None,
                    right: None,
                    bottom: None,
                    left: None,
                    font_size: crate::renderer::style::Dimension::Pixels(16.0),
                    font_weight: crate::renderer::style::FontWeight::Normal,
                    font_family: vec!["Arial".to_string(), "sans-serif".to_string()],
                    text_align: crate::renderer::style::TextAlign::Left,
                };

                // Apply element-specific styles
                match element_tag {
                    "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article" => {
                        style.display = crate::renderer::style::DisplayType::Block;
                    }
                    "a" => {
                        style.color = [0.0, 0.0, 1.0]; // Blue for links
                    }
                    "strong" | "b" => {
                        style.font_weight = crate::renderer::style::FontWeight::Bold;
                    }
                    "h1" => {
                        style.font_size = crate::renderer::style::Dimension::Pixels(32.0);
                        style.font_weight = crate::renderer::style::FontWeight::Bold;
                    }
                    "h2" => {
                        style.font_size = crate::renderer::style::Dimension::Pixels(24.0);
                        style.font_weight = crate::renderer::style::FontWeight::Bold;
                    }
                    _ => {}
                }

                style
            }
        }
    }
}
