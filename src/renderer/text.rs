use crate::renderer::gradient::to_peniko_gradient;
use crate::renderer::painter::ScenePainter;
use crate::dom::Dom;
use crate::ui::TextBrush;
use anyrender::{Paint, PaintScene};
use kurbo::{Affine, Rect, Stroke};
use parley::{Affinity, Cursor, Layout, Line, PositionedLayoutItem, Selection};
use peniko::{Color, Fill};
use std::collections::HashMap;
use style::values::generics::image::GenericImage;
use style::values::specified::TextDecorationLine;
use crate::renderer::painter::ToColorColor;

pub fn stroke_text<'a>(
    painter: &mut ScenePainter,
    lines: impl Iterator<Item = Line<'a, TextBrush>>,
    dom: &Dom,
    transform: Affine,
    scale_factor: f64,
) {
    let lines: Vec<_> = lines.collect();
    let mut inline_gradient_bounds: HashMap<usize, Rect> = HashMap::new();

    for line in &lines {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };

            let node_id = glyph_run.style().brush.id;
            let Some(styles) = dom
                .get_node(node_id)
                .and_then(|node| node.primary_styles())
            else {
                continue;
            };

            let has_gradient = styles
                .get_background()
                .background_image
                .0
                .iter()
                .any(|image| matches!(image, GenericImage::Gradient(_)));
            if !has_gradient {
                continue;
            }

            let metrics = glyph_run.run().metrics();
            let run_rect = Rect::new(
                glyph_run.offset() as f64,
                (glyph_run.baseline() - metrics.ascent) as f64,
                (glyph_run.offset() + glyph_run.advance()) as f64,
                (glyph_run.baseline() + metrics.descent) as f64,
            );

            inline_gradient_bounds
                .entry(node_id)
                .and_modify(|bounds| *bounds = bounds.union(run_rect))
                .or_insert(run_rect);
        }
    }

    for line in lines {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let font = run.font();
                let font_size = run.font_size();
                let metrics = run.metrics();
                let style = glyph_run.style();
                let synthesis = run.synthesis();
                let glyph_xform = synthesis
                    .skew()
                    .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));

                // Styles
                let styles = dom
                    .get_node(style.brush.id)
                    .unwrap()
                    .primary_styles()
                    .unwrap();
                let itext_styles = styles.get_inherited_text();
                let text_styles = styles.get_text();
                let text_color = itext_styles.color.as_color_color();
                let text_decoration_color = text_styles
                    .text_decoration_color
                    .as_absolute()
                    .map(ToColorColor::as_color_color)
                    .unwrap_or(text_color);
                let text_decoration_brush = anyrender::Paint::from(text_decoration_color);
                let text_decoration_line = text_styles.text_decoration_line;
                let has_underline = text_decoration_line.contains(TextDecorationLine::UNDERLINE);
                let has_strikethrough =
                    text_decoration_line.contains(TextDecorationLine::LINE_THROUGH);

                let gradient_bounds = inline_gradient_bounds.get(&style.brush.id).copied();
                let mut painted_gradient_glyphs = false;

                if let Some(bounds) = gradient_bounds {
                    let current_color = styles.clone_color();

                    for bg_image in styles.get_background().background_image.0.iter().rev() {
                        let GenericImage::Gradient(gradient) = bg_image else {
                            continue;
                        };

                        let (peniko_gradient, gradient_transform) = to_peniko_gradient(
                            gradient,
                            bounds,
                            bounds,
                            scale_factor,
                            &current_color,
                        );

                        painter.draw_glyphs_with_brush_transform(
                            font,
                            font_size,
                            true, // hint
                            run.normalized_coords(),
                            Fill::NonZero,
                            Paint::Gradient(&peniko_gradient),
                            gradient_transform,
                            1.0, // alpha
                            transform,
                            glyph_xform,
                            glyph_run.positioned_glyphs().map(|glyph| anyrender::Glyph {
                                id: glyph.id as _,
                                x: glyph.x,
                                y: glyph.y,
                            }),
                        );

                        painted_gradient_glyphs = true;
                    }
                }

                if !painted_gradient_glyphs {
                    painter.draw_glyphs(
                        font,
                        font_size,
                        true, // hint
                        run.normalized_coords(),
                        Fill::NonZero,
                        &anyrender::Paint::from(text_color),
                        1.0, // alpha
                        transform,
                        glyph_xform,
                        glyph_run.positioned_glyphs().map(|glyph| anyrender::Glyph {
                            id: glyph.id as _,
                            x: glyph.x,
                            y: glyph.y,
                        }),
                    );
                }

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
        }
    }
}

pub const SELECTION_COLOR: Color = Color::from_rgb8(180, 213, 255);

pub(crate) fn draw_text_selection(
    scene: &mut impl PaintScene,
    layout: &Layout<TextBrush>,
    transform: Affine,
    selection_start: usize,
    selection_end: usize,
) {
    let anchor = Cursor::from_byte_index(layout, selection_start, Affinity::Downstream);
    let focus = Cursor::from_byte_index(layout, selection_end, Affinity::Downstream);
    let selection = Selection::new(anchor, focus);

    selection.geometry_with(layout, |rect, _line_idx| {
        let rect = kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1);
        scene.fill(Fill::NonZero, transform, SELECTION_COLOR, None, &rect);
    });
}