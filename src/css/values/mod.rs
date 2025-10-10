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
        // Parse based on suffix - check longer suffixes first to avoid prefix matching issues

        // 5-character units
        if value.ends_with("svmin") {
            if let Ok(num) = value[..value.len()-5].parse::<f32>() {
                return Some(Length::svmin(num));
            }
        } else if value.ends_with("svmax") {
            if let Ok(num) = value[..value.len()-5].parse::<f32>() {
                return Some(Length::svmax(num));
            }
        } else if value.ends_with("lvmin") {
            if let Ok(num) = value[..value.len()-5].parse::<f32>() {
                return Some(Length::lvmin(num));
            }
        } else if value.ends_with("lvmax") {
            if let Ok(num) = value[..value.len()-5].parse::<f32>() {
                return Some(Length::lvmax(num));
            }
        } else if value.ends_with("dvmin") {
            if let Ok(num) = value[..value.len()-5].parse::<f32>() {
                return Some(Length::dvmin(num));
            }
        } else if value.ends_with("dvmax") {
            if let Ok(num) = value[..value.len()-5].parse::<f32>() {
                return Some(Length::dvmax(num));
            }
        } else if value.ends_with("cqmin") {
            if let Ok(num) = value[..value.len()-5].parse::<f32>() {
                return Some(Length::cqmin(num));
            }
        } else if value.ends_with("cqmax") {
            if let Ok(num) = value[..value.len()-5].parse::<f32>() {
                return Some(Length::cqmax(num));
            }
        }

        // 4-character units
        else if value.ends_with("vmin") {
            if let Ok(num) = value[..value.len()-4].parse::<f32>() {
                return Some(Length::vmin(num));
            }
        } else if value.ends_with("vmax") {
            if let Ok(num) = value[..value.len()-4].parse::<f32>() {
                return Some(Length::vmax(num));
            }
        }

        // 3-character units
        else if value.ends_with("rem") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::rem(num));
            }
        } else if value.ends_with("svw") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::svw(num));
            }
        } else if value.ends_with("svh") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::svh(num));
            }
        } else if value.ends_with("svi") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::svi(num));
            }
        } else if value.ends_with("svb") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::svb(num));
            }
        } else if value.ends_with("lvw") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::lvw(num));
            }
        } else if value.ends_with("lvh") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::lvh(num));
            }
        } else if value.ends_with("lvi") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::lvi(num));
            }
        } else if value.ends_with("lvb") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::lvb(num));
            }
        } else if value.ends_with("dvw") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::dvw(num));
            }
        } else if value.ends_with("dvh") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::dvh(num));
            }
        } else if value.ends_with("dvi") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::dvi(num));
            }
        } else if value.ends_with("dvb") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::dvb(num));
            }
        } else if value.ends_with("cqw") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::cqw(num));
            }
        } else if value.ends_with("cqh") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::cqh(num));
            }
        } else if value.ends_with("cqi") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::cqi(num));
            }
        } else if value.ends_with("cqb") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::cqb(num));
            }
        } else if value.ends_with("cap") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::cap(num));
            }
        } else if value.ends_with("rlh") {
            if let Ok(num) = value[..value.len()-3].parse::<f32>() {
                return Some(Length::rlh(num));
            }
        }

        // 2-character units
        else if value.ends_with("px") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::px(num));
            }
        } else if value.ends_with("em") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::em(num));
            }
        } else if value.ends_with("pt") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::pt(num));
            }
        } else if value.ends_with("pc") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::pc(num));
            }
        } else if value.ends_with("cm") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::cm(num));
            }
        } else if value.ends_with("mm") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::mm(num));
            }
        } else if value.ends_with("in") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::inch(num));
            }
        } else if value.ends_with("vw") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::vw(num));
            }
        } else if value.ends_with("vh") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::vh(num));
            }
        } else if value.ends_with("vi") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::vi(num));
            }
        } else if value.ends_with("vb") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::vb(num));
            }
        } else if value.ends_with("ex") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::ex(num));
            }
        } else if value.ends_with("ch") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::ch(num));
            }
        } else if value.ends_with("ic") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::ic(num));
            }
        } else if value.ends_with("lh") {
            if let Ok(num) = value[..value.len()-2].parse::<f32>() {
                return Some(Length::lh(num));
            }
        }

        // 1-character units
        else if value.ends_with('%') {
            if let Ok(num) = value[..value.len()-1].parse::<f32>() {
                return Some(Length::percent(num));
            }
        } else if value.ends_with('q') || value.ends_with('Q') {
            if let Ok(num) = value[..value.len()-1].parse::<f32>() {
                return Some(Length::q(num));
            }
        }

        println!("Warning: Unknown length unit in '{}'", value);
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
