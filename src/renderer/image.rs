use crate::dom::{Dom, DomNode, ImageData};
use crate::dom::ImageLoadingState;
use crate::layout::LayoutBox;
use crate::renderer::text::TextPainter;
// Image rendering functionality
use skia_safe::{Color, FilterMode, Font, MipmapMode, Paint, Rect, SamplingOptions, TextBlob};
use std::cell::RefCell;
use color::AlphaColor;
use kurbo::Affine;
use style::servo_arc::Arc;
use style::properties::generated::ComputedValues as StyloComputedValues;
use crate::dom::node::CachedImage;
use crate::renderer::background::{to_image_quality, to_peniko_image};

/// Render image content
pub fn render_image_node(painter: &mut TextPainter, node: &DomNode, dom: &Dom, layout_box: &LayoutBox, image_data: &RefCell<ImageData>, style: &Arc<StyloComputedValues>, scale_factor: f32, scroll_transform: kurbo::Affine) {
    let image_data = image_data.borrow();
    let content_rect = layout_box.dimensions.content;

    // Early exit if content rect is too small
    if content_rect.width() < 1.0 || content_rect.height() < 1.0 {
        return;
    }

    match &image_data.loading_state {
        ImageLoadingState::Loaded(data) => {
            // Try to get the cached decoded image
            let inherited_box = style.get_inherited_box();
            let image_rendering = inherited_box.image_rendering;

            // Calculate scale factors to fit image into content_rect
            let scale_x = content_rect.width() as f64 / data.width as f64;
            let scale_y = content_rect.height() as f64 / data.height as f64;

            let transform = scroll_transform
                * Affine::translate((content_rect.left as f64, content_rect.top as f64))
                * Affine::scale_non_uniform(scale_x, scale_y);

            // Draw the cached image scaled to fit the content rect

            painter.draw_image(to_peniko_image(data, to_image_quality(image_rendering)).as_ref(), transform);

        },
        ImageLoadingState::Loading => {
            // Show loading placeholder
            render_image_placeholder(painter, dom, &content_rect, "Loading...", scale_factor, scroll_transform);
        },
        ImageLoadingState::Failed(error) => {
            // Show error placeholder
            render_image_placeholder(painter, dom, &content_rect, &format!("Error: {}", error), scale_factor, scroll_transform);
        },
        ImageLoadingState::NotLoaded => {
            // Show placeholder with alt text or src
            let placeholder_text = if !image_data.alt.is_empty() {
                &image_data.alt
            } else {
                &image_data.src
            };
            render_image_placeholder(painter, dom, &content_rect, placeholder_text, scale_factor, scroll_transform);
        }
    }
}

/// Render a placeholder for images (when not loaded, loading, or failed)
pub fn render_image_placeholder(painter: &mut TextPainter, dom: &Dom, rect: &Rect, text: &str, scale_factor: f32, scroll_transform: kurbo::Affine) {
    // Convert to kurbo::Rect
    let kurbo_rect = kurbo::Rect::new(
        rect.left as f64,
        rect.top as f64,
        rect.right as f64,
        rect.bottom as f64,
    );
    
    // Draw a light gray background
    let bg_color = AlphaColor::from_rgb8(240, 240, 240);
    painter.fill(
        peniko::Fill::NonZero,
        scroll_transform,
        bg_color, // todo convert
        None,
        &kurbo_rect,
    );

    // Draw a border with scaled width
    let scaled_border_width = 1.0 * scale_factor;
    let border_color = AlphaColor::from_rgb8(180, 180, 180);
    painter.stroke(
        &kurbo::Stroke::new(scaled_border_width as f64),
        scroll_transform,
        border_color,
        None,
        &kurbo_rect,
    );

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

        // todo render placeholder text
        /*if let Some(text_blob) = TextBlob::new(&display_text, font) {
            let text_bounds = text_blob.bounds();

            // Center the text in the placeholder
            let text_x = rect.left + (rect.width() - text_bounds.width()) / 2.0;
            let text_y = rect.top + (rect.height() + text_bounds.height()) / 2.0;

            // Use inner canvas for text blob drawing (not yet abstracted in TextPainter)
            painter.inner.draw_text_blob(&text_blob, (text_x, text_y), &text_paint);
        }*/
    }

    // Draw a simple "broken image" icon if there's space
    if rect.width() > 40.0 && rect.height() > 40.0 {
        let icon_color = AlphaColor::from_rgb8(150, 150, 150);
        let scaled_icon_stroke = 2.0 * scale_factor;

        let scaled_icon_size = 16.0 * scale_factor;
        let icon_x = rect.left + (rect.width() - scaled_icon_size) / 2.0;
        let icon_y = rect.top + 8.0 * scale_factor;
        let icon_rect = Rect::from_xywh(icon_x, icon_y, scaled_icon_size, scaled_icon_size);

        // Convert to kurbo shapes
        let kurbo_icon_rect = kurbo::Rect::new(
            icon_rect.left as f64,
            icon_rect.top as f64,
            icon_rect.right as f64,
            icon_rect.bottom as f64,
        );

        // Draw a simple square with an X
        painter.stroke(
            &kurbo::Stroke::new(scaled_icon_stroke as f64),
            kurbo::Affine::IDENTITY,
            icon_color,
            None,
            &kurbo_icon_rect,
        );
        
        // Draw X lines
        let line1 = kurbo::Line::new(
            (icon_rect.left as f64, icon_rect.top as f64),
            (icon_rect.right as f64, icon_rect.bottom as f64)
        );
        painter.stroke(
            &kurbo::Stroke::new(scaled_icon_stroke as f64),
            kurbo::Affine::IDENTITY,
            icon_color,
            None,
            &line1,
        );
        
        let line2 = kurbo::Line::new(
            (icon_rect.right as f64, icon_rect.top as f64),
            (icon_rect.left as f64, icon_rect.bottom as f64)
        );
        painter.stroke(
            &kurbo::Stroke::new(scaled_icon_stroke as f64),
            kurbo::Affine::IDENTITY,
            icon_color,
            None,
            &line2,
        );
    }
}
