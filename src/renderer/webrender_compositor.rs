use crate::display_list::{
    DisplayBrush, DisplayCommand, DisplayFont, DisplayPathEl, DisplayShape, DisplayStyle,
};
use crate::fragment_tree::{
    FragmentNode, FragmentNodeKind, FragmentTree,
};
use crate::window::Env;
use gleam::gl::GlFns;
use kurbo::{Affine, Point, Rect, Vec2};
use std::collections::HashMap;
use webrender::{
    create_webrender_instance, RenderApi, Renderer, Transaction, WebRenderOptions,
};
use webrender_api::units::{DeviceIntRect, DeviceIntSize, LayoutPoint, LayoutRect, LayoutSize};
use webrender_api::{
    ColorF, CommonItemProperties, DocumentId, Epoch, FontInstanceKey, FontKey, GlyphInstance,
    GlyphOptions, PipelineId, PrimitiveFlags, RenderNotifier, SpaceAndClipInfo,
};

const AFFINE_EPSILON: f64 = 0.0001;

#[derive(Default)]
struct NoopNotifier;

impl RenderNotifier for NoopNotifier {
    fn clone(&self) -> Box<dyn RenderNotifier> {
        Box::new(Self)
    }

    fn wake_up(&self, _composite_needed: bool) {}

    fn new_frame_ready(
        &self,
        _: DocumentId,
        _: webrender_api::FramePublishId,
        _: &webrender_api::FrameReadyParams,
    ) {
    }
}

pub(crate) struct WebRenderCompositor {
    renderer: Renderer,
    api: RenderApi,
    document_id: DocumentId,
    pipeline_id: PipelineId,
    next_epoch: u32,
    next_frame_id: u64,
    font_keys: HashMap<DisplayFont, FontKey>,
    font_instances: HashMap<FontInstanceCacheKey, FontInstanceKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FontInstanceCacheKey {
    font: DisplayFont,
    size_bits: u32,
    hint: bool,
}

impl WebRenderCompositor {
    pub(crate) fn new(env: &Env, device_size: DeviceIntSize) -> Result<Self, String> {
        let gl = unsafe { GlFns::load_with(|name| (env.gl_proc_resolver)(name)) };
        let (renderer, sender) = create_webrender_instance(
            gl,
            Box::new(NoopNotifier),
            WebRenderOptions {
                clear_color: ColorF::new(1.0, 1.0, 1.0, 1.0),
                ..Default::default()
            },
            None,
        )
        .map_err(|error| format!("failed to initialize webrender: {error:?}"))?;

        let api = sender.create_api();
        let document_id = api.add_document(device_size);

        Ok(Self {
            renderer,
            api,
            document_id,
            pipeline_id: PipelineId(0, 0),
            next_epoch: 1,
            next_frame_id: 1,
            font_keys: HashMap::new(),
            font_instances: HashMap::new(),
        })
    }

    pub(crate) fn render_fragment_tree(
        &mut self,
        tree: &FragmentTree,
        device_size: DeviceIntSize,
        page_origin: (f32, f32),
        page_size: (f32, f32),
    ) -> Result<bool, String> {
        let Some(_) = tree.get_node(tree.root_element_id) else {
            return Ok(false);
        };

        if tree.debug_hitboxes {
            return Ok(false);
        }

        let page_clip = LayoutRect::from_origin_and_size(
            LayoutPoint::new(page_origin.0, page_origin.1),
            LayoutSize::new(page_size.0, page_size.1),
        );
        let mut builder = webrender_api::DisplayListBuilder::new(self.pipeline_id);
        builder.begin();
        let mut txn = Transaction::new();

        let mut encoder = FragmentTreeWebRenderEncoder {
            tree,
            builder: &mut builder,
            txn: &mut txn,
            api: &self.api,
            page_clip,
            root_transform: Affine::translate((page_origin.0 as f64, page_origin.1 as f64)),
            root_space_and_clip: SpaceAndClipInfo::root_scroll(self.pipeline_id),
            font_keys: &mut self.font_keys,
            font_instances: &mut self.font_instances,
        };

        encoder.encode();

        let (_, display_list) = builder.end();
        let epoch = Epoch(self.next_epoch);
        self.next_epoch = self.next_epoch.wrapping_add(1).max(1);
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.wrapping_add(1).max(1);

        txn.set_document_view(DeviceIntRect::from_size(device_size));
        txn.set_root_pipeline(self.pipeline_id);
        txn.set_display_list(epoch, (self.pipeline_id, display_list));
        txn.generate_frame(frame_id, true, true, webrender_api::RenderReasons::empty());
        self.api.send_transaction(self.document_id, txn);

        self.renderer.update();
        self.renderer
            .render(device_size, 0)
            .map_err(|errors| format!("webrender render failed: {errors:?}"))?;

        Ok(true)
    }
}

struct FragmentTreeWebRenderEncoder<'a> {
    tree: &'a FragmentTree,
    builder: &'a mut webrender_api::DisplayListBuilder,
    txn: &'a mut Transaction,
    api: &'a RenderApi,
    page_clip: LayoutRect,
    root_transform: Affine,
    root_space_and_clip: SpaceAndClipInfo,
    font_keys: &'a mut HashMap<DisplayFont, FontKey>,
    font_instances: &'a mut HashMap<FontInstanceCacheKey, FontInstanceKey>,
}

impl FragmentTreeWebRenderEncoder<'_> {
    fn encode(&mut self) -> bool {
        if let Some(color) = self.tree.background_color {
            self.push_solid_rect(
                self.page_clip,
                self.page_clip,
                ColorF::new(color[0], color[1], color[2], color[3]),
            );
        }

        let scroll = self.tree.viewport_scroll;
        self.encode_element(
            self.tree.root_element_id,
            Point {
                x: -scroll.x,
                y: -scroll.y,
            },
        )
    }

    fn encode_element(&mut self, node_id: usize, location: Point) -> bool {
        let Some(node) = self.tree.get_node(node_id) else {
            return true;
        };

        if matches!(node.display, taffy::Display::None) {
            return true;
        }

        let Some(resolved_style) = node.resolved_style.as_ref() else {
            return true;
        };

        if let Some(element_data) = node.element_data.as_ref() {
            if element_data.is_hidden_input {
                return true;
            }
        }

        if !resolved_style.visibility_visible || resolved_style.opacity == 0.0 {
            return true;
        }

        let is_image = node
            .element_data
            .as_ref()
            .map(|e| e.raster_image.is_some())
            .unwrap_or(false);
        let is_text_input = node
            .element_data
            .as_ref()
            .map(|e| e.has_text_input)
            .unwrap_or(false);
        // Images and text inputs can't be rendered by WebRender yet — skip entirely.
        if is_image || is_text_input {
            return true;
        }

        let layout = node.final_layout.to_taffy();
        let position = location + Vec2::new(layout.location.x as f64, layout.location.y as f64);

        let taffy::Layout {
            size,
            content_size,
            ..
        } = layout;

        let scaled_y = position.y * self.tree.scale_factor;
        let scaled_content_height = content_size.height.max(size.height) as f64 * self.tree.scale_factor;
        if scaled_y > self.tree.height as f64 || scaled_y + scaled_content_height < 0.0 {
            return true;
        }

        let transform = self.root_transform
            * Affine::translate(position.to_vec2() * self.tree.scale_factor);
        self.encode_commands(self.tree.pre_layer_commands.get(&node_id), transform);
        self.encode_commands(self.tree.element_commands.get(&node_id), transform);

        let scrolled_position = Point {
            x: position.x - node.scroll_offset.x,
            y: position.y - node.scroll_offset.y,
        };
        let scroll_transform = self.root_transform
            * Affine::translate(scrolled_position.to_vec2() * self.tree.scale_factor);
        self.encode_commands(self.tree.content_commands.get(&node_id), scroll_transform);

        self.encode_children(node, scrolled_position)
    }

    fn encode_children(&mut self, node: &FragmentNode, position: Point) -> bool {
        if let Some(stacking_context) = node.stacking_context.as_ref() {
            for child in stacking_context.neg_z_hoisted_children() {
                let pos = Point {
                    x: position.x + child.position.x as f64,
                    y: position.y + child.position.y as f64,
                };
                self.encode_node(child.node_id, pos);
            }
        }

        if let Some(children) = node.paint_children.as_ref() {
            for &child_id in children {
                self.encode_node(child_id, position);
            }
        }

        if let Some(stacking_context) = node.stacking_context.as_ref() {
            for child in stacking_context.pos_z_hoisted_children() {
                let pos = Point {
                    x: position.x + child.position.x as f64,
                    y: position.y + child.position.y as f64,
                };
                self.encode_node(child.node_id, pos);
            }
        }

        true
    }

    fn encode_node(&mut self, node_id: usize, location: Point) -> bool {
        let Some(node) = self.tree.get_node(node_id) else {
            return true;
        };

        match node.node_kind {
            FragmentNodeKind::Element { .. } | FragmentNodeKind::AnonymousBlock => {
                self.encode_element(node_id, location)
            }
            FragmentNodeKind::Text
            | FragmentNodeKind::Document
            | FragmentNodeKind::ShadowRoot
            | FragmentNodeKind::Comment => true,
        }
    }

    fn encode_commands(
        &mut self,
        commands: Option<&Vec<DisplayCommand>>,
        base_transform: Affine,
    ) -> bool {
        let Some(commands) = commands else {
            return true;
        };

        for command in commands {
            if !self.encode_command(command, base_transform) {
                return false;
            }
        }

        true
    }

    fn encode_command(&mut self, command: &DisplayCommand, base_transform: Affine) -> bool {
        match command {
            DisplayCommand::Fill {
                brush,
                brush_transform,
                shape,
                transform,
                ..
            } => {
                let DisplayBrush::Solid(color) = brush else {
                    return true; // skip gradient/pattern fills
                };
                if brush_transform.is_some() {
                    return true; // skip brush transforms
                }
                let rect = match shape {
                    DisplayShape::Rect(rect) => Some(*rect),
                    DisplayShape::Path(path) => match classify_fill_path(path) {
                        FillPathSupport::NoOp => return true,
                        FillPathSupport::Rect(rect) => Some(rect),
                        FillPathSupport::Unsupported => None,
                    },
                    DisplayShape::RoundedRect(_) => None,
                };
                let Some(rect) = rect else {
                    return true; // skip non-rect shapes
                };
                let final_rect = match translated_rect(base_transform * *transform, rect) {
                    Some(rect) => rect,
                    None => return true, // skip non-translate transforms
                };
                let bounds = layout_rect(final_rect);
                self.push_solid_rect(
                    self.page_clip,
                    bounds,
                    ColorF::new(color[0], color[1], color[2], color[3]),
                );
                true
            }
            DisplayCommand::DrawGlyphs {
                font_id,
                font_size,
                hint,
                style,
                brush,
                brush_alpha,
                transform,
                glyph_transform,
                glyphs,
                ..
            } => {
                let DisplayStyle::Fill(_) = style else {
                    return true; // skip stroked glyphs
                };
                let DisplayBrush::Solid(color) = brush else {
                    return true; // skip gradient-colored glyphs
                };
                if glyph_transform.is_some() {
                    return true; // skip per-glyph transforms
                }

                let Some(font) = self.tree.fonts.get(*font_id as usize) else {
                    return true;
                };
                let Some(font_instance_key) = self.ensure_font_instance(font, *font_size, *hint) else {
                    return true;
                };

                let transform = base_transform * *transform;
                let [a, b, c, d, tx, ty] = transform.as_coeffs();
                if (a - 1.0).abs() > AFFINE_EPSILON
                    || b.abs() > AFFINE_EPSILON
                    || c.abs() > AFFINE_EPSILON
                    || (d - 1.0).abs() > AFFINE_EPSILON
                {
                    return true; // skip non-translate text transforms
                }

                let Some(bounds) = glyph_bounds(glyphs, tx, ty, *font_size) else {
                    return true;
                };
                let glyphs: Vec<GlyphInstance> = glyphs
                    .iter()
                    .map(|glyph| GlyphInstance {
                        index: glyph.id,
                        point: LayoutPoint::new(glyph.x + tx as f32, glyph.y + ty as f32),
                    })
                    .collect();

                let color = ColorF::new(color[0], color[1], color[2], color[3] * *brush_alpha);
                let common = CommonItemProperties {
                    clip_rect: self.page_clip,
                    clip_chain_id: self.root_space_and_clip.clip_chain_id,
                    spatial_id: self.root_space_and_clip.spatial_id,
                    flags: PrimitiveFlags::default(),
                };
                self.builder.push_text(
                    &common,
                    bounds,
                    &glyphs,
                    font_instance_key,
                    color,
                    Some(GlyphOptions::default()),
                );
                true
            }
            _ => true, // skip all other command types
        }
    }

    fn ensure_font_instance(
        &mut self,
        font: &DisplayFont,
        font_size: f32,
        hint: bool,
    ) -> Option<FontInstanceKey> {
        let key = FontInstanceCacheKey {
            font: font.clone(),
            size_bits: font_size.to_bits(),
            hint,
        };
        if let Some(&font_instance_key) = self.font_instances.get(&key) {
            return Some(font_instance_key);
        }

        let font_key = if let Some(&font_key) = self.font_keys.get(font) {
            font_key
        } else {
            let payload = self.tree.font_payloads.iter().find(|payload| payload.font == *font)?;
            let font_key = self.api.generate_font_key();
            self.txn.add_raw_font(font_key, payload.bytes.clone(), font.index);
            self.font_keys.insert(font.clone(), font_key);
            font_key
        };

        let font_instance_key = self.api.generate_font_instance_key();
        self.txn.add_font_instance(
            font_instance_key,
            font_key,
            font_size,
            None,
            None,
            Vec::new(),
        );
        self.font_instances.insert(key, font_instance_key);
        Some(font_instance_key)
    }

    fn push_solid_rect(&mut self, clip_rect: LayoutRect, bounds: LayoutRect, color: ColorF) {
        let common = CommonItemProperties {
            clip_rect,
            clip_chain_id: self.root_space_and_clip.clip_chain_id,
            spatial_id: self.root_space_and_clip.spatial_id,
            flags: PrimitiveFlags::default(),
        };
        self.builder.push_rect(&common, bounds, color);
    }
}

fn translated_rect(transform: Affine, rect: Rect) -> Option<Rect> {
    let [a, b, c, d, tx, ty] = transform.as_coeffs();
    if (a - 1.0).abs() > AFFINE_EPSILON
        || b.abs() > AFFINE_EPSILON
        || c.abs() > AFFINE_EPSILON
        || (d - 1.0).abs() > AFFINE_EPSILON
    {
        return None;
    }

    Some(Rect::new(rect.x0 + tx, rect.y0 + ty, rect.x1 + tx, rect.y1 + ty))
}

fn layout_rect(rect: Rect) -> LayoutRect {
    LayoutRect::from_origin_and_size(
        LayoutPoint::new(rect.x0 as f32, rect.y0 as f32),
        LayoutSize::new(rect.width() as f32, rect.height() as f32),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum FillPathSupport {
    NoOp,
    Rect(Rect),
    Unsupported,
}

fn classify_fill_path(elements: &[DisplayPathEl]) -> FillPathSupport {
    let mut supported_rect: Option<Rect> = None;
    let mut current_points: Vec<(f64, f64)> = Vec::new();

    let mut finish_subpath = |points: &mut Vec<(f64, f64)>| -> Option<FillPathSupport> {
        let classification = classify_fill_subpath(points);
        points.clear();

        match classification {
            FillPathSupport::NoOp => None,
            FillPathSupport::Rect(rect) => {
                if let Some(existing) = supported_rect {
                    if !same_rect(existing, rect) {
                        return Some(FillPathSupport::Unsupported);
                    }
                } else {
                    supported_rect = Some(rect);
                }
                None
            }
            FillPathSupport::Unsupported => Some(FillPathSupport::Unsupported),
        }
    };

    for element in elements {
        match *element {
            DisplayPathEl::MoveTo(point) => {
                if let Some(result) = finish_subpath(&mut current_points) {
                    return result;
                }
                current_points.push(point);
            }
            DisplayPathEl::LineTo(point) => {
                if let Some(&last) = current_points.last() {
                    if same_point(last, point) {
                        continue;
                    }
                    if !is_axis_aligned_segment(last, point) {
                        return FillPathSupport::Unsupported;
                    }
                }
                current_points.push(point);
            }
            DisplayPathEl::ClosePath => {
                if let Some(result) = finish_subpath(&mut current_points) {
                    return result;
                }
            }
            DisplayPathEl::QuadTo(..) | DisplayPathEl::CurveTo(..) => {
                return FillPathSupport::Unsupported;
            }
        }
    }

    if let Some(result) = finish_subpath(&mut current_points) {
        return result;
    }

    supported_rect.map(FillPathSupport::Rect).unwrap_or(FillPathSupport::NoOp)
}

fn classify_fill_subpath(points: &[(f64, f64)]) -> FillPathSupport {
    if points.len() < 2 {
        return FillPathSupport::NoOp;
    }

    let mut normalized: Vec<(f64, f64)> = Vec::with_capacity(points.len() + 1);
    for &point in points {
        if normalized.last().copied() != Some(point) {
            normalized.push(point);
        }
    }
    if normalized.len() < 2 {
        return FillPathSupport::NoOp;
    }

    let first = normalized[0];
    let last = *normalized.last().unwrap();
    if !same_point(first, last) {
        if !is_axis_aligned_segment(last, first) {
            return FillPathSupport::Unsupported;
        }
        normalized.push(first);
    }

    let area = signed_area(&normalized);
    if area.abs() <= AFFINE_EPSILON {
        return FillPathSupport::NoOp;
    }

    let (min_x, max_x) = normalized.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |acc, point| {
        (acc.0.min(point.0), acc.1.max(point.0))
    });
    let (min_y, max_y) = normalized.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |acc, point| {
        (acc.0.min(point.1), acc.1.max(point.1))
    });

    let width = max_x - min_x;
    let height = max_y - min_y;
    if width <= AFFINE_EPSILON || height <= AFFINE_EPSILON {
        return FillPathSupport::NoOp;
    }

    let bbox_area = width * height;
    if (area.abs() - bbox_area).abs() > AFFINE_EPSILON {
        return FillPathSupport::Unsupported;
    }

    if normalized.iter().any(|&(x, y)| !point_on_rect_edge((x, y), min_x, max_x, min_y, max_y)) {
        return FillPathSupport::Unsupported;
    }

    FillPathSupport::Rect(Rect::new(min_x, min_y, max_x, max_y))
}

fn signed_area(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|pair| pair[0].0 * pair[1].1 - pair[1].0 * pair[0].1)
        .sum::<f64>()
        * 0.5
}

fn point_on_rect_edge(point: (f64, f64), min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool {
    let (x, y) = point;
    if x < min_x - AFFINE_EPSILON
        || x > max_x + AFFINE_EPSILON
        || y < min_y - AFFINE_EPSILON
        || y > max_y + AFFINE_EPSILON
    {
        return false;
    }

    nearly_equal(x, min_x)
        || nearly_equal(x, max_x)
        || nearly_equal(y, min_y)
        || nearly_equal(y, max_y)
}

fn is_axis_aligned_segment(start: (f64, f64), end: (f64, f64)) -> bool {
    nearly_equal(start.0, end.0) || nearly_equal(start.1, end.1)
}

fn same_rect(a: Rect, b: Rect) -> bool {
    nearly_equal(a.x0, b.x0)
        && nearly_equal(a.y0, b.y0)
        && nearly_equal(a.x1, b.x1)
        && nearly_equal(a.y1, b.y1)
}

fn same_point(a: (f64, f64), b: (f64, f64)) -> bool {
    nearly_equal(a.0, b.0) && nearly_equal(a.1, b.1)
}

fn nearly_equal(a: f64, b: f64) -> bool {
    (a - b).abs() <= AFFINE_EPSILON
}

fn glyph_bounds(
    glyphs: &[crate::display_list::DisplayGlyph],
    tx: f64,
    ty: f64,
    font_size: f32,
) -> Option<LayoutRect> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for glyph in glyphs {
        min_x = min_x.min(glyph.x);
        min_y = min_y.min(glyph.y - font_size);
        max_x = max_x.max(glyph.x + font_size);
        max_y = max_y.max(glyph.y + font_size);
    }

    if !min_x.is_finite() {
        return None;
    }

    Some(LayoutRect::from_origin_and_size(
        LayoutPoint::new(min_x + tx as f32, min_y + ty as f32),
        LayoutSize::new((max_x - min_x).max(0.0), (max_y - min_y).max(0.0)),
    ))
}

#[cfg(test)]
mod tests {
    use super::{classify_fill_path, FillPathSupport};
    use crate::display_list::DisplayPathEl;
    use kurbo::Rect;

    #[test]
    fn degenerate_page_edge_path_is_treated_as_no_op() {
        let path = vec![
            DisplayPathEl::MoveTo((0.0, 0.0)),
            DisplayPathEl::LineTo((0.0, 0.0)),
            DisplayPathEl::LineTo((1882.0, 0.0)),
            DisplayPathEl::LineTo((1882.0, 0.0)),
            DisplayPathEl::MoveTo((1882.0, 0.0)),
            DisplayPathEl::LineTo((1882.0, 0.0)),
            DisplayPathEl::LineTo((1882.0, 22144.0)),
            DisplayPathEl::LineTo((1882.0, 22144.0)),
            DisplayPathEl::MoveTo((1882.0, 22144.0)),
            DisplayPathEl::LineTo((1882.0, 22144.0)),
            DisplayPathEl::LineTo((0.0, 22144.0)),
            DisplayPathEl::LineTo((0.0, 22144.0)),
            DisplayPathEl::MoveTo((0.0, 22144.0)),
            DisplayPathEl::LineTo((0.0, 22144.0)),
            DisplayPathEl::LineTo((0.0, 0.0)),
            DisplayPathEl::LineTo((0.0, 0.0)),
        ];

        assert_eq!(classify_fill_path(&path), FillPathSupport::NoOp);
    }

    #[test]
    fn rectangle_path_is_recognized() {
        let path = vec![
            DisplayPathEl::MoveTo((10.0, 20.0)),
            DisplayPathEl::LineTo((30.0, 20.0)),
            DisplayPathEl::LineTo((30.0, 60.0)),
            DisplayPathEl::LineTo((10.0, 60.0)),
            DisplayPathEl::ClosePath,
        ];

        assert_eq!(
            classify_fill_path(&path),
            FillPathSupport::Rect(Rect::new(10.0, 20.0, 30.0, 60.0))
        );
    }

    #[test]
    fn diagonal_path_remains_unsupported() {
        let path = vec![
            DisplayPathEl::MoveTo((0.0, 0.0)),
            DisplayPathEl::LineTo((10.0, 10.0)),
            DisplayPathEl::LineTo((0.0, 10.0)),
            DisplayPathEl::ClosePath,
        ];

        assert_eq!(classify_fill_path(&path), FillPathSupport::Unsupported);
    }
}


