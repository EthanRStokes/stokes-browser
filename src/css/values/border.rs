// CSS border-related values

use super::length::Length;

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

