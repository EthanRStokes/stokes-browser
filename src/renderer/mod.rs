// HTML renderer for displaying web content
use std::collections::HashMap;
use std::sync::Arc;
use std::rc::Rc;
use std::cell::RefCell;

use skia_safe::{Canvas, Color, Paint, Rect, TextBlob, Font, FontStyle};
use wgpu::{Device, Queue, Surface, SurfaceConfiguration, TextureView};

use crate::dom::{DomNode, NodeType};
use crate::layout::{LayoutBox, Display};

/// Renderer for HTML content
pub struct HtmlRenderer {
    device: Arc<Device>,
    queue: Arc<Queue>,
    surface_config: SurfaceConfiguration,
    fonts: HashMap<String, Font>,
    default_font: Font,
}

impl HtmlRenderer {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        surface_config: SurfaceConfiguration,
    ) -> Self {
        // Initialize default font
        let default_font = Font::new(None, 16.0);
        
        Self {
            device,
            queue,
            surface_config,
            fonts: HashMap::new(),
            default_font,
        }
    }
    
    /// Render the layout tree to a canvas
    pub fn render(&self, canvas: &mut Canvas, layout: &LayoutBox, dom_nodes: &HashMap<usize, Rc<RefCell<DomNode>>>) {
        // Clear canvas
        canvas.clear(Color::WHITE);
        
        // Recursively render the layout tree
        self.render_node(canvas, layout, dom_nodes);
    }
    
    /// Render a single layout node and its children
    fn render_node(&self, canvas: &mut Canvas, layout: &LayoutBox, dom_nodes: &HashMap<usize, Rc<RefCell<DomNode>>>) {
        // Skip rendering if not visible
        if layout.styles.display == Display::None {
            return;
        }
        
        // Get the DOM node for this layout box
        let node_opt = dom_nodes.get(&layout.node_id);
        
        if let Some(node) = node_opt {
            let node_borrow = node.borrow();
            
            // Render based on node type
            match &node_borrow.node_type {
                NodeType::Element(data) => {
                    // Render element background if it has one
                    if layout.styles.background_color[3] > 0.0 {
                        let mut paint = Paint::new(Color4f_to_skia(layout.styles.background_color), None);
                        paint.set_anti_alias(true);
                        
                        let rect = Rect::new(
                            layout.x, 
                            layout.y, 
                            layout.x + layout.width, 
                            layout.y + layout.height
                        );
                        
                        canvas.draw_rect(rect, &paint);
                    }
                    
                    // Special rendering for specific elements
                    match data.tag_name.as_str() {
                        "img" => {
                            self.render_image(canvas, layout, data);
                        },
                        _ => {
                            // Generic element rendering handled by children
                        }
                    }
                },
                NodeType::Text(content) => {
                    self.render_text(canvas, layout, content);
                },
                _ => {
                    // Don't render other node types
                }
            }
        }
        
        // Render children
        for child in &layout.children {
            self.render_node(canvas, child, dom_nodes);
        }
    }
    
    /// Render a text node
    fn render_text(&self, canvas: &mut Canvas, layout: &LayoutBox, content: &str) {
        let mut paint = Paint::new(Color4f_to_skia(layout.styles.color), None);
        paint.set_anti_alias(true);
        
        // Get or create font based on style
        let font = self.get_font_for_style(&layout.styles);
        
        // Create text blob
        if let Some(blob) = TextBlob::new(content, &font) {
            canvas.draw_text_blob(blob, (layout.x, layout.y + layout.styles.font_size), &paint);
        }
    }
    
    /// Render an image element
    fn render_image(&self, canvas: &mut Canvas, layout: &LayoutBox, data: &crate::dom::ElementData) {
        // In a real implementation, you would:
        // 1. Get the src attribute from data
        // 2. Load the image (or use a cached version)
        // 3. Draw it at the layout position
        
        // For now, just draw a placeholder rectangle
        let mut paint = Paint::new(Color::from_rgb(200, 200, 200), None);
        paint.set_anti_alias(true);
        
        let rect = Rect::new(
            layout.x, 
            layout.y, 
            layout.x + layout.width, 
            layout.y + layout.height
        );
        
        canvas.draw_rect(rect, &paint);
        
        // Draw a crossed box to indicate an image placeholder
        let mut line_paint = Paint::new(Color::from_rgb(150, 150, 150), None);
        line_paint.set_anti_alias(true);
        line_paint.set_stroke_width(1.0);
        line_paint.set_style(skia_safe::PaintStyle::Stroke);
        
        canvas.draw_line(
            (layout.x, layout.y), 
            (layout.x + layout.width, layout.y + layout.height), 
            &line_paint
        );
        
        canvas.draw_line(
            (layout.x + layout.width, layout.y), 
            (layout.x, layout.y + layout.height), 
            &line_paint
        );
    }
    
    /// Get a font based on style properties
    fn get_font_for_style(&self, styles: &crate::layout::StyleProperties) -> &Font {
        // In a real implementation, you would:
        // 1. Create a cache key based on font family, size, weight, etc.
        // 2. Check if the font exists in the cache
        // 3. If not, create it and add to cache
        
        // For now, just return the default font
        &self.default_font
    }
}

/// Convert [f32; 4] color to skia Color
fn Color4f_to_skia(color: [f32; 4]) -> Color {
    Color::from_argb(
        (color[3] * 255.0) as u8,
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
    )
}
