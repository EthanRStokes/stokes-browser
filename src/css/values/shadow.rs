// CSS box-shadow values

use super::color::Color;
use super::length::Length;
use crate::css::values::CssValue;

/// Box shadow configuration
#[derive(Debug, Clone, PartialEq)]
pub struct BoxShadow {
    pub offset_x: Length,
    pub offset_y: Length,
    pub blur_radius: Length,
    pub spread_radius: Length,
    pub color: Color,
    pub inset: bool,
}

impl BoxShadow {
    /// Create a new box shadow with default values
    pub fn new(offset_x: Length, offset_y: Length, blur_radius: Length, color: Color) -> Self {
        Self {
            offset_x,
            offset_y,
            blur_radius,
            spread_radius: Length::px(0.0),
            color,
            inset: false,
        }
    }

    /// Create a box shadow with all parameters
    pub fn with_spread(
        offset_x: Length,
        offset_y: Length,
        blur_radius: Length,
        spread_radius: Length,
        color: Color,
        inset: bool,
    ) -> Self {
        Self {
            offset_x,
            offset_y,
            blur_radius,
            spread_radius,
            color,
            inset,
        }
    }

    /// Convert to pixel values for rendering
    pub fn to_px(&self, font_size: f32, parent_size: f32) -> BoxShadowPx {
        BoxShadowPx {
            offset_x: self.offset_x.to_px(font_size, parent_size),
            offset_y: self.offset_y.to_px(font_size, parent_size),
            blur_radius: self.blur_radius.to_px(font_size, parent_size),
            spread_radius: self.spread_radius.to_px(font_size, parent_size),
            color: self.color.clone(),
            inset: self.inset,
        }
    }

    /// Parse box-shadow from CSS string
    pub fn parse(value: &str) -> Option<Vec<BoxShadow>> {
        // Split by comma for multiple shadows
        let shadow_strings: Vec<&str> = value.split(',').map(|s| s.trim()).collect();
        let mut shadows = Vec::new();

        for shadow_str in shadow_strings {
            if let Some(shadow) = Self::parse_single_shadow(shadow_str) {
                shadows.push(shadow);
            }
        }

        if shadows.is_empty() {
            None
        } else {
            Some(shadows)
        }
    }

    fn parse_single_shadow(value: &str) -> Option<BoxShadow> {
        let value = value.trim();

        // Check for "none"
        if value == "none" {
            return None;
        }

        let mut parts: Vec<&str> = value.split_whitespace().collect();
        let mut inset = false;

        // Check for inset keyword
        if parts.first() == Some(&"inset") {
            inset = true;
            parts.remove(0);
        } else if parts.last() == Some(&"inset") {
            inset = true;
            parts.pop();
        }

        // Need at least 2 values (offset-x, offset-y)
        if parts.len() < 2 {
            return None;
        }

        // Parse offset-x and offset-y (required)
        let offset_x = CssValue::parse(parts[0]);
        let offset_y = CssValue::parse(parts[1]);

        let offset_x = if let CssValue::Length(len) = offset_x { len } else { return None; };
        let offset_y = if let CssValue::Length(len) = offset_y { len } else { return None; };

        let mut blur_radius = Length::px(0.0);
        let mut spread_radius = Length::px(0.0);
        let mut color = Color::Rgba { r: 0, g: 0, b: 0, a: 0.5 }; // Default shadow color

        // Parse remaining values
        let mut i = 2;
        while i < parts.len() {
            let part = parts[i];
            let css_value = CssValue::parse(part);

            match css_value {
                CssValue::Length(len) => {
                    if i == 2 {
                        blur_radius = len;
                    } else if i == 3 {
                        spread_radius = len;
                    }
                },
                CssValue::Color(c) => {
                    color = c;
                },
                _ => {
                    // Try to parse as color if it's a named color or hex
                    if Self::could_be_color(part) {
                        if let CssValue::Color(c) = CssValue::parse(part) {
                            color = c;
                        }
                    }
                }
            }
            i += 1;
        }

        Some(BoxShadow::with_spread(
            offset_x,
            offset_y,
            blur_radius,
            spread_radius,
            color,
            inset,
        ))
    }

    fn could_be_color(value: &str) -> bool {
        value.starts_with('#') ||
        value.starts_with("rgb") ||
        matches!(CssValue::parse(value), CssValue::Color(_))
    }
}

impl Default for BoxShadow {
    fn default() -> Self {
        Self {
            offset_x: Length::px(0.0),
            offset_y: Length::px(0.0),
            blur_radius: Length::px(0.0),
            spread_radius: Length::px(0.0),
            color: Color::Rgba { r: 0, g: 0, b: 0, a: 0.5 },
            inset: false,
        }
    }
}

/// Box shadow in pixels for rendering
#[derive(Debug, Clone, PartialEq)]
pub struct BoxShadowPx {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: Color,
    pub inset: bool,
}

impl BoxShadowPx {
    pub fn has_shadow(&self) -> bool {
        self.blur_radius > 0.0 || self.spread_radius > 0.0 ||
        self.offset_x != 0.0 || self.offset_y != 0.0
    }
}

