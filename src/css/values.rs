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
            Unit::Percent => self.value * parent_size / 100.0,
            Unit::Auto => 0.0, // Will be calculated based on context
        }
    }
}

/// CSS property values
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    Color(Color),
    Length(Length),
    String(String),
    Number(f32),
    Keyword(String),
    Auto,
}

impl CssValue {
    /// Parse a CSS value from a string
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();

        // Check for colors
        if trimmed.starts_with('#') {
            return CssValue::Color(Color::Hex(trimmed.to_string()));
        }

        if trimmed.starts_with("rgb(") && trimmed.ends_with(')') {
            return Self::parse_rgb(trimmed);
        }

        if trimmed.starts_with("rgba(") && trimmed.ends_with(')') {
            return Self::parse_rgba(trimmed);
        }

        // Check for named colors
        match trimmed.to_lowercase().as_str() {
            "black" | "white" | "red" | "green" | "blue" | "yellow" |
            "cyan" | "magenta" | "gray" | "grey" => {
                return CssValue::Color(Color::Named(trimmed.to_lowercase()));
            }
            _ => {}
        }

        // Check for lengths
        if let Some(length) = Self::parse_length(trimmed) {
            return CssValue::Length(length);
        }

        // Check for numbers
        if let Ok(num) = trimmed.parse::<f32>() {
            return CssValue::Number(num);
        }

        // Check for keywords
        match trimmed.to_lowercase().as_str() {
            "auto" => CssValue::Auto,
            _ => CssValue::Keyword(trimmed.to_string()),
        }
    }

    fn parse_rgb(input: &str) -> CssValue {
        let inner = &input[4..input.len()-1]; // Remove "rgb(" and ")"
        let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();

        if parts.len() == 3 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                parts[0].parse::<u8>(),
                parts[1].parse::<u8>(),
                parts[2].parse::<u8>(),
            ) {
                return CssValue::Color(Color::Rgb { r, g, b });
            }
        }

        CssValue::Color(Color::Named("black".to_string()))
    }

    fn parse_rgba(input: &str) -> CssValue {
        let inner = &input[5..input.len()-1]; // Remove "rgba(" and ")"
        let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();

        if parts.len() == 4 {
            if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
                parts[0].parse::<u8>(),
                parts[1].parse::<u8>(),
                parts[2].parse::<u8>(),
                parts[3].parse::<f32>(),
            ) {
                return CssValue::Color(Color::Rgba { r, g, b, a });
            }
        }

        CssValue::Color(Color::Named("black".to_string()))
    }

    fn parse_length(input: &str) -> Option<Length> {
        if input.ends_with("px") {
            if let Ok(value) = input[..input.len()-2].parse::<f32>() {
                return Some(Length::px(value));
            }
        } else if input.ends_with("em") {
            if let Ok(value) = input[..input.len()-2].parse::<f32>() {
                return Some(Length::em(value));
            }
        } else if input.ends_with('%') {
            if let Ok(value) = input[..input.len()-1].parse::<f32>() {
                return Some(Length::percent(value));
            }
        } else if input == "auto" {
            return Some(Length { value: 0.0, unit: Unit::Auto });
        }

        None
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
