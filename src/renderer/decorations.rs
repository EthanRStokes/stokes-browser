use crate::renderer::text::{TextPainter, ToColorColor};
use color::{AlphaColor, Srgb};
use kurbo::BezPath;
// Text decorations, borders, shadows, and outlines
use skia_safe::Rect;
use style::computed_values::box_shadow::ComputedList;
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::properties::style_structs::Border;
use style::servo_arc::Arc;
use style::values::computed::{Au, BorderStyle, OutlineStyle};

/// Render an element with rounded corners
pub fn render_rounded_element(
    painter: &mut TextPainter,
    rect: Rect,
    border: &Border,
    bg_color: AlphaColor<Srgb>,
    border_color: Option<AlphaColor<Srgb>>,
    scale_factor: f32,
    scroll_transform: kurbo::Affine,
) {
    // Apply scale factor to border radius values
    /*let scaled_top_left = border.border_top_left_radius.0.width.0.to_pixel_length(Au(rect.width() as i32)) * scale_factor;
    let scaled_top_right = border_radius_px.top_right * scale_factor;
    let scaled_bottom_right = border_radius_px.bottom_right * scale_factor;
    let scaled_bottom_left = border_radius_px.bottom_left * scale_factor;

    // For now, use uniform radius (average of all corners) for simplicity
    let avg_radius = (scaled_top_left + scaled_top_right + scaled_bottom_right + scaled_bottom_left) / 4.0;

    // Create kurbo rounded rectangle
    let kurbo_rect = kurbo::Rect::new(
        rect.left as f64,
        rect.top as f64,
        rect.right as f64,
        rect.bottom as f64,
    );
    let rounded_rect = kurbo::RoundedRect::from_rect(kurbo_rect, avg_radius as f64);

    // Draw the background with rounded corners
    painter.fill(peniko::Fill::NonZero, scroll_transform, bg_color, None, &rounded_rect);

    // Draw border if specified
    if let Some(border_color) = border_color {
        let stroke = kurbo::Stroke::new(1.0 * scale_factor as f64);
        painter.stroke(&stroke, scroll_transform, border_color, None, &rounded_rect);
    }*/
}

#[derive(Debug, Clone, Copy)]
pub enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

/// Render box shadows for an element
pub fn render_box_shadows(
    painter: &mut TextPainter,
    rect: &Rect,
    style: &Arc<StyloComputedValues>,
    scale_factor: f32,
    scroll_transform: kurbo::Affine,
) {
    // Render each box shadow
    let effects = style.get_effects();
    let box_shadow: &ComputedList = &effects.box_shadow;
    for shadow in box_shadow.0.iter() {
        let spread_px = shadow.spread.px();

        // Skip if no visible shadow
        if spread_px <= 0.0 {
            continue;
        }

        // Apply scale factor to shadow properties
        let scaled_offset_x = shadow.base.horizontal.px() * scale_factor;
        let scaled_offset_y = shadow.base.vertical.px() * scale_factor;
        let scaled_blur_radius = shadow.base.blur.px() * scale_factor;
        let scaled_spread_radius = spread_px * scale_factor;

        // Create shadow paint
        let color = shadow.base.color.as_absolute().unwrap().as_color_color();

        // Calculate shadow rectangle
        let shadow_rect = if shadow.inset {
            // Inset shadows are drawn inside the element
            Rect::from_xywh(
                rect.left + scaled_offset_x + scaled_spread_radius,
                rect.top + scaled_offset_y + scaled_spread_radius,
                rect.width() - 2.0 * scaled_spread_radius,
                rect.height() - 2.0 * scaled_spread_radius,
            )
        } else {
            // Outset shadows are drawn outside the element
            Rect::from_xywh(
                rect.left + scaled_offset_x - scaled_spread_radius,
                rect.top + scaled_offset_y - scaled_spread_radius,
                rect.width() + 2.0 * scaled_spread_radius,
                rect.height() + 2.0 * scaled_spread_radius,
            )
        };

        // For now, render a simple shadow without proper blur
        // In a full implementation, you'd use image filters for blur
        if scaled_blur_radius > 0.0 {
            // Simulate blur by drawing multiple offset rectangles with reduced opacity
            let blur_steps = (scaled_blur_radius / 2.0).max(1.0) as i32;
            let step_alpha = 1.0 / blur_steps as f32;

            for i in 0..blur_steps {
                let blur_offset = i as f32 * 2.0;
                let blur_rect = Rect::from_xywh(
                    shadow_rect.left - blur_offset,
                    shadow_rect.top - blur_offset,
                    shadow_rect.width() + 2.0 * blur_offset,
                    shadow_rect.height() + 2.0 * blur_offset,
                );

                // Reduce alpha for blur effect
                let alpha = (color.components[3] * step_alpha * 0.3);
                let blur_color = color::AlphaColor::new(
                    [color.components[0], color.components[1], color.components[2], alpha]
                );
                let blur_kurbo_rect = kurbo::Rect::new(
                    blur_rect.left as f64,
                    blur_rect.top as f64,
                    blur_rect.right as f64,
                    blur_rect.bottom as f64,
                );
                painter.fill(peniko::Fill::NonZero, scroll_transform, blur_color, None, &blur_kurbo_rect);
            }
        } else {
            // No blur, just draw the shadow directly
            let shadow_kurbo_rect = kurbo::Rect::new(
                shadow_rect.left as f64,
                shadow_rect.top as f64,
                shadow_rect.right as f64,
                shadow_rect.bottom as f64,
            );
            painter.fill(peniko::Fill::NonZero, scroll_transform, color, None, &shadow_kurbo_rect);
        }
    }
}


/// Render outline for an element with proper BorderStyle implementations
pub fn render_outline(
    painter: &mut TextPainter,
    rect: &Rect,
    style: &Arc<StyloComputedValues>,
    opacity: f32,
    scale_factor: f32,
    scroll_transform: kurbo::Affine,
) {
    let outline = style.get_outline();
    let width = outline.outline_width;

    if outline.outline_width.0 == 0 {
        return;
    }

    let outline_width_px = width.to_px();
    if outline_width_px <= 0 {
        return;
    }

    let offset = outline.outline_offset;
    let outline_offset_px = offset.px();
    let scaled_outline_width = outline_width_px as f32 * scale_factor;
    let scaled_outline_offset = outline_offset_px * scale_factor;

    let outline_rect = Rect::from_xywh(
        rect.left - scaled_outline_offset - scaled_outline_width / 2.0,
        rect.top - scaled_outline_offset - scaled_outline_width / 2.0,
        rect.width() + 2.0 * (scaled_outline_offset + scaled_outline_width / 2.0),
        rect.height() + 2.0 * (scaled_outline_offset + scaled_outline_width / 2.0),
    );

    let color = match &outline.outline_color.as_absolute() {
        Some(color) => color.as_color_color(),
        None => return, // No color specified, don't render
    };
    let kurbo_rect = kurbo::Rect::new(
        outline_rect.left as f64,
        outline_rect.top as f64,
        outline_rect.right as f64,
        outline_rect.bottom as f64,
    );

    match outline.outline_style {
        OutlineStyle::Auto | OutlineStyle::BorderStyle(BorderStyle::Solid) => {
            let stroke = kurbo::Stroke::new(scaled_outline_width as f64);
            painter.stroke(&stroke, scroll_transform, color, None, &kurbo_rect);
        }
        OutlineStyle::BorderStyle(BorderStyle::Hidden | BorderStyle::None) => {
            return;
        }
        OutlineStyle::BorderStyle(BorderStyle::Dotted) => {
            let dot_spacing = scaled_outline_width * 2.0;
            let perimeter = 2.0 * (outline_rect.width() + outline_rect.height());
            let num_dots = (perimeter / (scaled_outline_width + dot_spacing)).floor() as i32;

            // Draw individual dots around the perimeter
            let stroke = kurbo::Stroke::new(scaled_outline_width as f64);
            let step = perimeter / num_dots as f32;

            for i in 0..num_dots {
                let t = (i as f32 * step) / perimeter;
                let (x, y) = point_on_rect_perimeter(&outline_rect, t);
                let dot = kurbo::Circle::new((x as f64, y as f64), (scaled_outline_width / 2.0) as f64);
                painter.fill(peniko::Fill::NonZero, scroll_transform, color, None, &dot);
            }
        }
        OutlineStyle::BorderStyle(BorderStyle::Dashed) => {
            // For dashed, draw as solid for now or implement manual dash drawing
            let stroke = kurbo::Stroke::new(scaled_outline_width as f64);
            painter.stroke(&stroke, scroll_transform, color, None, &kurbo_rect);
        }

        OutlineStyle::BorderStyle(BorderStyle::Double) => {
            let inner_width = scaled_outline_width / 3.0;
            let gap = scaled_outline_width / 3.0;

            let outer_stroke = kurbo::Stroke::new(inner_width as f64);
            painter.stroke(&outer_stroke, scroll_transform, color, None, &kurbo_rect);

            let inner_rect = Rect::from_xywh(
                outline_rect.left + gap + inner_width,
                outline_rect.top + gap + inner_width,
                outline_rect.width() - 2.0 * (gap + inner_width),
                outline_rect.height() - 2.0 * (gap + inner_width),
            );
            let inner_kurbo_rect = kurbo::Rect::new(
                inner_rect.left as f64,
                inner_rect.top as f64,
                inner_rect.right as f64,
                inner_rect.bottom as f64,
            );
            painter.stroke(&outer_stroke, scroll_transform, color, None, &inner_kurbo_rect);
        }
        OutlineStyle::BorderStyle(BorderStyle::Groove) => {
            // Groove: appears carved into the page (darker on top/left, lighter on bottom/right)
            let half_width = scaled_outline_width / 2.0;

            // Darker outer edge
            let darker_color = darken_color(color, 0.4);
            let outer_stroke = kurbo::Stroke::new(half_width as f64);
            painter.stroke(&outer_stroke, scroll_transform, darker_color, None, &kurbo_rect);

            // Lighter inner edge
            let lighter_color = lighten_color(color, 0.4);
            let inner_rect = Rect::from_xywh(
                outline_rect.left + half_width / 2.0,
                outline_rect.top + half_width / 2.0,
                outline_rect.width() - half_width,
                outline_rect.height() - half_width,
            );
            let inner_kurbo_rect = kurbo::Rect::new(
                inner_rect.left as f64,
                inner_rect.top as f64,
                inner_rect.right as f64,
                inner_rect.bottom as f64,
            );
            painter.stroke(&outer_stroke, scroll_transform, lighter_color, None, &inner_kurbo_rect);
        }
        OutlineStyle::BorderStyle(BorderStyle::Ridge) => {
            // Ridge: appears raised (lighter on top/left, darker on bottom/right) - opposite of groove
            let half_width = scaled_outline_width / 2.0;

            // Lighter outer edge
            let lighter_color = lighten_color(color, 0.4);
            let outer_stroke = kurbo::Stroke::new(half_width as f64);
            painter.stroke(&outer_stroke, scroll_transform, lighter_color, None, &kurbo_rect);

            // Darker inner edge
            let darker_color = darken_color(color, 0.4);
            let inner_rect = Rect::from_xywh(
                outline_rect.left + half_width / 2.0,
                outline_rect.top + half_width / 2.0,
                outline_rect.width() - half_width,
                outline_rect.height() - half_width,
            );
            let inner_kurbo_rect = kurbo::Rect::new(
                inner_rect.left as f64,
                inner_rect.top as f64,
                inner_rect.right as f64,
                inner_rect.bottom as f64,
            );
            painter.stroke(&outer_stroke, scroll_transform, darker_color, None, &inner_kurbo_rect);
        }
        OutlineStyle::BorderStyle(BorderStyle::Inset) => {
            // Inset: border makes element appear embedded (darker on top/left, lighter on bottom/right)
            let darker_color = darken_color(color, 0.5);
            let stroke = kurbo::Stroke::new(scaled_outline_width as f64);
            painter.stroke(&stroke, scroll_transform, darker_color, None, &kurbo_rect);
        }
        OutlineStyle::BorderStyle(BorderStyle::Outset) => {
            // Outset: border makes element appear raised (lighter on top/left, darker on bottom/right)
            let lighter_color = lighten_color(color, 0.5);
            let stroke = kurbo::Stroke::new(scaled_outline_width as f64);
            painter.stroke(&stroke, scroll_transform, lighter_color, None, &kurbo_rect);
        }
    }
}

// Helper function to darken a color
fn darken_color(color: color::AlphaColor<color::Srgb>, factor: f32) -> color::AlphaColor<color::Srgb> {
    let [r, g, b, a] = color.components;
    color::AlphaColor::new([r * (1.0 - factor), g * (1.0 - factor), b * (1.0 - factor), a])
}

// Helper function to lighten a color
fn lighten_color(color: color::AlphaColor<color::Srgb>, factor: f32) -> color::AlphaColor<color::Srgb> {
    let [r, g, b, a] = color.components;
    color::AlphaColor::new([
        r + (1.0 - r) * factor,
        g + (1.0 - g) * factor,
        b + (1.0 - b) * factor,
        a
    ])
}

fn point_on_rect_perimeter(rect: &Rect, t: f32) -> (f32, f32) {
    let perimeter = 2.0 * (rect.width() + rect.height());
    let distance = t * perimeter;

    if distance < rect.width() {
        (rect.left + distance, rect.top)
    } else if distance < rect.width() + rect.height() {
        (rect.right, rect.top + (distance - rect.width()))
    } else if distance < 2.0 * rect.width() + rect.height() {
        (rect.right - (distance - rect.width() - rect.height()), rect.bottom)
    } else {
        (rect.left, rect.bottom - (distance - 2.0 * rect.width() - rect.height()))
    }
}

/*/// Render stroke for an element (similar to SVG stroke)
pub fn render_stroke(
    painter: &mut TextPainter,
    rect: &Rect,
    stroke: &Stroke,
    opacity: f32,
    scale_factor: f32,
    scroll_transform: kurbo::Affine,
) {
    // Only render if stroke is visible
    if !stroke.is_visible() {
        return;
    }

    let stroke_color = match &stroke.color {
        Some(color) => color,
        None => return, // No stroke color, don't render
    };

    // Get stroke width in pixels
    let stroke_width_px = stroke.width_px(16.0, 0.0);
    let scaled_stroke_width = stroke_width_px * scale_factor;

    // Convert Skia rect to kurbo rect
    let kurbo_rect = kurbo::Rect::new(
        rect.left as f64,
        rect.top as f64,
        rect.right as f64,
        rect.bottom as f64,
    );

    // Convert stroke color to AlphaColor with combined opacity
    let skia_color = stroke_color.to_skia_color();
    let combined_opacity = stroke.opacity * opacity;
    let stroke_alpha_color = color::AlphaColor::from_rgba8(
        skia_color.r(),
        skia_color.g(),
        skia_color.b(),
        (skia_color.a() as f32 * combined_opacity) as u8,
    );

    // Draw the stroke around the element using TextPainter's stroke method
    let kurbo_stroke = kurbo::Stroke::new(scaled_stroke_width as f64);
    painter.stroke(&kurbo_stroke, scroll_transform, stroke_alpha_color, None, &kurbo_rect);
}
*/