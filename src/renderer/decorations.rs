use color::{AlphaColor, Srgb};
use crate::css::{BorderRadiusPx, ComputedValues, OutlineStyle, Stroke, TextDecoration};
use crate::renderer::text::TextPainter;
// Text decorations, borders, shadows, and outlines
use skia_safe::{Color, Paint, Rect, TextBlob};

/// Render an element with rounded corners
pub fn render_rounded_element(
    painter: &mut TextPainter,
    rect: Rect,
    border_radius_px: &BorderRadiusPx,
    bg_color: AlphaColor<Srgb>,
    border_color: Option<AlphaColor<Srgb>>,
    scale_factor: f32,
    scroll_transform: kurbo::Affine,
) {
    // Apply scale factor to border radius values
    let scaled_top_left = border_radius_px.top_left * scale_factor;
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
    }
}

/// Render box shadows for an element
pub fn render_box_shadows(
    painter: &mut TextPainter,
    rect: &Rect,
    styles: &ComputedValues,
    scale_factor: f32,
    scroll_transform: kurbo::Affine,
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
                let original_color = shadow_px.color.to_skia_color();
                let alpha = (original_color.a() as f32 * step_alpha * 0.3) as u8;
                let blur_color = color::AlphaColor::from_rgba8(
                    original_color.r(),
                    original_color.g(),
                    original_color.b(),
                    alpha,
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
            let shadow_color = shadow_paint.color();
            let shadow_alpha_color = color::AlphaColor::from_rgba8(
                shadow_color.r(),
                shadow_color.g(),
                shadow_color.b(),
                shadow_color.a(),
            );
            let shadow_kurbo_rect = kurbo::Rect::new(
                shadow_rect.left as f64,
                shadow_rect.top as f64,
                shadow_rect.right as f64,
                shadow_rect.bottom as f64,
            );
            painter.fill(peniko::Fill::NonZero, scroll_transform, shadow_alpha_color, None, &shadow_kurbo_rect);
        }
    }
}

/// Render outline for an element
pub fn render_outline(
    painter: &mut TextPainter,
    rect: &Rect,
    styles: &ComputedValues,
    opacity: f32,
    scale_factor: f32,
    scroll_transform: kurbo::Affine,
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

            // Convert outline color to AlphaColor
            let outline_alpha_color = color::AlphaColor::from_rgba8(
                outline_color.r(),
                outline_color.g(),
                outline_color.b(),
                outline_color.a(),
            );

            // Draw outer outline
            let outer_kurbo_rect = kurbo::Rect::new(
                outline_rect.left as f64,
                outline_rect.top as f64,
                outline_rect.right as f64,
                outline_rect.bottom as f64,
            );
            let outer_stroke = kurbo::Stroke::new(inner_width as f64);
            painter.stroke(&outer_stroke, scroll_transform, outline_alpha_color, None, &outer_kurbo_rect);

            // Draw inner outline
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
            painter.stroke(&outer_stroke, scroll_transform, outline_alpha_color, None, &inner_kurbo_rect);
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

    // Draw the outline using stroke
    let outline_kurbo_rect = kurbo::Rect::new(
        outline_rect.left as f64,
        outline_rect.top as f64,
        outline_rect.right as f64,
        outline_rect.bottom as f64,
    );

    // Convert outline color to AlphaColor
    let outline_alpha_color = color::AlphaColor::from_rgba8(
        outline_color.r(),
        outline_color.g(),
        outline_color.b(),
        outline_color.a(),
    );

    let outline_stroke = kurbo::Stroke::new(scaled_outline_width as f64);
    painter.stroke(&outline_stroke, scroll_transform, outline_alpha_color, None, &outline_kurbo_rect);
}

/// Render stroke for an element (similar to SVG stroke)
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
