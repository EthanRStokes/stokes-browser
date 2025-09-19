// Box model implementation for CSS layout
use skia_safe::{Rect, Point, Size};

/// CSS box model dimensions
#[derive(Debug, Clone, Default)]
pub struct Dimensions {
    /// Content area
    pub content: Rect,

    /// Padding around content
    pub padding: EdgeSizes,

    /// Border around padding
    pub border: EdgeSizes,

    /// Margin around border
    pub margin: EdgeSizes,
}

impl Dimensions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the total width including padding, border, and margin
    pub fn total_width(&self) -> f32 {
        self.content.width() +
        self.padding.left + self.padding.right +
        self.border.left + self.border.right +
        self.margin.left + self.margin.right
    }

    /// Get the total height including padding, border, and margin
    pub fn total_height(&self) -> f32 {
        self.content.height() +
        self.padding.top + self.padding.bottom +
        self.border.top + self.border.bottom +
        self.margin.top + self.margin.bottom
    }

    /// Get the border box (content + padding + border)
    pub fn border_box(&self) -> Rect {
        Rect::from_xywh(
            self.content.left - self.padding.left - self.border.left,
            self.content.top - self.padding.top - self.border.top,
            self.content.width() + self.padding.left + self.padding.right + self.border.left + self.border.right,
            self.content.height() + self.padding.top + self.padding.bottom + self.border.top + self.border.bottom,
        )
    }

    /// Get the margin box (everything)
    pub fn margin_box(&self) -> Rect {
        Rect::from_xywh(
            self.content.left - self.padding.left - self.border.left - self.margin.left,
            self.content.top - self.padding.top - self.border.top - self.margin.top,
            self.total_width(),
            self.total_height(),
        )
    }
}

/// Edge sizes for margin, border, padding
#[derive(Debug, Clone, Default)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeSizes {
    pub fn uniform(size: f32) -> Self {
        Self {
            top: size,
            right: size,
            bottom: size,
            left: size,
        }
    }

    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self { top, right, bottom, left }
    }
}
