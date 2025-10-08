// Image rendering functionality
use skia_safe::{Canvas, Paint, Color, Rect, Font, TextBlob};
use crate::layout::LayoutBox;
use crate::dom::ImageData;
use crate::dom::ImageLoadingState;

/// Render image content
pub fn render_image_node(canvas: &Canvas, layout_box: &LayoutBox, image_data: &ImageData, scale_factor: f64, font: &Font) {
    let content_rect = layout_box.dimensions.content;

    match &image_data.loading_state {
        ImageLoadingState::Loaded(_) => {
            // Try to get the cached decoded image
            // We need to work around the borrowing issue by cloning the image if available
            if let Some(cached_image) = &image_data.cached_image {
                let mut paint = Paint::default();
                paint.set_anti_alias(true);

                // Draw the cached image scaled to fit the content rect
                canvas.draw_image_rect(
                    cached_image,
                    None, // Use entire source image
                    content_rect,
                    &paint
                );
            } else {
                // No cached image available, show placeholder indicating decoding issue
                render_image_placeholder(canvas, &content_rect, "Image decoding failed", scale_factor, font);
            }
        },
        ImageLoadingState::Loading => {
            // Show loading placeholder
            render_image_placeholder(canvas, &content_rect, "Loading...", scale_factor, font);
        },
        ImageLoadingState::Failed(error) => {
            // Show error placeholder
            render_image_placeholder(canvas, &content_rect, &format!("Error: {}", error), scale_factor, font);
        },
        ImageLoadingState::NotLoaded => {
            // Show placeholder with alt text or src
            let placeholder_text = if !image_data.alt.is_empty() {
                &image_data.alt
            } else {
                &image_data.src
            };
            render_image_placeholder(canvas, &content_rect, placeholder_text, scale_factor, font);
        }
    }
}

/// Render a placeholder for images (when not loaded, loading, or failed)
pub fn render_image_placeholder(canvas: &Canvas, rect: &Rect, text: &str, scale_factor: f64, font: &Font) {
    // Draw a light gray background
    let mut bg_paint = Paint::default();
    bg_paint.set_color(Color::from_rgb(240, 240, 240));
    canvas.draw_rect(*rect, &bg_paint);

    // Draw a border with scaled width
    let mut border_paint = Paint::default();
    border_paint.set_color(Color::from_rgb(180, 180, 180));
    border_paint.set_stroke(true);
    let scaled_border_width = 1.0 * scale_factor as f32;
    border_paint.set_stroke_width(scaled_border_width);
    canvas.draw_rect(*rect, &border_paint);

    // Draw placeholder text
    if !text.is_empty() && rect.width() > 20.0 && rect.height() > 20.0 {
        let mut text_paint = Paint::default();
        text_paint.set_color(Color::from_rgb(100, 100, 100));
        text_paint.set_anti_alias(true);

        // Truncate text if it's too long
        let display_text = if text.len() > 20 {
            format!("{}...", &text[..17])
        } else {
            text.to_string()
        };

        if let Some(text_blob) = TextBlob::new(&display_text, font) {
            let text_bounds = text_blob.bounds();

            // Center the text in the placeholder
            let text_x = rect.left + (rect.width() - text_bounds.width()) / 2.0;
            let text_y = rect.top + (rect.height() + text_bounds.height()) / 2.0;

            canvas.draw_text_blob(&text_blob, (text_x, text_y), &text_paint);
        }
    }

    // Draw a simple "broken image" icon if there's space
    if rect.width() > 40.0 && rect.height() > 40.0 {
        let mut icon_paint = Paint::default();
        icon_paint.set_color(Color::from_rgb(150, 150, 150));
        icon_paint.set_stroke(true);
        let scaled_icon_stroke = 2.0 * scale_factor as f32;
        icon_paint.set_stroke_width(scaled_icon_stroke);

        let scaled_icon_size = 16.0 * scale_factor as f32;
        let icon_x = rect.left + (rect.width() - scaled_icon_size) / 2.0;
        let icon_y = rect.top + 8.0 * scale_factor as f32;
        let icon_rect = Rect::from_xywh(icon_x, icon_y, scaled_icon_size, scaled_icon_size);

        // Draw a simple square with an X
        canvas.draw_rect(icon_rect, &icon_paint);
        canvas.draw_line(
            (icon_rect.left, icon_rect.top),
            (icon_rect.right, icon_rect.bottom),
            &icon_paint
        );
        canvas.draw_line(
            (icon_rect.right, icon_rect.top),
            (icon_rect.left, icon_rect.bottom),
            &icon_paint
        );
    }
}

