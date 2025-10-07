// CSS module for parsing and applying styles
mod parser;
mod values;
mod selector;
mod stylesheet;
pub(crate) mod computed;

pub use self::parser::CssParser;
pub use self::values::{CssValue, Color, Length, Unit, BorderRadius, BorderRadiusPx, BoxShadow, BoxShadowPx, TextDecoration, TextDecorationType, BackgroundImage, TextAlign, Clear, Overflow, FontStyle, FontVariant, LineHeight};
pub use self::selector::{Selector, SelectorType, PseudoClass};
pub use self::stylesheet::{Stylesheet, Rule, Declaration};
pub use self::computed::{ComputedValues, StyleResolver};

/// CSS property names
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum PropertyName {
    Color,
    Background,
    BackgroundColor,
    BackgroundImage,
    Width,
    Height,
    Margin,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Padding,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Border,
    BorderTop,
    BorderRight,
    BorderBottom,
    BorderLeft,
    BorderRadius,
    BorderTopLeftRadius,
    BorderTopRightRadius,
    BorderBottomLeftRadius,
    BorderBottomRightRadius,
    BoxShadow,
    Font,
    FontSize,
    FontFamily,
    FontWeight,
    FontStyle,
    FontVariant,
    LineHeight,
    TextDecoration,
    TextAlign,
    Clear,
    Overflow,
    Display,
    Position,
    Top,
    Right,
    Bottom,
    Left,
    Unknown(String),
}

impl From<&str> for PropertyName {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "color" => PropertyName::Color,
            "background" => PropertyName::Background,
            "background-color" => PropertyName::BackgroundColor,
            "background-image" => PropertyName::BackgroundImage,
            "width" => PropertyName::Width,
            "height" => PropertyName::Height,
            "margin" => PropertyName::Margin,
            "margin-top" => PropertyName::MarginTop,
            "margin-right" => PropertyName::MarginRight,
            "margin-bottom" => PropertyName::MarginBottom,
            "margin-left" => PropertyName::MarginLeft,
            "padding" => PropertyName::Padding,
            "padding-top" => PropertyName::PaddingTop,
            "padding-right" => PropertyName::PaddingRight,
            "padding-bottom" => PropertyName::PaddingBottom,
            "padding-left" => PropertyName::PaddingLeft,
            "border" => PropertyName::Border,
            "border-top" => PropertyName::BorderTop,
            "border-right" => PropertyName::BorderRight,
            "border-bottom" => PropertyName::BorderBottom,
            "border-left" => PropertyName::BorderLeft,
            "border-radius" => PropertyName::BorderRadius,
            "border-top-left-radius" => PropertyName::BorderTopLeftRadius,
            "border-top-right-radius" => PropertyName::BorderTopRightRadius,
            "border-bottom-left-radius" => PropertyName::BorderBottomLeftRadius,
            "border-bottom-right-radius" => PropertyName::BorderBottomRightRadius,
            "box-shadow" => PropertyName::BoxShadow,
            "font" => PropertyName::Font,
            "font-size" => PropertyName::FontSize,
            "font-family" => PropertyName::FontFamily,
            "font-weight" => PropertyName::FontWeight,
            "font-style" => PropertyName::FontStyle,
            "font-variant" => PropertyName::FontVariant,
            "line-height" => PropertyName::LineHeight,
            "text-decoration" => PropertyName::TextDecoration,
            "text-align" => PropertyName::TextAlign,
            "clear" => PropertyName::Clear,
            "overflow" => PropertyName::Overflow,
            "display" => PropertyName::Display,
            "position" => PropertyName::Position,
            "top" => PropertyName::Top,
            "right" => PropertyName::Right,
            "bottom" => PropertyName::Bottom,
            "left" => PropertyName::Left,
            _ => {
                println!("Warning: Unknown CSS property: {}", s);
                PropertyName::Unknown(s.to_string())
            },
        }
    }
}
