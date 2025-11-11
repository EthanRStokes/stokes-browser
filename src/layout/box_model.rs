// Box model implementation for CSS layout
use skia_safe::Rect;
use style::properties::style_structs::{Border, Margin, Padding};
use style::values::computed::Au;
use style::values::computed::length::Margin as MarginLength;
use style::values::generics::length::GenericMargin;
use style::values::generics::transform::ToAbsoluteLength;

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

pub trait ToEdgeSizes {
    fn as_edge_sizes(&self, content_width: i32) -> EdgeSizes;
}

impl ToEdgeSizes for Margin {
    fn as_edge_sizes(&self, content_width: i32) -> EdgeSizes {
        let top = &self.margin_top;
        let right = &self.margin_right;
        let bottom = &self.margin_bottom;
        let left = &self.margin_left;

        fn to_px(margin: &MarginLength, content_width: i32) -> f32 {
            match margin {
                MarginLength::LengthPercentage(length_percentage) => {
                    // For simplicity, assume length_percentage is in pixels
                    length_percentage.to_pixel_length(Au(content_width)).px()
                }
                MarginLength::Auto => 0.0, // Auto margins are treated as 0 for edge sizes
                MarginLength::AnchorSizeFunction(anchor) => 0.0, // Placeholder
                MarginLength::AnchorContainingCalcFunction(anchor) => 0.0, // Placeholder
            }
        }
        EdgeSizes {
            top: to_px(top, content_width),
            right: to_px(right, content_width),
            bottom: to_px(bottom, content_width),
            left: to_px(left, content_width),
        }
    }
}

impl ToEdgeSizes for Padding {
    fn as_edge_sizes(&self, content_width: i32) -> EdgeSizes {
        let top = &self.padding_top.0.to_pixel_length(Au(content_width)).px();
        let right = &self.padding_right.0.to_pixel_length(Au(content_width)).px();
        let bottom = &self.padding_bottom.0.to_pixel_length(Au(content_width)).px();
        let left = &self.padding_left.0.to_pixel_length(Au(content_width)).px();

        EdgeSizes {
            top: *top,
            right: *right,
            bottom: *bottom,
            left: *left,
        }
    }
}

impl ToEdgeSizes for Border {
    fn as_edge_sizes(&self, content_width: i32) -> EdgeSizes {
        let top = self.border_top_width.0 as f32;
        let right = self.border_right_width.0 as f32;
        let bottom = self.border_bottom_width.0 as f32;
        let left = self.border_left_width.0 as f32;

        EdgeSizes {
            top: top,
            right: right,
            bottom: bottom,
            left: left,
        }
    }
}