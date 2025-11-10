use super::font::FontManager;
use crate::css::ComputedValues;
use crate::dom::{AttributeMap, Dom, ElementData};
use crate::renderer::text::{TextPainter, ToColorColor};
// Pseudo-element rendering (::before, ::after)
use skia_safe::{Paint, Rect};
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::servo_arc::Arc;
use style::values::computed::counters::Content;
use style::values::generics::counters::GenericContentItem;
use parley::{Alignment, AlignmentOptions, FontWeight, GenericFamily, LineHeight, PositionedLayoutItem, StyleProperty};
use style::values::computed::font::{GenericFontFamily, SingleFontFamily};
use color::{AlphaColor, Srgb};
use kurbo::{Affine, Stroke};
use peniko::Fill;

/// Render pseudo-element generated content (::before or ::after)
pub fn render_pseudo_element_content(
    painter: &mut TextPainter,
    dom: &Dom,
    rect: &Rect,
    element_data: &ElementData,
    style: &Arc<StyloComputedValues>,
    scale_factor: f32,
    is_before: bool,
    scroll_transform: kurbo::Affine,
) {
    // Check if content property is set and not Normal/None
    let content = &style.get_counters().content;
    let content = content.to_display_string(Some(&element_data.attributes));

    // Skip if content is empty
    if content.is_empty() {
        return;
    }

    // Get font and layout contexts
    let mut font_ctx = dom.font_ctx.lock().unwrap();
    let mut layout_ctx = dom.layout_ctx.lock().unwrap();

    // Build layout for the content text
    let mut builder = layout_ctx.ranged_builder(
        &mut font_ctx,
        &content,
        scale_factor,
        true,
    );

    // Extract CSS properties
    let font = style.get_font();
    let font_size = font.font_size;
    let font_size = font_size.computed_size.px();
    let font_weight = &font.font_weight.value();

    // Set default font family
    let families = font.font_family.families.iter().map(|family| {
        match family {
            SingleFontFamily::FamilyName(a) => {
                let name = a.name.as_ref();
                name
            }
            SingleFontFamily::Generic(b) => {
                match b {
                    GenericFontFamily::Serif => "serif",
                    GenericFontFamily::SansSerif => "sans-serif",
                    GenericFontFamily::Monospace => "monospace",
                    GenericFontFamily::Cursive => "cursive",
                    GenericFontFamily::Fantasy => "fantasy",
                    GenericFontFamily::SystemUi => "system-ui",
                    GenericFontFamily::None => "sans-serif"
                }
            }
        }
    }).collect::<Vec<&str>>();
    let font_families: Vec<&str> = families
        .iter().map(|s| s.trim().trim_matches(|c| c == '"' || c == '\''))
        .collect();

    // Use first font family or default to system UI
    if let Some(first_family) = font_families.first() {
        let generic_family = match first_family.to_lowercase().as_str() {
            "serif" => GenericFamily::Serif,
            "sans-serif" => GenericFamily::SansSerif,
            "monospace" => GenericFamily::Monospace,
            "cursive" => GenericFamily::Cursive,
            "fantasy" => GenericFamily::Fantasy,
            "system-ui" | "-apple-system" | "blinkmacsystemfont" => GenericFamily::SystemUi,
            _ => GenericFamily::SystemUi,
        };
        builder.push_default(generic_family);
    } else {
        builder.push_default(GenericFamily::SystemUi);
    }

    // Set font size
    builder.push_default(StyleProperty::FontSize(font_size));

    // Set font weight
    let font_weight = FontWeight::new(*font_weight);
    builder.push_default(StyleProperty::FontWeight(font_weight));

    // Set line height
    let line_height_value = &font.line_height;
    let line_height_ratio = match line_height_value {
        style::values::computed::font::LineHeight::Normal => {
            1.2 // Typical default line height ratio
        }
        style::values::computed::font::LineHeight::Number(num) => {
            num.0
        }
        style::values::computed::font::LineHeight::Length(len) => {
            len.px()
        }
    };
    builder.push_default(LineHeight::FontSizeRelative(line_height_ratio));

    // Build the layout
    let mut layout = builder.build(&content);

    // Break lines (no width constraint for pseudo-elements)
    layout.break_all_lines(None);

    // Set text alignment (left for ::before, right for ::after)
    let alignment = if is_before { Alignment::Right } else { Alignment::Left };
    layout.align(None, alignment, AlignmentOptions::default());

    // Get text color
    let itext_style = style.get_inherited_text();
    let text_color: AlphaColor<Srgb> = itext_style.color.as_color_color();

    // Calculate position offset based on is_before
    let offset_x = if is_before {
        // Position at the left edge of the content area
        rect.left
    } else {
        // Position at the right edge of the content area
        rect.right
    };

    // Create transform for text position, combined with scroll transform
    let transform = scroll_transform * Affine::translate((offset_x as f64, rect.top as f64));

    // Render each line
    for line in layout.lines() {
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let mut run_x = glyph_run.offset();
                    let run_y = glyph_run.baseline();

                    let run = glyph_run.run();
                    let font = run.font();
                    let font_size = run.font_size();
                    let metrics = run.metrics();
                    let synthesis = run.synthesis();
                    let glyph_xform = synthesis.skew().map(|angle| {
                        Affine::skew(angle.to_radians().tan() as f64, 0.0)
                    });

                    painter.draw_glyphs(
                        font,
                        font_size,
                        true,
                        run.normalized_coords(),
                        Fill::NonZero,
                        &anyrender::Paint::from(text_color),
                        1.0,
                        transform,
                        glyph_xform,
                        glyph_run.glyphs().map(|glyph| {
                            let gx = run_x + glyph.x;
                            let gy = run_y - glyph.y;
                            run_x += glyph.advance;

                            anyrender::Glyph {
                                id: glyph.id as _,
                                x: gx,
                                y: gy,
                            }
                        })
                    );

                    // Handle text decorations if needed
                    let text_style = style.get_text();
                    let text_decoration_color = text_style
                        .text_decoration_color
                        .as_absolute()
                        .map(ToColorColor::as_color_color)
                        .unwrap_or(text_color);
                    let text_decoration_brush = anyrender::Paint::from(text_decoration_color);
                    let text_decoration_line = text_style.text_decoration_line;
                    let has_underline = text_decoration_line.contains(style::values::specified::TextDecorationLine::UNDERLINE);
                    let has_strikethrough =
                        text_decoration_line.contains(style::values::specified::TextDecorationLine::LINE_THROUGH);

                    let mut draw_decoration_line =
                        |offset: f32, size: f32, brush: &anyrender::Paint| {
                            let x = glyph_run.offset() as f64;
                            let w = glyph_run.advance() as f64;
                            let y = (glyph_run.baseline() - offset + size / 2.0) as f64;
                            let line = kurbo::Line::new((x, y), (x + w, y));
                            painter.stroke(&Stroke::new(size as f64), transform, brush, None, &line)
                        };

                    if has_underline {
                        let offset = metrics.underline_offset;
                        let size = metrics.underline_size;

                        draw_decoration_line(offset, size, &text_decoration_brush);
                    }
                    if has_strikethrough {
                        let offset = metrics.strikethrough_offset;
                        let size = metrics.strikethrough_size;

                        draw_decoration_line(offset, size, &text_decoration_brush);
                    }
                }
                PositionedLayoutItem::InlineBox(_) => {
                    // Inline boxes are not rendered in this context
                }
            }
        }
    }
}

trait ToAttrString {
    fn to_display_string(&self, attributes: Option<&AttributeMap>) -> String;
}

impl ToAttrString for Content {
    /// Convert content value to display string
    fn to_display_string(&self, element_attributes: Option<&AttributeMap>) -> String {
        match self {
            Content::None | Content::Normal => String::new(),
            Content::Items(items) => {
                let mut values: Vec<String> = Vec::new();

                for item in &items.items {
                    let str: String = match item {
                        GenericContentItem::String(s) => s.parse().unwrap(),
                        GenericContentItem::Counter(_id, _style_type) => {
                            // TODO
                            String::new()
                        }
                        GenericContentItem::Counters(_id, _str, _style_type) => {
                            // TODO
                            String::new()
                        }
                        GenericContentItem::OpenQuote => "\"".to_string(),
                        GenericContentItem::CloseQuote => "\"".to_string(),
                        GenericContentItem::NoOpenQuote | GenericContentItem::NoCloseQuote => String::new(),
                        GenericContentItem::Attr(attr) => {
                            if let Some(attrs) = element_attributes {
                                attrs.iter().find(|a| &a.name.local == &attr.attribute.to_string()).map(|a| a.value.clone()).unwrap_or_default()
                            } else {
                                String::new()
                            }
                        }
                        GenericContentItem::Image(_image) => {
                            // Image content is not handled in text representation
                            String::new()
                        }
                    };
                    values.push(str);
                }

                values.join("")
            }
        }
    }
}
