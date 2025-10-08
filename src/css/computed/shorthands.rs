// Shorthand property parsers
use super::values::ComputedValues;
use crate::css::CssValue;

/// Parse background shorthand property
/// Handles: background: [color] [image] [position] [size] [repeat] [origin] [clip] [attachment]
/// Examples:
///   background: red;
///   background: url(image.jpg);
///   background: #ff0000 url(image.jpg);
///   background: linear-gradient(...);
pub fn parse_background_shorthand(computed: &mut ComputedValues, value: &CssValue) {
    // Reset background properties to defaults when using shorthand
    computed.background_color = None;
    computed.background_image = crate::css::BackgroundImage::None;

    match value {
        CssValue::Keyword(keyword) => {
            let keyword_lower = keyword.to_lowercase();

            // Check if it's a named color or "none"
            if keyword_lower == "none" {
                // Everything stays at defaults (already set above)
                return;
            }

            // Try to parse as color
            let color_value = CssValue::parse(keyword);
            if let CssValue::Color(color) = color_value {
                computed.background_color = Some(color);
            } else if keyword.starts_with("url(") {
                // It's a background image
                computed.background_image = crate::css::BackgroundImage::parse(keyword);
            }
        }
        CssValue::String(s) => {
            // Could be a color, image URL, or complex value
            let s_lower = s.to_lowercase();

            if s_lower == "none" {
                return;
            }

            // Try to parse as color first
            let color_value = CssValue::parse(s);
            if let CssValue::Color(color) = color_value {
                computed.background_color = Some(color);
            } else if s.starts_with("url(") {
                computed.background_image = crate::css::BackgroundImage::parse(s);
            } else {
                // Try parsing the entire string for multiple values
                parse_complex_background(computed, s);
            }
        }
        CssValue::Color(color) => {
            computed.background_color = Some(color.clone());
        }
        CssValue::MultipleValues(values) => {
            // Parse multiple space-separated values
            // Could be: "red url(image.jpg)" or similar combinations
            for val in values {
                match val {
                    CssValue::Color(color) => {
                        computed.background_color = Some(color.clone());
                    }
                    CssValue::Keyword(keyword) => {
                        if keyword.starts_with("url(") {
                            computed.background_image = crate::css::BackgroundImage::parse(keyword);
                        } else {
                            // Try to parse as color
                            let color_value = CssValue::parse(keyword);
                            if let CssValue::Color(color) = color_value {
                                computed.background_color = Some(color);
                            }
                        }
                    }
                    CssValue::String(s) => {
                        if s.starts_with("url(") {
                            computed.background_image = crate::css::BackgroundImage::parse(s);
                        } else {
                            // Try to parse as color
                            let color_value = CssValue::parse(s);
                            if let CssValue::Color(color) = color_value {
                                computed.background_color = Some(color);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Parse complex background string with multiple values
fn parse_complex_background(computed: &mut ComputedValues, value: &str) {
    // Split by spaces but respect url() parentheses
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0;

    for ch in value.chars() {
        match ch {
            '(' => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' => {
                paren_depth -= 1;
                current.push(ch);
            }
            ' ' if paren_depth == 0 => {
                if !current.is_empty() {
                    parts.push(current.trim().to_string());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        parts.push(current.trim().to_string());
    }

    // Parse each part
    for part in parts {
        if part.starts_with("url(") {
            computed.background_image = crate::css::BackgroundImage::parse(&part);
        } else {
            // Try to parse as color
            let color_value = CssValue::parse(&part);
            if let CssValue::Color(color) = color_value {
                computed.background_color = Some(color);
            }
        }
    }
}

/// Parse font shorthand property
/// Syntax: font: [font-style] [font-variant] [font-weight] font-size[/line-height] font-family
/// Required: font-size and font-family
/// Examples:
///   font: 12px Arial;
///   font: italic bold 16px/1.5 "Times New Roman", serif;
///   font: small-caps 14px Georgia;
pub fn parse_font_shorthand(computed: &mut ComputedValues, value: &CssValue, parent_values: Option<&ComputedValues>) {
    // Reset font properties to defaults when using shorthand
    computed.font_style = crate::css::FontStyle::Normal;
    computed.font_variant = crate::css::FontVariant::Normal;
    computed.font_weight = "normal".to_string();
    computed.line_height = crate::css::LineHeight::Normal;

    // Extract the string value to parse
    let font_str = match value {
        CssValue::String(s) => s.clone(),
        CssValue::Keyword(k) => k.clone(),
        CssValue::MultipleValues(values) => {
            // Join multiple values into a single string
            values.iter()
                .map(|v| match v {
                    CssValue::String(s) => s.clone(),
                    CssValue::Keyword(k) => k.clone(),
                    CssValue::Length(len) => format!("{}{:?}", len.value, len.unit),
                    CssValue::Number(n) => n.to_string(),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => return, // Invalid value type for font shorthand
    };

    // Split by spaces but respect quoted strings
    let parts = split_font_value(&font_str);

    if parts.is_empty() {
        return;
    }

    // Parse font properties
    let mut i = 0;
    let mut font_size_found = false;
    let mut font_family_parts = Vec::new();

    while i < parts.len() {
        let part = &parts[i];
        let part_lower = part.to_lowercase();

        // Check for font-style (must come before font-size)
        if !font_size_found && matches!(part_lower.as_str(), "normal" | "italic" | "oblique") {
            computed.font_style = crate::css::FontStyle::parse(part);
            i += 1;
            continue;
        }

        // Check for font-variant (must come before font-size)
        if !font_size_found && part_lower == "small-caps" {
            computed.font_variant = crate::css::FontVariant::parse(part);
            i += 1;
            continue;
        }

        // Check for font-weight (must come before font-size)
        if !font_size_found && matches!(part_lower.as_str(),
            "normal" | "bold" | "bolder" | "lighter" |
            "100" | "200" | "300" | "400" | "500" | "600" | "700" | "800" | "900") {
            computed.font_weight = part.clone();
            i += 1;
            continue;
        }

        // Check for font-size (with optional line-height)
        if !font_size_found {
            // Check if this contains line-height (e.g., "16px/1.5")
            if part.contains('/') {
                let size_height: Vec<&str> = part.split('/').collect();
                if size_height.len() == 2 {
                    // Parse font-size
                    let size_value = CssValue::parse(size_height[0]);
                    if let CssValue::Length(length) = size_value {
                        let parent_font_size = parent_values.map(|p| p.font_size).unwrap_or(16.0);
                        computed.font_size = length.to_px(parent_font_size, parent_font_size);
                        font_size_found = true;
                    } else if let CssValue::Number(num) = size_value {
                        computed.font_size = num;
                        font_size_found = true;
                    }

                    // Parse line-height
                    computed.line_height = crate::css::LineHeight::parse(size_height[1]);
                    i += 1;
                    continue;
                }
            }

            // Try to parse as font-size without line-height
            let size_value = CssValue::parse(part);
            if let CssValue::Length(length) = size_value {
                let parent_font_size = parent_values.map(|p| p.font_size).unwrap_or(16.0);
                computed.font_size = length.to_px(parent_font_size, parent_font_size);
                font_size_found = true;
                i += 1;
                continue;
            } else if let CssValue::Number(num) = size_value {
                computed.font_size = num;
                font_size_found = true;
                i += 1;
                continue;
            }
        }

        // Everything after font-size is font-family
        if font_size_found {
            font_family_parts.push(part.clone());
        }

        i += 1;
    }

    // Join font-family parts (removing quotes if present)
    if !font_family_parts.is_empty() {
        let family = font_family_parts.join(" ");
        // Remove surrounding quotes if present
        computed.font_family = if (family.starts_with('"') && family.ends_with('"')) ||
                                   (family.starts_with('\'') && family.ends_with('\'')) {
            family[1..family.len()-1].to_string()
        } else {
            family
        };
    }
}

/// Split font value respecting quoted strings
fn split_font_value(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = ' ';

    for ch in value.chars() {
        match ch {
            '"' | '\'' => {
                if !in_quotes {
                    in_quotes = true;
                    quote_char = ch;
                    current.push(ch);
                } else if ch == quote_char {
                    in_quotes = false;
                    current.push(ch);
                } else {
                    current.push(ch);
                }
            }
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    parts.push(current.trim().to_string());
                    current.clear();
                }
            }
            ',' if !in_quotes => {
                // Comma separates font families, but we'll treat the whole thing as one family for simplicity
                if !current.is_empty() {
                    parts.push(current.trim().to_string());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

