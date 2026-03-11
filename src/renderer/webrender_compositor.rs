use crate::display_list::{DisplayBrush, DisplayCommand, DisplayShape};
use crate::fragment_tree::{
    FragmentNode, FragmentNodeKind, FragmentTree, SerializedOverflow,
};
use crate::window::Env;
use gleam::gl::GlFns;
use kurbo::{Affine, Point, Rect, Vec2};
use std::rc::Rc;
use webrender::{
    create_webrender_instance, RenderApi, Renderer, Transaction, WebRenderOptions,
};
use webrender_api::units::{DeviceIntRect, DeviceIntSize, LayoutPoint, LayoutRect, LayoutSize};
use webrender_api::{
    ColorF, CommonItemProperties, DocumentId, Epoch, PipelineId, PrimitiveFlags,
    RenderNotifier, SpaceAndClipInfo,
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

        let mut encoder = FragmentTreeWebRenderEncoder {
            tree,
            builder: &mut builder,
            page_clip,
            root_transform: Affine::translate((page_origin.0 as f64, page_origin.1 as f64)),
            root_space_and_clip: SpaceAndClipInfo::root_scroll(self.pipeline_id),
        };

        if !encoder.encode() {
            return Ok(false);
        }

        let (_, display_list) = builder.end();
        let epoch = Epoch(self.next_epoch);
        self.next_epoch = self.next_epoch.wrapping_add(1).max(1);
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.wrapping_add(1).max(1);

        let mut txn = Transaction::new();
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
    page_clip: LayoutRect,
    root_transform: Affine,
    root_space_and_clip: SpaceAndClipInfo,
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

        if (resolved_style.opacity - 1.0).abs() > AFFINE_EPSILON as f32 {
            return false;
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
        let should_clip = is_image
            || is_text_input
            || !matches!(resolved_style.overflow_x, SerializedOverflow::Visible)
            || !matches!(resolved_style.overflow_y, SerializedOverflow::Visible);
        if should_clip {
            return false;
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
        if !self.encode_commands(self.tree.pre_layer_commands.get(&node_id), transform) {
            return false;
        }
        if !self.encode_commands(self.tree.element_commands.get(&node_id), transform) {
            return false;
        }

        let scrolled_position = Point {
            x: position.x - node.scroll_offset.x,
            y: position.y - node.scroll_offset.y,
        };
        let scroll_transform = self.root_transform
            * Affine::translate(scrolled_position.to_vec2() * self.tree.scale_factor);
        if !self.encode_commands(self.tree.content_commands.get(&node_id), scroll_transform) {
            return false;
        }

        self.encode_children(node, scrolled_position)
    }

    fn encode_children(&mut self, node: &FragmentNode, position: Point) -> bool {
        if let Some(stacking_context) = node.stacking_context.as_ref() {
            for child in stacking_context.neg_z_hoisted_children() {
                let pos = Point {
                    x: position.x + child.position.x as f64,
                    y: position.y + child.position.y as f64,
                };
                if !self.encode_node(child.node_id, pos) {
                    return false;
                }
            }
        }

        if let Some(children) = node.paint_children.as_ref() {
            for &child_id in children {
                if !self.encode_node(child_id, position) {
                    return false;
                }
            }
        }

        if let Some(stacking_context) = node.stacking_context.as_ref() {
            for child in stacking_context.pos_z_hoisted_children() {
                let pos = Point {
                    x: position.x + child.position.x as f64,
                    y: position.y + child.position.y as f64,
                };
                if !self.encode_node(child.node_id, pos) {
                    return false;
                }
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
                shape: DisplayShape::Rect(rect),
                transform,
                ..
            } => {
                let DisplayBrush::Solid(color) = brush else {
                    return false;
                };
                if brush_transform.is_some() {
                    return false;
                }
                let final_rect = match translated_rect(base_transform * *transform, *rect) {
                    Some(rect) => rect,
                    None => return false,
                };
                let bounds = layout_rect(final_rect);
                self.push_solid_rect(
                    self.page_clip,
                    bounds,
                    ColorF::new(color[0], color[1], color[2], color[3]),
                );
                true
            }
            _ => false,
        }
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


