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
    Pt, // Points (1pt = 1/72 inch, typically 1pt ≈ 1.33px at 96 DPI)
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

    pub fn pt(value: f32) -> Self {
        Self { value, unit: Unit::Pt }
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
            Unit::Pt => self.value * 4.0 / 3.0, // Convert points to pixels (1pt = 4/3 px at standard 96 DPI)
            Unit::Em => self.value * font_size,
            Unit::Rem => self.value * 16.0, // Default root font size
            Unit::Percent => self.value / 100.0 * parent_size,
            Unit::Auto => 0.0, // Auto should be handled by layout algorithm
        }
    }
}

impl Default for Length {
    fn default() -> Self {
        Length::px(0.0)
    }
}

/// CSS border radius values
#[derive(Debug, Clone, PartialEq)]
pub struct BorderRadius {
    pub top_left: Length,
    pub top_right: Length,
    pub bottom_right: Length,
    pub bottom_left: Length,
}

impl BorderRadius {
    /// Create uniform border radius for all corners
    pub fn uniform(radius: Length) -> Self {
        Self {
            top_left: radius.clone(),
            top_right: radius.clone(),
            bottom_right: radius.clone(),
            bottom_left: radius,
        }
    }

    /// Create border radius with individual corner values
    pub fn new(top_left: Length, top_right: Length, bottom_right: Length, bottom_left: Length) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    /// Convert all corner radii to pixels
    pub fn to_px(&self, font_size: f32, parent_size: f32) -> BorderRadiusPx {
        BorderRadiusPx {
            top_left: self.top_left.to_px(font_size, parent_size),
            top_right: self.top_right.to_px(font_size, parent_size),
            bottom_right: self.bottom_right.to_px(font_size, parent_size),
            bottom_left: self.bottom_left.to_px(font_size, parent_size),
        }
    }
}

impl Default for BorderRadius {
    fn default() -> Self {
        Self::uniform(Length::px(0.0))
    }
}

/// Border radius in pixels for rendering
#[derive(Debug, Clone, PartialEq)]
pub struct BorderRadiusPx {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl BorderRadiusPx {
    pub fn uniform(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    pub fn has_radius(&self) -> bool {
        self.top_left > 0.0 || self.top_right > 0.0 ||
        self.bottom_right > 0.0 || self.bottom_left > 0.0
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

/// CSS background image
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundImage {
    None,
    Url(String),
}

impl BackgroundImage {
    /// Parse background-image from CSS string
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        // Check for "none"
        if value.to_lowercase() == "none" {
            return BackgroundImage::None;
        }

        // Check for url() format
        if value.starts_with("url(") && value.ends_with(')') {
            let url_content = &value[4..value.len()-1].trim();
            // Remove quotes if present
            let url = if (url_content.starts_with('"') && url_content.ends_with('"')) ||
                         (url_content.starts_with('\'') && url_content.ends_with('\'')) {
                url_content[1..url_content.len()-1].to_string()
            } else {
                url_content.to_string()
            };
            return BackgroundImage::Url(url);
        }

        // Default to None if parsing fails
        BackgroundImage::None
    }
}

impl Default for BackgroundImage {
    fn default() -> Self {
        BackgroundImage::None
    }
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

/// CSS cursor property
#[derive(Debug, Clone, PartialEq)]
pub enum Cursor {
    Auto,
    Default,
    Pointer,
    Text,
    Move,
    Wait,
    Help,
    NotAllowed,
    Crosshair,
    Grab,
    Grabbing,
    EResize,
    WResize,
    NResize,
    SResize,
    NEResize,
    NWResize,
    SEResize,
    SWResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
    Copy,
    Alias,
    ContextMenu,
    NoDrop,
    Progress,
    Cell,
    VerticalText,
}

impl Cursor {
    /// Parse cursor value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "auto" => Cursor::Auto,
            "default" => Cursor::Default,
            "pointer" => Cursor::Pointer,
            "text" => Cursor::Text,
            "move" => Cursor::Move,
            "wait" => Cursor::Wait,
            "help" => Cursor::Help,
            "not-allowed" => Cursor::NotAllowed,
            "crosshair" => Cursor::Crosshair,
            "grab" => Cursor::Grab,
            "grabbing" => Cursor::Grabbing,
            "e-resize" => Cursor::EResize,
            "w-resize" => Cursor::WResize,
            "n-resize" => Cursor::NResize,
            "s-resize" => Cursor::SResize,
            "ne-resize" => Cursor::NEResize,
            "nw-resize" => Cursor::NWResize,
            "se-resize" => Cursor::SEResize,
            "sw-resize" => Cursor::SWResize,
            "col-resize" => Cursor::ColResize,
            "row-resize" => Cursor::RowResize,
            "all-scroll" => Cursor::AllScroll,
            "zoom-in" => Cursor::ZoomIn,
            "zoom-out" => Cursor::ZoomOut,
            "copy" => Cursor::Copy,
            "alias" => Cursor::Alias,
            "context-menu" => Cursor::ContextMenu,
            "no-drop" => Cursor::NoDrop,
            "progress" => Cursor::Progress,
            "cell" => Cursor::Cell,
            "vertical-text" => Cursor::VerticalText,
            _ => Cursor::Auto, // Default to auto
        }
    }

    /// Convert to winit CursorIcon
    pub fn to_winit_cursor(&self) -> winit::window::CursorIcon {
        match self {
            Cursor::Auto => winit::window::CursorIcon::Default,
            Cursor::Default => winit::window::CursorIcon::Default,
            Cursor::Pointer => winit::window::CursorIcon::Pointer,
            Cursor::Text => winit::window::CursorIcon::Text,
            Cursor::Move => winit::window::CursorIcon::Move,
            Cursor::Wait => winit::window::CursorIcon::Wait,
            Cursor::Help => winit::window::CursorIcon::Help,
            Cursor::NotAllowed => winit::window::CursorIcon::NotAllowed,
            Cursor::Crosshair => winit::window::CursorIcon::Crosshair,
            Cursor::Grab => winit::window::CursorIcon::Grab,
            Cursor::Grabbing => winit::window::CursorIcon::Grabbing,
            Cursor::EResize => winit::window::CursorIcon::EResize,
            Cursor::WResize => winit::window::CursorIcon::WResize,
            Cursor::NResize => winit::window::CursorIcon::NResize,
            Cursor::SResize => winit::window::CursorIcon::SResize,
            Cursor::NEResize => winit::window::CursorIcon::NeResize,
            Cursor::NWResize => winit::window::CursorIcon::NwResize,
            Cursor::SEResize => winit::window::CursorIcon::SeResize,
            Cursor::SWResize => winit::window::CursorIcon::SwResize,
            Cursor::ColResize => winit::window::CursorIcon::ColResize,
            Cursor::RowResize => winit::window::CursorIcon::RowResize,
            Cursor::AllScroll => winit::window::CursorIcon::AllScroll,
            Cursor::ZoomIn => winit::window::CursorIcon::ZoomIn,
            Cursor::ZoomOut => winit::window::CursorIcon::ZoomOut,
            Cursor::Copy => winit::window::CursorIcon::Copy,
            Cursor::Alias => winit::window::CursorIcon::Alias,
            Cursor::ContextMenu => winit::window::CursorIcon::ContextMenu,
            Cursor::NoDrop => winit::window::CursorIcon::NoDrop,
            Cursor::Progress => winit::window::CursorIcon::Progress,
            Cursor::Cell => winit::window::CursorIcon::Cell,
            Cursor::VerticalText => winit::window::CursorIcon::VerticalText,
        }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor::Auto
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

/// CSS list-style-type property
#[derive(Debug, Clone, PartialEq)]
pub enum ListStyleType {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerRoman,
    UpperRoman,
    LowerAlpha,
    UpperAlpha,
    LowerGreek,
    LowerLatin,
    UpperLatin,
}

impl ListStyleType {
    /// Parse list-style-type value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "none" => ListStyleType::None,
            "disc" => ListStyleType::Disc,
            "circle" => ListStyleType::Circle,
            "square" => ListStyleType::Square,
            "decimal" => ListStyleType::Decimal,
            "decimal-leading-zero" => ListStyleType::DecimalLeadingZero,
            "lower-roman" => ListStyleType::LowerRoman,
            "upper-roman" => ListStyleType::UpperRoman,
            "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
            "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
            "lower-greek" => ListStyleType::LowerGreek,
            _ => ListStyleType::Disc, // Default to disc
        }
    }

    /// Get the marker/bullet for a given list item index (1-based)
    pub fn get_marker(&self, index: usize) -> String {
        match self {
            ListStyleType::None => String::new(),
            ListStyleType::Disc => "•".to_string(),
            ListStyleType::Circle => "◦".to_string(),
            ListStyleType::Square => "▪".to_string(),
            ListStyleType::Decimal => format!("{}.", index),
            ListStyleType::DecimalLeadingZero => format!("{:02}.", index),
            ListStyleType::LowerRoman => format!("{}.", Self::to_lower_roman(index)),
            ListStyleType::UpperRoman => format!("{}.", Self::to_upper_roman(index)),
            ListStyleType::LowerAlpha | ListStyleType::LowerLatin => {
                format!("{}.", Self::to_lower_alpha(index))
            }
            ListStyleType::UpperAlpha | ListStyleType::UpperLatin => {
                format!("{}.", Self::to_upper_alpha(index))
            }
            ListStyleType::LowerGreek => format!("{}.", Self::to_lower_greek(index)),
        }
    }

    /// Convert number to lowercase alphabetic representation (a, b, c, ...)
    fn to_lower_alpha(num: usize) -> String {
        if num == 0 {
            return String::new();
        }
        let mut result = String::new();
        let mut n = num;
        while n > 0 {
            n -= 1;
            result.insert(0, (b'a' + (n % 26) as u8) as char);
            n /= 26;
        }
        result
    }

    /// Convert number to uppercase alphabetic representation (A, B, C, ...)
    fn to_upper_alpha(num: usize) -> String {
        if num == 0 {
            return String::new();
        }
        let mut result = String::new();
        let mut n = num;
        while n > 0 {
            n -= 1;
            result.insert(0, (b'A' + (n % 26) as u8) as char);
            n /= 26;
        }
        result
    }

    /// Convert number to lowercase Roman numerals
    fn to_lower_roman(num: usize) -> String {
        Self::to_roman(num).to_lowercase()
    }

    /// Convert number to uppercase Roman numerals
    fn to_upper_roman(num: usize) -> String {
        Self::to_roman(num)
    }

    /// Convert number to Roman numerals
    fn to_roman(num: usize) -> String {
        if num == 0 {
            return String::new();
        }
        let values = [
            (1000, "M"),
            (900, "CM"),
            (500, "D"),
            (400, "CD"),
            (100, "C"),
            (90, "XC"),
            (50, "L"),
            (40, "XL"),
            (10, "X"),
            (9, "IX"),
            (5, "V"),
            (4, "IV"),
            (1, "I"),
        ];

        let mut result = String::new();
        let mut n = num;

        for (value, symbol) in values.iter() {
            while n >= *value {
                result.push_str(symbol);
                n -= value;
            }
        }

        result
    }

    /// Convert number to lowercase Greek letters
    fn to_lower_greek(num: usize) -> String {
        if num == 0 {
            return String::new();
        }
        // Greek alphabet: α, β, γ, δ, ε, ζ, η, θ, ι, κ, λ, μ, ν, ξ, ο, π, ρ, σ, τ, υ, φ, χ, ψ, ω
        let greek = ['α', 'β', 'γ', 'δ', 'ε', 'ζ', 'η', 'θ', 'ι', 'κ', 'λ', 'μ',
                     'ν', 'ξ', 'ο', 'π', 'ρ', 'σ', 'τ', 'υ', 'φ', 'χ', 'ψ', 'ω'];

        if num <= greek.len() {
            greek[num - 1].to_string()
        } else {
            // For numbers beyond the Greek alphabet, cycle through
            let idx = (num - 1) % greek.len();
            format!("{}{}", greek[idx], (num - 1) / greek.len() + 1)
        }
    }
}

impl Default for ListStyleType {
    fn default() -> Self {
        ListStyleType::Disc
    }
}

/// CSS transition timing function (easing)
#[derive(Debug, Clone, PartialEq)]
pub enum TimingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    CubicBezier(f32, f32, f32, f32),
    StepStart,
    StepEnd,
    Steps(i32, StepPosition),
}

/// Step position for steps() timing function
#[derive(Debug, Clone, PartialEq)]
pub enum StepPosition {
    Start,
    End,
}

impl TimingFunction {
    /// Parse timing function from CSS string
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        match value.to_lowercase().as_str() {
            "linear" => TimingFunction::Linear,
            "ease" => TimingFunction::Ease,
            "ease-in" => TimingFunction::EaseIn,
            "ease-out" => TimingFunction::EaseOut,
            "ease-in-out" => TimingFunction::EaseInOut,
            "step-start" => TimingFunction::StepStart,
            "step-end" => TimingFunction::StepEnd,
            _ => {
                // Try to parse cubic-bezier() or steps()
                if value.starts_with("cubic-bezier(") && value.ends_with(')') {
                    if let Some(bezier) = Self::parse_cubic_bezier(value) {
                        return bezier;
                    }
                } else if value.starts_with("steps(") && value.ends_with(')') {
                    if let Some(steps) = Self::parse_steps(value) {
                        return steps;
                    }
                }
                TimingFunction::Ease // Default
            }
        }
    }

    fn parse_cubic_bezier(value: &str) -> Option<TimingFunction> {
        let content = &value[13..value.len()-1];
        let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

        if parts.len() == 4 {
            if let (Ok(x1), Ok(y1), Ok(x2), Ok(y2)) = (
                parts[0].parse::<f32>(),
                parts[1].parse::<f32>(),
                parts[2].parse::<f32>(),
                parts[3].parse::<f32>(),
            ) {
                return Some(TimingFunction::CubicBezier(x1, y1, x2, y2));
            }
        }
        None
    }

    fn parse_steps(value: &str) -> Option<TimingFunction> {
        let content = &value[6..value.len()-1];
        let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

        if parts.is_empty() {
            return None;
        }

        if let Ok(steps) = parts[0].parse::<i32>() {
            let position = if parts.len() > 1 {
                match parts[1].to_lowercase().as_str() {
                    "start" => StepPosition::Start,
                    "end" => StepPosition::End,
                    _ => StepPosition::End,
                }
            } else {
                StepPosition::End
            };
            return Some(TimingFunction::Steps(steps, position));
        }
        None
    }

    /// Apply the timing function to a progress value (0.0 to 1.0)
    /// Returns the eased progress value
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);

        match self {
            TimingFunction::Linear => t,
            TimingFunction::Ease => {
                // cubic-bezier(0.25, 0.1, 0.25, 1.0)
                Self::cubic_bezier(t, 0.25, 0.1, 0.25, 1.0)
            }
            TimingFunction::EaseIn => {
                // cubic-bezier(0.42, 0, 1.0, 1.0)
                Self::cubic_bezier(t, 0.42, 0.0, 1.0, 1.0)
            }
            TimingFunction::EaseOut => {
                // cubic-bezier(0, 0, 0.58, 1.0)
                Self::cubic_bezier(t, 0.0, 0.0, 0.58, 1.0)
            }
            TimingFunction::EaseInOut => {
                // cubic-bezier(0.42, 0, 0.58, 1.0)
                Self::cubic_bezier(t, 0.42, 0.0, 0.58, 1.0)
            }
            TimingFunction::CubicBezier(x1, y1, x2, y2) => {
                Self::cubic_bezier(t, *x1, *y1, *x2, *y2)
            }
            TimingFunction::StepStart => {
                if t > 0.0 { 1.0 } else { 0.0 }
            }
            TimingFunction::StepEnd => {
                if t >= 1.0 { 1.0 } else { 0.0 }
            }
            TimingFunction::Steps(steps, position) => {
                Self::steps(t, *steps, position)
            }
        }
    }

    /// Calculate cubic bezier curve value
    /// Simplified implementation using Newton-Raphson method
    fn cubic_bezier(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
        // Simplified cubic bezier calculation
        // For a more accurate implementation, you'd solve for x(t) = t
        // and then calculate y at that t value

        // This is an approximation using the parametric form directly
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        // Cubic bezier formula: B(t) = (1-t)³P0 + 3(1-t)²tP1 + 3(1-t)t²P2 + t³P3
        // Where P0 = (0, 0) and P3 = (1, 1)
        3.0 * mt2 * t * y1 + 3.0 * mt * t2 * y2 + t3
    }

    /// Calculate step function value
    fn steps(t: f32, steps: i32, position: &StepPosition) -> f32 {
        if steps <= 0 {
            return t;
        }

        let steps_f = steps as f32;
        match position {
            StepPosition::Start => {
                ((t * steps_f).ceil() / steps_f).min(1.0)
            }
            StepPosition::End => {
                ((t * steps_f).floor() / steps_f).min(1.0)
            }
        }
    }
}

impl Default for TimingFunction {
    fn default() -> Self {
        TimingFunction::Ease
    }
}

/// CSS transition duration (in milliseconds)
#[derive(Debug, Clone, PartialEq)]
pub struct Duration(pub f32);

impl Duration {
    /// Parse duration from CSS string (supports s and ms)
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        if value.ends_with("ms") {
            if let Ok(ms) = value[..value.len()-2].trim().parse::<f32>() {
                return Duration(ms);
            }
        } else if value.ends_with('s') {
            if let Ok(s) = value[..value.len()-1].trim().parse::<f32>() {
                return Duration(s * 1000.0); // Convert to milliseconds
            }
        }

        Duration(0.0) // Default to 0
    }

    /// Get duration in milliseconds
    pub fn as_millis(&self) -> f32 {
        self.0
    }

    /// Get duration in seconds
    pub fn as_seconds(&self) -> f32 {
        self.0 / 1000.0
    }
}

impl Default for Duration {
    fn default() -> Self {
        Duration(0.0)
    }
}

/// Single transition configuration for one property
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub property: TransitionProperty,
    pub duration: Duration,
    pub timing_function: TimingFunction,
    pub delay: Duration,
}

impl Transition {
    /// Create a new transition with default values
    pub fn new(property: TransitionProperty) -> Self {
        Self {
            property,
            duration: Duration(0.0),
            timing_function: TimingFunction::Ease,
            delay: Duration(0.0),
        }
    }

    /// Parse a single transition from CSS string
    /// Format: <property> <duration> <timing-function> <delay>
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();

        if value == "none" {
            return None;
        }

        let parts: Vec<&str> = value.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let mut property = TransitionProperty::All;
        let mut duration = Duration(0.0);
        let mut timing_function = TimingFunction::Ease;
        let mut delay = Duration(0.0);

        let mut duration_set = false;

        for part in parts {
            // Try to parse as property name
            if !duration_set && TransitionProperty::is_property_name(part) {
                property = TransitionProperty::parse(part);
            }
            // Try to parse as duration
            else if part.ends_with('s') || part.ends_with("ms") {
                let parsed_duration = Duration::parse(part);
                if !duration_set {
                    duration = parsed_duration;
                    duration_set = true;
                } else {
                    delay = parsed_duration;
                }
            }
            // Try to parse as timing function
            else if Self::is_timing_function(part) {
                timing_function = TimingFunction::parse(part);
            }
        }

        Some(Transition {
            property,
            duration,
            timing_function,
            delay,
        })
    }

    fn is_timing_function(value: &str) -> bool {
        matches!(value.to_lowercase().as_str(),
            "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" |
            "step-start" | "step-end"
        ) || value.starts_with("cubic-bezier(") || value.starts_with("steps(")
    }
}

/// Property that can be transitioned
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionProperty {
    All,
    BackgroundColor,
    Color,
    Opacity,
    Width,
    Height,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    Transform,
    Custom(String),
}

impl TransitionProperty {
    /// Parse property name from string
    pub fn parse(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "all" => TransitionProperty::All,
            "background-color" => TransitionProperty::BackgroundColor,
            "color" => TransitionProperty::Color,
            "opacity" => TransitionProperty::Opacity,
            "width" => TransitionProperty::Width,
            "height" => TransitionProperty::Height,
            "margin-top" => TransitionProperty::MarginTop,
            "margin-right" => TransitionProperty::MarginRight,
            "margin-bottom" => TransitionProperty::MarginBottom,
            "margin-left" => TransitionProperty::MarginLeft,
            "padding-top" => TransitionProperty::PaddingTop,
            "padding-right" => TransitionProperty::PaddingRight,
            "padding-bottom" => TransitionProperty::PaddingBottom,
            "padding-left" => TransitionProperty::PaddingLeft,
            "border-top-width" => TransitionProperty::BorderTopWidth,
            "border-right-width" => TransitionProperty::BorderRightWidth,
            "border-bottom-width" => TransitionProperty::BorderBottomWidth,
            "border-left-width" => TransitionProperty::BorderLeftWidth,
            "transform" => TransitionProperty::Transform,
            _ => TransitionProperty::Custom(value.to_string()),
        }
    }

    /// Check if a string is a valid property name for transitions
    fn is_property_name(value: &str) -> bool {
        // Common property names
        matches!(value.to_lowercase().as_str(),
            "all" | "background-color" | "color" | "opacity" | "width" | "height" |
            "margin-top" | "margin-right" | "margin-bottom" | "margin-left" |
            "padding-top" | "padding-right" | "padding-bottom" | "padding-left" |
            "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" |
            "transform"
        )
    }
}

/// Complete transition specification (can contain multiple transitions)
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionSpec {
    pub transitions: Vec<Transition>,
}

impl TransitionSpec {
    /// Parse transition specification from CSS string
    /// Format: <property> <duration> <timing-function> <delay>, ...
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        if value == "none" {
            return TransitionSpec {
                transitions: Vec::new(),
            };
        }

        // Split by commas for multiple transitions
        let transition_strings: Vec<&str> = value.split(',').map(|s| s.trim()).collect();
        let mut transitions = Vec::new();

        for transition_str in transition_strings {
            if let Some(transition) = Transition::parse(transition_str) {
                transitions.push(transition);
            }
        }

        TransitionSpec { transitions }
    }

    /// Check if any property is being transitioned
    pub fn has_transitions(&self) -> bool {
        !self.transitions.is_empty()
    }

    /// Find transition for a specific property
    pub fn get_transition_for_property(&self, property_name: &str) -> Option<&Transition> {
        for transition in &self.transitions {
            match &transition.property {
                TransitionProperty::All => return Some(transition),
                TransitionProperty::Custom(name) if name == property_name => return Some(transition),
                _ => {
                    // Check if the property matches
                    if Self::property_matches(property_name, &transition.property) {
                        return Some(transition);
                    }
                }
            }
        }
        None
    }

    fn property_matches(property_name: &str, transition_property: &TransitionProperty) -> bool {
        match transition_property {
            TransitionProperty::BackgroundColor => property_name == "background-color",
            TransitionProperty::Color => property_name == "color",
            TransitionProperty::Opacity => property_name == "opacity",
            TransitionProperty::Width => property_name == "width",
            TransitionProperty::Height => property_name == "height",
            TransitionProperty::MarginTop => property_name == "margin-top",
            TransitionProperty::MarginRight => property_name == "margin-right",
            TransitionProperty::MarginBottom => property_name == "margin-bottom",
            TransitionProperty::MarginLeft => property_name == "margin-left",
            TransitionProperty::PaddingTop => property_name == "padding-top",
            TransitionProperty::PaddingRight => property_name == "padding-right",
            TransitionProperty::PaddingBottom => property_name == "padding-bottom",
            TransitionProperty::PaddingLeft => property_name == "padding-left",
            TransitionProperty::BorderTopWidth => property_name == "border-top-width",
            TransitionProperty::BorderRightWidth => property_name == "border-right-width",
            TransitionProperty::BorderBottomWidth => property_name == "border-bottom-width",
            TransitionProperty::BorderLeftWidth => property_name == "border-left-width",
            TransitionProperty::Transform => property_name == "transform",
            _ => false,
        }
    }
}

impl Default for TransitionSpec {
    fn default() -> Self {
        TransitionSpec {
            transitions: Vec::new(),
        }
    }
}

/// CSS outline style
#[derive(Debug, Clone, PartialEq)]
pub enum OutlineStyle {
    None,
    Hidden,
    Dotted,
    Dashed,
    Solid,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl OutlineStyle {
    /// Parse outline-style value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "none" => OutlineStyle::None,
            "hidden" => OutlineStyle::Hidden,
            "dotted" => OutlineStyle::Dotted,
            "dashed" => OutlineStyle::Dashed,
            "solid" => OutlineStyle::Solid,
            "double" => OutlineStyle::Double,
            "groove" => OutlineStyle::Groove,
            "ridge" => OutlineStyle::Ridge,
            "inset" => OutlineStyle::Inset,
            "outset" => OutlineStyle::Outset,
            _ => OutlineStyle::None, // Default to none
        }
    }
}

impl Default for OutlineStyle {
    fn default() -> Self {
        OutlineStyle::None
    }
}

/// CSS outline properties
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    pub width: Length,
    pub style: OutlineStyle,
    pub color: Color,
}

impl Outline {
    /// Create a new outline
    pub fn new(width: Length, style: OutlineStyle, color: Color) -> Self {
        Self { width, style, color }
    }

    /// Create a default outline (none)
    pub fn none() -> Self {
        Self {
            width: Length::px(0.0),
            style: OutlineStyle::None,
            color: Color::Named("black".to_string()),
        }
    }

    /// Parse outline shorthand from CSS string
    /// Format: <width> <style> <color> (in any order)
    /// Examples:
    ///   outline: 2px solid red;
    ///   outline: dashed blue 3px;
    ///   outline: none;
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        // Check for "none"
        if value.to_lowercase() == "none" {
            return Self::none();
        }

        let mut width = Length::px(3.0); // Default medium width
        let mut style = OutlineStyle::Solid; // Default style
        let mut color = Color::Named("currentcolor".to_string()); // Default to current color

        // Split by whitespace and parse each part
        let parts: Vec<&str> = value.split_whitespace().collect();

        for part in parts {
            // Try to parse as width (length)
            if let CssValue::Length(len) = CssValue::parse(part) {
                width = len;
            }
            // Try to parse as style
            else if Self::is_outline_style(part) {
                style = OutlineStyle::parse(part);
            }
            // Try to parse as color
            else if let CssValue::Color(c) = CssValue::parse(part) {
                color = c;
            }
            // Check for named outline width keywords
            else {
                match part.to_lowercase().as_str() {
                    "thin" => width = Length::px(1.0),
                    "medium" => width = Length::px(3.0),
                    "thick" => width = Length::px(5.0),
                    _ => {}
                }
            }
        }

        Self { width, style, color }
    }

    fn is_outline_style(value: &str) -> bool {
        matches!(value.to_lowercase().as_str(),
            "none" | "hidden" | "dotted" | "dashed" | "solid" |
            "double" | "groove" | "ridge" | "inset" | "outset"
        )
    }

    /// Check if outline is visible
    pub fn is_visible(&self) -> bool {
        !matches!(self.style, OutlineStyle::None | OutlineStyle::Hidden)
    }
}

impl Default for Outline {
    fn default() -> Self {
        Self::none()
    }
}

/// CSS flex-basis property
#[derive(Debug, Clone, PartialEq)]
pub enum FlexBasis {
    Auto,
    Length(Length),
    Content,
}

impl FlexBasis {
    /// Parse flex-basis value from string
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        match value.to_lowercase().as_str() {
            "auto" => FlexBasis::Auto,
            "content" => FlexBasis::Content,
            _ => {
                // Try to parse as a length
                if let CssValue::Length(length) = CssValue::parse(value) {
                    FlexBasis::Length(length)
                } else {
                    FlexBasis::Auto // Default to auto
                }
            }
        }
    }

    /// Convert to pixels given a context
    pub fn to_px(&self, font_size: f32, parent_size: f32) -> Option<f32> {
        match self {
            FlexBasis::Auto => None, // Auto should be handled by layout algorithm
            FlexBasis::Length(length) => Some(length.to_px(font_size, parent_size)),
            FlexBasis::Content => None, // Content sizing should be handled by layout algorithm
        }
    }
}

impl Default for FlexBasis {
    fn default() -> Self {
        FlexBasis::Auto
    }
}
