// HTML renderer module - organized into logical components
mod font;
mod paint;
mod text;
mod image;
mod background;
mod decorations;
mod pseudo;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use skia_safe::{Canvas, Color, Font, Paint};
use crate::css::ComputedValues;
use crate::css::transition_manager::TransitionManager;
use crate::dom::{DomNode, ElementData, NodeType};
use crate::layout::LayoutBox;
use crate::renderer::background::BackgroundImageCache;
use crate::renderer::font::FontManager;
use crate::renderer::paint::DefaultPaints;

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

        let default_font = Font::new(font_manager.typeface.clone(), 14.0);
        let heading_font = Font::new(font_manager.typeface.clone(), 18.0);

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
        &self,
        canvas: &Canvas,
        layout: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
        style_map: &HashMap<usize, ComputedValues>,
        transition_manager: Option<&TransitionManager>,
        scroll_x: f32,
        scroll_y: f32,
        scale_factor: f64,
    ) {
        // Save the current canvas state
        canvas.save();

        // Apply scroll offset by translating the canvas
        canvas.translate((-scroll_x, -scroll_y));

        // Render the layout tree with styles and scale factor
        self.render_box(canvas, layout, node_map, style_map, transition_manager, scale_factor);

        // Restore the canvas state
        canvas.restore();
    }

    /// Render a single layout box with CSS styles, transitions, and scale factor
    fn render_box(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        node_map: &HashMap<usize, Rc<RefCell<DomNode>>>,
        style_map: &HashMap<usize, ComputedValues>,
        transition_manager: Option<&TransitionManager>,
        scale_factor: f64,
    ) {
        // Get base computed styles for this node
        let base_styles = style_map.get(&layout_box.node_id);

        // Get interpolated styles if transitions are active
        let computed_styles = if let (Some(manager), Some(base)) = (transition_manager, base_styles) {
            let interpolated = manager.get_interpolated_styles(layout_box.node_id, base);
            Some(interpolated)
        } else {
            base_styles.cloned()
        };

        // Check visibility - if hidden, skip rendering visual aspects but still render children
        let is_visible = computed_styles.as_ref()
            .map(|styles| matches!(styles.visibility, crate::css::Visibility::Visible))
            .unwrap_or(true);

        // Get the DOM node for this layout box
        if let Some(dom_node_rc) = node_map.get(&layout_box.node_id) {
            let dom_node = dom_node_rc.borrow();

            // Check if this node should be skipped from rendering
            if should_skip_rendering(&dom_node) {
                return; // Skip rendering this node and its children
            }

            // Only render visual aspects if visible
            if is_visible {
                match &dom_node.node_type {
                    NodeType::Element(element_data) => {
                        self.render_element(canvas, layout_box, element_data, computed_styles.as_ref(), scale_factor);
                    },
                    NodeType::Text(_) => {
                        // Check if text node is inside a non-visual element
                        if !is_inside_non_visual_element(&dom_node) {
                            text::render_text_node(
                                canvas,
                                layout_box,
                                computed_styles.as_ref(),
                                &self.font_manager,
                                &self.paints.text_paint,
                                scale_factor,
                            );
                        }
                    },
                    NodeType::Image(image_data) => {
                        let placeholder_font = self.font_manager.get_font_for_size(12.0 * scale_factor as f32);
                        image::render_image_node(canvas, layout_box, image_data, scale_factor, &placeholder_font);
                    },
                    NodeType::Document => {
                        // Just render children for document
                    },
                    _ => {
                        // Skip other node types
                    }
                }
            }
        }

        // Render children regardless of visibility (they may have their own visibility settings)
        if let Some(dom_node_rc) = node_map.get(&layout_box.node_id) {
            let dom_node = dom_node_rc.borrow();
            if !should_skip_rendering(&dom_node) {
                // Sort children by z-index before rendering
                let mut children_with_z: Vec<(&LayoutBox, i32)> = layout_box.children.iter()
                    .map(|child| {
                        let z_index = style_map.get(&child.node_id)
                            .map(|styles| styles.z_index)
                            .unwrap_or(0);
                        (child, z_index)
                    })
                    .collect();

                // Sort by z-index (lower z-index rendered first, so they appear behind)
                children_with_z.sort_by_key(|(_, z)| *z);

                // Render children in z-index order
                for (child, _) in children_with_z {
                    self.render_box(canvas, child, node_map, style_map, transition_manager, scale_factor);
                }
            }
        }
    }

    /// Render an element with CSS styles applied
    fn render_element(
        &self,
        canvas: &Canvas,
        layout_box: &LayoutBox,
        element_data: &ElementData,
        computed_styles: Option<&ComputedValues>,
        scale_factor: f64,
    ) {
        let border_box = layout_box.dimensions.border_box();

        // Get opacity value (default to 1.0 if no styles)
        let opacity = computed_styles.map(|s| s.opacity).unwrap_or(1.0);

        // Render ::before pseudo-element content
        if let Some(styles) = computed_styles {
            pseudo::render_pseudo_element_content(
                canvas,
                &border_box,
                element_data,
                styles,
                scale_factor,
                true, // before
                &self.font_manager,
                &self.paints.text_paint,
            );
        }

        // Render box shadows first (behind the element)
        if let Some(styles) = computed_styles {
            decorations::render_box_shadows(canvas, &border_box, styles, scale_factor);
        }

        // Create background paint with CSS colors
        let mut bg_paint = Paint::default();
        if let Some(styles) = computed_styles {
            if let Some(bg_color) = &styles.background_color {
                let mut color = bg_color.to_skia_color();
                // Apply opacity to background color
                color = color.with_a((color.a() as f32 * opacity) as u8);
                bg_paint.set_color(color);
            } else {
                // Fallback to default background colors
                background::set_default_background_color(&mut bg_paint, &element_data.tag_name);
                // Apply opacity
                let mut color = bg_paint.color();
                color = color.with_a((color.a() as f32 * opacity) as u8);
                bg_paint.set_color(color);
            }
        } else {
            background::set_default_background_color(&mut bg_paint, &element_data.tag_name);
            // Apply opacity
            let mut color = bg_paint.color();
            color = color.with_a((color.a() as f32 * opacity) as u8);
            bg_paint.set_color(color);
        }

        // Draw border if specified in styles or default for certain elements
        let mut should_draw_border = false;
        let mut border_paint = self.paints.border_paint.clone();

        if let Some(styles) = computed_styles {
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
        }

        // Default border for certain elements with scaling
        if !should_draw_border {
            match element_data.tag_name.as_str() {
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
        if element_data.tag_name.starts_with('h') {
            let mut heading_paint = Paint::default();
            let mut heading_color = Color::from_rgb(50, 50, 150);
            heading_color = heading_color.with_a((255.0 * opacity) as u8);
            heading_paint.set_color(heading_color);
            heading_paint.set_stroke(true);
            let scaled_heading_border = 2.0 * scale_factor as f32;
            heading_paint.set_stroke_width(scaled_heading_border);
            canvas.draw_rect(border_box, &heading_paint);
        }

        // Render outline if specified
        if let Some(styles) = computed_styles {
            decorations::render_outline(canvas, &border_box, styles, opacity, scale_factor);
        }

        // Render rounded corners if border radius is specified
        if let Some(styles) = computed_styles {
            let border_radius_px = styles.border_radius.to_px(styles.font_size, 400.0);
            if border_radius_px.has_radius() {
                decorations::render_rounded_element(
                    canvas,
                    border_box,
                    &border_radius_px,
                    &bg_paint,
                    if should_draw_border { Some(&border_paint) } else { None },
                    scale_factor,
                );
                return; // Skip the regular rectangle drawing since we drew rounded shapes
            }
        }

        // Draw background (only if no rounded corners)
        canvas.draw_rect(border_box, &bg_paint);

        // Render background image if specified
        if let Some(styles) = computed_styles {
            background::render_background_image(canvas, &border_box, styles, scale_factor, &self.background_image_cache);
        }

        if should_draw_border {
            canvas.draw_rect(border_box, &border_paint);
        }
    }
}

/// Determine if rendering should be skipped for this node (and its children)
fn should_skip_rendering(dom_node: &DomNode) -> bool {
    // Skip rendering for non-visual elements like <style>, <script>, etc.
    match dom_node.node_type {
        NodeType::Element(ref element_data) => {
            let tag = element_data.tag_name.as_str();
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
    match dom_node.node_type {
        NodeType::Text(_) => {
            // For now, we'll be conservative and not traverse parents to avoid infinite loops
            // The better approach is to filter these out during layout building
            false
        },
        _ => false
    }
}