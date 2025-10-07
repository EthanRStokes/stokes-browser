// CSS text-related values

/// CSS text decoration types
#[derive(Debug, Clone, PartialEq)]
pub enum TextDecoration {
    None,
    Underline,
    Overline,
    LineThrough,
    Multiple(Vec<TextDecorationType>),
}

/// Individual text decoration types for multiple decorations
#[derive(Debug, Clone, PartialEq)]
pub enum TextDecorationType {
    Underline,
    Overline,
    LineThrough,
}

impl TextDecoration {
    /// Parse text-decoration value from string
    pub fn parse(value: &str) -> Self {
        let value = value.trim().to_lowercase();

        match value.as_str() {
            "none" => TextDecoration::None,
            "underline" => TextDecoration::Underline,
            "overline" => TextDecoration::Overline,
            "line-through" => TextDecoration::LineThrough,
            _ => {
                // Handle multiple values (e.g., "underline overline")
                let parts: Vec<&str> = value.split_whitespace().collect();
                if parts.len() > 1 {
                    let mut decorations = Vec::new();
                    for part in parts {
                        match part {
                            "underline" => decorations.push(TextDecorationType::Underline),
                            "overline" => decorations.push(TextDecorationType::Overline),
                            "line-through" => decorations.push(TextDecorationType::LineThrough),
                            _ => {} // Ignore unknown values
                        }
                    }
                    if !decorations.is_empty() {
                        TextDecoration::Multiple(decorations)
                    } else {
                        TextDecoration::None
                    }
                } else {
                    TextDecoration::None
                }
            }
        }
    }

    /// Check if decoration has underline
    pub fn has_underline(&self) -> bool {
        match self {
            TextDecoration::Underline => true,
            TextDecoration::Multiple(decorations) => {
                decorations.contains(&TextDecorationType::Underline)
            }
            _ => false,
        }
    }

    /// Check if decoration has overline
    pub fn has_overline(&self) -> bool {
        match self {
            TextDecoration::Overline => true,
            TextDecoration::Multiple(decorations) => {
                decorations.contains(&TextDecorationType::Overline)
            }
            _ => false,
        }
    }

    /// Check if decoration has line-through
    pub fn has_line_through(&self) -> bool {
        match self {
            TextDecoration::LineThrough => true,
            TextDecoration::Multiple(decorations) => {
                decorations.contains(&TextDecorationType::LineThrough)
            }
            _ => false,
        }
    }
}

impl Default for TextDecoration {
    fn default() -> Self {
        TextDecoration::None
    }
}

/// CSS text-align property
#[derive(Debug, Clone, PartialEq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Justify,
}

impl TextAlign {
    /// Parse text-align value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "left" => TextAlign::Left,
            "right" => TextAlign::Right,
            "center" => TextAlign::Center,
            "justify" => TextAlign::Justify,
            _ => TextAlign::Left, // Default to left
        }
    }
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign::Left
    }
}

/// CSS white-space property - controls text wrapping and whitespace handling
#[derive(Debug, Clone, PartialEq)]
pub enum WhiteSpace {
    Normal,    // Collapse whitespace, wrap text
    Nowrap,    // Collapse whitespace, don't wrap text
    Pre,       // Preserve whitespace, don't wrap text
    PreWrap,   // Preserve whitespace, wrap text
    PreLine,   // Collapse whitespace except newlines, wrap text
}

impl WhiteSpace {
    /// Parse white-space value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "normal" => WhiteSpace::Normal,
            "nowrap" => WhiteSpace::Nowrap,
            "pre" => WhiteSpace::Pre,
            "pre-wrap" => WhiteSpace::PreWrap,
            "pre-line" => WhiteSpace::PreLine,
            _ => WhiteSpace::Normal, // Default to normal
        }
    }

    /// Returns true if text should wrap
    pub fn should_wrap(&self) -> bool {
        matches!(self, WhiteSpace::Normal | WhiteSpace::PreWrap | WhiteSpace::PreLine)
    }

    /// Returns true if whitespace should be preserved
    pub fn preserve_whitespace(&self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine)
    }
}

impl Default for WhiteSpace {
    fn default() -> Self {
        WhiteSpace::Normal
    }
}

/// CSS text-transform property
#[derive(Debug, Clone, PartialEq)]
pub enum TextTransform {
    None,
    Capitalize,
    Uppercase,
    Lowercase,
    FullWidth,
}

impl TextTransform {
    /// Parse text-transform value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "none" => TextTransform::None,
            "capitalize" => TextTransform::Capitalize,
            "uppercase" => TextTransform::Uppercase,
            "lowercase" => TextTransform::Lowercase,
            "full-width" => TextTransform::FullWidth,
            _ => TextTransform::None, // Default to none
        }
    }

    /// Apply the text transformation to a string
    pub fn apply(&self, text: &str) -> String {
        match self {
            TextTransform::None => text.to_string(),
            TextTransform::Uppercase => text.to_uppercase(),
            TextTransform::Lowercase => text.to_lowercase(),
            TextTransform::Capitalize => {
                // Capitalize the first letter of each word
                text.split_whitespace()
                    .map(|word| {
                        let mut chars = word.chars();
                        match chars.next() {
                            None => String::new(),
                            Some(first) => {
                                first.to_uppercase().collect::<String>() + chars.as_str()
                            }
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            TextTransform::FullWidth => {
                // Convert characters to their full-width variants
                // For now, this is a simplified implementation
                text.chars()
                    .map(|c| match c {
                        'A'..='Z' | 'a'..='z' | '0'..='9' => {
                            // Convert to full-width character
                            char::from_u32(0xFF00 + c as u32 - 0x20).unwrap_or(c)
                        }
                        _ => c,
                    })
                    .collect()
            }
        }
    }
}

impl Default for TextTransform {
    fn default() -> Self {
        TextTransform::None
    }
}

