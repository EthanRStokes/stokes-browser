use anyrender::{Glyph, NormalizedCoord, Paint, PaintRef, PaintScene};
use kurbo::{Affine, BezPath, PathEl, Rect, RoundedRect, Shape, Stroke};
use peniko::{
    BlendMode, Extend, Fill, Gradient, ImageAlphaType, ImageBrush, ImageData, ImageFormat,
    ImageQuality, ImageSampler,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

const PATH_TOLERANCE: f64 = 0.1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayListFrame {
    pub frame_id: u64,
    pub width: u32,
    pub height: u32,
    pub fonts: Vec<DisplayFont>,
    pub commands: Vec<DisplayCommand>,
}

impl DisplayListFrame {
    pub fn replay(
        &self,
        painter: &mut impl PaintScene,
        root_transform: Affine,
        font_cache: &HashMap<DisplayFont, Arc<Vec<u8>>>,
    ) {
        let fonts = self
            .fonts
            .iter()
            .map(|font| font_cache.get(font).map(|bytes| font.to_peniko(bytes.clone())))
            .collect::<Vec<_>>();

        for command in &self.commands {
            command.replay(painter, root_transform, &fonts);
        }
    }
}

pub struct DisplayListRecorder {
    frame_id: u64,
    width: u32,
    height: u32,
    fonts: Vec<DisplayFont>,
    font_lookup: HashMap<DisplayFont, u32>,
    font_payloads: Vec<DisplayFontData>,
    commands: Vec<DisplayCommand>,
}

impl DisplayListRecorder {
    pub fn new(width: u32, height: u32, frame_id: u64) -> Self {
        Self {
            frame_id,
            width,
            height,
            fonts: Vec::new(),
            font_lookup: HashMap::new(),
            font_payloads: Vec::new(),
            commands: Vec::new(),
        }
    }

    pub fn into_frame_parts(self) -> (DisplayListFrame, Vec<DisplayFontData>) {
        (
            DisplayListFrame {
                frame_id: self.frame_id,
                width: self.width,
                height: self.height,
                fonts: self.fonts,
                commands: self.commands,
            },
            self.font_payloads,
        )
    }

    fn intern_font(&mut self, font: &peniko::FontData) -> u32 {
        let display_font = DisplayFont::from_peniko(font);
        if let Some(&font_id) = self.font_lookup.get(&display_font) {
            return font_id;
        }

        let font_id = self.fonts.len().try_into().expect("too many fonts in display list");
        self.fonts.push(display_font.clone());
        self.font_lookup.insert(display_font.clone(), font_id);
        self.font_payloads.push(DisplayFontData::from_peniko(font));
        font_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DisplayCommand {
    PushLayer {
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: DisplayShape,
    },
    PushClipLayer {
        transform: Affine,
        clip: DisplayShape,
    },
    PopLayer,
    Stroke {
        style: Stroke,
        transform: Affine,
        brush: DisplayBrush,
        brush_transform: Option<Affine>,
        shape: DisplayShape,
    },
    Fill {
        style: Fill,
        transform: Affine,
        brush: DisplayBrush,
        brush_transform: Option<Affine>,
        shape: DisplayShape,
    },
    DrawGlyphs {
        font_id: u32,
        font_size: f32,
        hint: bool,
        normalized_coords: Vec<NormalizedCoord>,
        style: DisplayStyle,
        brush: DisplayBrush,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: Vec<DisplayGlyph>,
    },
    DrawBoxShadow {
        transform: Affine,
        rect: Rect,
        brush: [f32; 4],
        radius: f64,
        std_dev: f64,
    },
    DrawImage {
        image: DisplayImageBrush,
        transform: Affine,
    },
}

impl DisplayCommand {
    fn replay(
        &self,
        painter: &mut impl PaintScene,
        root_transform: Affine,
        fonts: &[Option<peniko::FontData>],
    ) {
        match self {
            Self::PushLayer {
                blend,
                alpha,
                transform,
                clip,
            } => match clip {
                DisplayShape::Rect(rect) => {
                    painter.push_layer(*blend, *alpha, root_transform * *transform, rect);
                }
                DisplayShape::RoundedRect(rounded_rect) => {
                    painter.push_layer(*blend, *alpha, root_transform * *transform, rounded_rect);
                }
                DisplayShape::Path(path) => {
                    let bez_path = build_bez_path(path);
                    painter.push_layer(*blend, *alpha, root_transform * *transform, &bez_path);
                }
            },
            Self::PushClipLayer { transform, clip } => match clip {
                DisplayShape::Rect(rect) => {
                    painter.push_clip_layer(root_transform * *transform, rect);
                }
                DisplayShape::RoundedRect(rounded_rect) => {
                    painter.push_clip_layer(root_transform * *transform, rounded_rect);
                }
                DisplayShape::Path(path) => {
                    let bez_path = build_bez_path(path);
                    painter.push_clip_layer(root_transform * *transform, &bez_path);
                }
            },
            Self::PopLayer => painter.pop_layer(),
            Self::Stroke {
                style,
                transform,
                brush,
                brush_transform,
                shape,
            } => match shape {
                DisplayShape::Rect(rect) => {
                    brush.stroke(painter, style, root_transform * *transform, *brush_transform, rect);
                }
                DisplayShape::RoundedRect(rounded_rect) => {
                    brush.stroke(
                        painter,
                        style,
                        root_transform * *transform,
                        *brush_transform,
                        rounded_rect,
                    );
                }
                DisplayShape::Path(path) => {
                    let bez_path = build_bez_path(path);
                    brush.stroke(
                        painter,
                        style,
                        root_transform * *transform,
                        *brush_transform,
                        &bez_path,
                    );
                }
            },
            Self::Fill {
                style,
                transform,
                brush,
                brush_transform,
                shape,
            } => match shape {
                DisplayShape::Rect(rect) => {
                    brush.fill(painter, *style, root_transform * *transform, *brush_transform, rect);
                }
                DisplayShape::RoundedRect(rounded_rect) => {
                    brush.fill(
                        painter,
                        *style,
                        root_transform * *transform,
                        *brush_transform,
                        rounded_rect,
                    );
                }
                DisplayShape::Path(path) => {
                    let bez_path = build_bez_path(path);
                    brush.fill(
                        painter,
                        *style,
                        root_transform * *transform,
                        *brush_transform,
                        &bez_path,
                    );
                }
            },
            Self::DrawGlyphs {
                font_id,
                font_size,
                hint,
                normalized_coords,
                style,
                brush,
                brush_alpha,
                transform,
                glyph_transform,
                glyphs,
            } => {
                let Some(Some(font)) = fonts.get(*font_id as usize) else {
                    return;
                };
                let glyphs = glyphs.iter().copied().map(DisplayGlyph::into_anyrender);
                brush.draw_glyphs(
                    painter,
                    font,
                    *font_size,
                    *hint,
                    normalized_coords,
                    style,
                    *brush_alpha,
                    root_transform * *transform,
                    *glyph_transform,
                    glyphs,
                );
            }
            Self::DrawBoxShadow {
                transform,
                rect,
                brush,
                radius,
                std_dev,
            } => {
                painter.draw_box_shadow(
                    root_transform * *transform,
                    *rect,
                    peniko::Color::new(*brush),
                    *radius,
                    *std_dev,
                );
            }
            Self::DrawImage { image, transform } => {
                let image = image.to_peniko();
                painter.draw_image(image.as_ref(), root_transform * *transform);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DisplayStyle {
    Fill(Fill),
    Stroke(Stroke),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DisplayBrush {
    Solid([f32; 4]),
    Gradient(Gradient),
    Image(DisplayImageBrush),
    UnsupportedCustom,
}

impl DisplayBrush {
    fn from_paint_ref(brush: PaintRef<'_>) -> Self {
        match brush {
            Paint::Solid(color) => Self::Solid(color.components),
            Paint::Gradient(gradient) => Self::Gradient(gradient.clone()),
            Paint::Image(image) => Self::Image(DisplayImageBrush::from_ref(image)),
            Paint::Custom(_) => Self::UnsupportedCustom,
        }
    }

    fn fill(
        &self,
        painter: &mut impl PaintScene,
        style: Fill,
        transform: Affine,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        match self {
            Self::Solid(color) => {
                painter.fill(style, transform, peniko::Color::new(*color), brush_transform, shape);
            }
            Self::Gradient(gradient) => {
                painter.fill(style, transform, gradient, brush_transform, shape);
            }
            Self::Image(image) => {
                let image = image.to_peniko();
                painter.fill(style, transform, image.as_ref(), brush_transform, shape);
            }
            Self::UnsupportedCustom => {}
        }
    }

    fn stroke(
        &self,
        painter: &mut impl PaintScene,
        style: &Stroke,
        transform: Affine,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        match self {
            Self::Solid(color) => {
                painter.stroke(style, transform, peniko::Color::new(*color), brush_transform, shape);
            }
            Self::Gradient(gradient) => {
                painter.stroke(style, transform, gradient, brush_transform, shape);
            }
            Self::Image(image) => {
                let image = image.to_peniko();
                painter.stroke(style, transform, image.as_ref(), brush_transform, shape);
            }
            Self::UnsupportedCustom => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &self,
        painter: &mut impl PaintScene,
        font: &peniko::FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &[NormalizedCoord],
        style: &DisplayStyle,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph>,
    ) {
        match (self, style) {
            (Self::Solid(color), DisplayStyle::Fill(fill)) => painter.draw_glyphs(
                font,
                font_size,
                hint,
                normalized_coords,
                *fill,
                peniko::Color::new(*color),
                brush_alpha,
                transform,
                glyph_transform,
                glyphs,
            ),
            (Self::Solid(color), DisplayStyle::Stroke(stroke)) => painter.draw_glyphs(
                font,
                font_size,
                hint,
                normalized_coords,
                stroke,
                peniko::Color::new(*color),
                brush_alpha,
                transform,
                glyph_transform,
                glyphs,
            ),
            (Self::Gradient(gradient), DisplayStyle::Fill(fill)) => painter.draw_glyphs(
                font,
                font_size,
                hint,
                normalized_coords,
                *fill,
                gradient,
                brush_alpha,
                transform,
                glyph_transform,
                glyphs,
            ),
            (Self::Gradient(gradient), DisplayStyle::Stroke(stroke)) => painter.draw_glyphs(
                font,
                font_size,
                hint,
                normalized_coords,
                stroke,
                gradient,
                brush_alpha,
                transform,
                glyph_transform,
                glyphs,
            ),
            (Self::Image(image), DisplayStyle::Fill(fill)) => {
                let image = image.to_peniko();
                painter.draw_glyphs(
                    font,
                    font_size,
                    hint,
                    normalized_coords,
                    *fill,
                    image.as_ref(),
                    brush_alpha,
                    transform,
                    glyph_transform,
                    glyphs,
                )
            }
            (Self::Image(image), DisplayStyle::Stroke(stroke)) => {
                let image = image.to_peniko();
                painter.draw_glyphs(
                    font,
                    font_size,
                    hint,
                    normalized_coords,
                    stroke,
                    image.as_ref(),
                    brush_alpha,
                    transform,
                    glyph_transform,
                    glyphs,
                )
            }
            (Self::UnsupportedCustom, _) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DisplayFont {
    pub blob_id: u64,
    pub index: u32,
}

impl DisplayFont {
    fn from_peniko(font: &peniko::FontData) -> Self {
        Self {
            blob_id: font.data.id(),
            index: font.index,
        }
    }

    fn to_peniko(&self, bytes: Arc<Vec<u8>>) -> peniko::FontData {
        let blob = peniko::Blob::new(bytes);
        peniko::FontData::new(blob, self.index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayFontData {
    pub font: DisplayFont,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

impl DisplayFontData {
    fn from_peniko(font: &peniko::FontData) -> Self {
        Self {
            font: DisplayFont::from_peniko(font),
            bytes: font.data.data().to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DisplayGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

impl DisplayGlyph {
    fn from_anyrender(glyph: Glyph) -> Self {
        Self {
            id: glyph.id,
            x: glyph.x,
            y: glyph.y,
        }
    }

    fn into_anyrender(self) -> Glyph {
        Glyph {
            id: self.id,
            x: self.x,
            y: self.y,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayImageBrush {
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
    pub format: ImageFormat,
    pub alpha_type: ImageAlphaType,
    pub width: u32,
    pub height: u32,
    pub x_extend: Extend,
    pub y_extend: Extend,
    pub quality: ImageQuality,
    pub alpha: f32,
}

impl DisplayImageBrush {
    fn from_ref(image: peniko::ImageBrush<&ImageData>) -> Self {
        Self {
            data: image.image.data.data().to_vec(),
            format: image.image.format,
            alpha_type: image.image.alpha_type,
            width: image.image.width,
            height: image.image.height,
            x_extend: image.sampler.x_extend,
            y_extend: image.sampler.y_extend,
            quality: image.sampler.quality,
            alpha: image.sampler.alpha,
        }
    }

    fn to_peniko(&self) -> ImageBrush {
        ImageBrush {
            image: ImageData {
                data: peniko::Blob::new(Arc::new(self.data.clone())),
                format: self.format,
                alpha_type: self.alpha_type,
                width: self.width,
                height: self.height,
            },
            sampler: ImageSampler {
                x_extend: self.x_extend,
                y_extend: self.y_extend,
                quality: self.quality,
                alpha: self.alpha,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DisplayShape {
    Rect(Rect),
    RoundedRect(RoundedRect),
    Path(Vec<DisplayPathEl>),
}

impl DisplayShape {
    fn from_shape(shape: &impl Shape) -> Self {
        if let Some(rect) = shape.as_rect() {
            Self::Rect(rect)
        } else if let Some(rounded_rect) = shape.as_rounded_rect() {
            Self::RoundedRect(rounded_rect)
        } else {
            Self::Path(
                shape
                    .path_elements(PATH_TOLERANCE)
                    .map(DisplayPathEl::from)
                    .collect(),
            )
        }
    }
}

fn build_bez_path(elements: &[DisplayPathEl]) -> BezPath {
    let mut bez_path = BezPath::new();
    for element in elements {
        bez_path.push(element.to_kurbo());
    }
    bez_path
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DisplayPathEl {
    MoveTo((f64, f64)),
    LineTo((f64, f64)),
    QuadTo((f64, f64), (f64, f64)),
    CurveTo((f64, f64), (f64, f64), (f64, f64)),
    ClosePath,
}

impl From<PathEl> for DisplayPathEl {
    fn from(value: PathEl) -> Self {
        match value {
            PathEl::MoveTo(point) => Self::MoveTo((point.x, point.y)),
            PathEl::LineTo(point) => Self::LineTo((point.x, point.y)),
            PathEl::QuadTo(p1, p2) => Self::QuadTo((p1.x, p1.y), (p2.x, p2.y)),
            PathEl::CurveTo(p1, p2, p3) => {
                Self::CurveTo((p1.x, p1.y), (p2.x, p2.y), (p3.x, p3.y))
            }
            PathEl::ClosePath => Self::ClosePath,
        }
    }
}

impl DisplayPathEl {
    fn to_kurbo(&self) -> PathEl {
        match *self {
            Self::MoveTo((x, y)) => PathEl::MoveTo((x, y).into()),
            Self::LineTo((x, y)) => PathEl::LineTo((x, y).into()),
            Self::QuadTo((x1, y1), (x2, y2)) => PathEl::QuadTo((x1, y1).into(), (x2, y2).into()),
            Self::CurveTo((x1, y1), (x2, y2), (x3, y3)) => {
                PathEl::CurveTo((x1, y1).into(), (x2, y2).into(), (x3, y3).into())
            }
            Self::ClosePath => PathEl::ClosePath,
        }
    }
}

impl PaintScene for DisplayListRecorder {
    fn reset(&mut self) {
        self.fonts.clear();
        self.font_lookup.clear();
        self.font_payloads.clear();
        self.commands.clear();
    }

    fn push_layer(
        &mut self,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
    ) {
        self.commands.push(DisplayCommand::PushLayer {
            blend: blend.into(),
            alpha,
            transform,
            clip: DisplayShape::from_shape(clip),
        });
    }

    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        self.commands.push(DisplayCommand::PushClipLayer {
            transform,
            clip: DisplayShape::from_shape(clip),
        });
    }

    fn pop_layer(&mut self) {
        self.commands.push(DisplayCommand::PopLayer);
    }

    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.commands.push(DisplayCommand::Stroke {
            style: style.clone(),
            transform,
            brush: DisplayBrush::from_paint_ref(brush.into()),
            brush_transform,
            shape: DisplayShape::from_shape(shape),
        });
    }

    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.commands.push(DisplayCommand::Fill {
            style,
            transform,
            brush: DisplayBrush::from_paint_ref(brush.into()),
            brush_transform,
            shape: DisplayShape::from_shape(shape),
        });
    }

    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a peniko::FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        style: impl Into<peniko::StyleRef<'a>>,
        brush: impl Into<PaintRef<'a>>,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph>,
    ) {
        let style = match style.into() {
            peniko::StyleRef::Fill(fill) => DisplayStyle::Fill(fill),
            peniko::StyleRef::Stroke(stroke) => DisplayStyle::Stroke(stroke.clone()),
        };
        let font_id = self.intern_font(font);

        self.commands.push(DisplayCommand::DrawGlyphs {
            font_id,
            font_size,
            hint,
            normalized_coords: normalized_coords.to_vec(),
            style,
            brush: DisplayBrush::from_paint_ref(brush.into()),
            brush_alpha,
            transform,
            glyph_transform,
            glyphs: glyphs.map(DisplayGlyph::from_anyrender).collect(),
        });
    }

    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        brush: peniko::Color,
        radius: f64,
        std_dev: f64,
    ) {
        self.commands.push(DisplayCommand::DrawBoxShadow {
            transform,
            rect,
            brush: brush.components,
            radius,
            std_dev,
        });
    }

    fn draw_image(&mut self, image: peniko::ImageBrushRef, transform: Affine) {
        self.commands.push(DisplayCommand::DrawImage {
            image: DisplayImageBrush::from_ref(image),
            transform,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DisplayCommand, DisplayFont, DisplayFontData, DisplayListRecorder, DisplayPathEl,
        DisplayShape,
    };
    use anyrender::{Glyph, PaintScene};
    use kurbo::{Affine, BezPath, Shape};
    use peniko::{Blob, Color, Fill};
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn path_shape_round_trips_elements() {
        let mut path = BezPath::new();
        path.move_to((1.0, 2.0));
        path.line_to((3.0, 4.0));
        path.quad_to((5.0, 6.0), (7.0, 8.0));
        path.curve_to((9.0, 10.0), (11.0, 12.0), (13.0, 14.0));
        path.close_path();

        let shape = DisplayShape::from_shape(&path);
        let DisplayShape::Path(elements) = shape else {
            panic!("expected path shape");
        };

        assert_eq!(elements.len(), path.path_elements(0.1).count());
        assert!(matches!(elements.first(), Some(DisplayPathEl::MoveTo(_))));
        assert!(matches!(elements.last(), Some(DisplayPathEl::ClosePath)));
    }

    #[test]
    fn display_font_round_trips_through_json() {
        let font = DisplayFont {
            blob_id: 99,
            index: 7,
        };

        let json = serde_json::to_string(&font).unwrap();
        let decoded: DisplayFont = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, font);
    }

    #[test]
    fn display_font_data_round_trips_empty_bytes() {
        let font = DisplayFontData {
            font: DisplayFont {
                blob_id: 42,
                index: 3,
            },
            bytes: Vec::new(),
        };

        let json = serde_json::to_string(&font).unwrap();
        let decoded: DisplayFontData = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, font);
    }

    #[test]
    fn recorder_interns_fonts_and_exports_payload_once() {
        let mut recorder = DisplayListRecorder::new(800, 600, 9);
        let font = peniko::FontData::new(Blob::new(Arc::new(vec![1, 2, 3, 4, 5])), 3);
        let brush = Color::new([0.2, 0.3, 0.4, 1.0]);

        recorder.draw_glyphs(
            &font,
            16.0,
            false,
            &[],
            Fill::NonZero,
            brush,
            1.0,
            Affine::IDENTITY,
            None,
            [Glyph { id: 1, x: 10.0, y: 20.0 }].into_iter(),
        );
        recorder.draw_glyphs(
            &font,
            18.0,
            true,
            &[],
            Fill::NonZero,
            brush,
            0.5,
            Affine::IDENTITY,
            None,
            [Glyph { id: 2, x: 30.0, y: 40.0 }].into_iter(),
        );

        let (frame, font_payloads) = recorder.into_frame_parts();
        assert_eq!(frame.fonts.len(), 1);
        assert_eq!(font_payloads.len(), 1);
        assert_eq!(font_payloads[0].font, frame.fonts[0]);
        assert_eq!(font_payloads[0].bytes, vec![1, 2, 3, 4, 5]);

        let font_ids = frame
            .commands
            .iter()
            .map(|command| match command {
                DisplayCommand::DrawGlyphs { font_id, .. } => *font_id,
                other => panic!("expected DrawGlyphs command, got {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(font_ids, vec![0, 0]);
    }

    #[test]
    fn frame_replay_uses_cached_font_payloads() {
        let mut recorder = DisplayListRecorder::new(800, 600, 10);
        let font = peniko::FontData::new(Blob::new(Arc::new(vec![7, 8, 9, 10])), 1);
        let brush = Color::new([0.1, 0.2, 0.3, 1.0]);

        recorder.draw_glyphs(
            &font,
            14.0,
            false,
            &[],
            Fill::NonZero,
            brush,
            1.0,
            Affine::IDENTITY,
            None,
            [Glyph { id: 12, x: 1.0, y: 2.0 }].into_iter(),
        );

        let (frame, font_payloads) = recorder.into_frame_parts();
        let font_cache = font_payloads
            .into_iter()
            .map(|font| (font.font, Arc::new(font.bytes)))
            .collect::<HashMap<_, _>>();

        struct CountingPainter {
            glyph_calls: usize,
        }

        impl PaintScene for CountingPainter {
            fn reset(&mut self) {}
            fn push_layer(
                &mut self,
                _blend: impl Into<peniko::BlendMode>,
                _alpha: f32,
                _transform: Affine,
                _clip: &impl Shape,
            ) {
            }
            fn push_clip_layer(&mut self, _transform: Affine, _clip: &impl Shape) {}
            fn pop_layer(&mut self) {}
            fn stroke<'a>(
                &mut self,
                _style: &kurbo::Stroke,
                _transform: Affine,
                _brush: impl Into<anyrender::PaintRef<'a>>,
                _brush_transform: Option<Affine>,
                _shape: &impl Shape,
            ) {
            }
            fn fill<'a>(
                &mut self,
                _style: peniko::Fill,
                _transform: Affine,
                _brush: impl Into<anyrender::PaintRef<'a>>,
                _brush_transform: Option<Affine>,
                _shape: &impl Shape,
            ) {
            }
            fn draw_glyphs<'a, 's: 'a>(
                &'s mut self,
                _font: &'a peniko::FontData,
                _font_size: f32,
                _hint: bool,
                _normalized_coords: &'a [anyrender::NormalizedCoord],
                _style: impl Into<peniko::StyleRef<'a>>,
                _brush: impl Into<anyrender::PaintRef<'a>>,
                _brush_alpha: f32,
                _transform: Affine,
                _glyph_transform: Option<Affine>,
                _glyphs: impl Iterator<Item = Glyph>,
            ) {
                self.glyph_calls += 1;
            }
            fn draw_box_shadow(
                &mut self,
                _transform: Affine,
                _rect: kurbo::Rect,
                _brush: peniko::Color,
                _radius: f64,
                _std_dev: f64,
            ) {
            }
            fn draw_image(&mut self, _image: peniko::ImageBrushRef, _transform: Affine) {}
        }

        let mut painter = CountingPainter { glyph_calls: 0 };
        frame.replay(&mut painter, Affine::IDENTITY, &font_cache);
        assert_eq!(painter.glyph_calls, 1);
    }
}
