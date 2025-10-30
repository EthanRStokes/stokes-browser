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
pub mod stroke;
pub use background::BackgroundImage;
pub use border::{BorderRadius, BorderRadiusPx, Outline, OutlineStyle};
// Re-export commonly used types
pub use color::Color;
pub use cursor::Cursor;
pub use font::{FontStyle, FontVariant, LineHeight};
pub use layout::{BoxSizing, Clear, ContentValue, Flex, FlexBasis, FlexGrow, FlexShrink, Float, Gap, Overflow, VerticalAlign, Visibility};
pub use length::Length;
pub use list::ListStyleType;
pub use shadow::{BoxShadow, TextShadow};
pub use stroke::Stroke;
pub use text::{TextAlign, TextDecoration, TextDecorationType, TextTransform, WhiteSpace};
pub use transition::{TimingFunction, TransitionSpec};

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

        // Fast empty check
        if value.is_empty() {
            return CssValue::Keyword(String::new());
        }

        // Check for multiple space-separated values (shorthand syntax)
        // Only check if there's a space
        if value.contains(' ') {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() > 1 {
                let parsed_values: Vec<CssValue> = parts.iter()
                    .map(|part| Self::parse_single_value(part))
                    .collect();
                return CssValue::MultipleValues(parsed_values);
            }
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

        let bytes = value.as_bytes();
        if bytes.is_empty() {
            return CssValue::Keyword(String::new());
        }

        // Check for color values
        let first_byte = bytes[0];

        if first_byte == b'#' {
            return CssValue::Color(Color::Hex(value.to_string()));
        }

        // Check for rgb/rgba colors
        if first_byte == b'r' && value.len() > 4 {
            if value.starts_with("rgb(") {
                return Self::parse_rgb_color(value);
            } else if value.starts_with("rgba(") {
                return Self::parse_rgb_color(value);
            }
        }

        // Check for named colors (fast path for common colors)
        if Self::is_named_color_fast(value) {
            return CssValue::Color(Color::Named(value.to_string()));
        }

        // Check for length values (px, em, rem, %)
        // Most CSS values are lengths, so check this before numbers
        if let Some(length) = Self::parse_length(value) {
            return CssValue::Length(length);
        }

        // Check for pure numbers
        if let Ok(num) = value.parse::<f32>() {
            return CssValue::Number(num);
        }

        // Check for quoted strings
        if bytes.len() >= 2 {
            let first = bytes[0];
            let last = bytes[bytes.len() - 1];

            if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                let unquoted = &value[1..value.len() - 1];
                return CssValue::String(unquoted.to_string());
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

    // Fast path for common named colors
    #[inline]
    fn is_named_color_fast(value: &str) -> bool {
        // Check most common colors first
        matches!(value,
            "red" | "blue" | "green" | "white" | "black" | "gray" | "grey" |
            "yellow" | "orange" | "purple" | "pink" | "brown" | "cyan" | "magenta" |
            "transparent" | "inherit" | "initial" | "currentColor" |
            // Additional common colors
            "silver" | "maroon" | "olive" | "lime" | "aqua" | "teal" | "navy" | "fuchsia"
        ) || Self::is_named_color(value)
    }

    pub(crate) fn parse_length(value: &str) -> Option<Length> {
        if value.is_empty() {
            return None;
        }

        // CSS length values must be ASCII (numbers + unit). If the string contains
        // non-ASCII characters, it's not a valid length value.
        if !value.is_ascii() {
            return None;
        }

        let bytes = value.as_bytes();
        let len = bytes.len();

        // Need at least one digit and one unit character
        if len < 2 {
            return None;
        }

        // Fast path: check last 2 bytes for common units
        let last = bytes[len - 1];
        let second_last = bytes[len - 2];

        // Single character units (%, q)
        if last == b'%' {
            if let Ok(num) = value[..len-1].parse::<f32>() {
                return Some(Length::percent(num));
            }
            return None;
        }

        // Two character units (most common: px, em, vh, vw, etc.)
        if len >= 3 {
            match (second_last, last) {
                (b'p', b'x') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::px(num));
                    }
                }
                (b'e', b'm') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::em(num));
                    }
                }
                (b'v', b'w') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::vw(num));
                    }
                }
                (b'v', b'h') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::vh(num));
                    }
                }
                (b'p', b't') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::pt(num));
                    }
                }
                (b'p', b'c') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::pc(num));
                    }
                }
                (b'c', b'm') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::cm(num));
                    }
                }
                (b'm', b'm') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::mm(num));
                    }
                }
                (b'i', b'n') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::inch(num));
                    }
                }
                (b'v', b'i') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::vi(num));
                    }
                }
                (b'v', b'b') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::vb(num));
                    }
                }
                (b'e', b'x') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::ex(num));
                    }
                }
                (b'c', b'h') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::ch(num));
                    }
                }
                (b'i', b'c') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::ic(num));
                    }
                }
                (b'l', b'h') => {
                    if let Ok(num) = value[..len-2].parse::<f32>() {
                        return Some(Length::lh(num));
                    }
                }
                _ => {}
            }
        }

        // Three character units (rem and viewport units)
        if len >= 4 && bytes[len-3] == b'r' && second_last == b'e' && last == b'm' {
            if let Ok(num) = value[..len-3].parse::<f32>() {
                return Some(Length::rem(num));
            }
        }

        // Check other 3-char units if needed
        if len >= 4 {
            let unit = &value[len-3..];
            match unit {
                "svw" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::svw(num)); }
                "svh" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::svh(num)); }
                "svi" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::svi(num)); }
                "svb" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::svb(num)); }
                "lvw" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::lvw(num)); }
                "lvh" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::lvh(num)); }
                "lvi" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::lvi(num)); }
                "lvb" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::lvb(num)); }
                "dvw" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::dvw(num)); }
                "dvh" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::dvh(num)); }
                "dvi" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::dvi(num)); }
                "dvb" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::dvb(num)); }
                "cqw" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::cqw(num)); }
                "cqh" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::cqh(num)); }
                "cqi" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::cqi(num)); }
                "cqb" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::cqb(num)); }
                "cap" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::cap(num)); }
                "rlh" => if let Ok(num) = value[..len-3].parse::<f32>() { return Some(Length::rlh(num)); }
                _ => {}
            }
        }

        // Four character units (less common)
        if len >= 5 {
            let last_4 = &value[len-4..];
            match last_4 {
                "vmin" => if let Ok(num) = value[..len-4].parse::<f32>() { return Some(Length::vmin(num)); }
                "vmax" => if let Ok(num) = value[..len-4].parse::<f32>() { return Some(Length::vmax(num)); }
                _ => {}
            }
        }

        // Five character units (least common)
        if len >= 6 {
            let last_5 = &value[len-5..];
            match last_5 {
                "svmin" => if let Ok(num) = value[..len-5].parse::<f32>() { return Some(Length::svmin(num)); }
                "svmax" => if let Ok(num) = value[..len-5].parse::<f32>() { return Some(Length::svmax(num)); }
                "lvmin" => if let Ok(num) = value[..len-5].parse::<f32>() { return Some(Length::lvmin(num)); }
                "lvmax" => if let Ok(num) = value[..len-5].parse::<f32>() { return Some(Length::lvmax(num)); }
                "dvmin" => if let Ok(num) = value[..len-5].parse::<f32>() { return Some(Length::dvmin(num)); }
                "dvmax" => if let Ok(num) = value[..len-5].parse::<f32>() { return Some(Length::dvmax(num)); }
                "cqmin" => if let Ok(num) = value[..len-5].parse::<f32>() { return Some(Length::cqmin(num)); }
                "cqmax" => if let Ok(num) = value[..len-5].parse::<f32>() { return Some(Length::cqmax(num)); }
                _ => {}
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
