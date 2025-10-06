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
    pub text_decoration: TextDecoration,
    pub text_align: super::TextAlign,
    pub clear: super::Clear,
    pub display: DisplayType,
    pub width: Option<super::values::Length>,
    pub height: Option<super::values::Length>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub border_radius: BorderRadius,
    pub box_shadow: Vec<BoxShadow>,
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
            text_decoration: TextDecoration::default(),
            text_align: super::TextAlign::default(),
            clear: super::Clear::None,
            display: DisplayType::Block,
            width: None,
            height: None,
            margin: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            border: EdgeSizes::default(),
            border_radius: BorderRadius::default(),
            box_shadow: Vec::new(),
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
            PropertyName::Clear => {
                if let CssValue::Keyword(clear_value) = &declaration.value {
                    computed.clear = super::Clear::parse(clear_value);
                } else if let CssValue::String(clear_value) = &declaration.value {
                    computed.clear = super::Clear::parse(clear_value);
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
            _ => {
                // Handle other properties as needed
            }
        }
    }
}
