use crate::css::{BorderRadiusPx, ComputedValues, OutlineStyle, Stroke, TextDecoration};
use crate::renderer::text::TextPainter;
// Text decorations, borders, shadows, and outlines
use skia_safe::{Color, Paint, Rect, TextBlob};

/// Render text decorations (underline, overline, line-through)
pub fn render_text_decorations(
    painter: &mut TextPainter,
    text_blob: &TextBlob,
    text_position: (f32, f32),
    text_decoration: &TextDecoration,
    text_paint: &Paint,
    font_size: f32,
    scale_factor: f32,
) {
    // Skip if no decorations
    if matches!(text_decoration, TextDecoration::None) {
        return;
    }

    let text_bounds = text_blob.bounds();
    let text_width = text_bounds.width();
    let (text_x, text_y) = text_position;

    // Create decoration paint based on text paint
    let mut decoration_paint = text_paint.clone();
    decoration_paint.set_stroke(true);
    let decoration_thickness = (font_size / 16.0).max(1.0) * scale_factor;
    decoration_paint.set_stroke_width(decoration_thickness);

    // Render underline
    if text_decoration.has_underline() {
        let underline_y = text_y + font_size * 0.1; // Position below baseline
        painter.draw_line(
            (text_x, underline_y),
            (text_x + text_width, underline_y),
            &decoration_paint,
        );
    }

    // Render overline
    if text_decoration.has_overline() {
        let overline_y = text_y - font_size * 0.8; // Position above text
        painter.draw_line(
            (text_x, overline_y),
            (text_x + text_width, overline_y),
            &decoration_paint,
        );
    }

    // Render line-through (strikethrough)
    if text_decoration.has_line_through() {
        let line_through_y = text_y - font_size * 0.3; // Position through middle of text
        painter.draw_line(
            (text_x, line_through_y),
            (text_x + text_width, line_through_y),
            &decoration_paint,
        );
    }
}

/// Render an element with rounded corners
pub fn render_rounded_element(
    painter: &TextPainter,
    rect: Rect,
    border_radius_px: &BorderRadiusPx,
    bg_paint: &Paint,
    border_paint: Option<&Paint>,
    scale_factor: f32,
) {
    // Apply scale factor to border radius values
    let scaled_top_left = border_radius_px.top_left * scale_factor;
    let scaled_top_right = border_radius_px.top_right * scale_factor;
    let scaled_bottom_right = border_radius_px.bottom_right * scale_factor;
    let scaled_bottom_left = border_radius_px.bottom_left * scale_factor;

    // For now, use uniform radius (average of all corners) for simplicity
    // Skia's add_round_rect method expects a tuple for radius
    let avg_radius = (scaled_top_left + scaled_top_right + scaled_bottom_right + scaled_bottom_left) / 4.0;

    // Create a path with rounded rectangle for background
    let mut bg_path = skia_safe::Path::new();
    bg_path.add_round_rect(rect, (avg_radius, avg_radius), None);

    // Draw the background with rounded corners
    painter.draw_path(&bg_path, bg_paint);

    // Draw border if specified
    if let Some(border_paint) = border_paint {
        let mut border_path = skia_safe::Path::new();
        border_path.add_round_rect(rect, (avg_radius, avg_radius), None);
        painter.draw_path(&border_path, border_paint);
    }
}

/// Render box shadows for an element
pub fn render_box_shadows(
    painter: &TextPainter,
    rect: &Rect,
    styles: &ComputedValues,
    scale_factor: f32,
) {
    // Render each box shadow
    for shadow in &styles.box_shadow {
        // Convert shadow to pixel values
        let shadow_px = shadow.to_px(styles.font_size, 400.0);

        // Skip if no visible shadow
        if !shadow_px.has_shadow() {
            continue;
        }

        // Apply scale factor to shadow properties
        let scaled_offset_x = shadow_px.offset_x * scale_factor;
        let scaled_offset_y = shadow_px.offset_y * scale_factor;
        let scaled_blur_radius = shadow_px.blur_radius * scale_factor;
        let scaled_spread_radius = shadow_px.spread_radius * scale_factor;

        // Create shadow paint
        let mut shadow_paint = Paint::default();
        shadow_paint.set_color(shadow_px.color.to_skia_color());
        shadow_paint.set_anti_alias(true);

        // Calculate shadow rectangle
        let shadow_rect = if shadow_px.inset {
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
                let mut blur_paint = shadow_paint.clone();
                let original_color = shadow_px.color.to_skia_color();
                let alpha = (original_color.a() as f32 * step_alpha * 0.3) as u8;
                blur_paint.set_color(Color::from_argb(
                    alpha,
                    original_color.r(),
                    original_color.g(),
                    original_color.b(),
                ));

                painter.draw_rect(blur_rect, &blur_paint);
            }
        } else {
            // No blur, just draw the shadow directly
            painter.draw_rect(shadow_rect, &shadow_paint);
        }
    }
}

/// Render outline for an element
pub fn render_outline(
    painter: &TextPainter,
    rect: &Rect,
    styles: &ComputedValues,
    opacity: f32,
    scale_factor: f32,
) {
    // Check if outline is visible
    if !styles.outline.is_visible() {
        return;
    }

    // Get outline width in pixels
    let outline_width_px = styles.outline.width.to_px(styles.font_size, 400.0);
    if outline_width_px <= 0.0 {
        return;
    }

    // Get outline offset in pixels
    let outline_offset_px = styles.outline_offset.to_px(styles.font_size, 400.0);

    // Apply scale factor
    let scaled_outline_width = outline_width_px * scale_factor;
    let scaled_outline_offset = outline_offset_px * scale_factor;

    // Calculate outline rectangle (outside the border box, with offset)
    let outline_rect = Rect::from_xywh(
        rect.left - scaled_outline_offset - scaled_outline_width / 2.0,
        rect.top - scaled_outline_offset - scaled_outline_width / 2.0,
        rect.width() + 2.0 * (scaled_outline_offset + scaled_outline_width / 2.0),
        rect.height() + 2.0 * (scaled_outline_offset + scaled_outline_width / 2.0),
    );

    // Create outline paint
    let mut outline_paint = Paint::default();
    let mut outline_color = styles.outline.color.to_skia_color();
    outline_color = outline_color.with_a((outline_color.a() as f32 * opacity) as u8);
    outline_paint.set_color(outline_color);
    outline_paint.set_stroke(true);
    outline_paint.set_stroke_width(scaled_outline_width);
    outline_paint.set_anti_alias(true);

    // Set outline style (dashed, dotted, etc.)
    match styles.outline.style {
        OutlineStyle::Solid => {
            // Default solid line, no path effect needed
        }
        OutlineStyle::Dashed => {
            // Create dashed line effect
            let intervals = [scaled_outline_width * 3.0, scaled_outline_width * 2.0];
            if let Some(path_effect) = skia_safe::PathEffect::dash(&intervals, 0.0) {
                outline_paint.set_path_effect(path_effect);
            }
        }
        OutlineStyle::Dotted => {
            // Create dotted line effect
            let intervals = [scaled_outline_width, scaled_outline_width];
            if let Some(path_effect) = skia_safe::PathEffect::dash(&intervals, 0.0) {
                outline_paint.set_path_effect(path_effect);
            }
        }
        OutlineStyle::Double => {
            // Draw two outlines (simplified implementation)
            let inner_width = scaled_outline_width / 3.0;
            let gap = scaled_outline_width / 3.0;

            // Draw outer outline
            let mut outer_paint = outline_paint.clone();
            outer_paint.set_stroke_width(inner_width);
            painter.draw_rect(outline_rect, &outer_paint);

            // Draw inner outline
            let inner_rect = Rect::from_xywh(
                outline_rect.left + gap + inner_width,
                outline_rect.top + gap + inner_width,
                outline_rect.width() - 2.0 * (gap + inner_width),
                outline_rect.height() - 2.0 * (gap + inner_width),
            );
            painter.draw_rect(inner_rect, &outer_paint);
            return; // Skip the main draw below
        }
        OutlineStyle::Groove | OutlineStyle::Ridge |
        OutlineStyle::Inset | OutlineStyle::Outset => {
            // These styles create 3D effects - simplified to solid for now
            // A full implementation would draw with different shades
        }
        OutlineStyle::None | OutlineStyle::Hidden => {
            return; // Already checked, but handle explicitly
        }
    }

    // Draw the outline
    painter.draw_rect(outline_rect, &outline_paint);
}

/// Render stroke for an element (similar to SVG stroke)
pub fn render_stroke(
    painter: &TextPainter,
    rect: &Rect,
    stroke: &Stroke,
    opacity: f32,
    scale_factor: f32,
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

    // Create stroke paint
    let mut stroke_paint = Paint::default();
    let mut color = stroke_color.to_skia_color();
    
    // Apply both stroke opacity and element opacity
    let combined_opacity = stroke.opacity * opacity;
    color = color.with_a((color.a() as f32 * combined_opacity) as u8);
    
    stroke_paint.set_color(color);
    stroke_paint.set_stroke(true);
    stroke_paint.set_stroke_width(scaled_stroke_width);
    stroke_paint.set_anti_alias(true);

    // Draw the stroke around the element
    painter.draw_rect(*rect, &stroke_paint);
}
