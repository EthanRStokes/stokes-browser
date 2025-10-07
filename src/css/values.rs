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
