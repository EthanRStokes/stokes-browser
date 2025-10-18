use super::font::FontManager;
use crate::css::{ComputedValues, ContentValue};
use crate::dom::ElementData;
// Pseudo-element rendering (::before, ::after)
use skia_safe::{Canvas, Paint, Rect, TextBlob};

/// Render pseudo-element generated content (::before or ::after)
pub fn render_pseudo_element_content(
    canvas: &Canvas,
    rect: &Rect,
    element_data: &ElementData,
    styles: &ComputedValues,
    scale_factor: f64,
    is_before: bool,
    font_manager: &FontManager,
    default_text_paint: &Paint,
) {
    // Check if content property is set and not Normal/None
    let content_text = match &styles.content {
        ContentValue::None | ContentValue::Normal => return, // No content to render
        _ => styles.content.to_display_string(Some(&element_data.attributes)),
    };

    // Skip if content is empty
    if content_text.is_empty() {
        return;
    }

    // Prepare text rendering
    let mut text_paint = default_text_paint.clone();
    if let Some(text_color) = &styles.color {
        text_paint.set_color(text_color.to_skia_color());
    }

    let scaled_font_size = styles.font_size * scale_factor as f32;
    let font = font_manager.placeholder_font_for_size_and_style(scaled_font_size, &styles.font_style);

    // Create text blob
    if let Some(text_blob) = TextBlob::new(&content_text, &font) {
        let text_bounds = text_blob.bounds();

        // Position the text
        // For ::before, position at the start of the element
        // For ::after, position at the end of the element
        let (text_x, text_y) = if is_before {
            // Position at the left edge of the content area
            let x = rect.left - text_bounds.width() - 2.0 * scale_factor as f32;
            let y = rect.top + scaled_font_size;
            (x, y)
        } else {
            // Position at the right edge of the content area
            let x = rect.right + 2.0 * scale_factor as f32;
            let y = rect.top + scaled_font_size;
            (x, y)
        };

        // Draw the generated content
        canvas.draw_text_blob(&text_blob, (text_x, text_y), &text_paint);

        // Apply text decorations if specified
        super::decorations::render_text_decorations(
            canvas,
            &text_blob,
            (text_x, text_y),
            &styles.text_decoration,
            &text_paint,
            scaled_font_size,
            scale_factor,
        );
    }
}

