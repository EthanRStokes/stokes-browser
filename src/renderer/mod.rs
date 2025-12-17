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
use parley::PositionedLayoutItem;
use peniko::Fill;
use crate::dom::{Dom, DomNode, ElementData, ImageData, NodeData};
use crate::dom::node::DomNodeFlags;
use crate::layout::LayoutBox;
use crate::renderer::background::BackgroundImageCache;
use crate::renderer::paint::DefaultPaints;
use crate::renderer::text::{TextPainter, ToColorColor, render_text_at_position};
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::properties::generated::longhands::visibility::computed_value::T as Visibility;
use style::properties::{longhands, ComputedValues};
use style::properties::style_structs::Font;
use style::servo_arc::Arc;
use style::values::computed::{BorderCornerRadius, BorderStyle, CSSPixelLength, OutlineStyle, Overflow, ZIndex};
use style::values::generics::color::GenericColor;
use style::values::specified::TextDecorationLine;
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

        element.draw_background(painter);
        element.draw_image(painter);
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
            NodeData::Text { contents } => {
                let style = node.style_arc();
                let scroll_transform = Affine::translate((-self.dom.viewport_scroll.x, -self.dom.viewport_scroll.y));
                text::render_text_node(
                    scene,
                    node,
                    self.dom,
                    contents,
                    &style,
                    self.scale_factor as f32,
                    scroll_transform,
                );
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
        // Check if this is an inline root - if so, render inline layout instead of children
        if self.node.flags.contains(DomNodeFlags::IS_INLINE_ROOT) {
            self.draw_inline_layout(painter);
            return;
        }

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

    fn draw_inline_layout(&self, painter: &mut TextPainter) {
        let Some(inline_layout) = self.element.inline_layout_data.as_ref() else {
            // No inline layout data - fall back to rendering text children directly
            self.render_text_children_fallback(painter);
            return;
        };

        let layout = &inline_layout.layout;

        // Check if the layout is empty (no glyph runs were generated)
        // This happens when inline layout construction hasn't populated the text
        let has_content = layout.lines().next().is_some();
        if !has_content {
            // Fall back to rendering text children directly
            self.render_text_children_fallback(painter);
            return;
        }

        let transform = Affine::translate((self.position.x * self.scale_factor, self.position.y * self.scale_factor));

        // Get padding and border to offset content
        let node_layout = self.node.final_layout;
        let padding_border = node_layout.padding + node_layout.border;
        let content_offset = Affine::translate((
            padding_border.left as f64 * self.scale_factor,
            padding_border.top as f64 * self.scale_factor,
        ));
        let transform = transform * content_offset;

        // Render each line
        for line in layout.lines() {
            for item in line.items() {
                match item {
                    PositionedLayoutItem::GlyphRun(glyph_run) => {
                        let run = glyph_run.run();
                        let font = run.font();
                        let font_size = run.font_size();
                        let metrics = run.metrics();
                        let style = glyph_run.style();
                        let synthesis = run.synthesis();
                        let glyph_xform = synthesis
                            .skew()
                            .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));

                        // Get styles from the node associated with this text run
                        let styles = self.context.dom
                            .get_node(style.brush.id)
                            .and_then(|n| n.primary_styles());

                        let (text_color, text_decoration_color, text_decoration_line) = if let Some(styles) = styles {
                            let itext_styles = styles.get_inherited_text();
                            let text_styles = styles.get_text();
                            let text_color = itext_styles.color.as_color_color();
                            let text_decoration_color = text_styles
                                .text_decoration_color
                                .as_absolute()
                                .map(ToColorColor::as_color_color)
                                .unwrap_or(text_color);
                            let text_decoration_line = text_styles.text_decoration_line;
                            (text_color, text_decoration_color, text_decoration_line)
                        } else {
                            // Default to black text with no decoration
                            let black = color::AlphaColor::new([0.0, 0.0, 0.0, 1.0]);
                            (black, black, TextDecorationLine::empty())
                        };

                        let text_decoration_brush = anyrender::Paint::from(text_decoration_color);
                        let has_underline = text_decoration_line.contains(TextDecorationLine::UNDERLINE);
                        let has_strikethrough = text_decoration_line.contains(TextDecorationLine::LINE_THROUGH);

                        painter.draw_glyphs(
                            font,
                            font_size,
                            true, // hint
                            run.normalized_coords(),
                            Fill::NonZero,
                            &anyrender::Paint::from(text_color),
                            1.0, // alpha
                            transform,
                            glyph_xform,
                            glyph_run.positioned_glyphs().map(|glyph| anyrender::Glyph {
                                id: glyph.id as _,
                                x: glyph.x,
                                y: glyph.y,
                            }),
                        );

                        // Draw underline
                        if has_underline {
                            let offset = metrics.underline_offset;
                            let size = metrics.underline_size;
                            let x = glyph_run.offset() as f64;
                            let w = glyph_run.advance() as f64;
                            let y = (glyph_run.baseline() - offset + size / 2.0) as f64;
                            let line = kurbo::Line::new((x, y), (x + w, y));
                            painter.stroke(&Stroke::new(size as f64), transform, &text_decoration_brush, None, &line);
                        }

                        // Draw strikethrough
                        if has_strikethrough {
                            let offset = metrics.strikethrough_offset;
                            let size = metrics.strikethrough_size;
                            let x = glyph_run.offset() as f64;
                            let w = glyph_run.advance() as f64;
                            let y = (glyph_run.baseline() - offset + size / 2.0) as f64;
                            let line = kurbo::Line::new((x, y), (x + w, y));
                            painter.stroke(&Stroke::new(size as f64), transform, &text_decoration_brush, None, &line);
                        }
                    }
                    PositionedLayoutItem::InlineBox(inline_box) => {
                        // Render inline box (embedded element like <img> or inline-block)
                        let box_id = inline_box.id as usize;
                        self.context.render_node(painter, box_id, self.position);
                    }
                }
            }
        }
    }

    /// Fallback method to render text children directly when inline layout is not populated
    fn render_text_children_fallback(&self, painter: &mut TextPainter) {
        self.render_text_children_recursive(painter, self.node, self.position);
    }

    fn render_text_children_recursive(&self, painter: &mut TextPainter, node: &DomNode, position: Point) {
        for child_id in node.children.iter() {
            let child = self.context.dom.get_node(*child_id);
            if let Some(child) = child {
                match &child.data {
                    NodeData::Text { contents } => {
                        // Render this text node
                        let style = child.style_arc();
                        let scroll_transform = Affine::translate((
                            -self.context.dom.viewport_scroll.x,
                            -self.context.dom.viewport_scroll.y,
                        ));

                        // Use parent's position since text nodes in inline contexts
                        // don't have their own layout position set
                        render_text_at_position(
                            painter,
                            child,
                            self.context.dom,
                            contents,
                            &style,
                            self.context.scale_factor as f32,
                            scroll_transform,
                            position,
                        );
                    }
                    NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                        // For nested elements (like <span>), recurse
                        // Skip if it's a block element
                        let display = child.taffy_style.display;
                        if !matches!(display, taffy::Display::Block) {
                            self.render_text_children_recursive(painter, child, position);
                        }
                    }
                    _ => {}
                }
            }
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

    fn draw_image(&self, painter: &mut TextPainter) {
        // Check if this element has image data
        if let Some(image_data) = self.element.image_data() {
            println!("draw_image called with image_data: {:?}", std::mem::discriminant(image_data));

            // Use the element's transform (which includes position and scale)
            // and the content box for positioning
            let content_box = self.frame.content_box;

            match image_data {
                ImageData::Raster(data) => {
                    println!("draw_image: Rendering raster image {}x{}", data.width, data.height);
                    println!("draw_image: content_box={:?}, self.transform={:?}", content_box, self.transform);

                    // Calculate scale factors to fit image into content box
                    let scale_x = content_box.width() / data.width as f64;
                    let scale_y = content_box.height() / data.height as f64;

                    println!("draw_image: scale_x={}, scale_y={}", scale_x, scale_y);

                    // Apply the element's transform, then translate to content box origin, then scale the image
                    let image_transform = self.transform
                        * Affine::translate((content_box.x0, content_box.y0))
                        * Affine::scale_non_uniform(scale_x, scale_y);

                    println!("draw_image: image_transform={:?}", image_transform);

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

    fn draw_background(&self, painter: &mut TextPainter) {
        let background = self.style.get_background();
        let bg_color = background.background_color.as_absolute().unwrap().as_color_color();

        // Fill background color
        let background_rect = self.frame.padding_box;
        if bg_color.components[3] > 0.0 {
            painter.fill(Fill::NonZero, self.transform, bg_color, None, &background_rect);
        }

        // Render background images
        let layout = self.node.final_layout;
        let content_rect = Rect::new(
            layout.location.x as f64,
            layout.location.y as f64,
            (layout.location.x + layout.size.width) as f64,
            (layout.location.y + layout.size.height) as f64,
        );
        let scroll_transform = Affine::translate((
            -self.context.dom.viewport_scroll.x,
            -self.context.dom.viewport_scroll.y,
        ));
        background::render_background_image(
            painter,
            &content_rect,
            &self.style,
            self.scale_factor as f32,
            &self.context.background_image_cache,
            scroll_transform,
        );
    }
}
