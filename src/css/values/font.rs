use crate::css::{CssValue, Length};

/// CSS font-style property
#[derive(Debug, Clone, PartialEq)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

impl FontStyle {
    /// Parse font-style value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "normal" => FontStyle::Normal,
            "italic" => FontStyle::Italic,
            "oblique" => FontStyle::Oblique,
            _ => FontStyle::Normal, // Default to normal
        }
    }
}

impl Default for FontStyle {
    fn default() -> Self {
        FontStyle::Normal
    }
}

/// CSS font-variant property
#[derive(Debug, Clone, PartialEq)]
pub enum FontVariant {
    Normal,
    SmallCaps,
}

impl FontVariant {
    /// Parse font-variant value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "normal" => FontVariant::Normal,
            "small-caps" => FontVariant::SmallCaps,
            _ => FontVariant::Normal, // Default to normal
        }
    }
}

impl Default for FontVariant {
    fn default() -> Self {
        FontVariant::Normal
    }
}

/// CSS line-height property
#[derive(Debug, Clone, PartialEq)]
pub enum LineHeight {
    Normal,
    Length(Length),
    Number(f32), // Unitless multiplier
}

impl LineHeight {
    /// Parse line-height value from string
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        if value == "normal" {
            return LineHeight::Normal;
        }

        // Try to parse as a pure number (unitless multiplier)
        if let Ok(num) = value.parse::<f32>() {
            return LineHeight::Number(num);
        }

        // Try to parse as a length
        if let Some(length) = CssValue::parse_length(value) {
            return LineHeight::Length(length);
        }

        LineHeight::Normal
    }

    /// Convert to pixels given font size
    pub fn to_px(&self, font_size: f32) -> f32 {
        match self {
            LineHeight::Normal => font_size * 1.2, // Default line-height multiplier
            LineHeight::Length(length) => length.to_px(font_size, font_size),
            LineHeight::Number(multiplier) => font_size * multiplier,
        }
    }
}

impl Default for LineHeight {
    fn default() -> Self {
        LineHeight::Normal
    }
}