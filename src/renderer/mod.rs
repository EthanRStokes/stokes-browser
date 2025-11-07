// HTML renderer module - organized into logical components
mod font;
mod paint;
pub(crate) mod text;
mod image;
mod background;
mod decorations;
mod pseudo;
mod cache;

use crate::css::transition_manager::TransitionManager;
use crate::css::ComputedValues;
use crate::dom::{DomNode, ElementData, NodeData};
use crate::layout::LayoutBox;
use crate::renderer::background::BackgroundImageCache;
use crate::renderer::font::FontManager;
use crate::renderer::paint::DefaultPaints;
use skia_safe::{Canvas, Color, Font, Paint, Rect};
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::properties::generated::style_structs::Font as StyloFont;
use style::properties::longhands;
use style::servo_arc::Arc;
use crate::renderer::text::TextPainter;

/// HTML renderer that draws layout boxes to a canvas
pub struct HtmlRenderer {
    default_font: Font,
    heading_font: Font,
    paints: DefaultPaints,
    font_manager: FontManager,
    background_image_cache: BackgroundImageCache,
}

impl HtmlRenderer {
    pub fn new() -> Self {
        let font_manager = FontManager::new();
        let paints = DefaultPaints::new();

        let default_font = Font::new(font_manager.placeholder_typeface.clone(), 14.0);
        let heading_font = Font::new(font_manager.placeholder_typeface.clone(), 18.0);

        Self {
            default_font,
            heading_font,
            paints,
            font_manager,
            background_image_cache: BackgroundImageCache::new(),
        }
    }

    /// Render a layout tree to the canvas with transition support
    pub fn render(
        &mut self,
        canvas: &Canvas,
        node: &DomNode,
        layout_box: &LayoutBox,
        transition_manager: Option<&TransitionManager>,
        painter: &mut TextPainter,
        scroll_x: f32,
        scroll_y: f32,
        scale_factor: f32,
    ) {
        // Save the current canvas state
        canvas.save();

        // Apply scroll offset by translating the canvas
        canvas.translate((-scroll_x, -scroll_y));

        // Calculate viewport bounds for culling off-screen elements
        let viewport_rect = Rect::from_xywh(
            scroll_x,
            scroll_y,
            canvas.base_layer_size().width as f32,
            canvas.base_layer_size().height as f32,
        );

        // Render the layout tree with styles, scale factor, and viewport culling
        self.render_box(canvas, &node, layout_box, transition_manager, painter, scale_factor, &viewport_rect);

        // Restore the canvas state
        canvas.restore();
    }

    /// Render a single layout box with CSS styles, transitions, and scale factor
    fn render_box(
        &mut self,
        canvas: &Canvas,
        node: &DomNode,
        layout_box: &LayoutBox,
        transition_manager: Option<&TransitionManager>,
        painter: &mut TextPainter,
        scale_factor: f32,
        viewport_rect: &Rect,
    ) {
        let style = node.style_arc();

        // TODO Early culling: Skip rendering if box is completely outside viewport
        //let border_box = layout_box.dimensions.border_box();
        //if !viewport_rect.intersects(border_box) {
        //    return; // Skip this box and all its children
        //}

        // Get base computed styles for this node
        let base_styles = &node.style;

        // TODO Get interpolated styles if transitions are active
        let computed_styles = base_styles;
        /*let computed_styles = if let (Some(manager), Some(base)) = (transition_manager, base_styles) {
            let interpolated = manager.get_interpolated_styles(node.node_id, base);
            Some(interpolated)
        } else {
            base_styles.cloned()
        };*/

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
                    self.render_element(canvas, node, layout_box, element_data, &computed_styles, &style, scale_factor);
                },
                NodeData::Text { contents } => {
                    // Check if text node is inside a non-visual element
                    if !is_inside_non_visual_element(&dom_node) {
                        text::render_text_node(
                            canvas,
                            node,
                            layout_box,
                            contents,
                            &computed_styles,
                            &self.font_manager,
                            &self.paints.text_paint,
                            painter,
                            scale_factor,
                        );
                    }
                },
                NodeData::Image(image_data) => {
                    let placeholder_font = self.font_manager.placeholder_font_for_size(12.0 * scale_factor as f32);
                    image::render_image_node(canvas, node, layout_box, image_data, scale_factor, &placeholder_font);
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
            let mut children_with_z: Vec<(&DomNode, &LayoutBox, i32)> = layout_box.children.iter()
                .map(|child| {
                    let node = node.get_node(child.node_id);
                    let z_index = node.style.z_index;
                    (node, child, z_index)
                })
                .collect();

            // Sort by z-index (lower z-index rendered first, so they appear behind)
            children_with_z.sort_by_key(|(_, _, z)| *z);

            // Render children in z-index order
            for (child_node, child, _) in children_with_z {
                self.render_box(canvas, &*child_node, child, transition_manager, painter, scale_factor, viewport_rect);
            }
        }
    }

    /// Render an element with CSS styles applied
    fn render_element(
        &mut self,
        canvas: &Canvas,
        node: &DomNode,
        layout_box: &LayoutBox,
        element_data: &ElementData,
        styles: &ComputedValues,
        style: &Arc<StyloComputedValues>,
        scale_factor: f32,
    ) {
        let content_rect = layout_box.dimensions.content;

        // Get opacity value (default to 1.0 if no styles)
        let effects = style.get_effects();
        let opacity = effects.opacity;

        // Render ::before pseudo-element content
        pseudo::render_pseudo_element_content(
            canvas,
            &content_rect,
            element_data,
            styles,
            style,
            scale_factor,
            true, // before
            &self.font_manager,
            &self.paints.text_paint,
        );

        // Render box shadows first (behind the element)
        decorations::render_box_shadows(canvas, &content_rect, styles, scale_factor);

        // Create background paint with CSS colors
        let mut bg_paint = &mut self.paints.background_paint;
        let background = style.get_background();
        let bg_color = &background.background_color;
        // todo replace bg_color impl
        if let Some(bg_color) = &styles.background_color {
            let mut color = bg_color.to_skia_color();
            // Apply opacity to background color
            color = color.with_a((color.a() as f32 * opacity) as u8);
            bg_paint.set_color(color);
        } else {
            // Fallback to default background colors
            background::set_default_background_color(&mut bg_paint, &element_data.name.local);
            // Apply opacity
            let mut color = bg_paint.color();
            color = color.with_a((color.a() as f32 * opacity) as u8);
            bg_paint.set_color(color);
        }

        // Draw border if specified in styles or default for certain elements
        let mut should_draw_border = false;
        let mut border_paint = self.paints.border_paint.clone();

        // Check if border is specified in styles
        if styles.border.top > 0.0 || styles.border.right > 0.0 ||
            styles.border.bottom > 0.0 || styles.border.left > 0.0 {
            should_draw_border = true;
            // Use average border width for simplicity and apply scale factor
            let avg_border_width = (styles.border.top + styles.border.right +
                styles.border.bottom + styles.border.left) / 4.0;
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
            let mut heading_paint = Paint::default();
            let mut heading_color = Color::from_rgb(50, 50, 150);
            heading_color = heading_color.with_a((255.0 * opacity) as u8);
            heading_paint.set_color(heading_color);
            heading_paint.set_stroke(true);
            let scaled_heading_border = 2.0 * scale_factor as f32;
            heading_paint.set_stroke_width(scaled_heading_border);
            canvas.draw_rect(content_rect, &heading_paint);
        }

        // Render outline if specified
        decorations::render_outline(canvas, &content_rect, styles, opacity, scale_factor);

        // Render stroke if specified (CSS stroke property)
        decorations::render_stroke(canvas, &content_rect, &styles.stroke, opacity, scale_factor);

        // Render rounded corners if border radius is specified
        let border_radius_px = styles.border_radius.to_px(styles.font_size, 400.0);
        if border_radius_px.has_radius() {
            decorations::render_rounded_element(
                canvas,
                content_rect,
                &border_radius_px,
                &bg_paint,
                if should_draw_border { Some(&border_paint) } else { None },
                scale_factor,
            );
            return; // Skip the regular rectangle drawing since we drew rounded shapes
        }

        // Draw background (only if no rounded corners)
        canvas.draw_rect(content_rect, &bg_paint);

        // Render background image if specified
        background::render_background_image(canvas, &content_rect, styles, scale_factor, &self.background_image_cache);

        if should_draw_border {
            canvas.draw_rect(content_rect, &border_paint);
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