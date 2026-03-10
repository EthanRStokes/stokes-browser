//! Fragment Tree Renderer — renders a `FragmentTree` on the compositor (parent) side.
//!
//! The tab process builds a `FragmentTree` that captures per-node pre-rendered
//! display commands and structural data (layout, children, stacking contexts).
//! This renderer walks the tree on the parent side, handling compositing
//! (clipping, layers, transforms, child ordering) and replaying per-node
//! display commands.

use crate::display_list::{DisplayCommand, DisplayFont};
use crate::fragment_tree::{
    FragmentNode, FragmentNodeKind, FragmentTree, SerializedOverflow,
};
use anyrender::PaintScene;
use color::AlphaColor;
use kurbo::{Affine, Point, Rect, Vec2};
use peniko::{Color, Fill, Mix};
use std::collections::HashMap;
use std::sync::Arc;

/// Renderer that composites a `FragmentTree` on the parent process side.
///
/// The tab process records per-node display commands for element appearance
/// (backgrounds, borders, shadows, text, images). This renderer handles the
/// tree walk, clipping, layers, opacity, transforms, and child ordering —
/// effectively the compositing logic that was previously in `HtmlRenderer`.
pub struct FragmentTreeRenderer<'ft> {
    pub tree: &'ft FragmentTree,
    pub scale_factor: f64,
    pub width: u32,
    pub height: u32,
    pub font_cache: &'ft HashMap<DisplayFont, Arc<Vec<u8>>>,
}

impl<'ft> FragmentTreeRenderer<'ft> {
    /// Render the entire fragment tree to the painter.
    pub fn render(&self, painter: &mut impl PaintScene) {
        let root_id = self.tree.root_element_id;
        let Some(root_node) = self.tree.get_node(root_id) else {
            return;
        };

        let scroll = self.tree.viewport_scroll;

        let bg_width = (self.width as f32).max(root_node.final_layout.size.width);
        let bg_height = (self.height as f32).max(root_node.final_layout.size.height);

        // Draw background color
        if let Some(bg_color) = self.tree.background_color {
            let rect = Rect::from_origin_size((0.0, 0.0), (bg_width as f64, bg_height as f64));
            painter.fill(Fill::NonZero, Affine::IDENTITY, Color::new(bg_color), None, &rect);
        }

        // Resolve fonts for display command replay
        let fonts: Vec<Option<peniko::FontData>> = self
            .tree
            .fonts
            .iter()
            .map(|font| {
                self.font_cache
                    .get(font)
                    .map(|bytes| font.to_peniko(bytes.clone()))
            })
            .collect();

        self.render_element(
            painter,
            root_id,
            Point {
                x: -scroll.x,
                y: -scroll.y,
            },
            &fonts,
        );

        // Debug hitboxes
        if self.tree.debug_hitboxes {
            self.render_debug_hitboxes(painter, root_id, 0.0, 0.0);
        }
    }

    fn render_element(
        &self,
        painter: &mut impl PaintScene,
        node_id: usize,
        location: Point,
        fonts: &[Option<peniko::FontData>],
    ) {
        let Some(node) = self.tree.get_node(node_id) else {
            return;
        };

        if matches!(node.display, taffy::Display::None) {
            return;
        }

        let Some(ref resolved_style) = node.resolved_style else {
            return;
        };

        if let Some(ref elem_data) = node.element_data {
            if elem_data.is_hidden_input {
                return;
            }
        }

        if !resolved_style.visibility_visible {
            return;
        }

        let opacity = resolved_style.opacity;
        if opacity == 0.0 {
            return;
        }
        let has_opacity = opacity < 1.0;

        let overflow_x = &resolved_style.overflow_x;
        let overflow_y = &resolved_style.overflow_y;
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
            || !matches!(overflow_x, SerializedOverflow::Visible)
            || !matches!(overflow_y, SerializedOverflow::Visible);

        let layout = node.final_layout.to_taffy();
        let position = location + Vec2::new(layout.location.x as f64, layout.location.y as f64);

        let taffy::Layout {
            size,
            border,
            padding,
            content_size,
            ..
        } = layout;

        let scaled_padding_border = (padding + border).map(f64::from);
        let content_box_size = kurbo::Size {
            width: (size.width as f64 - scaled_padding_border.left - scaled_padding_border.right)
                * self.scale_factor,
            height: (size.height as f64 - scaled_padding_border.top - scaled_padding_border.bottom)
                * self.scale_factor,
        };

        let scaled_y = position.y * self.scale_factor;
        let scaled_content_height =
            content_size.height.max(size.height) as f64 * self.scale_factor;
        if scaled_y > self.height as f64 || scaled_y + scaled_content_height < 0.0 {
            return; // Skip rendering boxes outside viewport
        }

        let clip_area = content_box_size.width * content_box_size.height;
        if should_clip && clip_area < 0.01 {
            return;
        }

        // Replay the per-node display commands (outline + outset box shadow are
        // drawn before the opacity/clip layer, so they are stored in
        // `pre_layer_commands`; everything else is in `element_commands`).
        let transform = Affine::translate(position.to_vec2() * self.scale_factor);

        // Replay pre-layer commands (outline, outset box shadow)
        if let Some(commands) = self.tree.pre_layer_commands.get(&node_id) {
            for cmd in commands {
                cmd.replay(painter, transform, fonts);
            }
        }

        // Build clip paths from layout + style for the layer
        let frame = crate::renderer::create_css_rect_from_fragment(resolved_style, &layout, self.scale_factor);

        let border_box_path = frame.border_box_path();
        let padding_box_path = frame.padding_box_path();
        let content_box_path = frame.content_box_path();

        // Opacity layer wrapping the element content
        let opacity_layer_pushed = if has_opacity {
            painter.push_layer(Mix::Normal, opacity, transform, &border_box_path);
            true
        } else {
            false
        };

        // Replay element commands (background, inset box shadow, table row bg,
        // table borders, border)
        if let Some(commands) = self.tree.element_commands.get(&node_id) {
            for cmd in commands {
                cmd.replay(painter, transform, fonts);
            }
        }

        // Clip layer for overflow / text input
        let clip_shape = if is_text_input {
            &content_box_path
        } else {
            &padding_box_path
        };
        let clip_layer_pushed = if should_clip {
            painter.push_clip_layer(transform, clip_shape);
            true
        } else {
            false
        };

        // Adjust position for scroll offset
        let scrolled_position = Point {
            x: position.x - node.scroll_offset.x,
            y: position.y - node.scroll_offset.y,
        };
        let scroll_transform = Affine::translate(scrolled_position.to_vec2() * self.scale_factor);

        // Replay content commands (images, SVG, canvas, text input, inline text, markers)
        if let Some(commands) = self.tree.content_commands.get(&node_id) {
            for cmd in commands {
                cmd.replay(painter, scroll_transform, fonts);
            }
        }

        // Render children
        self.render_children(painter, node, scrolled_position, fonts);

        if clip_layer_pushed {
            painter.pop_layer();
        }
        if opacity_layer_pushed {
            painter.pop_layer();
        }
    }

    fn render_children(
        &self,
        painter: &mut impl PaintScene,
        node: &FragmentNode,
        position: Point,
        fonts: &[Option<peniko::FontData>],
    ) {
        // Negative z-index hoisted children
        if let Some(ref sc) = node.stacking_context {
            for child in sc.neg_z_hoisted_children() {
                let pos = Point {
                    x: position.x + child.position.x as f64,
                    y: position.y + child.position.y as f64,
                };
                self.render_node(painter, child.node_id, pos, fonts);
            }
        }

        // Regular paint children
        if let Some(ref children) = node.paint_children {
            for &child_id in children {
                self.render_node(painter, child_id, position, fonts);
            }
        }

        // Positive z-index hoisted children
        if let Some(ref sc) = node.stacking_context {
            for child in sc.pos_z_hoisted_children() {
                let pos = Point {
                    x: position.x + child.position.x as f64,
                    y: position.y + child.position.y as f64,
                };
                self.render_node(painter, child.node_id, pos, fonts);
            }
        }
    }

    fn render_node(
        &self,
        painter: &mut impl PaintScene,
        node_id: usize,
        location: Point,
        fonts: &[Option<peniko::FontData>],
    ) {
        let Some(node) = self.tree.get_node(node_id) else {
            return;
        };

        match &node.node_kind {
            FragmentNodeKind::Element { .. } | FragmentNodeKind::AnonymousBlock => {
                self.render_element(painter, node_id, location, fonts);
            }
            FragmentNodeKind::Text | FragmentNodeKind::Document | FragmentNodeKind::ShadowRoot | FragmentNodeKind::Comment => {}
        }
    }

    fn render_debug_hitboxes(
        &self,
        painter: &mut impl PaintScene,
        node_id: usize,
        parent_x: f64,
        parent_y: f64,
    ) {
        let Some(node) = self.tree.get_node(node_id) else {
            return;
        };
        let layout = node.final_layout;

        let abs_x = parent_x + layout.location.x as f64;
        let abs_y = parent_y + layout.location.y as f64;

        if layout.size.width > 0.0 && layout.size.height > 0.0 {
            let color = match &node.node_kind {
                FragmentNodeKind::Element { tag_name } => match tag_name.as_str() {
                    "a" => Color::new([0.0, 0.0, 1.0, 0.3]),
                    "button" | "input" => Color::new([0.0, 1.0, 0.0, 0.3]),
                    _ => Color::new([1.0, 0.0, 0.0, 0.1]),
                },
                FragmentNodeKind::Text => Color::new([1.0, 1.0, 0.0, 0.3]),
                _ => Color::new([0.5, 0.5, 0.5, 0.1]),
            };

            let scroll = self.tree.viewport_scroll;
            let draw_x = (abs_x - scroll.x) * self.scale_factor;
            let draw_y = (abs_y - scroll.y) * self.scale_factor;
            let draw_w = layout.size.width as f64 * self.scale_factor;
            let draw_h = layout.size.height as f64 * self.scale_factor;

            let rect = Rect::from_origin_size((draw_x, draw_y), (draw_w, draw_h));
            painter.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);

            let border_color =
                Color::new([color.components[0], color.components[1], color.components[2], 0.8]);
            painter.stroke(
                &kurbo::Stroke::new(1.0),
                Affine::IDENTITY,
                border_color,
                None,
                &rect,
            );
        }

        if let Some(ref children) = node.layout_children {
            for &child_id in children {
                self.render_debug_hitboxes(painter, child_id, abs_x, abs_y);
            }
        }
    }
}

