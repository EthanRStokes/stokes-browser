// Computed CSS values and style resolution
use super::{PropertyName, CssValue, Stylesheet, Declaration, Selector, BorderRadius, BoxShadow, TextDecoration};
use crate::dom::{DomNode, NodeType, ElementData};
use crate::layout::box_model::EdgeSizes;

/// Computed CSS values for a node
#[derive(Debug, Clone)]
pub struct ComputedValues {
    pub color: Option<super::values::Color>,
    pub background_color: Option<super::values::Color>,
    pub background_image: super::BackgroundImage,
    pub font_size: f32,
    pub font_family: String,
    pub font_weight: String,
    pub font_style: super::FontStyle,
    pub font_variant: super::FontVariant,
    pub line_height: super::LineHeight,
    pub text_decoration: TextDecoration,
    pub text_align: super::TextAlign,
    pub vertical_align: super::VerticalAlign,
    pub content: super::ContentValue,
    pub clear: super::Clear,
    pub overflow: super::Overflow,
    pub display: DisplayType,
    pub width: Option<super::values::Length>,
    pub height: Option<super::values::Length>,
    pub max_width: Option<super::values::Length>,
    pub min_width: Option<super::values::Length>,
    pub max_height: Option<super::values::Length>,
    pub min_height: Option<super::values::Length>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub border_radius: BorderRadius,
    pub box_shadow: Vec<BoxShadow>,
    pub box_sizing: super::BoxSizing,
    pub cursor: super::Cursor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayType {
    Block,
    Inline,
    InlineBlock,
    None,
}

impl Default for ComputedValues {
    fn default() -> Self {
        Self {
            color: Some(super::values::Color::Named("black".to_string())),
            background_color: Some(super::values::Color::Named("white".to_string())),
            background_image: super::BackgroundImage::None,
            font_size: 16.0,
            font_family: "Arial".to_string(),
            font_weight: "normal".to_string(),
            font_style: super::FontStyle::Normal,
            font_variant: super::FontVariant::Normal,
            line_height: super::LineHeight::Normal,
            text_decoration: TextDecoration::default(),
            text_align: super::TextAlign::default(),
            vertical_align: super::VerticalAlign::default(),
            content: super::ContentValue::Normal,
            clear: super::Clear::None,
            overflow: super::Overflow::default(),
            display: DisplayType::Block,
            width: None,
            height: None,
            max_width: None,
            min_width: None,
            max_height: None,
            min_height: None,
            margin: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            border: EdgeSizes::default(),
            border_radius: BorderRadius::default(),
            box_shadow: Vec::new(),
            box_sizing: super::BoxSizing::ContentBox,
            cursor: super::Cursor::Auto,
        }
    }
}

impl ComputedValues {
    /// Create default computed values for an element
    pub fn default_for_element(tag_name: &str) -> Self {
        let mut values = Self::default();

        // Set default display type based on element
        values.display = match tag_name {
            "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" |
            "section" | "article" | "header" | "footer" | "nav" |
            "main" | "aside" | "blockquote" | "ul" | "ol" | "li" => DisplayType::Block,
            "span" | "a" | "em" | "strong" | "code" | "b" | "i" | "u" => DisplayType::Inline,
            "img" => DisplayType::InlineBlock,
            _ => DisplayType::Inline,
        };

        // Set default font sizes for headings
        match tag_name {
            "h1" => values.font_size = 32.0,
            "h2" => values.font_size = 24.0,
            "h3" => values.font_size = 18.72,
            "h4" => values.font_size = 16.0,
            "h5" => values.font_size = 13.28,
            "h6" => values.font_size = 10.72,
            _ => {}
        }

        // Set default colors for links (similar to browser defaults)
        if tag_name == "a" {
            values.color = Some(super::values::Color::Hex("#0000EE".to_string())); // Blue for unvisited links
            values.text_decoration = super::TextDecoration::Underline;
        }

        values
    }
}

/// Style resolver that computes final styles for DOM nodes
pub struct StyleResolver {
    stylesheets: Vec<Stylesheet>,
}

impl StyleResolver {
    pub fn new() -> Self {
        let mut resolver = Self {
            stylesheets: Vec::new(),
        };

        // Add default user agent stylesheet
        resolver.add_stylesheet(Stylesheet::default_styles());

        resolver
    }

    /// Add a stylesheet to consider during style resolution
    pub fn add_stylesheet(&mut self, stylesheet: Stylesheet) {
        self.stylesheets.push(stylesheet);
    }

    /// Resolve styles for a DOM node
    pub fn resolve_styles(&self, node: &DomNode, parent_values: Option<&ComputedValues>) -> ComputedValues {
        let mut computed = match &node.node_type {
            NodeType::Element(element_data) => {
                ComputedValues::default_for_element(&element_data.tag_name)
            }
            _ => ComputedValues::default(),
        };

        // Inherit from parent where appropriate
        if let Some(parent) = parent_values {
            computed.color = computed.color.or_else(|| parent.color.clone());
            computed.font_family = parent.font_family.clone();
            computed.font_size = parent.font_size; // Will be adjusted by relative units
        }

        // Apply matching CSS rules
        if let NodeType::Element(element_data) = &node.node_type {
            let matching_rules = self.find_matching_rules(element_data);

            // Sort by specificity (lower specificity first)
            let mut sorted_rules: Vec<_> = matching_rules.into_iter().collect();
            sorted_rules.sort_by_key(|(_, rule)| rule.specificity());

            // Apply declarations in specificity order
            for (_, rule) in sorted_rules {
                for declaration in &rule.declarations {
                    self.apply_declaration(&mut computed, declaration, parent_values);
                }
            }

            // Apply inline styles (highest specificity)
            if let Some(style_attr) = element_data.attributes.get("style") {
                let parser = super::parser::CssParser::new();
                let inline_declarations = parser.parse_inline_styles(style_attr);
                for declaration in inline_declarations {
                    self.apply_declaration(&mut computed, &declaration, parent_values);
                }
            }
        }

        computed
    }

    /// Find all rules that match an element
    fn find_matching_rules<'a>(&'a self, element_data: &ElementData) -> Vec<(&'a Selector, &'a super::stylesheet::Rule)> {
        let mut matching_rules = Vec::new();

        for stylesheet in &self.stylesheets {
            for rule in &stylesheet.rules {
                for selector in &rule.selectors {
                    if selector.matches_element(element_data) {
                        matching_rules.push((selector, rule));
                        break; // Only need one matching selector per rule
                    }
                }
            }
        }

        matching_rules
    }

    /// Apply a CSS declaration to computed values
    fn apply_declaration(&self, computed: &mut ComputedValues, declaration: &Declaration, parent_values: Option<&ComputedValues>) {
        match &declaration.property {
            PropertyName::Color => {
                if let CssValue::Color(color) = &declaration.value {
                    computed.color = Some(color.clone());
                }
            }
            PropertyName::Background => {
                // Parse background shorthand property
                // This can contain: color, image, position, size, repeat, origin, clip, attachment
                // For now, we'll handle color and image (url)
                self.parse_background_shorthand(computed, &declaration.value);
            }
            PropertyName::BackgroundColor => {
                if let CssValue::Color(color) = &declaration.value {
                    computed.background_color = Some(color.clone());
                }
            }
            PropertyName::BackgroundImage => {
                // Parse background-image value
                match &declaration.value {
                    CssValue::String(url_str) => {
                        computed.background_image = super::BackgroundImage::parse(url_str);
                    }
                    CssValue::Keyword(keyword) => {
                        if keyword == "none" {
                            computed.background_image = super::BackgroundImage::None;
                        } else {
                            // Try to parse as url() format
                            computed.background_image = super::BackgroundImage::parse(keyword);
                        }
                    }
                    _ => {}
                }
            }
            PropertyName::FontSize => {
                match &declaration.value {
                    CssValue::Length(length) => {
                        let parent_font_size = parent_values.map(|p| p.font_size).unwrap_or(16.0);
                        computed.font_size = length.to_px(parent_font_size, parent_font_size);
                    }
                    CssValue::Number(num) => {
                        computed.font_size = *num;
                    }
                    _ => {}
                }
            }
            PropertyName::FontFamily => {
                if let CssValue::String(family) = &declaration.value {
                    computed.font_family = family.clone();
                } else if let CssValue::Keyword(family) = &declaration.value {
                    computed.font_family = family.clone();
                }
            }
            PropertyName::FontWeight => {
                if let CssValue::Keyword(weight) = &declaration.value {
                    computed.font_weight = weight.clone();
                }
            }
            PropertyName::Font => {
                // Parse font shorthand property
                // Syntax: font: [font-style] [font-variant] [font-weight] font-size[/line-height] font-family
                // Required: font-size and font-family
                // Examples:
                //   font: 12px Arial;
                //   font: italic bold 16px/1.5 "Times New Roman", serif;
                //   font: small-caps 14px Georgia;
                self.parse_font_shorthand(computed, &declaration.value, parent_values);
            }
            PropertyName::FontStyle => {
                if let CssValue::Keyword(style) = &declaration.value {
                    computed.font_style = super::FontStyle::parse(style);
                } else if let CssValue::String(style) = &declaration.value {
                    computed.font_style = super::FontStyle::parse(style);
                }
            }
            PropertyName::FontVariant => {
                if let CssValue::Keyword(variant) = &declaration.value {
                    computed.font_variant = super::FontVariant::parse(variant);
                } else if let CssValue::String(variant) = &declaration.value {
                    computed.font_variant = super::FontVariant::parse(variant);
                }
            }
            PropertyName::LineHeight => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.line_height = super::LineHeight::Length(length.clone());
                } else if let CssValue::Number(num) = &declaration.value {
                    computed.line_height = super::LineHeight::Number(*num);
                } else if let CssValue::Keyword(keyword) = &declaration.value {
                    computed.line_height = super::LineHeight::parse(keyword);
                } else if let CssValue::String(s) = &declaration.value {
                    computed.line_height = super::LineHeight::parse(s);
                }
            }
            PropertyName::TextDecoration => {
                if let CssValue::Keyword(decoration) = &declaration.value {
                    computed.text_decoration = TextDecoration::parse(decoration);
                } else if let CssValue::String(decoration) = &declaration.value {
                    computed.text_decoration = TextDecoration::parse(decoration);
                }
            }
            PropertyName::TextAlign => {
                if let CssValue::Keyword(align) = &declaration.value {
                    computed.text_align = super::TextAlign::parse(align);
                } else if let CssValue::String(align) = &declaration.value {
                    computed.text_align = super::TextAlign::parse(align);
                }
            }
            PropertyName::VerticalAlign => {
                if let CssValue::Keyword(align) = &declaration.value {
                    computed.vertical_align = super::VerticalAlign::parse(align);
                } else if let CssValue::String(align) = &declaration.value {
                    computed.vertical_align = super::VerticalAlign::parse(align);
                } else if let CssValue::Length(length) = &declaration.value {
                    computed.vertical_align = super::VerticalAlign::Length(length.clone());
                }
            }
            PropertyName::Content => {
                if let CssValue::String(content) = &declaration.value {
                    computed.content = super::ContentValue::String(content.clone());
                } else if let CssValue::Keyword(keyword) = &declaration.value {
                    computed.content = super::ContentValue::parse(keyword);
                }
            }
            PropertyName::Clear => {
                if let CssValue::Keyword(clear_value) = &declaration.value {
                    computed.clear = super::Clear::parse(clear_value);
                } else if let CssValue::String(clear_value) = &declaration.value {
                    computed.clear = super::Clear::parse(clear_value);
                }
            }
            PropertyName::Overflow => {
                if let CssValue::Keyword(overflow_value) = &declaration.value {
                    computed.overflow = super::Overflow::parse(overflow_value);
                } else if let CssValue::String(overflow_value) = &declaration.value {
                    computed.overflow = super::Overflow::parse(overflow_value);
                }
            }
            PropertyName::Display => {
                if let CssValue::Keyword(display) = &declaration.value {
                    computed.display = match display.as_str() {
                        "block" => DisplayType::Block,
                        "inline" => DisplayType::Inline,
                        "inline-block" => DisplayType::InlineBlock,
                        "none" => DisplayType::None,
                        _ => computed.display.clone(),
                    };
                }
            }
            PropertyName::Width => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.width = Some(length.clone());
                } else if let CssValue::Auto = &declaration.value {
                    computed.width = None; // Auto width
                }
            }
            PropertyName::Height => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.height = Some(length.clone());
                } else if let CssValue::Auto = &declaration.value {
                    computed.height = None; // Auto height
                }
            }
            PropertyName::MaxWidth => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.max_width = Some(length.clone());
                } else if let CssValue::Auto = &declaration.value {
                    computed.max_width = None; // Auto max-width
                }
            }
            PropertyName::MinWidth => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.min_width = Some(length.clone());
                } else if let CssValue::Auto = &declaration.value {
                    computed.min_width = None; // Auto min-width
                }
            }
            PropertyName::MaxHeight => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.max_height = Some(length.clone());
                } else if let CssValue::Auto = &declaration.value {
                    computed.max_height = None; // Auto max-height
                }
            }
            PropertyName::MinHeight => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.min_height = Some(length.clone());
                } else if let CssValue::Auto = &declaration.value {
                    computed.min_height = None; // Auto min-height
                }
            }
            PropertyName::Margin => {
                match &declaration.value {
                    CssValue::Length(length) => {
                        let parent_size = 400.0; // Default container width
                        let px_value = length.to_px(computed.font_size, parent_size);
                        computed.margin = EdgeSizes::uniform(px_value);
                    }
                    CssValue::Auto => {
                        // Auto margins - keep as 0 for now, will be resolved during layout
                        computed.margin = EdgeSizes::uniform(0.0);
                    }
                    CssValue::MultipleValues(values) => {
                        // Handle CSS margin shorthand syntax
                        let parent_size = 400.0;
                        match values.len() {
                            1 => {
                                // margin: value -> all sides
                                if let Some(val) = values.first() {
                                    match val {
                                        CssValue::Length(length) => {
                                            let px_value = length.to_px(computed.font_size, parent_size);
                                            computed.margin = EdgeSizes::uniform(px_value);
                                        }
                                        CssValue::Auto => {
                                            computed.margin = EdgeSizes::uniform(0.0);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            2 => {
                                // margin: vertical horizontal (e.g., "5em auto")
                                let vertical = &values[0];
                                let horizontal = &values[1];

                                let top_bottom = match vertical {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0,
                                    _ => 0.0,
                                };

                                let left_right = match horizontal {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0, // Changed from f32::INFINITY to 0.0
                                    _ => 0.0,
                                };

                                computed.margin = EdgeSizes::new(top_bottom, left_right, top_bottom, left_right);
                            }
                            3 => {
                                // margin: top horizontal bottom
                                let top = match &values[0] {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0,
                                    _ => 0.0,
                                };
                                let horizontal = match &values[1] {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0, // Changed from f32::INFINITY to 0.0
                                    _ => 0.0,
                                };
                                let bottom = match &values[2] {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0,
                                    _ => 0.0,
                                };

                                computed.margin = EdgeSizes::new(top, horizontal, bottom, horizontal);
                            }
                            4 => {
                                // margin: top right bottom left
                                let top = match &values[0] {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0,
                                    _ => 0.0,
                                };
                                let right = match &values[1] {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0, // Changed from f32::INFINITY to 0.0
                                    _ => 0.0,
                                };
                                let bottom = match &values[2] {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0,
                                    _ => 0.0,
                                };
                                let left = match &values[3] {
                                    CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                                    CssValue::Auto => 0.0, // Changed from f32::INFINITY to 0.0
                                    _ => 0.0,
                                };

                                computed.margin = EdgeSizes::new(top, right, bottom, left);
                            }
                            _ => {
                                // Invalid number of values, ignore
                            }
                        }
                    }
                    _ => {
                        // Unsupported value type for margin
                    }
                }
            }
            PropertyName::MarginTop => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    computed.margin.top = length.to_px(computed.font_size, parent_size);
                }
            }
            PropertyName::MarginRight => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    computed.margin.right = length.to_px(computed.font_size, parent_size);
                } else if let CssValue::Auto = &declaration.value {
                    computed.margin.right = 0.0; // Changed from f32::INFINITY to 0.0
                }
            }
            PropertyName::MarginBottom => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    computed.margin.bottom = length.to_px(computed.font_size, parent_size);
                }
            }
            PropertyName::MarginLeft => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    computed.margin.left = length.to_px(computed.font_size, parent_size);
                } else if let CssValue::Auto = &declaration.value {
                    computed.margin.left = 0.0; // Changed from f32::INFINITY to 0.0
                }
            }
            PropertyName::Padding => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    let px_value = length.to_px(computed.font_size, parent_size);
                    computed.padding = EdgeSizes::uniform(px_value);
                }
            }
            PropertyName::PaddingTop => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    computed.padding.top = length.to_px(computed.font_size, parent_size);
                }
            }
            PropertyName::PaddingRight => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    computed.padding.right = length.to_px(computed.font_size, parent_size);
                }
            }
            PropertyName::PaddingBottom => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    computed.padding.bottom = length.to_px(computed.font_size, parent_size);
                }
            }
            PropertyName::PaddingLeft => {
                if let CssValue::Length(length) = &declaration.value {
                    let parent_size = 400.0;
                    computed.padding.left = length.to_px(computed.font_size, parent_size);
                }
            }
            PropertyName::BorderRadius => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.border_radius = BorderRadius::uniform(length.clone());
                }
            }
            PropertyName::BorderTopLeftRadius => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.border_radius.top_left = length.clone();
                }
            }
            PropertyName::BorderTopRightRadius => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.border_radius.top_right = length.clone();
                }
            }
            PropertyName::BorderBottomLeftRadius => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.border_radius.bottom_left = length.clone();
                }
            }
            PropertyName::BorderBottomRightRadius => {
                if let CssValue::Length(length) = &declaration.value {
                    computed.border_radius.bottom_right = length.clone();
                }
            }
            PropertyName::BoxShadow => {
                if let CssValue::String(shadow_str) = &declaration.value {
                    if let Some(shadows) = BoxShadow::parse(shadow_str) {
                        computed.box_shadow = shadows;
                    }
                } else if let CssValue::Keyword(keyword) = &declaration.value {
                    if keyword == "none" {
                        computed.box_shadow.clear();
                    } else {
                        // Try to parse the keyword as a shadow value
                        if let Some(shadows) = BoxShadow::parse(keyword) {
                            computed.box_shadow = shadows;
                        }
                    }
                }
            }
            PropertyName::BoxSizing => {
                if let CssValue::Keyword(sizing) = &declaration.value {
                    computed.box_sizing = super::BoxSizing::parse(sizing);
                } else if let CssValue::String(sizing) = &declaration.value {
                    computed.box_sizing = super::BoxSizing::parse(sizing);
                }
            }
            PropertyName::Cursor => {
                if let CssValue::Keyword(cursor) = &declaration.value {
                    computed.cursor = super::Cursor::parse(cursor);
                } else if let CssValue::String(cursor) = &declaration.value {
                    computed.cursor = super::Cursor::parse(cursor);
                }
            }
            _ => {
                // Handle other properties as needed
            }
        }
    }

    /// Parse background shorthand property
    /// Handles: background: [color] [image] [position] [size] [repeat] [origin] [clip] [attachment]
    /// Examples:
    ///   background: red;
    ///   background: url(image.jpg);
    ///   background: #ff0000 url(image.jpg);
    ///   background: linear-gradient(...);
    fn parse_background_shorthand(&self, computed: &mut ComputedValues, value: &CssValue) {
        // Reset background properties to defaults when using shorthand
        computed.background_color = None;
        computed.background_image = super::BackgroundImage::None;

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
                    computed.background_image = super::BackgroundImage::parse(keyword);
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
                    computed.background_image = super::BackgroundImage::parse(s);
                } else {
                    // Try parsing the entire string for multiple values
                    self.parse_complex_background(computed, s);
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
                                computed.background_image = super::BackgroundImage::parse(keyword);
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
                                computed.background_image = super::BackgroundImage::parse(s);
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
    fn parse_complex_background(&self, computed: &mut ComputedValues, value: &str) {
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
                computed.background_image = super::BackgroundImage::parse(&part);
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
    fn parse_font_shorthand(&self, computed: &mut ComputedValues, value: &CssValue, parent_values: Option<&ComputedValues>) {
        // Reset font properties to defaults when using shorthand
        computed.font_style = super::FontStyle::Normal;
        computed.font_variant = super::FontVariant::Normal;
        computed.font_weight = "normal".to_string();
        computed.line_height = super::LineHeight::Normal;

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
        let parts = self.split_font_value(&font_str);

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
                computed.font_style = super::FontStyle::parse(part);
                i += 1;
                continue;
            }

            // Check for font-variant (must come before font-size)
            if !font_size_found && part_lower == "small-caps" {
                computed.font_variant = super::FontVariant::parse(part);
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
                        computed.line_height = super::LineHeight::parse(size_height[1]);
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
    fn split_font_value(&self, value: &str) -> Vec<String> {
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
}
