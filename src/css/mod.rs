// CSS module for parsing and applying styles
mod parser;
mod values;
mod selector;
mod stylesheet;
mod computed;

pub use self::parser::CssParser;
pub use self::values::{CssValue, Color, Length, Unit};
pub use self::selector::{Selector, SelectorType};
pub use self::stylesheet::{Stylesheet, Rule, Declaration};
pub use self::computed::{ComputedValues, StyleResolver};

/// CSS property names
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum PropertyName {
    Color,
    BackgroundColor,
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
    FontSize,
    FontFamily,
    FontWeight,
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
            "background-color" => PropertyName::BackgroundColor,
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
            "font-size" => PropertyName::FontSize,
            "font-family" => PropertyName::FontFamily,
            "font-weight" => PropertyName::FontWeight,
            "display" => PropertyName::Display,
            "position" => PropertyName::Position,
            "top" => PropertyName::Top,
            "right" => PropertyName::Right,
            "bottom" => PropertyName::Bottom,
            "left" => PropertyName::Left,
            _ => PropertyName::Unknown(s.to_string()),
        }
    }
}
