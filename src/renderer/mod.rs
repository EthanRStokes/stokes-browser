pub(crate) mod text;
mod image;
pub(crate) mod background;
mod decorations;
mod cache;
mod kurbo_css;
mod layers;
mod shadow;
mod gradient;
mod sizing;

use std::any::Any;
use crate::dom::node::{ListItemLayout, ListItemLayoutPosition, Marker, SpecialElementData};
use crate::dom::{Dom, DomNode, ElementData, ImageData, NodeData};
use crate::renderer::kurbo_css::{CssBox, Edge, NonUniformRoundedRectRadii};
use crate::renderer::layers::maybe_with_layer;
use crate::renderer::text::{stroke_text, TextPainter, ToColorColor};
use anyrender::{CustomPaint, Paint, PaintScene};
use color::AlphaColor;
use kurbo::{Affine, Insets, Point, Rect, Stroke, Vec2};
use markup5ever::local_name;
use parley::PositionedLayoutItem;
use peniko::Fill;
use style::properties::generated::longhands::border_collapse::computed_value::T as BorderCollapse;
use style::properties::generated::longhands::visibility::computed_value::T as Visibility;
use style::properties::style_structs::Font;
use style::properties::ComputedValues;
use style::servo_arc::Arc;
use style::values::computed::{BorderCornerRadius, BorderStyle, CSSPixelLength, OutlineStyle, Overflow, ZIndex};
use style::values::generics::color::GenericColor;
use taffy::Layout;
use crate::renderer::sizing::compute_object_fit;

/// HTML renderer that draws layout boxes to a canvas
pub struct HtmlRenderer<'dom> {
    pub(crate) dom: &'dom Dom,
    pub(crate) scale_factor: f64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Debug: Show hitboxes for all elements
    pub(crate) debug_hitboxes: bool,
}

impl HtmlRenderer<'_> {
    fn node_position(&self, node: usize, location: Point) -> (Layout, Point) {
        let layout = self.layout(node);
        let position = location + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        (layout, position)
    }

    fn layout(&self, node: usize) -> Layout {
        self.dom.tree()[node].final_layout
    }

    /// Render a layout tree to the canvas with transition support
    pub fn render(
        &mut self,
        painter: &mut TextPainter,
        node: &DomNode,
    ) {
        let scroll = self.dom.viewport_scroll;

        let root_element = self.dom.root_element();
        let root_id = root_element.id;
        let bg_width = (self.width as f32).max(root_element.final_layout.size.width);
        let bg_height = (self.height as f32).max(root_element.final_layout.size.height);

        let background_color = {
            let html_color = root_element
                .primary_styles()
                .map(|s| s.clone_background_color())
                .unwrap_or(GenericColor::TRANSPARENT_BLACK);
            if html_color == GenericColor::TRANSPARENT_BLACK {
                root_element
                    .children
                    .iter()
                    .find_map(|id| {
                        self.dom
                            .get_node(*id)
                            .filter(|node| node.data.is_element_with_tag_name(&local_name!("body")))
                    })
                    .and_then(|body| body.primary_styles())
                    .map(|style| {
                        let current_color = style.clone_color();
                        style
                            .clone_background_color()
                            .resolve_to_absolute(&current_color)
                    })
            } else {
                let current_color = root_element.primary_styles().unwrap().clone_color();
                Some(html_color.resolve_to_absolute(&current_color))
            }
        };

        if let Some(bg_color) = background_color {
            let bg_color = bg_color.as_color_color();
            let rect = Rect::from_origin_size((0.0, 0.0), (bg_width as f64, bg_height as f64));
            painter.fill(Fill::NonZero, Affine::IDENTITY, bg_color, None, &rect);
        }

        self.render_element(
            painter,
            root_id,
            Point {
                x: -scroll.x,
                y: -scroll.y,
            },
        );

        // Draw debug hitboxes if enabled
        if self.debug_hitboxes {
            self.render_debug_hitboxes(painter, root_id, 0.0, 0.0);
        }
    }

    /// Render debug hitboxes for all elements (showing click target areas)
    fn render_debug_hitboxes(&self, painter: &mut TextPainter, node_id: usize, parent_x: f64, parent_y: f64) {
        let node = &self.dom.tree()[node_id];
        let layout = node.final_layout;

        // Calculate absolute position (same logic as find_element_at_position)
        let abs_x = parent_x + layout.location.x as f64;
        let abs_y = parent_y + layout.location.y as f64;

        // Only draw hitbox if node has non-zero size
        if layout.size.width > 0.0 && layout.size.height > 0.0 {
            // Determine hitbox color based on element type
            let color = match &node.data {
                NodeData::Element(elem) => {
                    let tag = elem.name.local.as_ref();
                    if tag == "a" {
                        // Links are blue
                        peniko::Color::new([0.0, 0.0, 1.0, 0.3])
                    } else if tag == "button" || tag == "input" {
                        // Interactive elements are green
                        peniko::Color::new([0.0, 1.0, 0.0, 0.3])
                    } else {
                        // Other elements are red (very transparent)
                        peniko::Color::new([1.0, 0.0, 0.0, 0.1])
                    }
                }
                NodeData::Text { .. } => {
                    // Text nodes are yellow
                    peniko::Color::new([1.0, 1.0, 0.0, 0.3])
                }
                _ => {
                    // Other nodes are gray
                    peniko::Color::new([0.5, 0.5, 0.5, 0.1])
                }
            };

            // Apply scroll offset for drawing
            let scroll = self.dom.viewport_scroll;
            let draw_x = (abs_x - scroll.x) * self.scale_factor;
            let draw_y = (abs_y - scroll.y) * self.scale_factor;
            let draw_w = layout.size.width as f64 * self.scale_factor;
            let draw_h = layout.size.height as f64 * self.scale_factor;

            let rect = Rect::from_origin_size((draw_x, draw_y), (draw_w, draw_h));

            // Fill with semi-transparent color
            painter.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);

            // Draw border
            let border_color = peniko::Color::new([color.components[0], color.components[1], color.components[2], 0.8]);
            painter.stroke(&Stroke::new(1.0), Affine::IDENTITY, border_color, None, &rect);
        }

        // Recursively draw hitboxes for layout children
        if let Some(layout_children) = node.layout_children.borrow().as_ref() {
            for &child_id in layout_children.iter() {
                self.render_debug_hitboxes(painter, child_id, abs_x, abs_y);
            }
        }

        // Draw hitboxes for inline boxes (hyperlinks and inline elements inside text)
        if let Some(element_data) = node.element_data() {
            if let Some(inline_layout) = &element_data.inline_layout_data {
                // Get content offset (padding + border)
                let padding_border = layout.padding + layout.border;
                let content_x = abs_x + padding_border.left as f64;
                let content_y = abs_y + padding_border.top as f64;

                for line in inline_layout.layout.lines() {
                    for item in line.items() {
                        match item {
                            PositionedLayoutItem::InlineBox(ibox) => {
                                let box_id = ibox.id as usize;
                                let box_node = &self.dom.tree()[box_id];

                                let box_x = content_x + ibox.x as f64;
                                let box_y = content_y + ibox.y as f64;
                                let box_w = ibox.width as f64;
                                let box_h = ibox.height as f64;

                                let color = match &box_node.data {
                                    NodeData::Element(elem) => {
                                        let tag = elem.name.local.as_ref();
                                        if tag == "a" {
                                            peniko::Color::new([0.0, 0.5, 1.0, 0.4])
                                        } else if tag == "button" || tag == "input" {
                                            peniko::Color::new([0.0, 1.0, 0.0, 0.4])
                                        } else {
                                            peniko::Color::new([0.0, 1.0, 1.0, 0.3])
                                        }
                                    }
                                    _ => peniko::Color::new([0.5, 0.5, 0.5, 0.2]),
                                };

                                let scroll = self.dom.viewport_scroll;
                                let draw_x = (box_x - scroll.x) * self.scale_factor;
                                let draw_y = (box_y - scroll.y) * self.scale_factor;
                                let draw_w = box_w * self.scale_factor;
                                let draw_h = box_h * self.scale_factor;

                                let rect = Rect::from_origin_size((draw_x, draw_y), (draw_w, draw_h));
                                painter.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);

                                let border_color = peniko::Color::new([color.components[0], color.components[1], color.components[2], 0.9]);
                                painter.stroke(&Stroke::new(2.0), Affine::IDENTITY, border_color, None, &rect);

                                self.render_debug_hitboxes_inline(painter, box_id, content_x, content_y);
                            }
                            PositionedLayoutItem::GlyphRun(glyph_run) => {
                                // Draw hitbox for glyph runs - these represent text inside inline elements
                                let brush_node_id = glyph_run.style().brush.id;

                                // Check if this text belongs to a link
                                let is_link = self.is_node_or_ancestor_link(brush_node_id);

                                if is_link {
                                    let run_x = content_x + glyph_run.offset() as f64;
                                    let run_y = content_y + (glyph_run.baseline() - glyph_run.run().metrics().ascent) as f64;
                                    let run_w = glyph_run.advance() as f64;
                                    let run_h = (glyph_run.run().metrics().ascent + glyph_run.run().metrics().descent) as f64;
                                    let scroll = self.dom.viewport_scroll;
                                    let draw_x = (run_x - scroll.x) * self.scale_factor;
                                    let draw_y = (run_y - scroll.y) * self.scale_factor;
                                    let draw_w = run_w * self.scale_factor;
                                    let draw_h = run_h * self.scale_factor;

                                    // Blue for links
                                    let color = peniko::Color::new([0.0, 0.5, 1.0, 0.4]);
                                    let rect = Rect::from_origin_size((draw_x, draw_y), (draw_w, draw_h));
                                    painter.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);

                                    let border_color = peniko::Color::new([0.0, 0.5, 1.0, 0.9]);
                                    painter.stroke(&Stroke::new(2.0), Affine::IDENTITY, border_color, None, &rect);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Check if a node or any of its ancestors is a link (<a> tag)
    fn is_node_or_ancestor_link(&self, node_id: usize) -> bool {
        let mut current_id = Some(node_id);
        let mut depth = 0;
        while let Some(id) = current_id {
            if depth > 50 { break; } // Prevent infinite loops
            depth += 1;

            let node = &self.dom.tree()[id];
            if let NodeData::Element(elem) = &node.data {
                if elem.name.local.as_ref() == "a" {
                    return true;
                }
            }
            current_id = node.parent;
        }
        false
    }

    /// Render debug hitboxes for inline box children
    fn render_debug_hitboxes_inline(&self, painter: &mut TextPainter, node_id: usize, container_x: f64, container_y: f64) {
        let node = &self.dom.tree()[node_id];
        let layout = node.final_layout;

        // For inline boxes, location is relative to the inline container
        let abs_x = container_x + layout.location.x as f64;
        let abs_y = container_y + layout.location.y as f64;

        // Recursively check layout children
        if let Some(layout_children) = node.layout_children.borrow().as_ref() {
            for &child_id in layout_children.iter() {
                self.render_debug_hitboxes(painter, child_id, abs_x, abs_y);
            }
        }

        // Check for nested inline layouts
        if let Some(element_data) = node.element_data() {
            if let Some(inline_layout) = &element_data.inline_layout_data {
                let padding_border = layout.padding + layout.border;
                let content_x = abs_x + padding_border.left as f64;
                let content_y = abs_y + padding_border.top as f64;

                for line in inline_layout.layout.lines() {
                    for item in line.items() {
                        if let PositionedLayoutItem::InlineBox(ibox) = item {
                            let box_id = ibox.id as usize;
                            let box_node = &self.dom.tree()[box_id];

                            let box_x = content_x + ibox.x as f64;
                            let box_y = content_y + ibox.y as f64;
                            let box_w = ibox.width as f64;
                            let box_h = ibox.height as f64;

                            let color = match &box_node.data {
                                NodeData::Element(elem) if elem.name.local.as_ref() == "a" => {
                                    peniko::Color::new([0.0, 0.5, 1.0, 0.4])
                                }
                                _ => peniko::Color::new([0.0, 1.0, 1.0, 0.3]),
                            };

                            let scroll = self.dom.viewport_scroll;
                            let draw_x = (box_x - scroll.x) * self.scale_factor;
                            let draw_y = (box_y - scroll.y) * self.scale_factor;
                            let draw_w = box_w * self.scale_factor;
                            let draw_h = box_h * self.scale_factor;

                            let rect = Rect::from_origin_size((draw_x, draw_y), (draw_w, draw_h));
                            painter.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);

                            let border_color = peniko::Color::new([color.components[0], color.components[1], color.components[2], 0.9]);
                            painter.stroke(&Stroke::new(2.0), Affine::IDENTITY, border_color, None, &rect);

                            self.render_debug_hitboxes_inline(painter, box_id, content_x, content_y);
                        }
                    }
                }
            }
        }
    }

    fn render_element(
        &self,
        painter: &mut TextPainter,
        node_id: usize,
        location: Point,
    ) {
        let node = &self.dom.tree()[node_id];

        if matches!(node.taffy_style.display, taffy::Display::None) {
            return; // Skip rendering for display: none
        }

        let Some(styles) = node.primary_styles() else {
            return;
        };

        if styles.get_inherited_box().visibility != Visibility::Visible {
            return;
        }

        let opacity = styles.get_effects().opacity;
        if opacity == 0.0 {
            return;
        }
        let has_opacity = opacity < 1.0;

        let overflow_x = styles.get_box().overflow_x;
        let overflow_y = styles.get_box().overflow_y;
        let is_image = node.element_data().and_then(|e| e.raster_image_data()).is_some();
        let should_clip = is_image || !matches!(overflow_x, Overflow::Visible) || !matches!(overflow_y, Overflow::Visible);

        let (layout, position) = self.node_position(node_id, location);
        let taffy::Layout {
            size,
            border,
            padding,
            content_size,
            ..
        } = node.final_layout;

        let scaled_padding_border = (padding + border).map(f64::from);
        let content_pos = Point {
            x: position.x + scaled_padding_border.left,
            y: position.y + scaled_padding_border.top,
        };
        let content_box_size = kurbo::Size {
            width: (size.width as f64 - scaled_padding_border.left - scaled_padding_border.right) * self.scale_factor,
            height: (size.height as f64 - scaled_padding_border.top - scaled_padding_border.bottom) * self.scale_factor,
        };

        let scaled_y = position.y * self.scale_factor;
        let scaled_content_height = content_size.height.max(size.height) as f64 * self.scale_factor;
        if scaled_y > self.height as f64 || scaled_y + scaled_content_height < 0.0 {
            return; // Skip rendering boxes outside viewport
        }

        let clip_area = content_box_size.width * content_box_size.height;
        if should_clip && clip_area < 0.01 {
            return;
        }

        let mut element = self.element(node, layout, position);

        element.draw_outline(painter);
        element.draw_outset_box_shadow(painter);

        maybe_with_layer(
            painter,
            has_opacity,
            opacity,
            element.transform,
            &element.frame.border_box_path(),
            |painter| {
                element.draw_background(painter);
                element.draw_inset_box_shadow(painter);
                element.draw_table_row_backgrounds(painter);
                element.draw_table_borders(painter);
                element.draw_border(painter);

                //let wants_layer = should_clip | has_opacity;
                let clip = &element.frame.padding_box_path(); // todo content_box_path for text input
                maybe_with_layer(painter, should_clip, 1.0, element.transform, clip, |painter| {
                    let position = Point {
                        x: position.x - node.scroll_offset.x,
                        y: position.y - node.scroll_offset.y,
                    };
                    element.position = Point {
                        x: element.position.x - node.scroll_offset.x,
                        y: element.position.y - node.scroll_offset.y,
                    };
                    element.transform = element.transform.then_translate(Vec2 {
                        x: -node.scroll_offset.x,
                        y: -node.scroll_offset.y
                    });
                    element.draw_image(painter);
                    element.draw_svg(painter);
                    element.draw_canvas(painter);
                    element.draw_inline_layout(painter, position);
                    element.draw_marker(painter, position);
                    element.draw_children(painter);
                });
            }
        );
    }

    fn render_node(&self, scene: &mut TextPainter, node_id: usize, location: Point) {
        let node = &self.dom.tree()[node_id];

        match &node.data {
            NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                self.render_element(scene, node_id, location)
            }
            NodeData::Text(text) => {
                unreachable!()
            }
            NodeData::Document => {}
            // NodeData::Doctype => {}
            NodeData::Comment { .. } => {}
        }
    }

    fn element<'a>(
        &'a self,
        node: &'a DomNode,
        layout: Layout,
        position: Point,
    ) -> Element<'a> {
        let style = node.stylo_data.borrow().as_ref().map(|elem_data| elem_data.styles.primary().clone())
            .unwrap_or(
                ComputedValues::initial_values_with_font_override(Font::initial_values())
            );

        let scale = self.scale_factor;

        let frame = create_css_rect(&style, &layout, scale);

        let transform = Affine::translate(position.to_vec2() * scale);

        let element = node.element_data().unwrap();

        Element {
            context: self,
            frame,
            style,
            position,
            scale_factor: scale,
            node,
            element,
            transform,
            svg: element.svg_data(),
            list_item: element.list_item_data.as_deref(),
        }
    }
}

fn insets_from_taffy_rect(input: taffy::Rect<f64>) -> Insets {
    Insets {
        x0: input.left,
        y0: input.top,
        x1: input.right,
        y1: input.bottom,
    }
}

/// Convert Stylo and Taffy types into Kurbo types
fn create_css_rect(style: &ComputedValues, layout: &Layout, scale: f64) -> CssBox {
    // Resolve and rescale
    // We have to scale since document pixels are not same same as rendered pixels
    let width: f64 = layout.size.width as f64;
    let height: f64 = layout.size.height as f64;
    let border_box = Rect::new(0.0, 0.0, width * scale, height * scale);
    let border = insets_from_taffy_rect(layout.border.map(|p| p as f64 * scale));
    let padding = insets_from_taffy_rect(layout.padding.map(|p| p as f64 * scale));
    let outline_width = style.get_outline().outline_width.0.to_f64_px() * scale;

    // Resolve the radii to a length. need to downscale since the radii are in document pixels
    let resolve_w = CSSPixelLength::new(width as _);
    let resolve_h = CSSPixelLength::new(height as _);
    let resolve_radii = |radius: &BorderCornerRadius| -> Vec2 {
        Vec2 {
            x: scale * radius.0.width.0.resolve(resolve_w).px() as f64,
            y: scale * radius.0.height.0.resolve(resolve_h).px() as f64,
        }
    };
    let s_border = style.get_border();
    let border_radii = NonUniformRoundedRectRadii {
        top_left: resolve_radii(&s_border.border_top_left_radius),
        top_right: resolve_radii(&s_border.border_top_right_radius),
        bottom_right: resolve_radii(&s_border.border_bottom_right_radius),
        bottom_left: resolve_radii(&s_border.border_bottom_left_radius),
    };

    CssBox::new(border_box, border, padding, outline_width, border_radii)
}

struct Element<'a> {
    context: &'a HtmlRenderer<'a>,
    frame: CssBox,
    style: Arc<ComputedValues>,
    position: Point,
    scale_factor: f64,
    node: &'a DomNode,
    element: &'a ElementData,
    transform: Affine,
    svg: Option<&'a usvg::Tree>,
    list_item: Option<&'a ListItemLayout>,
}

impl Element<'_> {
    fn draw_children(&self, painter: &mut TextPainter) {
        // Negative z_index hoisted nodes
        if let Some(hoisted) = &self.node.stacking_context {
            for child in hoisted.neg_z_hoisted_children() {
                let pos = Point {
                    x: self.position.x + child.position.x as f64,
                    y: self.position.y + child.position.y as f64,
                };
                self.context.render_node(painter, child.node_id, pos);
            }
        }

        // regular
        if let Some(children) = &*self.node.paint_children.borrow() {
            for child_id in children {
                self.context.render_node(painter, *child_id, self.position);
            }
        }

        // Positive z_index hoisted nodes
        if let Some(hoisted) = &self.node.stacking_context {
            for child in hoisted.pos_z_hoisted_children() {
                let pos = Point {
                    x: self.position.x + child.position.x as f64,
                    y: self.position.y + child.position.y as f64,
                };
                self.context.render_node(painter, child.node_id, pos);
            }
        }
    }

    fn draw_marker(&self, painter: &mut impl PaintScene, pos: Point) {
        if let Some(ListItemLayout {
                        marker,
                        position: ListItemLayoutPosition::Outside(layout),
                    }) = self.list_item
        {
            // Right align and pad the bullet when rendering outside
            let x_padding = match marker {
                Marker::Char(_) => 8.0,
                Marker::String(_) => 0.0,
            };
            let x_offset = -(layout.full_width() / layout.scale() + x_padding);

            // Align the marker with the baseline of the first line of text in the list item
            let y_offset = if let Some(first_text_line) = &self
                .element
                .inline_layout_data
                .as_ref()
                .and_then(|text_layout| text_layout.layout.lines().next())
            {
                (first_text_line.metrics().baseline
                    - layout.lines().next().unwrap().metrics().baseline)
                    / layout.scale()
            } else {
                0.0
            };

            let pos = Point {
                x: pos.x + x_offset as f64,
                y: pos.y + y_offset as f64,
            };

            let transform =
                Affine::translate((pos.x * self.scale_factor, pos.y * self.scale_factor)) * self.transform;

            stroke_text(painter, layout.lines(), self.context.dom, transform);
        }
    }

    fn draw_inline_layout(&self, painter: &mut TextPainter, pos: Point) {
        if self.node.flags.is_inline_root() {
            let text_layout = self.element.inline_layout_data.as_ref().unwrap_or_else(|| {
                panic!("Tried to render node marked as inline root but has no inline layout data: {:?}", self.node)
            });
            if text_layout.text.contains("freestar") {
                println!("YO WTF")
            }

            let transform =
                Affine::translate((pos.x * self.scale_factor, pos.y * self.scale_factor)) * self.transform;

            stroke_text(
                painter,
                text_layout.layout.lines(),
                self.context.dom,
                transform,
            )
        }
    }

    fn draw_border(&self, painter: &mut TextPainter) {
        for edge in [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left] {
            self.draw_border_edge(painter, edge);
        }
    }

    fn draw_border_edge(&self, painter: &mut TextPainter, edge: Edge) {
        let style = &*self.style;
        let border = style.get_border();
        let path = self.frame.border_edge_shape(edge);

        let current_color = style.clone_color();
        let color = match edge {
            Edge::Top => border
                .border_top_color
                .resolve_to_absolute(&current_color)
                .as_color_color(),
            Edge::Right => border
                .border_right_color
                .resolve_to_absolute(&current_color)
                .as_color_color(),
            Edge::Bottom => border
                .border_bottom_color
                .resolve_to_absolute(&current_color)
                .as_color_color(),
            Edge::Left => border
                .border_left_color
                .resolve_to_absolute(&current_color)
                .as_color_color(),
        };

        let alpha = color.components[3];
        if alpha != 0.0 {
            painter.fill(Fill::NonZero, self.transform, color, None, &path);
        }
    }

    fn draw_outline(&self, painter: &mut TextPainter) {
        let outline = self.style.get_outline();

        let current_color = self.style.clone_color();
        let color = outline.outline_color.resolve_to_absolute(&current_color).as_color_color();

        let style = match outline.outline_style {
            OutlineStyle::Auto => return,
            OutlineStyle::BorderStyle(style) => style,
        };

        let path = match style {
            BorderStyle::None | BorderStyle::Hidden => return,
            BorderStyle::Solid => self.frame.outline(),

            _ => {
                // TODO For other styles, just draw solid for now
                self.frame.outline()
            }
        };

        painter.fill(Fill::NonZero, self.transform, color, None, &path)
    }

    fn draw_table_borders(&self, scene: &mut impl PaintScene) {
        let SpecialElementData::TableRoot(table) = &self.element.special_data else {
            return;
        };
        // Borders are only handled at the table level when BorderCollapse::Collapse
        if table.border_collapse != BorderCollapse::Collapse {
            return;
        }

        let Some(grid_info) = &mut *table.computed_grid_info.borrow_mut() else {
            return;
        };
        let Some(border_style) = table.border_style.as_deref() else {
            return;
        };

        let outer_border_style = self.style.get_border();

        let cols = &grid_info.columns;
        let rows = &grid_info.rows;

        let inner_width =
            (cols.sizes.iter().sum::<f32>() + cols.gutters.iter().sum::<f32>()) as f64;
        let inner_height =
            (rows.sizes.iter().sum::<f32>() + rows.gutters.iter().sum::<f32>()) as f64;

        // TODO: support different colors for different borders
        let current_color = self.style.clone_color();
        let border_color = border_style
            .border_top_color
            .resolve_to_absolute(&current_color)
            .as_color_color();

        // No need to draw transparent borders (as they won't be visible anyway)
        if border_color == AlphaColor::TRANSPARENT {
            return;
        }

        let border_width = border_style.border_top_width.0.to_f64_px();

        // Draw horizontal inner borders
        let mut y = 0.0;
        for (&height, &gutter) in rows.sizes.iter().zip(rows.gutters.iter()) {
            let shape =
                Rect::new(0.0, y, inner_width, y + gutter as f64).scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);

            y += (height + gutter) as f64;
        }

        // Draw horizontal outer borders
        // Top border
        if outer_border_style.border_top_style != BorderStyle::Hidden {
            let shape =
                Rect::new(0.0, 0.0, inner_width, border_width).scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
        // Bottom border
        if outer_border_style.border_bottom_style != BorderStyle::Hidden {
            let shape = Rect::new(0.0, inner_height, inner_width, inner_height + border_width)
                .scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }

        // Draw vertical inner borders
        let mut x = 0.0;
        for (&width, &gutter) in cols.sizes.iter().zip(cols.gutters.iter()) {
            let shape =
                Rect::new(x, 0.0, x + gutter as f64, inner_height).scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);

            x += (width + gutter) as f64;
        }

        // Draw vertical outer borders
        // Left border
        if outer_border_style.border_left_style != BorderStyle::Hidden {
            let shape =
                Rect::new(0.0, 0.0, border_width, inner_height).scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
        // Right border
        if outer_border_style.border_right_style != BorderStyle::Hidden {
            let shape = Rect::new(inner_width, 0.0, inner_width + border_width, inner_height)
                .scale_from_origin(self.scale_factor);
            scene.fill(Fill::NonZero, self.transform, border_color, None, &shape);
        }
    }

    fn draw_svg(&self, scene: &mut impl PaintScene) {
        use style::properties::generated::longhands::object_fit::computed_value::T as ObjectFit;

        let Some(svg) = self.svg else {
            return;
        };

        let width = self.frame.content_box.width() as u32;
        let height = self.frame.content_box.height() as u32;
        let svg_size = svg.size();

        let x = self.frame.content_box.origin().x;
        let y = self.frame.content_box.origin().y;

        // let object_fit = self.style.clone_object_fit();
        let object_position = self.style.clone_object_position();

        // Apply object-fit algorithm
        let container_size = taffy::Size {
            width: width as f32,
            height: height as f32,
        };
        let object_size = taffy::Size {
            width: svg_size.width(),
            height: svg_size.height(),
        };
        let paint_size = compute_object_fit(container_size, Some(object_size), ObjectFit::Contain);

        // Compute object-position
        let x_offset = object_position.horizontal.resolve(
            CSSPixelLength::new(container_size.width - paint_size.width) / self.scale_factor as f32,
        ) * self.scale_factor as f32;
        let y_offset = object_position.vertical.resolve(
            CSSPixelLength::new(container_size.height - paint_size.height) / self.scale_factor as f32,
        ) * self.scale_factor as f32;
        let x = x + x_offset.px() as f64;
        let y = y + y_offset.px() as f64;

        let x_scale = paint_size.width as f64 / object_size.width as f64;
        let y_scale = paint_size.height as f64 / object_size.height as f64;

        let transform = self
            .transform
            .pre_scale_non_uniform(x_scale, y_scale)
            .then_translate(Vec2 { x, y });

        anyrender_svg::render_svg_tree(scene, svg, transform);
    }

    fn draw_image(&self, painter: &mut TextPainter) {
        // Check if this element has image data
        if let Some(image_data) = self.element.image_data() {
            // Use the element's transform (which includes position and scale)
            // and the content box for positioning
            let content_box = self.frame.content_box;

            match image_data {
                ImageData::Raster(data) => {
                    // Calculate scale factors to fit image into content box
                    let scale_x = content_box.width() / data.width as f64;
                    let scale_y = content_box.height() / data.height as f64;

                    // Apply the element's transform, then translate to content box origin, then scale the image
                    let image_transform = self.transform
                        * Affine::translate((content_box.x0, content_box.y0))
                        * Affine::scale_non_uniform(scale_x, scale_y);

                    let inherited_box = self.style.get_inherited_box();
                    let image_rendering = inherited_box.image_rendering;

                    painter.draw_image(
                        background::to_peniko_image(data, background::to_image_quality(image_rendering)).as_ref(),
                        image_transform
                    );
                },
                ImageData::Svg(_svg_tree) => {
                    // SVG rendering - render placeholder for now
                    let scroll_transform = Affine::translate((
                        -self.context.dom.viewport_scroll.x,
                        -self.context.dom.viewport_scroll.y,
                    ));
                    let layout = self.node.final_layout;
                    let content_rect = skia_safe::Rect::from_xywh(
                        layout.location.x,
                        layout.location.y,
                        layout.size.width,
                        layout.size.height
                    );
                    image::render_image_placeholder(painter, self.context.dom, &content_rect, "SVG", self.scale_factor as f32, scroll_transform);
                },
                ImageData::None => {
                    // Show placeholder
                    let scroll_transform = Affine::translate((
                        -self.context.dom.viewport_scroll.x,
                        -self.context.dom.viewport_scroll.y,
                    ));
                    let layout = self.node.final_layout;
                    let content_rect = skia_safe::Rect::from_xywh(
                        layout.location.x,
                        layout.location.y,
                        layout.size.width,
                        layout.size.height
                    );
                    image::render_image_placeholder(painter, self.context.dom, &content_rect, "No image", self.scale_factor as f32, scroll_transform);
                }
            }
        }
    }

    fn draw_canvas(&self, painter: &mut TextPainter) {
        let Some(custom_paint_source) = self.element.canvas_data() else {
            return;
        };

        let width = self.frame.content_box.width() as u32;
        let height = self.frame.content_box.height() as u32;
        let x = self.frame.content_box.origin().x;
        let y = self.frame.content_box.origin().y;

        let transform = self.transform.then_translate(Vec2 { x, y} );

        painter.fill(
            Fill::NonZero,
            transform,
            Paint::Custom(&CustomPaint {
                source_id: custom_paint_source.custom_paint_source_id,
                width,
                height,
                scale: self.scale_factor
            } as &(dyn Any + Send + Sync)),
            None,
            &Rect::from_origin_size((0.0, 0.0), (width as f64, height as f64)),
        );
    }
}
