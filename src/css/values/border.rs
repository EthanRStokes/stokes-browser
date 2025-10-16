// CSS border-related values

use super::color::Color;
use super::length::Length;
use crate::css::values::CssValue;

/// CSS border radius values
#[derive(Debug, Clone, PartialEq)]
pub struct BorderRadius {
    pub top_left: Length,
    pub top_right: Length,
    pub bottom_right: Length,
    pub bottom_left: Length,
}

impl BorderRadius {
    /// Create uniform border radius for all corners
    pub fn uniform(radius: Length) -> Self {
        Self {
            top_left: radius.clone(),
            top_right: radius.clone(),
            bottom_right: radius.clone(),
            bottom_left: radius,
        }
    }

    /// Create border radius with individual corner values
    pub fn new(top_left: Length, top_right: Length, bottom_right: Length, bottom_left: Length) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    /// Convert all corner radii to pixels
    pub fn to_px(&self, font_size: f32, parent_size: f32) -> BorderRadiusPx {
        BorderRadiusPx {
            top_left: self.top_left.to_px(font_size, parent_size),
            top_right: self.top_right.to_px(font_size, parent_size),
            bottom_right: self.bottom_right.to_px(font_size, parent_size),
            bottom_left: self.bottom_left.to_px(font_size, parent_size),
        }
    }
}

impl Default for BorderRadius {
    fn default() -> Self {
        Self::uniform(Length::px(0.0))
    }
}

/// Border radius in pixels for rendering
#[derive(Debug, Clone, PartialEq)]
pub struct BorderRadiusPx {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl BorderRadiusPx {
    pub fn uniform(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    pub fn has_radius(&self) -> bool {
        self.top_left > 0.0 || self.top_right > 0.0 ||
        self.bottom_right > 0.0 || self.bottom_left > 0.0
    }
}

/// CSS outline style
#[derive(Debug, Clone, PartialEq)]
pub enum OutlineStyle {
    None,
    Hidden,
    Dotted,
    Dashed,
    Solid,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl OutlineStyle {
    /// Parse outline-style value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "none" => OutlineStyle::None,
            "hidden" => OutlineStyle::Hidden,
            "dotted" => OutlineStyle::Dotted,
            "dashed" => OutlineStyle::Dashed,
            "solid" => OutlineStyle::Solid,
            "double" => OutlineStyle::Double,
            "groove" => OutlineStyle::Groove,
            "ridge" => OutlineStyle::Ridge,
            "inset" => OutlineStyle::Inset,
            "outset" => OutlineStyle::Outset,
            _ => OutlineStyle::None, // Default to none
        }
    }
}

impl Default for OutlineStyle {
    fn default() -> Self {
        OutlineStyle::None
    }
}

/// CSS outline properties
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    pub width: Length,
    pub style: OutlineStyle,
    pub color: Color,
}

impl Outline {
    /// Create a new outline
    pub fn new(width: Length, style: OutlineStyle, color: Color) -> Self {
        Self { width, style, color }
    }

    /// Create a default outline (none)
    pub fn none() -> Self {
        Self {
            width: Length::px(0.0),
            style: OutlineStyle::None,
            color: Color::Named("black".to_string()),
        }
    }

    /// Parse outline shorthand from CSS string
    /// Format: <width> <style> <color> (in any order)
    /// Examples:
    ///   outline: 2px solid red;
    ///   outline: dashed blue 3px;
    ///   outline: none;
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        // Check for "none"
        if value.to_lowercase() == "none" {
            return Self::none();
        }

        let mut width = Length::px(3.0); // Default medium width
        let mut style = OutlineStyle::Solid; // Default style
        let mut color = Color::Named("currentcolor".to_string()); // Default to current color

        // Split by whitespace and parse each part
        let parts: Vec<&str> = value.split_whitespace().collect();

        for part in parts {
            // Try to parse as width (length)
            if let CssValue::Length(len) = CssValue::parse(part) {
                width = len;
            }
            // Try to parse as style
            else if Self::is_outline_style(part) {
                style = OutlineStyle::parse(part);
            }
            // Try to parse as color
            else if let CssValue::Color(c) = CssValue::parse(part) {
                color = c;
            }
            // Check for named outline width keywords
            else {
                match part.to_lowercase().as_str() {
                    "thin" => width = Length::px(1.0),
                    "medium" => width = Length::px(3.0),
                    "thick" => width = Length::px(5.0),
                    _ => {}
                }
            }
        }

        Self { width, style, color }
    }

    fn is_outline_style(value: &str) -> bool {
        matches!(value.to_lowercase().as_str(),
            "none" | "hidden" | "dotted" | "dashed" | "solid" |
            "double" | "groove" | "ridge" | "inset" | "outset"
        )
    }

    /// Check if outline is visible
    pub fn is_visible(&self) -> bool {
        !matches!(self.style, OutlineStyle::None | OutlineStyle::Hidden)
    }
}

impl Default for Outline {
    fn default() -> Self {
        Self::none()
    }
}
