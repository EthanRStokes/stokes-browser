// Declaration application logic for CSS properties
use super::values::{ComputedValues, DisplayType};
use crate::css::{PropertyName, CssValue, Declaration, BorderRadius, BoxShadow, TextDecoration};
use crate::layout::box_model::EdgeSizes;

/// Apply a CSS declaration to computed values
pub fn apply_declaration(computed: &mut ComputedValues, declaration: &Declaration, parent_values: Option<&ComputedValues>) {
    match &declaration.property {
        PropertyName::Color => {
            if let CssValue::Color(color) = &declaration.value {
                computed.color = Some(color.clone());
            }
        }
        PropertyName::Background => {
            // Parse background shorthand property
            super::shorthands::parse_background_shorthand(computed, &declaration.value);
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
                    computed.background_image = crate::css::BackgroundImage::parse(url_str);
                }
                CssValue::Keyword(keyword) => {
                    if keyword == "none" {
                        computed.background_image = crate::css::BackgroundImage::None;
                    } else {
                        // Try to parse as url() format
                        computed.background_image = crate::css::BackgroundImage::parse(keyword);
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
            super::shorthands::parse_font_shorthand(computed, &declaration.value, parent_values);
        }
        PropertyName::FontStyle => {
            if let CssValue::Keyword(style) = &declaration.value {
                computed.font_style = crate::css::FontStyle::parse(style);
            } else if let CssValue::String(style) = &declaration.value {
                computed.font_style = crate::css::FontStyle::parse(style);
            }
        }
        PropertyName::FontVariant => {
            if let CssValue::Keyword(variant) = &declaration.value {
                computed.font_variant = crate::css::FontVariant::parse(variant);
            } else if let CssValue::String(variant) = &declaration.value {
                computed.font_variant = crate::css::FontVariant::parse(variant);
            }
        }
        PropertyName::LineHeight => {
            if let CssValue::Length(length) = &declaration.value {
                computed.line_height = crate::css::LineHeight::Length(length.clone());
            } else if let CssValue::Number(num) = &declaration.value {
                computed.line_height = crate::css::LineHeight::Number(*num);
            } else if let CssValue::Keyword(keyword) = &declaration.value {
                computed.line_height = crate::css::LineHeight::parse(keyword);
            } else if let CssValue::String(s) = &declaration.value {
                computed.line_height = crate::css::LineHeight::parse(s);
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
                computed.text_align = crate::css::TextAlign::parse(align);
            } else if let CssValue::String(align) = &declaration.value {
                computed.text_align = crate::css::TextAlign::parse(align);
            }
        }
        PropertyName::TextTransform => {
            if let CssValue::Keyword(transform) = &declaration.value {
                computed.text_transform = crate::css::TextTransform::parse(transform);
            } else if let CssValue::String(transform) = &declaration.value {
                computed.text_transform = crate::css::TextTransform::parse(transform);
            }
        }
        PropertyName::WhiteSpace => {
            if let CssValue::Keyword(white_space) = &declaration.value {
                computed.white_space = crate::css::WhiteSpace::parse(white_space);
            } else if let CssValue::String(white_space) = &declaration.value {
                computed.white_space = crate::css::WhiteSpace::parse(white_space);
            }
        }
        PropertyName::VerticalAlign => {
            if let CssValue::Keyword(align) = &declaration.value {
                computed.vertical_align = crate::css::VerticalAlign::parse(align);
            } else if let CssValue::String(align) = &declaration.value {
                computed.vertical_align = crate::css::VerticalAlign::parse(align);
            } else if let CssValue::Length(length) = &declaration.value {
                computed.vertical_align = crate::css::VerticalAlign::Length(length.clone());
            }
        }
        PropertyName::Content => {
            if let CssValue::String(content) = &declaration.value {
                computed.content = crate::css::ContentValue::String(content.clone());
            } else if let CssValue::Keyword(keyword) = &declaration.value {
                computed.content = crate::css::ContentValue::parse(keyword);
            }
        }
        PropertyName::Clear => {
            if let CssValue::Keyword(clear_value) = &declaration.value {
                computed.clear = crate::css::Clear::parse(clear_value);
            } else if let CssValue::String(clear_value) = &declaration.value {
                computed.clear = crate::css::Clear::parse(clear_value);
            }
        }
        PropertyName::Float => {
            if let CssValue::Keyword(float_value) = &declaration.value {
                computed.float = crate::css::Float::parse(float_value);
            } else if let CssValue::String(float_value) = &declaration.value {
                computed.float = crate::css::Float::parse(float_value);
            }
        }
        PropertyName::Overflow => {
            if let CssValue::Keyword(overflow_value) = &declaration.value {
                computed.overflow = crate::css::Overflow::parse(overflow_value);
            } else if let CssValue::String(overflow_value) = &declaration.value {
                computed.overflow = crate::css::Overflow::parse(overflow_value);
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
        PropertyName::Visibility => {
            if let CssValue::Keyword(visibility) = &declaration.value {
                computed.visibility = crate::css::Visibility::parse(visibility);
            } else if let CssValue::String(visibility) = &declaration.value {
                computed.visibility = crate::css::Visibility::parse(visibility);
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
            apply_margin_property(computed, &declaration.value);
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
                computed.margin.right = 0.0;
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
                computed.margin.left = 0.0;
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
                computed.box_sizing = crate::css::BoxSizing::parse(sizing);
            } else if let CssValue::String(sizing) = &declaration.value {
                computed.box_sizing = crate::css::BoxSizing::parse(sizing);
            }
        }
        PropertyName::Cursor => {
            if let CssValue::Keyword(cursor) = &declaration.value {
                computed.cursor = crate::css::Cursor::parse(cursor);
            } else if let CssValue::String(cursor) = &declaration.value {
                computed.cursor = crate::css::Cursor::parse(cursor);
            }
        }
        PropertyName::ZIndex => {
            match &declaration.value {
                CssValue::Number(num) => {
                    computed.z_index = *num as i32;
                }
                CssValue::Keyword(keyword) => {
                    if keyword == "auto" {
                        computed.z_index = 0; // Auto z-index defaults to 0
                    } else if let Ok(num) = keyword.parse::<i32>() {
                        computed.z_index = num;
                    }
                }
                _ => {}
            }
        }
        PropertyName::Opacity => {
            match &declaration.value {
                CssValue::Number(num) => {
                    // Clamp opacity between 0.0 and 1.0
                    computed.opacity = num.clamp(0.0, 1.0);
                }
                CssValue::Keyword(keyword) => {
                    if let Ok(num) = keyword.parse::<f32>() {
                        computed.opacity = num.clamp(0.0, 1.0);
                    }
                }
                _ => {}
            }
        }
        PropertyName::Transition => {
            // Parse transition property (e.g., "all 0.3s ease")
            if let CssValue::String(transition_str) = &declaration.value {
                computed.transition = crate::css::TransitionSpec::parse(transition_str);
            } else if let CssValue::Keyword(keyword) = &declaration.value {
                if keyword == "none" {
                    computed.transition = crate::css::TransitionSpec::default();
                }
            }
        }
        PropertyName::ListStyleType => {
            if let CssValue::Keyword(list_style) = &declaration.value {
                computed.list_style_type = crate::css::ListStyleType::parse(list_style);
            } else if let CssValue::String(list_style) = &declaration.value {
                computed.list_style_type = crate::css::ListStyleType::parse(list_style);
            }
        }
        PropertyName::Outline => {
            // Parse outline shorthand (e.g., "2px solid red")
            match &declaration.value {
                CssValue::String(outline_str) => {
                    computed.outline = crate::css::Outline::parse(outline_str);
                }
                CssValue::Keyword(keyword) => {
                    computed.outline = crate::css::Outline::parse(keyword);
                }
                _ => {}
            }
        }
        PropertyName::OutlineWidth => {
            if let CssValue::Length(length) = &declaration.value {
                computed.outline.width = length.clone();
            } else if let CssValue::Keyword(keyword) = &declaration.value {
                // Handle named width keywords
                match keyword.to_lowercase().as_str() {
                    "thin" => computed.outline.width = crate::css::values::Length::px(1.0),
                    "medium" => computed.outline.width = crate::css::values::Length::px(3.0),
                    "thick" => computed.outline.width = crate::css::values::Length::px(5.0),
                    _ => {}
                }
            }
        }
        PropertyName::OutlineStyle => {
            if let CssValue::Keyword(style) = &declaration.value {
                computed.outline.style = crate::css::OutlineStyle::parse(style);
            } else if let CssValue::String(style) = &declaration.value {
                computed.outline.style = crate::css::OutlineStyle::parse(style);
            }
        }
        PropertyName::OutlineColor => {
            if let CssValue::Color(color) = &declaration.value {
                computed.outline.color = color.clone();
            }
        }
        PropertyName::OutlineOffset => {
            if let CssValue::Length(length) = &declaration.value {
                computed.outline_offset = length.clone();
            }
        }
        PropertyName::FlexBasis => {
            match &declaration.value {
                CssValue::Length(length) => {
                    computed.flex_basis = crate::css::FlexBasis::Length(length.clone());
                }
                CssValue::Auto => {
                    computed.flex_basis = crate::css::FlexBasis::Auto;
                }
                CssValue::Keyword(keyword) => {
                    computed.flex_basis = crate::css::FlexBasis::parse(keyword);
                }
                CssValue::String(value) => {
                    computed.flex_basis = crate::css::FlexBasis::parse(value);
                }
                _ => {}
            }
        }
        PropertyName::Gap => {
            match &declaration.value {
                CssValue::Length(length) => {
                    // Single value applies to both row and column
                    computed.gap = crate::css::Gap::uniform(length.clone());
                }
                CssValue::MultipleValues(values) => {
                    // Parse as gap shorthand (row-gap column-gap)
                    if values.len() >= 2 {
                        let row = if let CssValue::Length(len) = &values[0] {
                            len.clone()
                        } else {
                            crate::css::Length::default()
                        };
                        let column = if let CssValue::Length(len) = &values[1] {
                            len.clone()
                        } else {
                            crate::css::Length::default()
                        };
                        computed.gap = crate::css::Gap { row, column };
                    } else if values.len() == 1 {
                        if let CssValue::Length(len) = &values[0] {
                            computed.gap = crate::css::Gap::uniform(len.clone());
                        }
                    }
                }
                CssValue::String(value) => {
                    computed.gap = crate::css::Gap::parse(value);
                }
                CssValue::Keyword(keyword) => {
                    computed.gap = crate::css::Gap::parse(keyword);
                }
                _ => {}
            }
        }
        _ => {
            // Handle other properties as needed
        }
    }
}

/// Apply margin property (handles shorthand syntax)
fn apply_margin_property(computed: &mut ComputedValues, value: &CssValue) {
    match value {
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
                        CssValue::Auto => 0.0,
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
                        CssValue::Auto => 0.0,
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
                        CssValue::Auto => 0.0,
                        _ => 0.0,
                    };
                    let bottom = match &values[2] {
                        CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                        CssValue::Auto => 0.0,
                        _ => 0.0,
                    };
                    let left = match &values[3] {
                        CssValue::Length(length) => length.to_px(computed.font_size, parent_size),
                        CssValue::Auto => 0.0,
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
