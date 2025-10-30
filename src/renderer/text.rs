use std::cell::RefCell;
use html5ever::tendril::StrTendril;
use super::font::FontManager;
use crate::css::ComputedValues;
use crate::layout::LayoutBox;
// Text rendering functionality
use skia_safe::{Canvas, Font, Paint};
use skia_safe::textlayout::{
    FontCollection, ParagraphBuilder, ParagraphStyle, TextAlign as SkiaTextAlign,
    TextStyle,
};
use crate::dom::DomNode;

/// Render text with CSS styles applied and DPI scale factor using Skia's textlayout
pub fn render_text_node(
    canvas: &Canvas,
    _node: &DomNode,
    layout_box: &LayoutBox,
    contents: &RefCell<StrTendril>,
    styles: &ComputedValues,
    font_manager: &FontManager,
    _default_text_paint: &Paint,
    scale_factor: f32,
) {
    let text = contents.borrow();
    let content_rect = layout_box.dimensions.content;

    // Apply text transformation to the content
    let transformed_text = styles.text_transform.apply(&text);

    // Apply DPI scaling to font size
    let scaled_font_size = styles.font_size * scale_factor;

    // Calculate line height based on CSS line-height property
    let line_height = styles.line_height.to_px(scaled_font_size);

    // Set up font collection
    let mut font_collection = FontCollection::new();
    font_collection.set_default_font_manager(font_manager.font_mgr.clone(), None);

    // Create paragraph style with text alignment
    let mut paragraph_style = ParagraphStyle::new();

    // Map CSS text-align to Skia TextAlign
    let skia_text_align = match styles.text_align {
        crate::css::TextAlign::Left => SkiaTextAlign::Left,
        crate::css::TextAlign::Right => SkiaTextAlign::Right,
        crate::css::TextAlign::Center => SkiaTextAlign::Center,
        crate::css::TextAlign::Justify => SkiaTextAlign::Justify,
    };
    paragraph_style.set_text_align(skia_text_align);

    // Set line height
    paragraph_style.set_height(line_height / scaled_font_size);

    // Create text style with font properties
    let mut text_style = TextStyle::new();
    text_style.set_font_size(scaled_font_size);

    // Set font families
    let font_families: Vec<String> = styles.font_family
        .split(',')
        .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .collect();
    text_style.set_font_families(&font_families);

    // Set font weight
    let font_weight = super::font::parse_font_weight(&styles.font_weight);
    text_style.set_font_style(skia_safe::FontStyle::new(
        skia_safe::font_style::Weight::from(font_weight),
        skia_safe::font_style::Width::NORMAL,
        match styles.font_style {
            crate::css::FontStyle::Normal => skia_safe::font_style::Slant::Upright,
            crate::css::FontStyle::Italic => skia_safe::font_style::Slant::Italic,
            crate::css::FontStyle::Oblique => skia_safe::font_style::Slant::Oblique,
        },
    ));

    // Set text color with opacity
    if let Some(text_color) = &styles.color {
        let mut color = text_color.to_skia_color();
        color = color.with_a((color.a() as f32 * styles.opacity) as u8);
        text_style.set_color(color);
    }

    // Add text shadows
    if !styles.text_shadow.is_empty() {
        let mut shadows = Vec::new();
        for shadow in &styles.text_shadow {
            let shadow_px = shadow.to_px(scaled_font_size, 0.0);
            if shadow_px.has_shadow() {
                let mut shadow_color = shadow_px.color.to_skia_color();
                shadow_color = shadow_color.with_a((shadow_color.a() as f32 * styles.opacity) as u8);

                let text_shadow = skia_safe::textlayout::TextShadow::new(
                    shadow_color,
                    (shadow_px.offset_x * scale_factor, shadow_px.offset_y * scale_factor),
                    (shadow_px.blur_radius / 2.0) as f64,
                );
                shadows.push(text_shadow);
            }
        }
        if !shadows.is_empty() {
            text_style.add_shadow(shadows[0]);
            for shadow in &shadows[1..] {
                text_style.add_shadow(*shadow);
            }
        }
    }

    // Add text decorations
    match &styles.text_decoration {
        crate::css::TextDecoration::None => {},
        crate::css::TextDecoration::Underline => {
            let color = if let Some(text_color) = &styles.color {
                text_color.to_skia_color()
            } else {
                skia_safe::Color::BLACK
            };
            text_style.set_decoration(&skia_safe::textlayout::Decoration {
                ty: skia_safe::textlayout::TextDecoration::UNDERLINE,
                mode: skia_safe::textlayout::TextDecorationMode::Gaps,
                color,
                style: skia_safe::textlayout::TextDecorationStyle::Solid,
                thickness_multiplier: 1.0,
            });
        },
        crate::css::TextDecoration::Overline => {
            let color = if let Some(text_color) = &styles.color {
                text_color.to_skia_color()
            } else {
                skia_safe::Color::BLACK
            };
            text_style.set_decoration(&skia_safe::textlayout::Decoration {
                ty: skia_safe::textlayout::TextDecoration::OVERLINE,
                mode: skia_safe::textlayout::TextDecorationMode::Gaps,
                color,
                style: skia_safe::textlayout::TextDecorationStyle::Solid,
                thickness_multiplier: 1.0,
            });
        },
        crate::css::TextDecoration::LineThrough => {
            let color = if let Some(text_color) = &styles.color {
                text_color.to_skia_color()
            } else {
                skia_safe::Color::BLACK
            };
            text_style.set_decoration(&skia_safe::textlayout::Decoration {
                ty: skia_safe::textlayout::TextDecoration::LINE_THROUGH,
                mode: skia_safe::textlayout::TextDecorationMode::Gaps,
                color,
                style: skia_safe::textlayout::TextDecorationStyle::Solid,
                thickness_multiplier: 1.0,
            });
        },
        crate::css::TextDecoration::Multiple(decorations) => {
            let mut decoration_flags = skia_safe::textlayout::TextDecoration::NO_DECORATION;
            for dec in decorations {
                match dec {
                    crate::css::TextDecorationType::Underline => {
                        decoration_flags |= skia_safe::textlayout::TextDecoration::UNDERLINE;
                    }
                    crate::css::TextDecorationType::Overline => {
                        decoration_flags |= skia_safe::textlayout::TextDecoration::OVERLINE;
                    }
                    crate::css::TextDecorationType::LineThrough => {
                        decoration_flags |= skia_safe::textlayout::TextDecoration::LINE_THROUGH;
                    }
                }
            }
            let color = if let Some(text_color) = &styles.color {
                text_color.to_skia_color()
            } else {
                skia_safe::Color::BLACK
            };
            text_style.set_decoration(&skia_safe::textlayout::Decoration {
                ty: decoration_flags,
                mode: skia_safe::textlayout::TextDecorationMode::Gaps,
                color,
                style: skia_safe::textlayout::TextDecorationStyle::Solid,
                thickness_multiplier: 1.0,
            });
        }
    }

    paragraph_style.set_text_style(&text_style);

    // Build the paragraph
    let mut paragraph_builder = ParagraphBuilder::new(&paragraph_style, font_collection);
    paragraph_builder.push_style(&text_style);
    paragraph_builder.add_text(&transformed_text);

    let mut paragraph = paragraph_builder.build();

    // Layout the paragraph with the available width
    let available_width = content_rect.width().max(0.0);
    paragraph.layout(available_width);

    // Calculate vertical alignment offset
    let vertical_align_offset = styles.vertical_align.to_px(scaled_font_size, line_height) * scale_factor;

    // Position and render the paragraph
    let x = content_rect.left;
    let y = content_rect.top + vertical_align_offset;

    paragraph.paint(canvas, (x, y));
}

/// Wrap text based on actual font metrics and available width using Skia's Paragraph API
pub fn wrap_text_with_font(text: &str, font: &Font, max_width: f32, white_space: &crate::css::WhiteSpace) -> Vec<String> {
    // Handle special white-space modes
    if !white_space.should_wrap() {
        if white_space.preserve_whitespace() {
            // For pre/pre-wrap modes, preserve all whitespace including newlines
            return text.lines().map(|s| s.to_string()).collect();
        } else {
            // For nowrap, collapse whitespace but don't wrap
            let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
            return vec![collapsed];
        }
    }

    // Use Paragraph API for proper text wrapping
    let mut font_collection = FontCollection::new();
    font_collection.set_default_font_manager(skia_safe::FontMgr::new(), None);

    let mut paragraph_style = ParagraphStyle::new();
    paragraph_style.set_text_align(SkiaTextAlign::Left);

    let mut text_style = TextStyle::new();
    text_style.set_font_size(font.size());

    // Get font families from the font's typeface
    let typeface = font.typeface();
        let family_name = typeface.family_name();
    text_style.set_font_families(&[family_name]);

    paragraph_style.set_text_style(&text_style);

    let mut paragraph_builder = ParagraphBuilder::new(&paragraph_style, font_collection);
    paragraph_builder.push_style(&text_style);
    paragraph_builder.add_text(text);

    let mut paragraph = paragraph_builder.build();
    paragraph.layout(max_width.max(1.0));

    // Extract lines from the laid out paragraph
    let mut wrapped_lines = Vec::new();
    let line_count = paragraph.line_number() as usize;

    for line_idx in 0..line_count {
        // Get the text range for this line
        if let Some(line_metrics) = paragraph.get_line_metrics_at(line_idx) {
            let start = line_metrics.start_index;
            let end = line_metrics.end_index;

            if start < text.len() && end <= text.len() {
                let line_text = &text[start..end];
                wrapped_lines.push(line_text.to_string());
            }
        }
    }

    // Return at least one empty line if everything was empty
    if wrapped_lines.is_empty() {
        wrapped_lines.push(String::new());
    }

    wrapped_lines
}
