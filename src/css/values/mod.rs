// CSS value types and parsing - organized module structure

pub mod color;
pub mod length;
pub mod border;
pub mod shadow;
pub mod text;
pub mod layout;
pub mod font;
pub mod background;
pub mod cursor;
pub mod list;
pub mod transition;
// Re-export commonly used types
pub use color::Color;
pub use length::{Length, Unit};
pub use border::{BorderRadius, BorderRadiusPx, Outline, OutlineStyle};
pub use shadow::{BoxShadow, BoxShadowPx};
pub use text::{TextDecoration, TextDecorationType, TextAlign, TextTransform, WhiteSpace};
pub use layout::{Clear, Float, Overflow, BoxSizing, Visibility, VerticalAlign, ContentValue, FlexBasis, FlexGrow, FlexShrink, Flex, Gap};
pub use font::{FontStyle, FontVariant, LineHeight};
pub use background::BackgroundImage;
pub use cursor::Cursor;
pub use list::ListStyleType;
pub use transition::{TimingFunction, StepPosition, Duration, Transition, TransitionProperty, TransitionSpec};

use std::fmt;

/// CSS property values
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    Length(Length),
    Color(Color),
    Number(f32),
    String(String),
    Keyword(String),
    Auto,
    MultipleValues(Vec<CssValue>), // For shorthand properties like "5em auto"
}

impl CssValue {
    /// Parse a CSS value from a string
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        // Check if this contains multiple space-separated values (shorthand syntax)
        let parts: Vec<&str> = value.split_whitespace().collect();
        if parts.len() > 1 {
            let parsed_values: Vec<CssValue> = parts.iter()
                .map(|part| Self::parse_single_value(part))
                .collect();
            return CssValue::MultipleValues(parsed_values);
        }

        // Single value
        Self::parse_single_value(value)
    }

    /// Parse a single CSS value (no spaces)
    fn parse_single_value(value: &str) -> Self {
        let value = value.trim();

        // Check for auto
        if value == "auto" {
            return CssValue::Auto;
        }

        // Check for color values
        if value.starts_with('#') {
            return CssValue::Color(Color::Hex(value.to_string()));
        }

        // Check for rgb/rgba colors
        if value.starts_with("rgb(") || value.starts_with("rgba(") {
            return Self::parse_rgb_color(value);
        }

        // Check for named colors
        if Self::is_named_color(value) {
            return CssValue::Color(Color::Named(value.to_string()));
        }

        // Check for length values (px, em, rem, %)
        if let Some(length) = Self::parse_length(value) {
            return CssValue::Length(length);
        }

        // Check for pure numbers
        if let Ok(num) = value.parse::<f32>() {
            return CssValue::Number(num);
        }

        // Check for quoted strings
        if (value.starts_with('"') && value.ends_with('"')) ||
           (value.starts_with('\'') && value.ends_with('\'')) {
            // Check if the string has content between quotes
            return if value.len() >= 2 {
                let unquoted = &value[1..value.len() - 1];
                CssValue::String(unquoted.to_string())
            } else {
                // Empty quotes, return empty string
                CssValue::String(String::new())
            }
        }

        // Default to keyword
        CssValue::Keyword(value.to_string())
    }

    fn parse_rgb_color(value: &str) -> CssValue {
        // Simple RGB/RGBA parsing
        let content = if value.starts_with("rgb(") {
            &value[4..value.len()-1]
        } else if value.starts_with("rgba(") {
            &value[5..value.len()-1]
        } else {
            return CssValue::Keyword(value.to_string());
        };

        let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

        if parts.len() >= 3 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                parts[0].parse::<u8>(),
                parts[1].parse::<u8>(),
                parts[2].parse::<u8>(),
            ) {
                if parts.len() >= 4 {
                    if let Ok(a) = parts[3].parse::<f32>() {
                        return CssValue::Color(Color::Rgba { r, g, b, a });
                    }
                }
                return CssValue::Color(Color::Rgb { r, g, b });
            }
        }

        CssValue::Keyword(value.to_string())
    }

    pub(crate) fn parse_length(value: &str) -> Option<Length> {
        if value.ends_with("px") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::px(num));
            }
        } else if value.ends_with("em") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::em(num));
            }
        } else if value.ends_with("rem") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length { value: num, unit: Unit::Rem });
            }
        } else if value.ends_with('%') {
            if let Ok(num) = value[..value.len()-1].parse::<f32>() {
                return Some(Length::percent(num));
            }
        } else if value.ends_with("pt") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::pt(num));
            }
        }
        None
    }

    fn is_named_color(value: &str) -> bool {
        matches!(value.to_lowercase().as_str(),
            "black" | "white" | "red" | "green" | "blue" | "yellow" |
            "cyan" | "magenta" | "gray" | "grey" | "orange" | "purple" |
            "brown" | "pink" | "lime" | "navy" | "teal" | "silver" |
            "maroon" | "olive" | "aqua" | "fuchsia"
        )
    }
}

impl fmt::Display for CssValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CssValue::Color(color) => write!(f, "{:?}", color),
            CssValue::Length(length) => write!(f, "{}{:?}", length.value, length.unit),
            CssValue::String(s) => write!(f, "{}", s),
            CssValue::Number(n) => write!(f, "{}", n),
            CssValue::Keyword(k) => write!(f, "{}", k),
            CssValue::Auto => write!(f, "auto"),
            CssValue::MultipleValues(values) => {
                let mut iter = values.iter();
                if let Some(first) = iter.next() {
                    write!(f, "{}", first)?;
                    for value in iter {
                        write!(f, " {}", value)?;
                    }
                }
                Ok(())
            },
        }
    }
}
