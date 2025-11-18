// HTML renderer module - organized into logical components
mod paint;
pub(crate) mod text;
mod image;
pub(crate) mod background;
mod decorations;
mod pseudo;
mod cache;

use crate::dom::{Dom, DomNode, ElementData, NodeData};
use crate::layout::LayoutBox;
use crate::renderer::background::BackgroundImageCache;
use crate::renderer::paint::DefaultPaints;
use crate::renderer::text::{TextPainter, ToColorColor};
use skia_safe::Rect;
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::properties::longhands;
use style::servo_arc::Arc;
use style::values::computed::ZIndex;
use crate::dom::node::SpecialElementData;

/// HTML renderer that draws layout boxes to a canvas
pub struct HtmlRenderer {
    paints: DefaultPaints,
    background_image_cache: BackgroundImageCache,
}

impl HtmlRenderer {
    pub fn new() -> Self {
        let paints = DefaultPaints::new();

        Self {
            paints,
            background_image_cache: BackgroundImageCache::new(),
        }
    }

    /// Render a layout tree to the canvas with transition support
    pub fn render(
        &mut self,
        painter: &mut TextPainter,
        node: &DomNode,
        dom: &Dom,
        scroll_x: f32,
        scroll_y: f32,
        scale_factor: f32,
    ) {
        // Create scroll transform to offset the view
        let scroll_transform = kurbo::Affine::translate((-scroll_x as f64, -scroll_y as f64));

        // Calculate viewport bounds for culling off-screen elements
        let viewport_rect = Rect::from_xywh(
            scroll_x,
            scroll_y,
            painter.base_layer_size().width as f32,
            painter.base_layer_size().height as f32,
        );

        // Render the layout tree with styles, scale factor, and viewport culling
        self.render_box(painter, &node, dom, scale_factor, &viewport_rect, scroll_transform);
    }

    /// Render a single layout box with CSS styles, transitions, and scale factor
    fn render_box(
        &mut self,
        painter: &mut TextPainter,
        node: &DomNode,
        dom: &Dom,
        scale_factor: f32,
        viewport_rect: &Rect,
        scroll_transform: kurbo::Affine,
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
                            image::render_image_node(painter, node, dom, &data, &style, scale_factor, scroll_transform);
                        },
                        _ => {
                            self.render_element(painter, node, dom, element_data, &style, scale_factor, scroll_transform)
                        }
                    }
                },
                NodeData::Text { contents } => {
                    // Check if text node is inside a non-visual element
                    if !is_inside_non_visual_element(&dom_node) {
                        text::render_text_node(
                            painter,
                            node,
                            dom,
                            contents,
                            &style,
                            scale_factor,
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
                self.render_box(painter, &*child_node, dom, scale_factor, viewport_rect, scroll_transform);
            }
        }
    }

    /// Render an element with CSS styles applied
    fn render_element(
        &mut self,
        painter: &mut TextPainter,
        node: &DomNode,
        dom: &Dom,
        element_data: &ElementData,
        style: &Arc<StyloComputedValues>,
        scale_factor: f32,
        scroll_transform: kurbo::Affine,
    ) {
        let layout = node.final_layout;
        let content_rect = Rect::from_xywh(layout.location.x, layout.location.y, layout.size.width, layout.size.height);

        // Get opacity value (default to 1.0 if no styles)
        let effects = style.get_effects();
        let opacity = effects.opacity;

        // Render ::before pseudo-element content
        pseudo::render_pseudo_element_content(
            painter,
            dom,
            &content_rect,
            element_data,
            style,
            scale_factor,
            true, // before
            scroll_transform,
        );

        // Render box shadows first (behind the element)
        decorations::render_box_shadows(painter, &content_rect, style, scale_factor, scroll_transform);

        // Create background paint with CSS colors
        let background = style.get_background();
        let bg_color = background.background_color.as_absolute().unwrap().as_color_color();

        // Draw border if specified in styles or default for certain elements
        let mut should_draw_border = false;
        let mut border_paint = self.paints.border_paint.clone();

        let border = style.get_border();
        // Check if border is specified in styles
        if border.border_top_width.0 > 0 || border.border_right_width.0 > 0 ||
            border.border_bottom_width.0 > 0 || border.border_left_width.0 > 0 {
            should_draw_border = true;
            // Use average border width for simplicity and apply scale factor
            let avg_border_width = (border.border_top_width.0 + border.border_right_width.0 +
                border.border_bottom_width.0 + border.border_left_width.0) as f32 / 4.0;
            let scaled_border_width = avg_border_width * scale_factor as f32;
            border_paint.set_stroke_width(scaled_border_width);
        }


        // Default border for certain elements with scaling
        if !should_draw_border {
            match element_data.name.local.to_string().as_str() {
                "div" | "section" | "article" | "header" | "footer" => {
                    should_draw_border = true;
                    let scaled_border_width = 1.0 * scale_factor as f32;
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
            let scaled_heading_border = 2.0 * scale_factor;
            let stroke = kurbo::Stroke::new(scaled_heading_border as f64);
            let rect = kurbo::Rect::new(
                content_rect.left as f64,
                content_rect.top as f64,
                content_rect.right as f64,
                content_rect.bottom as f64,
            );
            painter.stroke(&stroke, scroll_transform, heading_color, None, &rect);
        }

        // Render outline if specified
        decorations::render_outline(painter, &content_rect, style, opacity, scale_factor, scroll_transform);

        // TODORender stroke if specified (CSS stroke property)
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
        let rect = kurbo::Rect::new(
            content_rect.left as f64,
            content_rect.top as f64,
            content_rect.right as f64,
            content_rect.bottom as f64,
        );
        painter.fill(peniko::Fill::NonZero, scroll_transform, bg_color, None, &rect);

        // Render background image if specified
        background::render_background_image(painter, &content_rect, style, scale_factor, &self.background_image_cache, scroll_transform);

        if should_draw_border {
            let border_stroke = kurbo::Stroke::new(border_paint.stroke_width() as f64);
            painter.stroke(&border_stroke, scroll_transform, border_alpha_color, None, &rect);
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