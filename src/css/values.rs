// CSS value types and parsing
use std::fmt;

/// CSS color representation
#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    Rgb { r: u8, g: u8, b: u8 },
    Rgba { r: u8, g: u8, b: u8, a: f32 },
    Named(String),
    Hex(String),
}

impl Color {
    /// Convert to Skia color
    pub fn to_skia_color(&self) -> skia_safe::Color {
        match self {
            Color::Rgb { r, g, b } => skia_safe::Color::from_rgb(*r, *g, *b),
            Color::Rgba { r, g, b, a } => skia_safe::Color::from_argb((*a * 255.0) as u8, *r, *g, *b),
            Color::Named(name) => match name.to_lowercase().as_str() {
                "black" => skia_safe::Color::BLACK,
                "white" => skia_safe::Color::WHITE,
                "red" => skia_safe::Color::RED,
                "green" => skia_safe::Color::GREEN,
                "blue" => skia_safe::Color::BLUE,
                "yellow" => skia_safe::Color::YELLOW,
                "cyan" => skia_safe::Color::CYAN,
                "magenta" => skia_safe::Color::MAGENTA,
                "gray" | "grey" => skia_safe::Color::GRAY,
                _ => skia_safe::Color::BLACK, // Default fallback
            },
            Color::Hex(hex) => {
                // Parse hex color (e.g., "#ff0000" or "#f00")
                let hex = hex.trim_start_matches('#');
                match hex.len() {
                    3 => {
                        // Short hex (#f00 -> #ff0000)
                        if let (Ok(r), Ok(g), Ok(b)) = (
                            u8::from_str_radix(&hex[0..1].repeat(2), 16),
                            u8::from_str_radix(&hex[1..2].repeat(2), 16),
                            u8::from_str_radix(&hex[2..3].repeat(2), 16),
                        ) {
                            skia_safe::Color::from_rgb(r, g, b)
                        } else {
                            skia_safe::Color::BLACK
                        }
                    },
                    6 => {
                        // Full hex (#ff0000)
                        if let (Ok(r), Ok(g), Ok(b)) = (
                            u8::from_str_radix(&hex[0..2], 16),
                            u8::from_str_radix(&hex[2..4], 16),
                            u8::from_str_radix(&hex[4..6], 16),
                        ) {
                            skia_safe::Color::from_rgb(r, g, b)
                        } else {
                            skia_safe::Color::BLACK
                        }
                    },
                    _ => skia_safe::Color::BLACK,
                }
            }
        }
    }
}

/// CSS length units
#[derive(Debug, Clone, PartialEq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Percent,
    Auto,
}

/// CSS length value
#[derive(Debug, Clone, PartialEq)]
pub struct Length {
    pub value: f32,
    pub unit: Unit,
}

impl Length {
    pub fn px(value: f32) -> Self {
        Self { value, unit: Unit::Px }
    }

    pub fn em(value: f32) -> Self {
        Self { value, unit: Unit::Em }
    }

    pub fn percent(value: f32) -> Self {
        Self { value, unit: Unit::Percent }
    }

    /// Convert to pixels given a context
    pub fn to_px(&self, font_size: f32, parent_size: f32) -> f32 {
        match self.unit {
            Unit::Px => self.value,
            Unit::Em => self.value * font_size,
            Unit::Rem => self.value * 16.0, // Default root font size
            Unit::Percent => self.value / 100.0 * parent_size,
            Unit::Auto => 0.0, // Auto should be handled by layout algorithm
        }
    }
}

/// CSS property values
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    Length(Length),
    Color(Color),
    Number(f32),
    String(String),
    Keyword(String),
    Auto,
}

impl CssValue {
    /// Parse a CSS value from a string
    pub fn parse(value: &str) -> Self {
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
            let unquoted = &value[1..value.len()-1];
            return CssValue::String(unquoted.to_string());
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

    fn parse_length(value: &str) -> Option<Length> {
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
        }
    }
}
