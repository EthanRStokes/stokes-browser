// CSS stroke-related values for SVG and graphics rendering

use super::color::Color;
use super::length::Length;

/// CSS stroke configuration
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub color: Option<Color>,
    pub width: Length,
    pub opacity: f32,
}

impl Stroke {
    /// Create a new stroke with default values
    pub fn new() -> Self {
        Self {
            color: None,
            width: Length::px(1.0),
            opacity: 1.0,
        }
    }

    /// Create a stroke with a specific color
    pub fn with_color(color: Color) -> Self {
        Self {
            color: Some(color),
            width: Length::px(1.0),
            opacity: 1.0,
        }
    }

    /// Create a stroke with specific width
    pub fn with_width(width: Length) -> Self {
        Self {
            color: None,
            width,
            opacity: 1.0,
        }
    }

    /// Check if stroke should be rendered
    pub fn is_visible(&self) -> bool {
        self.color.is_some() && self.opacity > 0.0 && self.width.to_px(16.0, 0.0) > 0.0
    }

    /// Get the stroke width in pixels
    pub fn width_px(&self, font_size: f32, parent_size: f32) -> f32 {
        self.width.to_px(font_size, parent_size)
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Self::new()
    }
}

