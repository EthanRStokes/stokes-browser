// HTML renderer module - organized into logical components
pub(crate) mod paint;
pub(crate) mod text;
mod image;
pub(crate) mod background;
mod decorations;
mod pseudo;
mod cache;
mod kurbo_css;
mod layers;

use kurbo::{Affine, Insets, Point, Rect, Stroke, Vec2};
use markup5ever::local_name;
use peniko::Fill;
use crate::dom::{Dom, DomNode, ElementData, NodeData};
use crate::layout::LayoutBox;
use crate::renderer::background::BackgroundImageCache;
use crate::renderer::paint::DefaultPaints;
use crate::renderer::text::{TextPainter, ToColorColor};
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::properties::generated::longhands::visibility::computed_value::T as Visibility;
use style::properties::{longhands, ComputedValues};
use style::properties::style_structs::Font;
use style::servo_arc::Arc;
use style::values::computed::{BorderCornerRadius, BorderStyle, CSSPixelLength, OutlineStyle, Overflow, ZIndex};
use style::values::generics::color::GenericColor;
use taffy::Layout;
use crate::dom::node::SpecialElementData;
use crate::renderer::kurbo_css::{CssBox, Edge, NonUniformRoundedRectRadii};

/// HTML renderer that draws layout boxes to a canvas
pub struct HtmlRenderer<'dom> {
    pub(crate) dom: &'dom Dom,
    pub(crate) scale_factor: f64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) paints: DefaultPaints,
    pub(crate) background_image_cache: BackgroundImageCache,
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
        )
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

        let scaled_y = position.y * self.scale_factor;
        let scaled_content_height = content_size.height.max(size.height) as f64 * self.scale_factor;
        if scaled_y > self.height as f64 || scaled_y + scaled_content_height < 0.0 {
            return; // Skip rendering boxes outside viewport
        }

        // Create scroll transform to offset the view
        let scroll_transform = kurbo::Affine::translate((-self.dom.viewport_scroll.x, -self.dom.viewport_scroll.y));
        // Calculate viewport bounds for culling off-screen elements
        let viewport_rect = Rect::new(
            self.dom.viewport_scroll.x,
            self.dom.viewport_scroll.y,
            painter.base_layer_size().width as f64,
            painter.base_layer_size().height as f64,
        );

        let mut element = self.element(node, layout, position);
        //element.render_box(painter, node, &viewport_rect, scroll_transform);

        element.draw_outline(painter);
        element.draw_border(painter);

        element.draw_children(painter);
    }

    fn render_node(&self, scene: &mut TextPainter, node_id: usize, location: Point) {
        let node = &self.dom.tree()[node_id];

        match &node.data {
            NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                self.render_element(scene, node_id, location)
            }
            NodeData::Text { .. } => {
                // Text nodes should never be rendered directly
                // (they should always be rendered as part of an inline layout)
                // unreachable!()
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
    let outline_width = style.get_outline().outline_width.to_f64_px() * scale;

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
}

impl Element<'_> {

    fn draw_children(&self, painter: &mut TextPainter) {
        let layout_children = self.node.layout_children.borrow();
        let mut children_with_z: Vec<(&DomNode, i32)> = layout_children.as_ref().unwrap().iter()
            .map(|child| {
                let node = self.node.get_node(*child);
                let z_index = node.style_arc().get_position().z_index;
                let z_index = match z_index {
                    ZIndex::Integer(i) => {
                        i
                    }
                    ZIndex::Auto => {
                        0
                    }
                };
                (node, z_index)
            })
            .collect();

        // Sort by z-index (lower z-index rendered first, so they appear behind)
        children_with_z.sort_by_key(|(_, z)| *z);

        for (child_node, _) in children_with_z {
            self.context.render_node(painter, child_node.id, self.position);
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

    /// Render a single layout box with CSS styles, transitions, and scale factor
    fn render_box(
        &mut self,
        painter: &mut TextPainter,
        node: &DomNode,
        viewport_rect: &Rect,
        scroll_transform: Affine,
    ) {
        let style = node.style_arc();

        // TODO Early culling: Skip rendering if box is completely outside viewport
        //let border_box = layout_box.dimensions.border_box();
        //if !viewport_rect.intersects(border_box) {
        //    return; // Skip this box and all its children
        //}

        // Check visibility - if hidden, skip rendering visual aspects but still render children
        let inherited_box = style.get_inherited_box();
        let is_visible = inherited_box.visibility == longhands::visibility::SpecifiedValue::Visible;

        // Get the DOM node for this layout box
        let dom_node = node.get_node(node.id);

        // Check if this node should be skipped from rendering
        if should_skip_rendering(&dom_node) {
            return; // Skip rendering this node and its children
        }

        // Only render visual aspects if visible
        if is_visible {
            match &dom_node.data {
                NodeData::Element(element_data) => {
                    match &element_data.special_data {
                        SpecialElementData::Image(data) => {
                            image::render_image_node(painter, node, self.context.dom, &data, &style, self.context.scale_factor as f32, scroll_transform);
                        },
                        _ => {
                            self.render_element(painter, node, element_data, &style, scroll_transform)
                        }
                    }
                },
                NodeData::Text { contents } => {
                    // Check if text node is inside a non-visual element
                    if !is_inside_non_visual_element(&dom_node) {
                        text::render_text_node(
                            painter,
                            node,
                            self.context.dom,
                            contents,
                            &style,
                            self.context.scale_factor as f32,
                            scroll_transform,
                        );
                    }
                },
                NodeData::Document => {
                    // Just render children for document
                },
                _ => {
                    // Skip other node types
                }
            }
        }

        // Render children regardless of visibility (they may have their own visibility settings)
        if !should_skip_rendering(&dom_node) {
            // Sort children by z-index before rendering
            /*let mut children_with_z: Vec<(&DomNode, i32)> = node.children.iter()
                .map(|child| {
                    let child = node.get_node(*child);
                    let z_index = child.style.z_index;
                    (child, z_index)
                })
                .collect();*/
            let layout_children = node.layout_children.borrow();
            let mut children_with_z: Vec<(&DomNode, i32)> = layout_children.as_ref().unwrap().iter()
                .map(|child| {
                    let node = node.get_node(*child);
                    let z_index = node.style_arc().get_position().z_index;
                    let z_index = match z_index {
                        ZIndex::Integer(i) => {
                            i
                        }
                        ZIndex::Auto => {
                            0
                        }
                    };
                    (node, z_index)
                })
                .collect();

            // Sort by z-index (lower z-index rendered first, so they appear behind)
            children_with_z.sort_by_key(|(_, z)| *z);

            // Render children in z-index order
            for (child_node, _) in children_with_z {
                self.render_box(painter, &*child_node, viewport_rect, scroll_transform);
            }
        }
    }

    /// Render an element with CSS styles applied
    fn render_element(
        &mut self,
        painter: &mut TextPainter,
        node: &DomNode,
        element_data: &ElementData,
        style: &Arc<StyloComputedValues>,
        scroll_transform: Affine,
    ) {
        let layout = node.final_layout;
        let content_rect = Rect::new(layout.location.x as f64, layout.location.y as f64, layout.size.width as f64, layout.size.height as f64);

        // Get opacity value (default to 1.0 if no styles)
        let effects = style.get_effects();
        let opacity = effects.opacity;

        // Render ::before pseudo-element content
        pseudo::render_pseudo_element_content(
            painter,
            self.context.dom,
            &content_rect,
            element_data,
            style,
            self.context.scale_factor as f32,
            true, // before
            scroll_transform,
        );

        // Render box shadows first (behind the element)
        decorations::render_box_shadows(painter, &content_rect, style, self.context.scale_factor as f32, scroll_transform);

        // Create background paint with CSS colors
        let background = style.get_background();
        let bg_color = background.background_color.as_absolute().unwrap().as_color_color();

        // Draw border if specified in styles or default for certain elements
        let mut should_draw_border = false;
        let mut border_paint = self.context.paints.border_paint.clone();

        let border = style.get_border();
        // Check if border is specified in styles
        if border.border_top_width.0 > 0 || border.border_right_width.0 > 0 ||
            border.border_bottom_width.0 > 0 || border.border_left_width.0 > 0 {
            should_draw_border = true;
            // Use average border width for simplicity and apply scale factor
            let avg_border_width = (border.border_top_width.0 + border.border_right_width.0 +
                border.border_bottom_width.0 + border.border_left_width.0) as f32 / 4.0;
            let scaled_border_width = avg_border_width * self.context.scale_factor as f32;
            border_paint.set_stroke_width(scaled_border_width);
        }


        // Default border for certain elements with scaling
        if !should_draw_border {
            match element_data.name.local.to_string().as_str() {
                "div" | "section" | "article" | "header" | "footer" => {
                    should_draw_border = true;
                    let scaled_border_width = 1.0 * self.context.scale_factor as f32;
                    border_paint.set_stroke_width(scaled_border_width);
                },
                _ => {}
            }
        }

        // Apply opacity to border
        if should_draw_border {
            let mut border_color = border_paint.color();
            border_color = border_color.with_a((border_color.a() as f32 * opacity) as u8);
            border_paint.set_color(border_color);
        }

        // Add visual indicators for headings with scaled border width
        if element_data.name.local.starts_with('h') {
            let heading_color = color::AlphaColor::from_rgba8(50, 50, 150, (255.0 * opacity) as u8);
            let scaled_heading_border = 2.0 * self.context.scale_factor;
            let stroke = Stroke::new(scaled_heading_border);
            painter.stroke(&stroke, scroll_transform, heading_color, None, &content_rect);
        }

        // Render outline if specified
        decorations::render_outline(painter, &content_rect, style, opacity, self.context.scale_factor as f32, scroll_transform);

        // TODO Render stroke if specified (CSS stroke property)
        //decorations::render_stroke(painter, &content_rect, &styles.stroke, opacity, scale_factor, scroll_transform);

        let border_color = border_paint.color();
        let border_alpha_color = color::AlphaColor::from_rgba8(
            border_color.r(),
            border_color.g(),
            border_color.b(),
            border_color.a(),
        );

        // Render rounded corners if border radius is specified
        let border = style.get_border();

        // Calculate border radius in pixels from the Stylo computed values
        let font = style.get_font();
        let font_size = font.font_size.computed_size().px();

        // TODO
        /*decorations::render_rounded_element(
            painter,
            content_rect,
            &border,
            bg_color.clone(),
            if should_draw_border { Some(border_alpha_color.clone()) } else { None },
            scale_factor,
            scroll_transform,
        );*/


        // Draw background (only if no rounded corners)
        painter.fill(peniko::Fill::NonZero, scroll_transform, bg_color, None, &content_rect);

        // Render background image if specified
        background::render_background_image(painter, &content_rect, style, self.context.scale_factor as f32, &self.context.background_image_cache, scroll_transform);

        if should_draw_border {
            let border_stroke = Stroke::new(border_paint.stroke_width() as f64);
            painter.stroke(&border_stroke, scroll_transform, border_alpha_color, None, &content_rect);
        }
    }
}

/// Determine if rendering should be skipped for this node (and its children)
fn should_skip_rendering(dom_node: &DomNode) -> bool {
    // Skip rendering for non-visual elements like <style>, <script>, etc.
    match dom_node.data {
        NodeData::Element(ref element_data) => {
            let tag = element_data.name.local.to_string();
            let tag = tag.as_str();
            // Skip if the tag is one of the non-visual elements
            tag == "style" || tag == "script" || tag == "head" || tag == "title"
        },
        _ => false
    }
}

/// Check if the current node is inside a non-visual element
fn is_inside_non_visual_element(dom_node: &DomNode) -> bool {
    // Simple approach: just check if this is a text node and skip the parent traversal
    // Text nodes inside style/script tags should be filtered out during DOM building
    // or layout phase, not during rendering
    match &dom_node.data {
        NodeData::Text { contents } => {
            // For now, we'll be conservative and not traverse parents to avoid infinite loops
            // The better approach is to filter these out during layout building
            false
        },
        _ => false
    }
}