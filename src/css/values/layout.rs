// CSS layout-related values

use super::length::Length;
use crate::css::values::CssValue;

/// CSS clear property
#[derive(Debug, Clone, PartialEq)]
pub enum Clear {
    None,
    Left,
    Right,
    Both,
}

impl Clear {
    /// Parse clear value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "none" => Clear::None,
            "left" => Clear::Left,
            "right" => Clear::Right,
            "both" => Clear::Both,
            _ => Clear::None, // Default to none
        }
    }
}

impl Default for Clear {
    fn default() -> Self {
        Clear::None
    }
}

/// CSS float property
#[derive(Debug, Clone, PartialEq)]
pub enum Float {
    None,
    Left,
    Right,
}

impl Float {
    /// Parse float value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "none" => Float::None,
            "left" => Float::Left,
            "right" => Float::Right,
            _ => Float::None, // Default to none
        }
    }
}

impl Default for Float {
    fn default() -> Self {
        Float::None
    }
}

/// CSS overflow property
#[derive(Debug, Clone, PartialEq)]
pub enum Overflow {
    Visible,
    Hidden,
    Scroll,
    Auto,
}

impl Overflow {
    /// Parse overflow value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "visible" => Overflow::Visible,
            "hidden" => Overflow::Hidden,
            "scroll" => Overflow::Scroll,
            "auto" => Overflow::Auto,
            _ => Overflow::Visible, // Default to visible
        }
    }
}

impl Default for Overflow {
    fn default() -> Self {
        Overflow::Visible
    }
}

/// CSS box-sizing property
#[derive(Debug, Clone, PartialEq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

impl BoxSizing {
    /// Parse box-sizing value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "border-box" => BoxSizing::BorderBox,
            "content-box" => BoxSizing::ContentBox,
            _ => BoxSizing::ContentBox, // Default to content-box
        }
    }
}

impl Default for BoxSizing {
    fn default() -> Self {
        BoxSizing::ContentBox
    }
}

/// CSS visibility property
#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Visible,
    Hidden,
    Collapse,
}

impl Visibility {
    /// Parse visibility value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "visible" => Visibility::Visible,
            "hidden" => Visibility::Hidden,
            "collapse" => Visibility::Collapse,
            _ => Visibility::Visible, // Default to visible
        }
    }
}

impl Default for Visibility {
    fn default() -> Self {
        Visibility::Visible
    }
}

/// CSS vertical-align property
#[derive(Debug, Clone, PartialEq)]
pub enum VerticalAlign {
    Baseline,
    Sub,
    Super,
    Top,
    TextTop,
    Middle,
    Bottom,
    TextBottom,
    Length(Length),
    Percent(f32),
}

impl VerticalAlign {
    /// Parse vertical-align value from string
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        match value.to_lowercase().as_str() {
            "baseline" => VerticalAlign::Baseline,
            "sub" => VerticalAlign::Sub,
            "super" => VerticalAlign::Super,
            "top" => VerticalAlign::Top,
            "text-top" => VerticalAlign::TextTop,
            "middle" => VerticalAlign::Middle,
            "bottom" => VerticalAlign::Bottom,
            "text-bottom" => VerticalAlign::TextBottom,
            _ => {
                // Try to parse as a length or percentage
                if value.ends_with('%') {
                    if let Ok(num) = value.trim_end_matches('%').parse::<f32>() {
                        return VerticalAlign::Percent(num);
                    }
                }

                // Try to parse as a length
                if let Some(length) = CssValue::parse_length(value) {
                    return VerticalAlign::Length(length);
                }

                VerticalAlign::Baseline // Default to baseline
            }
        }
    }

    /// Convert to pixels given font size and line height
    pub fn to_px(&self, font_size: f32, line_height: f32) -> f32 {
        match self {
            VerticalAlign::Baseline => 0.0,
            VerticalAlign::Sub => -font_size * 0.2, // Lower by 20% of font size
            VerticalAlign::Super => font_size * 0.4, // Raise by 40% of font size
            VerticalAlign::Top => line_height * 0.5, // Align with top of line box
            VerticalAlign::TextTop => font_size * 0.8, // Align with top of font
            VerticalAlign::Middle => font_size * 0.3, // Align middle of element with baseline + x-height/2
            VerticalAlign::Bottom => -line_height * 0.5, // Align with bottom of line box
            VerticalAlign::TextBottom => -font_size * 0.2, // Align with bottom of font
            VerticalAlign::Length(length) => length.to_px(font_size, font_size),
            VerticalAlign::Percent(percent) => (percent / 100.0) * line_height,
        }
    }
}

impl Default for VerticalAlign {
    fn default() -> Self {
        VerticalAlign::Baseline
    }
}

/// CSS content property value
#[derive(Debug, Clone, PartialEq)]
pub enum ContentValue {
    None,
    Normal,
    String(String),
    Attr(String), // attr(attribute-name)
    Counter(String), // counter(name)
    OpenQuote,
    CloseQuote,
    NoOpenQuote,
    NoCloseQuote,
    Multiple(Vec<ContentValue>), // Multiple values concatenated
}

impl ContentValue {
    /// Parse content value from string
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        match value.to_lowercase().as_str() {
            "none" => ContentValue::None,
            "normal" => ContentValue::Normal,
            "open-quote" => ContentValue::OpenQuote,
            "close-quote" => ContentValue::CloseQuote,
            "no-open-quote" => ContentValue::NoOpenQuote,
            "no-close-quote" => ContentValue::NoCloseQuote,
            _ => {
                // Check for quoted string
                if (value.starts_with('"') && value.ends_with('"')) ||
                   (value.starts_with('\'') && value.ends_with('\'')) {
                    let unquoted = &value[1..value.len() - 1];
                    return ContentValue::String(unquoted.to_string());
                }

                // Check for attr() function
                if value.starts_with("attr(") && value.ends_with(')') {
                    let attr_name = &value[5..value.len() - 1].trim();
                    return ContentValue::Attr(attr_name.to_string());
                }

                // Check for counter() function
                if value.starts_with("counter(") && value.ends_with(')') {
                    let counter_name = &value[8..value.len() - 1].trim();
                    return ContentValue::Counter(counter_name.to_string());
                }

                // Try to parse as multiple values
                if value.contains(' ') {
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    if parts.len() > 1 {
                        let mut values = Vec::new();
                        for part in parts {
                            values.push(Self::parse(part));
                        }
                        return ContentValue::Multiple(values);
                    }
                }

                // Default to treating as a string (without quotes)
                ContentValue::String(value.to_string())
            }
        }
    }

    /// Convert content value to display string
    pub fn to_display_string(&self, element_attributes: Option<&std::collections::HashMap<String, String>>) -> String {
        match self {
            ContentValue::None | ContentValue::Normal => String::new(),
            ContentValue::String(s) => s.clone(),
            ContentValue::Attr(attr_name) => {
                if let Some(attrs) = element_attributes {
                    attrs.get(attr_name).cloned().unwrap_or_default()
                } else {
                    String::new()
                }
            }
            ContentValue::Counter(_) => {
                // Counter implementation would require maintaining counter state
                // For now, return empty string
                String::new()
            }
            ContentValue::OpenQuote => "\"".to_string(),
            ContentValue::CloseQuote => "\"".to_string(),
            ContentValue::NoOpenQuote | ContentValue::NoCloseQuote => String::new(),
            ContentValue::Multiple(values) => {
                values.iter()
                    .map(|v| v.to_display_string(element_attributes))
                    .collect::<Vec<_>>()
                    .join("")
            }
        }
    }
}

impl Default for ContentValue {
    fn default() -> Self {
        ContentValue::Normal
    }
}

