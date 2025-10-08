// Paint and styling utilities
use skia_safe::{Paint, Color};

/// Create default paints for rendering
pub struct DefaultPaints {
    pub text_paint: Paint,
    pub background_paint: Paint,
    pub border_paint: Paint,
}

impl DefaultPaints {
    pub fn new() -> Self {
        let mut text_paint = Paint::default();
        text_paint.set_color(Color::BLACK);
        text_paint.set_anti_alias(true);

        let mut background_paint = Paint::default();
        background_paint.set_color(Color::WHITE);

        let mut border_paint = Paint::default();
        border_paint.set_color(Color::from_rgb(200, 200, 200));
        border_paint.set_stroke(true);
        border_paint.set_stroke_width(1.0);

        Self {
            text_paint,
            background_paint,
            border_paint,
        }
    }
}

