use std::cell::RefCell;
use html5ever::tendril::StrTendril;
use super::font::FontManager;
use crate::css::ComputedValues;
use crate::layout::LayoutBox;
// Text rendering functionality
use skia_safe::{BlurStyle, Canvas, Color, ColorSpace, Font, FontArguments, FontHinting, FontMgr, GlyphId, MaskFilter, Paint, PaintCap, PaintJoin, PaintStyle, Point, RRect, Rect, Shader, TextBlob, Typeface};
use skia_safe::canvas::{GlyphPositions, SaveLayerRec, SrcRectConstraint};
use skia_safe::font::Edging;
use skia_safe::font_arguments::variation_position::Coordinate;
use skia_safe::font_arguments::VariationPosition;
use crate::dom::{Dom, DomNode};
use crate::renderer::cache::{FontCacheKey, FontCacheKeyBorrowed, GenerationalCache, NormalizedTypefaceCacheKey, NormalizedTypefaceCacheKeyBorrowed};
use parley::{Alignment, AlignmentOptions, FontWeight, GenericFamily, LineHeight, PositionedLayoutItem, StyleProperty};
use color::{AlphaColor, Srgb};
use kurbo::{Affine, Stroke};
use peniko::{Fill, ImageBrushRef};
use style::color::AbsoluteColor;
use style::values::specified::TextDecorationLine;

/// Render text with CSS styles applied and DPI scale factor using Skia's textlayout
pub fn render_text_node(
    canvas: &Canvas,
    node: &DomNode,
    dom: &Dom,
    layout_box: &LayoutBox,
    contents: &RefCell<StrTendril>,
    styles: &ComputedValues,
    font_manager: &FontManager,
    default_text_paint: &Paint,
    painter: &mut TextPainter,
    scale_factor: f32,
) {
    let text = contents.borrow();
    let content_rect = layout_box.dimensions.content;

    let mut font_ctx = dom.font_ctx.lock().unwrap();
    let mut layout_ctx = dom.layout_ctx.lock().unwrap();

    // Apply text transformation to the content
    let transformed_text = styles.text_transform.apply(&text);

    let mut builder = layout_ctx.ranged_builder(
        &mut font_ctx,
        &transformed_text,
        scale_factor,
        true,
    );

    // Extract CSS properties
    let font_size = styles.font_size;
    let text_align = &styles.text_align;
    let font_weight_str = &styles.font_weight;
    let line_height_value = &styles.line_height;

    // Set default font family - parse comma-separated list
    let font_families: Vec<&str> = styles.font_family
        .split(',')
        .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\''))
        .collect();
    
    // Use first font family or default to system UI
    if let Some(first_family) = font_families.first() {
        let generic_family = match first_family.to_lowercase().as_str() {
            "serif" => GenericFamily::Serif,
            "sans-serif" => GenericFamily::SansSerif,
            "monospace" => GenericFamily::Monospace,
            "cursive" => GenericFamily::Cursive,
            "fantasy" => GenericFamily::Fantasy,
            "system-ui" | "-apple-system" | "blinkmacsystemfont" => GenericFamily::SystemUi,
            _ => GenericFamily::SystemUi,
        };
        builder.push_default(generic_family);
    } else {
        builder.push_default(GenericFamily::SystemUi);
    }

    // Set font size
    builder.push_default(StyleProperty::FontSize(font_size));

    // Set font weight
    let font_weight = match font_weight_str.to_lowercase().as_str() {
        "normal" => FontWeight::new(400.0),
        "bold" => FontWeight::new(700.0),
        "bolder" => FontWeight::new(900.0),
        "lighter" => FontWeight::new(300.0),
        weight_str => {
            if let Ok(weight_num) = weight_str.parse::<f32>() {
                FontWeight::new(weight_num)
            } else {
                FontWeight::new(400.0)
            }
        }
    };
    builder.push_default(StyleProperty::FontWeight(font_weight));

    // Set line height
    let scaled_font_size = font_size * scale_factor;
    let line_height_ratio = line_height_value.to_px(scaled_font_size) / scaled_font_size;
    builder.push_default(LineHeight::FontSizeRelative(line_height_ratio));

    // Build the layout
    let mut layout = builder.build(&transformed_text);

    // Break lines based on content width
    let max_width = if content_rect.width() > 0.0 {
        Some(content_rect.width())
    } else {
        None
    };
    layout.break_all_lines(max_width);

    // Set text alignment
    let alignment = match text_align {
        crate::css::TextAlign::Left => Alignment::Start,
        crate::css::TextAlign::Right => Alignment::End,
        crate::css::TextAlign::Center => Alignment::Center,
        crate::css::TextAlign::Justify => Alignment::Justify,
    };
    layout.align(max_width, alignment, AlignmentOptions::default());

    // Get text color
    let text_color: AlphaColor<Srgb> = if let Some(css_color) = &styles.color {
        let skia_color = css_color.to_skia_color();
        let r = skia_color.r() as f32 / 255.0;
        let g = skia_color.g() as f32 / 255.0;
        let b = skia_color.b() as f32 / 255.0;
        let a = (skia_color.a() as f32 / 255.0) * styles.opacity;
        AlphaColor::new([r, g, b, a])
    } else {
        AlphaColor::new([0.0, 0.0, 0.0, styles.opacity])
    };

    // Create transform for text position
    let transform = Affine::translate((content_rect.left as f64, content_rect.top as f64));

    // Render each line
    for line in layout.lines() {
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let mut run_x = glyph_run.offset();
                    let run_y = glyph_run.baseline();

                    let run = glyph_run.run();
                    let font = run.font();
                    let font_size = run.font_size();
                    let metrics = run.metrics();
                    let style = glyph_run.style();
                    let synthesis = run.synthesis();
                    let glyph_xform = synthesis.skew().map(|angle| {
                        Affine::skew(angle.to_radians().tan() as f64, 0.0)
                    });

                    // style
                    let style= dom.get_node(style.brush.id).unwrap().primary_styles().unwrap();
                    let itext_style = style.get_inherited_text();
                    let text_style = style.get_text();
                    let text_color = itext_style.color.as_color_color();
                    let text_decoration_color = text_style
                        .text_decoration_color
                        .as_absolute()
                        .map(ToColorColor::as_color_color)
                        .unwrap_or(text_color);
                    let text_decoration_brush = anyrender::Paint::from(text_decoration_color);
                    let text_decoration_line = text_style.text_decoration_line;
                    let has_underline = text_decoration_line.contains(TextDecorationLine::UNDERLINE);
                    let has_strikethrough =
                        text_decoration_line.contains(TextDecorationLine::LINE_THROUGH);

                    painter.draw_glyphs(
                        font,
                        font_size,
                        true,
                        run.normalized_coords(),
                        Fill::NonZero,
                        &anyrender::Paint::from(text_color),
                        1.0,
                        transform,
                        glyph_xform,
                        glyph_run.glyphs().map(|glyph| {
                            let gx = run_x + glyph.x;
                            let gy = run_y - glyph.y;
                            run_x += glyph.advance;

                            anyrender::Glyph {
                                id: glyph.id as _,
                                x: gx,
                                y: gy,
                            }
                        })
                    );

                    let mut draw_decoration_line =
                        |offset: f32, size: f32, brush: &anyrender::Paint| {
                            let x = glyph_run.offset() as f64;
                            let w = glyph_run.advance() as f64;
                            let y = (glyph_run.baseline() - offset + size / 2.0) as f64;
                            let line = kurbo::Line::new((x, y), (x + w, y));
                            painter.stroke(&Stroke::new(size as f64), transform, brush, None, &line)
                        };

                    if has_underline {
                        let offset = metrics.underline_offset;
                        let size = metrics.underline_size;

                        // TODO: intercept line when crossing an descending character like "gqy"
                        draw_decoration_line(offset, size, &text_decoration_brush);
                    }
                    if has_strikethrough {
                        let offset = metrics.strikethrough_offset;
                        let size = metrics.strikethrough_size;

                        draw_decoration_line(offset, size, &text_decoration_brush);
                    }
                }
                PositionedLayoutItem::InlineBox(_) => {
                    // Inline boxes are not rendered in this context
                }
            }
        }
    }
}

// Under this line, anyrender_skia is referenced to create a text renderer
// Copyright DioxusLabs
// Licensed under the Apache License, Version 2.0 or the MIT license.

pub(crate) struct SkiaCache {
    paint: Paint,
    typeface: GenerationalCache<(u64, u32), Typeface>,
    normalized_typeface: GenerationalCache<NormalizedTypefaceCacheKey, Typeface>,
    image_shader: GenerationalCache<u64, Shader>,
    font: GenerationalCache<FontCacheKey, Font>,
    font_mgr: FontMgr,
    glyph_id_buf: Vec<GlyphId>,
    glyph_pos_buf: Vec<Point>,
}

impl SkiaCache {
    pub(crate) fn next_gen(&mut self) {
        self.typeface.next_gen();
        self.normalized_typeface.next_gen();
        self.image_shader.next_gen();
        self.font.next_gen();
    }
}

impl Default for SkiaCache {
    fn default() -> Self {
        Self {
            paint: Paint::default(),
            typeface: GenerationalCache::new(1),
            normalized_typeface: GenerationalCache::new(1),
            image_shader: GenerationalCache::new(1),
            font: GenerationalCache::new(10),
            font_mgr: FontMgr::default(),
            glyph_id_buf: Default::default(),
            glyph_pos_buf: Default::default(),
        }
    }
}

pub(crate) struct TextPainter<'a> {
    pub(crate) inner: &'a Canvas,
    pub(crate) cache: &'a mut SkiaCache,
}

impl TextPainter<'_> {
    pub(crate) fn reset_paint(&mut self) {
        self.cache.paint.reset();
        self.cache.paint.set_anti_alias(true);
    }

    pub(crate) fn set_paint_alpha(&mut self, alpha: f32) {
        self.cache.paint.set_alpha_f(alpha);
    }

    pub(crate) fn set_paint_blend_mode(&mut self, blend_mode: impl Into<peniko::BlendMode>) {
        self.cache
            .paint
            .set_blend_mode(sk_peniko::blend_mode_from(blend_mode.into()));
    }

    pub(crate) fn set_matrix(&self, transform: kurbo::Affine) {
        self.inner.set_matrix(&sk_kurbo::m44_from_affine(transform));
    }

    pub(crate) fn concat_matrix(&self, transform: kurbo::Affine) {
        self.inner.concat(&sk_kurbo::matrix_from_affine(transform));
    }

    pub(crate) fn clip(&self, shape: &impl kurbo::Shape) {
        if let Some(rect) = shape.as_rect() {
            self.inner.clip_rect(sk_kurbo::rect_from(rect), None, true);
        } else if let Some(rrect) = shape.as_rounded_rect() {
            self.inner
                .clip_rrect(sk_kurbo::rrect_from(rrect), None, true);
        } else {
            self.inner
                .clip_path(&sk_kurbo::path_from_shape(shape), None, true);
        }
    }

    pub(crate) fn set_paint_brush<'a>(
        &mut self,
        brush: impl Into<anyrender::PaintRef<'a>>,
        brush_transform: Option<kurbo::Affine>,
    ) {
        let brush: anyrender::PaintRef<'a> = brush.into();
        match brush {
            anyrender::Paint::Solid(alpha_color) => {
                self.cache.paint.set_color4f(
                    sk_peniko::color4f_from_alpha_color(alpha_color),
                    &ColorSpace::new_srgb(),
                );
            }
            anyrender::Paint::Gradient(gradient) => {
                self.cache
                    .paint
                    .set_shader(sk_peniko::shader_from_gradient(gradient, brush_transform));
            }
            anyrender::Paint::Image(image_brush) => {
                if let Some(shader) = self.cache.image_shader.hit(&image_brush.image.data.id()) {
                    self.cache.paint.set_shader(shader.clone());
                    return;
                }

                let image_shader = sk_peniko::shader_from_image_brush(image_brush, brush_transform);

                if let Some(shader) = &image_shader {
                    self.cache
                        .image_shader
                        .insert(image_brush.image.data.id(), shader.clone());
                }

                self.cache.paint.set_shader(image_shader);
            }
            anyrender::Paint::Custom(_) => unreachable!(), // ToDo: figure out what to do with this
        }
    }

    pub(crate) fn set_paint_style<'a>(&mut self, style: impl Into<peniko::StyleRef<'a>>) {
        match style.into() {
            peniko::StyleRef::Fill(_) => {
                self.cache.paint.set_style(PaintStyle::Fill);
            }
            peniko::StyleRef::Stroke(stroke) => {
                self.cache.paint.set_style(PaintStyle::Stroke);
                self.cache.paint.set_stroke(true);
                self.cache.paint.set_stroke_width(stroke.width as f32);
                self.cache.paint.set_stroke_join(match stroke.join {
                    kurbo::Join::Bevel => PaintJoin::Bevel,
                    kurbo::Join::Miter => PaintJoin::Miter,
                    kurbo::Join::Round => PaintJoin::Round,
                });
                self.cache.paint.set_stroke_cap(match stroke.start_cap {
                    kurbo::Cap::Butt => PaintCap::Butt,
                    kurbo::Cap::Square => PaintCap::Square,
                    kurbo::Cap::Round => PaintCap::Round,
                });
            }
        }
    }

    pub(crate) fn draw_shape(&mut self, shape: &impl kurbo::Shape) {
        self.draw_shape_with_fill(shape, None);
    }

    pub(crate) fn draw_shape_with_fill(
        &mut self,
        shape: &impl kurbo::Shape,
        fill: impl Into<Option<peniko::Fill>>,
    ) {
        if let Some(rect) = shape.as_rect() {
            self.inner
                .draw_rect(sk_kurbo::rect_from(rect), &self.cache.paint);
        } else if let Some(rrect) = shape.as_rounded_rect() {
            self.inner
                .draw_rrect(sk_kurbo::rrect_from(rrect), &self.cache.paint);
        } else if let Some(line) = shape.as_line() {
            self.inner.draw_line(
                (line.p0.x as f32, line.p0.y as f32),
                (line.p1.x as f32, line.p1.y as f32),
                &self.cache.paint,
            );
        } else if let Some(circle) = shape.as_circle() {
            self.inner.draw_circle(
                (circle.center.x as f32, circle.center.y as f32),
                circle.radius as f32,
                &self.cache.paint,
            );
        } else {
            let mut path = sk_kurbo::path_from_shape(shape);
            if let Some(fill) = fill.into() {
                path.set_fill_type(sk_peniko::path_fill_type_from_fill(fill));
            }
            self.inner.draw_path(&path, &self.cache.paint);
        }
    }

    pub(crate) fn get_or_cache_font(
        &mut self,
        font: &peniko::FontData,
        normalized_coords: &[anyrender::NormalizedCoord],
        font_size: f32,
        hint: bool,
    ) -> Option<Font> {
        let cache_key_borrowed = FontCacheKeyBorrowed {
            typeface_id: font.data.id(),
            typeface_index: font.index,
            normalized_coords,
            font_size: font_size.to_bits(),
            hint,
        };

        if let Some(cached) = self.cache.font.hit(&cache_key_borrowed) {
            return Some(cached.clone());
        }

        let typeface = self.get_or_cache_normalized_typeface(font, normalized_coords)?;

        let cache_key = FontCacheKey {
            typeface_id: font.data.id(),
            typeface_index: font.index,
            normalized_coords: normalized_coords.to_vec(),
            font_size: font_size.to_bits(),
            hint,
        };

        let mut font = Font::from_typeface(typeface, font_size);
        font.set_hinting(if hint {
            FontHinting::Normal
        } else {
            FontHinting::None
        });
        font.set_edging(Edging::SubpixelAntiAlias);

        self.cache.font.insert(cache_key, font.clone());

        Some(font)
    }

    pub(crate) fn get_or_cache_normalized_typeface(
        &mut self,
        font: &peniko::FontData,
        normalized_coords: &[anyrender::NormalizedCoord],
    ) -> Option<Typeface> {
        fn f2dot14_to_f32(raw_value: i16) -> f32 {
            let int = (raw_value >> 14) as f32;
            let fract = (raw_value & !(!0 << 14)) as f32 / (1 << 14) as f32;
            int + fract
        }

        if normalized_coords.is_empty() {
            return self.get_or_cache_typeface(font);
        }

        let cache_key_borrowed = NormalizedTypefaceCacheKeyBorrowed {
            typeface_id: font.data.id(),
            typeface_index: font.index,
            normalized_coords,
        };

        if let Some(cached) = self.cache.normalized_typeface.hit(&cache_key_borrowed) {
            return Some(cached.clone());
        }

        let typeface = self.get_or_cache_typeface(font)?;

        let axes = typeface.variation_design_parameters().unwrap_or_default();

        if axes.is_empty() {
            return Some(typeface);
        }

        let coordinates: Vec<Coordinate> = axes
            .iter()
            .zip(normalized_coords.iter().map(|c| f2dot14_to_f32(*c)))
            .filter(|(_, value)| *value != 0.0)
            .map(|(axis, factor)| {
                let value = if factor < 0.0 {
                    lerp_f32(axis.min, axis.def, -factor)
                } else {
                    lerp_f32(axis.def, axis.max, factor)
                };

                Coordinate {
                    axis: axis.tag,
                    value,
                }
            })
            .collect();
        let variation_position = VariationPosition {
            coordinates: &coordinates,
        };

        let normalized_typeface = typeface
            .clone_with_arguments(
                &FontArguments::new().set_variation_design_position(variation_position),
            )
            .unwrap();

        self.cache.normalized_typeface.insert(
            NormalizedTypefaceCacheKey {
                typeface_id: font.data.id(),
                typeface_index: font.index,
                normalized_coords: normalized_coords.to_vec(),
            },
            normalized_typeface.clone(),
        );

        Some(normalized_typeface)
    }

    pub(crate) fn get_or_cache_typeface<'a>(
        &'a mut self,
        #[allow(unused_mut)] mut font: &'a peniko::FontData,
    ) -> Option<Typeface> {
        let cache_key = (font.data.id(), font.index);

        #[cfg(target_os = "macos")]
        #[allow(clippy::map_entry, reason = "Cannot early-return with entry API")]
        {
            use peniko::Blob;
            use std::sync::Arc;

            if let Some(collection) = oaty::Collection::new(font.data.data()) {
                if !self.cache.extracted_font_data.contains_key(&cache_key) {
                    let Some(data) = collection
                        .get_font(font.index)
                        .and_then(|font| font.copy_data())
                    else {
                        eprintln!(
                            "WARNING: failed to extract font {} {}",
                            cache_key.0, cache_key.1
                        );
                        return None;
                    };

                    let blob = Blob::new(Arc::new(data));
                    let font_data = peniko::FontData::new(blob, 0);
                    self.cache.extracted_font_data.insert(cache_key, font_data);
                }
                font = self.cache.extracted_font_data.hit(&cache_key).unwrap()
            }
        }

        if let Some(cached) = self.cache.typeface.hit(&cache_key) {
            return Some(cached.clone());
        }

        let Some(typeface) = self
            .cache
            .font_mgr
            .new_from_data(font.data.data(), font.index as usize)
        else {
            let tf = Typeface::make_deserialize(font.data.data(), None);
            eprintln!(
                "WARNING: failed to load font {} {} {} {}",
                cache_key.0,
                cache_key.1,
                tf.is_some(),
                font.index
            );
            return None;
        };

        self.cache.typeface.insert(cache_key, typeface.clone());

        Some(typeface)
    }
}

// painter
impl TextPainter<'_> {
    pub(crate) fn reset(&mut self) {
        self.inner.clear(Color::WHITE);
    }

    pub(crate) fn push_layer(
        &mut self,
        blend: impl Into<peniko::BlendMode>,
        alpha: f32,
        transform: kurbo::Affine,
        clip: &impl kurbo::Shape,
    ) {
        let blend: peniko::BlendMode = blend.into();

        self.reset_paint();
        self.set_paint_alpha(alpha);
        self.set_paint_blend_mode(blend);

        self.inner.save();

        self.set_matrix(transform);
        self.clip(clip);

        #[allow(deprecated)] // Mix::Clip
        if blend.mix == peniko::Mix::Clip && alpha == 1f32 {
            self.inner.save();
        } else {
            self.inner
                .save_layer(&SaveLayerRec::default().paint(&self.cache.paint));
        }
    }

    pub(crate) fn push_clip_layer(&mut self, transform: kurbo::Affine, clip: &impl kurbo::Shape) {
        self.inner.save(); // we need to do two saves because of pop_layer

        self.set_matrix(transform);
        self.clip(clip);
        self.inner.save();
    }

    pub(crate) fn pop_layer(&mut self) {
        self.inner.restore();
        self.inner.restore();
    }

    pub(crate) fn stroke<'a>(
        &mut self,
        style: &kurbo::Stroke,
        transform: kurbo::Affine,
        brush: impl Into<anyrender::PaintRef<'a>>,
        brush_transform: Option<kurbo::Affine>,
        shape: &impl kurbo::Shape,
    ) {
        self.set_matrix(transform);

        self.reset_paint();
        self.set_paint_brush(brush, brush_transform);
        self.set_paint_style(style);
        self.draw_shape(shape);
    }

    pub(crate) fn fill<'a>(
        &mut self,
        style: peniko::Fill,
        transform: kurbo::Affine,
        brush: impl Into<anyrender::PaintRef<'a>>,
        brush_transform: Option<kurbo::Affine>,
        shape: &impl kurbo::Shape,
    ) {
        self.set_matrix(transform);

        self.reset_paint();
        self.set_paint_brush(brush, brush_transform);
        self.set_paint_style(style);
        self.draw_shape_with_fill(shape, style);
    }

    pub(crate) fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        #[allow(unused_mut)] mut font: &'a peniko::FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [anyrender::NormalizedCoord],
        style: impl Into<peniko::StyleRef<'a>>,
        brush: impl Into<anyrender::PaintRef<'a>>,
        brush_alpha: f32,
        transform: kurbo::Affine,
        glyph_transform: Option<kurbo::Affine>,
        glyphs: impl Iterator<Item = anyrender::Glyph>,
    ) {
        self.set_matrix(transform);

        if let Some(glyph_transform) = glyph_transform {
            self.concat_matrix(glyph_transform);
        }

        self.reset_paint();
        self.set_paint_brush(brush, None);
        self.set_paint_style(style);
        self.set_paint_alpha(brush_alpha);

        let Some(font) = self.get_or_cache_font(font, normalized_coords, font_size, hint) else {
            return;
        };

        let (min_size, _) = glyphs.size_hint();
        self.cache.glyph_id_buf.reserve(min_size);
        self.cache.glyph_pos_buf.reserve(min_size);

        for glyph in glyphs {
            self.cache.glyph_id_buf.push(GlyphId::from(glyph.id as u16));
            self.cache.glyph_pos_buf.push(Point::new(glyph.x, glyph.y));
        }

        self.inner.draw_glyphs_at(
            &self.cache.glyph_id_buf[..],
            GlyphPositions::Points(&self.cache.glyph_pos_buf[..]),
            Point::new(0.0, 0.0),
            &font,
            &self.cache.paint,
        );

        self.cache.glyph_id_buf.clear();
        self.cache.glyph_pos_buf.clear();
    }

    pub(crate) fn draw_box_shadow(
        &mut self,
        transform: kurbo::Affine,
        rect: kurbo::Rect,
        brush: peniko::Color,
        radius: f64,
        std_dev: f64,
    ) {
        self.set_matrix(transform);

        self.reset_paint();
        self.set_paint_brush(brush, None);
        self.cache.paint.set_style(PaintStyle::Fill);

        if std_dev > 0.0 {
            self.cache.paint.set_mask_filter(
                MaskFilter::blur(BlurStyle::Normal, std_dev as f32, false).unwrap(),
            );
        }

        let rrect = RRect::new_nine_patch(
            Rect::new(
                rect.x0 as f32,
                rect.y0 as f32,
                rect.x1 as f32,
                rect.y1 as f32,
            ),
            radius as f32,
            radius as f32,
            radius as f32,
            radius as f32,
        );

        self.inner.draw_rrect(rrect, &self.cache.paint);
    }

    // Canvas wrapper methods for general rendering

    pub(crate) fn save(&self) {
        self.inner.save();
    }

    pub(crate) fn restore(&self) {
        self.inner.restore();
    }

    pub(crate) fn translate(&self, dx: f32, dy: f32) {
        self.inner.translate((dx, dy));
    }

    pub(crate) fn base_layer_size(&self) -> skia_safe::ISize {
        self.inner.base_layer_size()
    }

    pub(crate) fn draw_rect(&self, rect: Rect, paint: &Paint) {
        self.inner.draw_rect(rect, paint);
    }

    pub(crate) fn draw_line(&self, p0: impl Into<Point>, p1: impl Into<Point>, paint: &Paint) {
        self.inner.draw_line(p0, p1, paint);
    }

    pub(crate) fn draw_path(&self, path: &skia_safe::Path, paint: &Paint) {
        self.inner.draw_path(path, paint);
    }

    pub(crate) fn draw_text_blob(&self, blob: &TextBlob, origin: impl Into<Point>, paint: &Paint) {
        self.inner.draw_text_blob(blob, origin, paint);
    }

    pub(crate) fn draw_image(&mut self, image: ImageBrushRef, transform: Affine) {
        self.fill(
            Fill::NonZero,
            transform,
            image,
            None,
            &kurbo::Rect::new(
                0.0,
                0.0,
                image.image.width as f64,
                image.image.height as f64,
            ),
        );
    }
}

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

mod sk_peniko {
    use peniko::color::{AlphaColor, ColorSpaceTag, HueDirection, Srgb};
    use peniko::{
        BlendMode, Compose, Extend, Gradient, GradientKind, ImageAlphaType, ImageBrush, ImageData,
        ImageFormat, Mix,
    };
    use peniko::{Fill, color::DynamicColor};
    use skia_safe::AlphaType as SkAlphaType;
    use skia_safe::BlendMode as SkBlendMode;
    use skia_safe::Color4f as SkColor4f;
    use skia_safe::ColorType as SkColorType;
    use skia_safe::Data as SkData;
    use skia_safe::ImageInfo as SkImageInfo;
    use skia_safe::PathFillType as SkPathFillType;
    use skia_safe::SamplingOptions as SkSamplingOptions;
    use skia_safe::Shader as SkShader;
    use skia_safe::TileMode as SkTileMode;
    use skia_safe::gradient_shader::interpolation::ColorSpace as SkGradientShaderColorSpace;
    use skia_safe::gradient_shader::interpolation::HueMethod as SkGradientShaderHueMethod;

    pub(super) fn shader_from_image_brush(
        image_brush: ImageBrush<&ImageData>,
        brush_transform: Option<kurbo::Affine>,
    ) -> Option<SkShader> {
        let image_data = image_brush.image;

        let image_info = SkImageInfo::new(
            (image_data.width as i32, image_data.height as i32),
            match image_data.format {
                ImageFormat::Rgba8 => SkColorType::RGBA8888,
                ImageFormat::Bgra8 => SkColorType::BGRA8888,
                _ => unreachable!(),
            },
            match image_data.alpha_type {
                ImageAlphaType::Alpha => SkAlphaType::Unpremul,
                ImageAlphaType::AlphaPremultiplied => SkAlphaType::Premul,
            },
            None,
        );
        let pixels = unsafe {
            SkData::new_bytes(image_data.data.data()) // We have to ensure the src image data lives long enough
        };
        let image = skia_safe::image::Image::from_encoded(pixels).unwrap();
        //let image =
        //    skia_safe::images::raster_from_data(&image_info, pixels, image_info.min_row_bytes())?;

        let sampling = match image_brush.sampler.quality {
            peniko::ImageQuality::Low => {
                SkSamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None)
            }
            peniko::ImageQuality::Medium => SkSamplingOptions::new(
                skia_safe::FilterMode::Linear,
                skia_safe::MipmapMode::Nearest,
            ),
            peniko::ImageQuality::High => {
                SkSamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::Linear)
            }
        };

        skia_safe::shaders::image(
            image,
            (
                tile_mode_from_extend(image_brush.sampler.x_extend),
                tile_mode_from_extend(image_brush.sampler.y_extend),
            ),
            &sampling,
            &brush_transform.map(super::sk_kurbo::matrix_from_affine),
        )
    }

    pub(super) fn shader_from_gradient(
        gradient: &Gradient,
        brush_transform: Option<kurbo::Affine>,
    ) -> SkShader {
        fn rad_to_deg(rad: f32) -> f32 {
            if rad == 0.0 {
                return 0.0;
            }

            rad * 180.0 / std::f32::consts::PI
        }

        match gradient.kind {
            GradientKind::Linear(linear_gradient_position) => {
                let mut colors: Vec<SkColor4f> = vec![];
                let mut positions: Vec<f32> = vec![];

                for color_stop in gradient.stops.iter() {
                    colors.push(color4f_from_dynamic_color(color_stop.color));
                    positions.push(color_stop.offset);
                }
                let start = super::sk_kurbo::pt_from(linear_gradient_position.start);
                let end = super::sk_kurbo::pt_from(linear_gradient_position.end);

                let interpolation = skia_safe::gradient_shader::Interpolation {
                    color_space: gradient_shader_cs_from_cs_tag(gradient.interpolation_cs),
                    in_premul: skia_safe::gradient_shader::interpolation::InPremul::Yes,
                    hue_method: gradient_shader_hue_method_from_hue_direction(
                        gradient.hue_direction,
                    ),
                };

                SkShader::linear_gradient_with_interpolation(
                    (start, end),
                    (&colors[..], None),
                    &positions[..],
                    tile_mode_from_extend(gradient.extend),
                    interpolation,
                    &brush_transform.map(super::sk_kurbo::matrix_from_affine),
                )
                    .unwrap()
            }
            GradientKind::Radial(radial_gradient_position) => {
                let mut colors: Vec<SkColor4f> = vec![];
                let mut positions: Vec<f32> = vec![];

                for color_stop in gradient.stops.iter() {
                    colors.push(color4f_from_dynamic_color(color_stop.color));
                    positions.push(color_stop.offset);
                }

                let start_center = super::sk_kurbo::pt_from(radial_gradient_position.start_center);
                let start_radius = radial_gradient_position.start_radius;
                let end_center = super::sk_kurbo::pt_from(radial_gradient_position.end_center);
                let end_radius = radial_gradient_position.end_radius;

                let interpolation = skia_safe::gradient_shader::Interpolation {
                    color_space: gradient_shader_cs_from_cs_tag(gradient.interpolation_cs),
                    in_premul: skia_safe::gradient_shader::interpolation::InPremul::Yes,
                    hue_method: gradient_shader_hue_method_from_hue_direction(
                        gradient.hue_direction,
                    ),
                };

                if start_center == end_center && start_radius == end_radius {
                    SkShader::radial_gradient_with_interpolation(
                        (start_center, start_radius),
                        (&colors[..], None),
                        &positions[..],
                        tile_mode_from_extend(gradient.extend),
                        interpolation,
                        &brush_transform.map(super::sk_kurbo::matrix_from_affine),
                    )
                        .unwrap()
                } else {
                    SkShader::two_point_conical_gradient_with_interpolation(
                        (start_center, start_radius),
                        (end_center, end_radius),
                        (&colors[..], None),
                        &positions[..],
                        tile_mode_from_extend(gradient.extend),
                        interpolation,
                        &brush_transform.map(super::sk_kurbo::matrix_from_affine),
                    )
                        .unwrap()
                }
            }
            GradientKind::Sweep(sweep_gradient_position) => {
                let mut colors: Vec<SkColor4f> = vec![];
                let mut positions: Vec<f32> = vec![];

                for color_stop in gradient.stops.iter() {
                    colors.push(color4f_from_dynamic_color(color_stop.color));
                    positions.push(color_stop.offset);
                }
                let center = super::sk_kurbo::pt_from(sweep_gradient_position.center);

                let interpolation = skia_safe::gradient_shader::Interpolation {
                    color_space: gradient_shader_cs_from_cs_tag(gradient.interpolation_cs),
                    in_premul: skia_safe::gradient_shader::interpolation::InPremul::Yes,
                    hue_method: gradient_shader_hue_method_from_hue_direction(
                        gradient.hue_direction,
                    ),
                };

                SkShader::sweep_gradient_with_interpolation(
                    center,
                    (&colors[..], None),
                    &positions[..],
                    tile_mode_from_extend(gradient.extend),
                    (
                        rad_to_deg(sweep_gradient_position.start_angle),
                        rad_to_deg(sweep_gradient_position.end_angle),
                    ),
                    interpolation,
                    &brush_transform.map(super::sk_kurbo::matrix_from_affine),
                )
                    .unwrap()
            }
        }
    }

    pub(super) fn path_fill_type_from_fill(fill: Fill) -> SkPathFillType {
        match fill {
            Fill::NonZero => SkPathFillType::Winding,
            Fill::EvenOdd => SkPathFillType::EvenOdd,
        }
    }

    pub(super) fn color4f_from_alpha_color(color: AlphaColor<Srgb>) -> SkColor4f {
        SkColor4f::new(
            color.components[0],
            color.components[1],
            color.components[2],
            color.components[3],
        )
    }

    pub(super) fn color4f_from_dynamic_color(color: DynamicColor) -> SkColor4f {
        SkColor4f::new(
            color.components[0],
            color.components[1],
            color.components[2],
            color.components[3],
        )
    }

    pub(super) fn gradient_shader_cs_from_cs_tag(
        color_space: ColorSpaceTag,
    ) -> SkGradientShaderColorSpace {
        match color_space {
            ColorSpaceTag::Srgb => SkGradientShaderColorSpace::SRGB,
            ColorSpaceTag::LinearSrgb => SkGradientShaderColorSpace::SRGBLinear,
            ColorSpaceTag::Lab => SkGradientShaderColorSpace::Lab,
            ColorSpaceTag::Lch => SkGradientShaderColorSpace::LCH,
            ColorSpaceTag::Hsl => SkGradientShaderColorSpace::HSL,
            ColorSpaceTag::Hwb => SkGradientShaderColorSpace::HWB,
            ColorSpaceTag::Oklab => SkGradientShaderColorSpace::OKLab,
            ColorSpaceTag::Oklch => SkGradientShaderColorSpace::OKLCH,
            ColorSpaceTag::DisplayP3 => SkGradientShaderColorSpace::DisplayP3,
            ColorSpaceTag::A98Rgb => SkGradientShaderColorSpace::A98RGB,
            ColorSpaceTag::ProphotoRgb => SkGradientShaderColorSpace::ProphotoRGB,
            ColorSpaceTag::Rec2020 => SkGradientShaderColorSpace::Rec2020,
            _ => SkGradientShaderColorSpace::SRGB, // ToDo: overview unsupported color space tags and possibly document it, for now just fallback
        }
    }

    pub(super) fn gradient_shader_hue_method_from_hue_direction(
        direction: HueDirection,
    ) -> SkGradientShaderHueMethod {
        match direction {
            HueDirection::Shorter => SkGradientShaderHueMethod::Shorter,
            HueDirection::Longer => SkGradientShaderHueMethod::Longer,
            HueDirection::Increasing => SkGradientShaderHueMethod::Increasing,
            HueDirection::Decreasing => SkGradientShaderHueMethod::Decreasing,
            _ => unreachable!(),
        }
    }

    pub(super) fn tile_mode_from_extend(extend: Extend) -> SkTileMode {
        match extend {
            Extend::Pad => SkTileMode::Clamp,
            Extend::Repeat => SkTileMode::Repeat,
            Extend::Reflect => SkTileMode::Mirror,
        }
    }

    #[allow(deprecated)] // We need to support Mix::Clip
    pub(super) fn blend_mode_from(blend_mode: BlendMode) -> SkBlendMode {
        if blend_mode.mix == Mix::Normal || blend_mode.mix == Mix::Clip {
            match blend_mode.compose {
                Compose::Clear => SkBlendMode::Clear,
                Compose::Copy => SkBlendMode::Src,
                Compose::Dest => SkBlendMode::Dst,
                Compose::SrcOver => SkBlendMode::SrcOver,
                Compose::DestOver => SkBlendMode::DstOver,
                Compose::SrcIn => SkBlendMode::SrcIn,
                Compose::DestIn => SkBlendMode::DstIn,
                Compose::SrcOut => SkBlendMode::SrcOut,
                Compose::DestOut => SkBlendMode::DstOut,
                Compose::SrcAtop => SkBlendMode::SrcATop,
                Compose::DestAtop => SkBlendMode::DstATop,
                Compose::Xor => SkBlendMode::Xor,
                Compose::Plus => SkBlendMode::Plus,
                Compose::PlusLighter => SkBlendMode::Plus,
            }
        } else {
            match blend_mode.mix {
                Mix::Normal => unreachable!(), // Handled above
                Mix::Multiply => SkBlendMode::Multiply,
                Mix::Screen => SkBlendMode::Screen,
                Mix::Overlay => SkBlendMode::Overlay,
                Mix::Darken => SkBlendMode::Darken,
                Mix::Lighten => SkBlendMode::Lighten,
                Mix::ColorDodge => SkBlendMode::ColorDodge,
                Mix::ColorBurn => SkBlendMode::ColorBurn,
                Mix::HardLight => SkBlendMode::HardLight,
                Mix::SoftLight => SkBlendMode::SoftLight,
                Mix::Difference => SkBlendMode::Difference,
                Mix::Exclusion => SkBlendMode::Exclusion,
                Mix::Hue => SkBlendMode::Hue,
                Mix::Saturation => SkBlendMode::Saturation,
                Mix::Color => SkBlendMode::Color,
                Mix::Luminosity => SkBlendMode::Luminosity,
                Mix::Clip => unreachable!(), // Handled above
            }
        }
    }
}

mod sk_kurbo {
    use kurbo::{Affine, PathEl, Point};
    use kurbo::{Rect, RoundedRect, Shape};
    use skia_safe::M44 as SkM44;
    use skia_safe::Matrix as SkMatrix;
    use skia_safe::Path as SkPath;
    use skia_safe::Point as SkPoint;
    use skia_safe::RRect as SkRRect;
    use skia_safe::Rect as SkRect;

    pub(super) fn rect_from(rect: Rect) -> SkRect {
        SkRect::new(
            rect.x0 as f32,
            rect.y0 as f32,
            rect.x1 as f32,
            rect.y1 as f32,
        )
    }

    pub(super) fn rrect_from(rrect: RoundedRect) -> SkRRect {
        let rect = rect_from(rrect.rect());
        SkRRect::new_nine_patch(
            rect,
            rrect.radii().bottom_left as f32,
            rrect.radii().top_left as f32,
            rrect.radii().top_right as f32,
            rrect.radii().bottom_right as f32,
        )
    }

    pub(super) fn m44_from_affine(affine: Affine) -> SkM44 {
        let m = affine.as_coeffs();
        let scale_x = m[0] as f32;
        let shear_y = m[1] as f32;
        let shear_x = m[2] as f32;
        let scale_y = m[3] as f32;
        let translate_x = m[4] as f32;
        let translate_y = m[5] as f32;

        SkM44::col_major(&[
            scale_x,
            shear_y,
            0.0,
            0.0,
            shear_x,
            scale_y,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            translate_x,
            translate_y,
            0.0,
            1.0,
        ])
    }

    pub(super) fn matrix_from_affine(affine: Affine) -> SkMatrix {
        let m = affine.as_coeffs();
        let scale_x = m[0] as f32;
        let shear_y = m[1] as f32;
        let shear_x = m[2] as f32;
        let scale_y = m[3] as f32;
        let translate_x = m[4] as f32;
        let translate_y = m[5] as f32;

        SkMatrix::new_all(
            scale_x,
            shear_x,
            translate_x,
            shear_y,
            scale_y,
            translate_y,
            0.0,
            0.0,
            1.0,
        )
    }

    pub(super) fn pt_from(p: Point) -> SkPoint {
        SkPoint::new(p.x as f32, p.y as f32)
    }

    pub(super) fn path_from_shape(shape: &impl Shape) -> SkPath {
        let mut sk_path = SkPath::new();

        if let Some(path_els) = shape.as_path_slice() {
            for path_el in path_els {
                append_path_el_to_sk_path(path_el, &mut sk_path);
            }
        } else {
            for path_el in shape.path_elements(0.1) {
                append_path_el_to_sk_path(&path_el, &mut sk_path);
            }
        }

        sk_path
    }

    fn append_path_el_to_sk_path(path_el: &PathEl, sk_path: &mut SkPath) {
        match path_el {
            PathEl::MoveTo(p) => _ = sk_path.move_to(pt_from(*p)),
            PathEl::LineTo(p) => _ = sk_path.line_to(pt_from(*p)),
            PathEl::QuadTo(p1, p2) => _ = sk_path.quad_to(pt_from(*p1), pt_from(*p2)),
            PathEl::CurveTo(p1, p2, p3) => {
                _ = sk_path.cubic_to(pt_from(*p1), pt_from(*p2), pt_from(*p3))
            }
            PathEl::ClosePath => _ = sk_path.close(),
        };
    }
}

pub trait ToColorColor {
    /// Converts a color into the `AlphaColor<Srgb>` type from the `color` crate
    fn as_color_color(&self) -> AlphaColor<Srgb>;
}
impl ToColorColor for AbsoluteColor {
    fn as_color_color(&self) -> AlphaColor<Srgb> {
        AlphaColor::new(
            *self
                .to_color_space(style::color::ColorSpace::Srgb)
                .raw_components(),
        )
    }
}
