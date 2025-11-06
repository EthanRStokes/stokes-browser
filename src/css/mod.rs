// CSS module for parsing and applying styles
mod parser;
mod values;
mod selector;
mod stylesheet;
pub(crate) mod computed;
pub mod transition_manager;
pub(crate) mod stylo;
mod parse;

pub use self::computed::{ComputedValues, StyleResolver};
pub use self::parser::CssParser;
pub use self::selector::Selector;
pub use self::stylesheet::{Declaration, Rule, Stylesheet};
pub use values::{BackgroundImage, BorderRadius, BorderRadiusPx, BoxShadow, BoxSizing, Clear, Color, ContentValue, CssValue, Cursor, Flex, FlexBasis, FlexGrow, FlexShrink, Float, FontStyle, FontVariant, Gap, Length, LineHeight, ListStyleType, Outline, OutlineStyle, Overflow, Stroke, TextAlign, TextDecoration, TextDecorationType, TextShadow, TextTransform, TimingFunction, TransitionSpec, VerticalAlign, Visibility, WhiteSpace};

/// CSS property names
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum PropertyName {
    Color,
    Background,
    BackgroundColor,
    BackgroundImage,
    Width,
    Height,
    MaxWidth,
    MinWidth,
    MaxHeight,
    MinHeight,
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
    BoxSizing,
    Font,
    FontSize,
    FontFamily,
    FontWeight,
    FontStyle,
    FontVariant,
    LineHeight,
    TextDecoration,
    TextAlign,
    TextTransform,
    TextShadow,
    WhiteSpace,
    VerticalAlign,
    Content,
    Clear,
    Float,
    Overflow,
    OverflowX,
    OverflowY,
    Display,
    Position,
    Top,
    Right,
    Bottom,
    Left,
    Cursor,
    ZIndex,
    Visibility,
    Opacity,
    Transition,
    TransitionProperty,
    TransitionDuration,
    TransitionTimingFunction,
    TransitionDelay,
    ListStyleType,
    Outline,
    OutlineWidth,
    OutlineStyle,
    OutlineColor,
    OutlineOffset,
    Flex,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Gap,
    Stroke,
    StrokeWidth,
    StrokeOpacity,
    Unknown(String),
}

impl From<&str> for PropertyName {
    fn from(s: &str) -> Self {
        // Fast path: check if already lowercase (common case)
        let bytes = s.as_bytes();
        let is_lowercase = bytes.iter().all(|&b| !b.is_ascii_uppercase());

        if is_lowercase {
            // Direct match without allocation
            match s {
                "color" => return PropertyName::Color,
                "background" => return PropertyName::Background,
                "background-color" => return PropertyName::BackgroundColor,
                "background-image" => return PropertyName::BackgroundImage,
                "width" => return PropertyName::Width,
                "height" => return PropertyName::Height,
                "max-width" => return PropertyName::MaxWidth,
                "min-width" => return PropertyName::MinWidth,
                "max-height" => return PropertyName::MaxHeight,
                "min-height" => return PropertyName::MinHeight,
                "margin" => return PropertyName::Margin,
                "margin-top" => return PropertyName::MarginTop,
                "margin-right" => return PropertyName::MarginRight,
                "margin-bottom" => return PropertyName::MarginBottom,
                "margin-left" => return PropertyName::MarginLeft,
                "padding" => return PropertyName::Padding,
                "padding-top" => return PropertyName::PaddingTop,
                "padding-right" => return PropertyName::PaddingRight,
                "padding-bottom" => return PropertyName::PaddingBottom,
                "padding-left" => return PropertyName::PaddingLeft,
                "border" => return PropertyName::Border,
                "border-top" => return PropertyName::BorderTop,
                "border-right" => return PropertyName::BorderRight,
                "border-bottom" => return PropertyName::BorderBottom,
                "border-left" => return PropertyName::BorderLeft,
                "border-radius" => return PropertyName::BorderRadius,
                "border-top-left-radius" => return PropertyName::BorderTopLeftRadius,
                "border-top-right-radius" => return PropertyName::BorderTopRightRadius,
                "border-bottom-left-radius" => return PropertyName::BorderBottomLeftRadius,
                "border-bottom-right-radius" => return PropertyName::BorderBottomRightRadius,
                "box-shadow" => return PropertyName::BoxShadow,
                "box-sizing" => return PropertyName::BoxSizing,
                "font" => return PropertyName::Font,
                "font-size" => return PropertyName::FontSize,
                "font-family" => return PropertyName::FontFamily,
                "font-weight" => return PropertyName::FontWeight,
                "font-style" => return PropertyName::FontStyle,
                "font-variant" => return PropertyName::FontVariant,
                "line-height" => return PropertyName::LineHeight,
                "text-decoration" => return PropertyName::TextDecoration,
                "text-align" => return PropertyName::TextAlign,
                "text-transform" => return PropertyName::TextTransform,
                "text-shadow" => return PropertyName::TextShadow,
                "white-space" => return PropertyName::WhiteSpace,
                "vertical-align" => return PropertyName::VerticalAlign,
                "content" => return PropertyName::Content,
                "clear" => return PropertyName::Clear,
                "float" => return PropertyName::Float,
                "overflow" => return PropertyName::Overflow,
                "overflow-x" => return PropertyName::OverflowX,
                "overflow-y" => return PropertyName::OverflowY,
                "display" => return PropertyName::Display,
                "position" => return PropertyName::Position,
                "top" => return PropertyName::Top,
                "right" => return PropertyName::Right,
                "bottom" => return PropertyName::Bottom,
                "left" => return PropertyName::Left,
                "cursor" => return PropertyName::Cursor,
                "z-index" => return PropertyName::ZIndex,
                "visibility" => return PropertyName::Visibility,
                "opacity" => return PropertyName::Opacity,
                "transition" => return PropertyName::Transition,
                "transition-property" => return PropertyName::TransitionProperty,
                "transition-duration" => return PropertyName::TransitionDuration,
                "transition-timing-function" => return PropertyName::TransitionTimingFunction,
                "transition-delay" => return PropertyName::TransitionDelay,
                "list-style-type" => return PropertyName::ListStyleType,
                "outline" => return PropertyName::Outline,
                "outline-width" => return PropertyName::OutlineWidth,
                "outline-style" => return PropertyName::OutlineStyle,
                "outline-color" => return PropertyName::OutlineColor,
                "outline-offset" => return PropertyName::OutlineOffset,
                "flex" => return PropertyName::Flex,
                "flex-grow" => return PropertyName::FlexGrow,
                "flex-shrink" => return PropertyName::FlexShrink,
                "flex-basis" => return PropertyName::FlexBasis,
                "gap" => return PropertyName::Gap,
                "stroke" => return PropertyName::Stroke,
                "stroke-width" => return PropertyName::StrokeWidth,
                "stroke-opacity" => return PropertyName::StrokeOpacity,
                _ => {
                    println!("Warning: Unknown CSS property: {}", s);
                    return PropertyName::Unknown(s.to_string());
                }
            }
        }

        // Slow path: has uppercase, need to compare case-insensitively
        // Use eq_ignore_ascii_case to avoid allocation
        if s.eq_ignore_ascii_case("color") { return PropertyName::Color; }
        if s.eq_ignore_ascii_case("background") { return PropertyName::Background; }
        if s.eq_ignore_ascii_case("background-color") { return PropertyName::BackgroundColor; }
        if s.eq_ignore_ascii_case("background-image") { return PropertyName::BackgroundImage; }
        if s.eq_ignore_ascii_case("width") { return PropertyName::Width; }
        if s.eq_ignore_ascii_case("height") { return PropertyName::Height; }
        if s.eq_ignore_ascii_case("max-width") { return PropertyName::MaxWidth; }
        if s.eq_ignore_ascii_case("min-width") { return PropertyName::MinWidth; }
        if s.eq_ignore_ascii_case("max-height") { return PropertyName::MaxHeight; }
        if s.eq_ignore_ascii_case("min-height") { return PropertyName::MinHeight; }
        if s.eq_ignore_ascii_case("margin") { return PropertyName::Margin; }
        if s.eq_ignore_ascii_case("margin-top") { return PropertyName::MarginTop; }
        if s.eq_ignore_ascii_case("margin-right") { return PropertyName::MarginRight; }
        if s.eq_ignore_ascii_case("margin-bottom") { return PropertyName::MarginBottom; }
        if s.eq_ignore_ascii_case("margin-left") { return PropertyName::MarginLeft; }
        if s.eq_ignore_ascii_case("padding") { return PropertyName::Padding; }
        if s.eq_ignore_ascii_case("padding-top") { return PropertyName::PaddingTop; }
        if s.eq_ignore_ascii_case("padding-right") { return PropertyName::PaddingRight; }
        if s.eq_ignore_ascii_case("padding-bottom") { return PropertyName::PaddingBottom; }
        if s.eq_ignore_ascii_case("padding-left") { return PropertyName::PaddingLeft; }
        if s.eq_ignore_ascii_case("border") { return PropertyName::Border; }
        if s.eq_ignore_ascii_case("border-top") { return PropertyName::BorderTop; }
        if s.eq_ignore_ascii_case("border-right") { return PropertyName::BorderRight; }
        if s.eq_ignore_ascii_case("border-bottom") { return PropertyName::BorderBottom; }
        if s.eq_ignore_ascii_case("border-left") { return PropertyName::BorderLeft; }
        if s.eq_ignore_ascii_case("border-radius") { return PropertyName::BorderRadius; }
        if s.eq_ignore_ascii_case("border-top-left-radius") { return PropertyName::BorderTopLeftRadius; }
        if s.eq_ignore_ascii_case("border-top-right-radius") { return PropertyName::BorderTopRightRadius; }
        if s.eq_ignore_ascii_case("border-bottom-left-radius") { return PropertyName::BorderBottomLeftRadius; }
        if s.eq_ignore_ascii_case("border-bottom-right-radius") { return PropertyName::BorderBottomRightRadius; }
        if s.eq_ignore_ascii_case("box-shadow") { return PropertyName::BoxShadow; }
        if s.eq_ignore_ascii_case("box-sizing") { return PropertyName::BoxSizing; }
        if s.eq_ignore_ascii_case("font") { return PropertyName::Font; }
        if s.eq_ignore_ascii_case("font-size") { return PropertyName::FontSize; }
        if s.eq_ignore_ascii_case("font-family") { return PropertyName::FontFamily; }
        if s.eq_ignore_ascii_case("font-weight") { return PropertyName::FontWeight; }
        if s.eq_ignore_ascii_case("font-style") { return PropertyName::FontStyle; }
        if s.eq_ignore_ascii_case("font-variant") { return PropertyName::FontVariant; }
        if s.eq_ignore_ascii_case("line-height") { return PropertyName::LineHeight; }
        if s.eq_ignore_ascii_case("text-decoration") { return PropertyName::TextDecoration; }
        if s.eq_ignore_ascii_case("text-align") { return PropertyName::TextAlign; }
        if s.eq_ignore_ascii_case("text-transform") { return PropertyName::TextTransform; }
        if s.eq_ignore_ascii_case("text-shadow") { return PropertyName::TextShadow; }
        if s.eq_ignore_ascii_case("white-space") { return PropertyName::WhiteSpace; }
        if s.eq_ignore_ascii_case("vertical-align") { return PropertyName::VerticalAlign; }
        if s.eq_ignore_ascii_case("content") { return PropertyName::Content; }
        if s.eq_ignore_ascii_case("clear") { return PropertyName::Clear; }
        if s.eq_ignore_ascii_case("float") { return PropertyName::Float; }
        if s.eq_ignore_ascii_case("overflow") { return PropertyName::Overflow; }
        if s.eq_ignore_ascii_case("overflow-x") { return PropertyName::OverflowX; }
        if s.eq_ignore_ascii_case("overflow-y") { return PropertyName::OverflowY; }
        if s.eq_ignore_ascii_case("display") { return PropertyName::Display; }
        if s.eq_ignore_ascii_case("position") { return PropertyName::Position; }
        if s.eq_ignore_ascii_case("top") { return PropertyName::Top; }
        if s.eq_ignore_ascii_case("right") { return PropertyName::Right; }
        if s.eq_ignore_ascii_case("bottom") { return PropertyName::Bottom; }
        if s.eq_ignore_ascii_case("left") { return PropertyName::Left; }
        if s.eq_ignore_ascii_case("cursor") { return PropertyName::Cursor; }
        if s.eq_ignore_ascii_case("z-index") { return PropertyName::ZIndex; }
        if s.eq_ignore_ascii_case("visibility") { return PropertyName::Visibility; }
        if s.eq_ignore_ascii_case("opacity") { return PropertyName::Opacity; }
        if s.eq_ignore_ascii_case("transition") { return PropertyName::Transition; }
        if s.eq_ignore_ascii_case("transition-property") { return PropertyName::TransitionProperty; }
        if s.eq_ignore_ascii_case("transition-duration") { return PropertyName::TransitionDuration; }
        if s.eq_ignore_ascii_case("transition-timing-function") { return PropertyName::TransitionTimingFunction; }
        if s.eq_ignore_ascii_case("transition-delay") { return PropertyName::TransitionDelay; }
        if s.eq_ignore_ascii_case("list-style-type") { return PropertyName::ListStyleType; }
        if s.eq_ignore_ascii_case("outline") { return PropertyName::Outline; }
        if s.eq_ignore_ascii_case("outline-width") { return PropertyName::OutlineWidth; }
        if s.eq_ignore_ascii_case("outline-style") { return PropertyName::OutlineStyle; }
        if s.eq_ignore_ascii_case("outline-color") { return PropertyName::OutlineColor; }
        if s.eq_ignore_ascii_case("outline-offset") { return PropertyName::OutlineOffset; }
        if s.eq_ignore_ascii_case("flex") { return PropertyName::Flex; }
        if s.eq_ignore_ascii_case("flex-grow") { return PropertyName::FlexGrow; }
        if s.eq_ignore_ascii_case("flex-shrink") { return PropertyName::FlexShrink; }
        if s.eq_ignore_ascii_case("flex-basis") { return PropertyName::FlexBasis; }
        if s.eq_ignore_ascii_case("gap") { return PropertyName::Gap; }
        if s.eq_ignore_ascii_case("stroke") { return PropertyName::Stroke; }
        if s.eq_ignore_ascii_case("stroke-width") { return PropertyName::StrokeWidth; }
        if s.eq_ignore_ascii_case("stroke-opacity") { return PropertyName::StrokeOpacity; }

        println!("Warning: Unknown CSS property: {}", s);
        PropertyName::Unknown(s.to_string())
    }
}