// Computed CSS values structure
use crate::css::{BorderRadius, BoxShadow, Stroke, TextDecoration, TextShadow};
use crate::layout::box_model::EdgeSizes;

/// Computed CSS values for a node
#[derive(Debug, Clone)]
pub struct ComputedValues {
    pub color: Option<crate::css::values::Color>,
    pub background_color: Option<crate::css::values::Color>,
    pub background_image: crate::css::BackgroundImage,
    pub font_size: f32,
    pub font_family: String,
    pub font_weight: String,
    pub font_style: crate::css::FontStyle,
    pub font_variant: crate::css::FontVariant,
    pub line_height: crate::css::LineHeight,
    pub text_decoration: TextDecoration,
    pub text_align: crate::css::TextAlign,
    pub text_transform: crate::css::TextTransform,
    pub text_shadow: Vec<TextShadow>,
    pub white_space: crate::css::WhiteSpace,
    pub vertical_align: crate::css::VerticalAlign,
    pub content: crate::css::ContentValue,
    pub clear: crate::css::Clear,
    pub float: crate::css::Float,
    pub overflow: crate::css::Overflow,
    pub overflow_x: crate::css::Overflow,
    pub overflow_y: crate::css::Overflow,
    pub display: DisplayType,
    pub visibility: crate::css::Visibility,
    pub width: Option<crate::css::values::Length>,
    pub height: Option<crate::css::values::Length>,
    pub max_width: Option<crate::css::values::Length>,
    pub min_width: Option<crate::css::values::Length>,
    pub max_height: Option<crate::css::values::Length>,
    pub min_height: Option<crate::css::values::Length>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub border_radius: BorderRadius,
    pub box_shadow: Vec<BoxShadow>,
    pub box_sizing: crate::css::BoxSizing,
    pub cursor: crate::css::Cursor,
    pub z_index: i32,
    pub opacity: f32,
    pub transition: crate::css::TransitionSpec,
    pub list_style_type: crate::css::ListStyleType,
    pub outline: crate::css::Outline,
    pub outline_offset: crate::css::values::Length,
    pub flex_grow: crate::css::FlexGrow,
    pub flex_shrink: crate::css::FlexShrink,
    pub flex_basis: crate::css::FlexBasis,
    pub gap: crate::css::Gap,
    pub stroke: Stroke,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayType {
    Block,
    Inline,
    InlineBlock,
    Flex,
    None,
}

impl Default for ComputedValues {
    fn default() -> Self {
        Self {
            color: Some(crate::css::values::Color::Named("black".to_string())),
            background_color: Some(crate::css::values::Color::Named("white".to_string())),
            background_image: crate::css::BackgroundImage::None,
            font_size: 16.0,
            font_family: "Arial".to_string(),
            font_weight: "normal".to_string(),
            font_style: crate::css::FontStyle::Normal,
            font_variant: crate::css::FontVariant::Normal,
            line_height: crate::css::LineHeight::Normal,
            text_decoration: TextDecoration::default(),
            text_align: crate::css::TextAlign::default(),
            text_transform: crate::css::TextTransform::default(),
            text_shadow: Vec::new(),
            white_space: crate::css::WhiteSpace::Normal,
            vertical_align: crate::css::VerticalAlign::default(),
            content: crate::css::ContentValue::Normal,
            clear: crate::css::Clear::None,
            float: crate::css::Float::None,
            overflow: crate::css::Overflow::default(),
            overflow_x: crate::css::Overflow::default(),
            overflow_y: crate::css::Overflow::default(),
            display: DisplayType::Block,
            visibility: crate::css::Visibility::Visible,
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
            box_sizing: crate::css::BoxSizing::ContentBox,
            cursor: crate::css::Cursor::Auto,
            z_index: 0,
            opacity: 1.0,
            transition: crate::css::TransitionSpec::default(),
            list_style_type: crate::css::ListStyleType::None,
            outline: crate::css::Outline::none(),
            outline_offset: crate::css::values::Length::default(),
            flex_grow: crate::css::FlexGrow::default(),
            flex_shrink: crate::css::FlexShrink::default(),
            flex_basis: crate::css::FlexBasis::default(),
            gap: crate::css::Gap::default(),
            stroke: Stroke::default(),
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
            "button" => DisplayType::InlineBlock,
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

        // Set default list-style-type for list elements
        match tag_name {
            "ul" => values.list_style_type = crate::css::ListStyleType::Disc,
            "ol" => values.list_style_type = crate::css::ListStyleType::Decimal,
            "li" => {
                // li elements inherit from their parent list (ul or ol)
                // For now, default to disc, but this will be overridden by CSS cascade
                values.list_style_type = crate::css::ListStyleType::Disc;
            }
            _ => {}
        }

        // Set default colors for links (similar to browser defaults)
        if tag_name == "a" {
            values.color = Some(crate::css::values::Color::Hex("#0000EE".to_string())); // Blue for unvisited links
            values.text_decoration = crate::css::TextDecoration::Underline;
        }

        // Default button styles
        if tag_name == "button" {
            values.cursor = crate::css::Cursor::Pointer;
            values.background_color = Some(crate::css::values::Color::Named("#f2f2f2".to_string()));
            // Add a small border by default
            values.border = EdgeSizes { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 };
        }

        values
    }
}
