// Element renderer module - responsible for rendering different element types
use wgpu::{Device, Queue, RenderPass, Buffer};
use std::sync::Arc;
use wgpu::util::DeviceExt;
use crate::renderer::layout::{LayoutBox, BoxType};
use crate::Vertex;

/// Trait for rendering different types of HTML elements
pub trait ElementRenderer {
    fn can_render(&self, layout_box: &LayoutBox) -> bool;
    fn render(&self, layout_box: &LayoutBox, render_pass: &mut wgpu::RenderPass, index_buffer: &wgpu::Buffer);
    fn create_vertices(&self, layout_box: &LayoutBox) -> Vec<Vertex>;
}

/// Renders block elements (div, section, etc.)
pub struct BlockElementRenderer {
    device: Arc<wgpu::Device>,
}

impl BlockElementRenderer {
    pub fn new(device: Arc<wgpu::Device>) -> Self {
        Self { device }
    }
}

impl ElementRenderer for BlockElementRenderer {
    fn can_render(&self, layout_box: &LayoutBox) -> bool {
        matches!(layout_box.box_type, BoxType::Block) || matches!(layout_box.box_type, BoxType::Root)
    }

    fn render(&self, layout_box: &LayoutBox, render_pass: &mut wgpu::RenderPass, index_buffer: &wgpu::Buffer) {
        // Create vertices for this box
        let vertices = self.create_vertices(layout_box);

        // Skip empty boxes
        if vertices.is_empty() {
            return;
        }

        // Create vertex buffer
        let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Block Element Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Draw the box
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);
    }

    fn create_vertices(&self, layout_box: &LayoutBox) -> Vec<Vertex> {
        let x = layout_box.x;
        let y = layout_box.y;
        let width = layout_box.width;
        let height = layout_box.height;
        let color = layout_box.style.background_color;

        // Only create vertices for visible elements
        if width <= 0.0 || height <= 0.0 {
            return Vec::new();
        }

        vec![
            Vertex { position: [x, y, 0.0], color },
            Vertex { position: [x + width, y, 0.0], color },
            Vertex { position: [x + width, y + height, 0.0], color },
            Vertex { position: [x, y + height, 0.0], color },
        ]
    }
}

/// Renders inline elements (span, a, etc.)
pub struct InlineElementRenderer {
    device: Arc<wgpu::Device>,
}

impl InlineElementRenderer {
    pub fn new(device: Arc<wgpu::Device>) -> Self {
        Self { device }
    }
}

impl ElementRenderer for InlineElementRenderer {
    fn can_render(&self, layout_box: &LayoutBox) -> bool {
        matches!(layout_box.box_type, BoxType::Inline) || matches!(layout_box.box_type, BoxType::Anonymous)
    }

    fn render(&self, layout_box: &LayoutBox, render_pass: &mut wgpu::RenderPass, index_buffer: &wgpu::Buffer) {
        // Create vertices for this box
        let vertices = self.create_vertices(layout_box);

        // Skip empty boxes
        if vertices.is_empty() {
            return;
        }

        // Create vertex buffer
        let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Inline Element Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Draw the box
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);
    }

    fn create_vertices(&self, layout_box: &LayoutBox) -> Vec<Vertex> {
        let x = layout_box.x;
        let y = layout_box.y;
        let width = layout_box.width;
        let height = layout_box.height;
        let color = layout_box.style.color; // Use text color for inline elements

        // Only create vertices for visible elements
        if width <= 0.0 || height <= 0.0 {
            return Vec::new();
        }

        vec![
            Vertex { position: [x, y, 0.0], color },
            Vertex { position: [x + width, y, 0.0], color },
            Vertex { position: [x + width, y + height, 0.0], color },
            Vertex { position: [x, y + height, 0.0], color },
        ]
    }
}

/// Renders text elements
pub struct TextElementRenderer {
    device: Arc<wgpu::Device>,
    // TODO: Add font handling
}

impl TextElementRenderer {
    pub fn new(device: Arc<wgpu::Device>) -> Self {
        Self { device }
    }

    fn is_text_node(&self, layout_box: &LayoutBox) -> bool {
        use markup5ever_rcdom::NodeData;

        match &layout_box.node.data {
            NodeData::Text { .. } => true,
            _ => false,
        }
    }
}

impl ElementRenderer for TextElementRenderer {
    fn can_render(&self, layout_box: &LayoutBox) -> bool {
        matches!(layout_box.box_type, BoxType::Inline) && self.is_text_node(layout_box)
    }

    fn render(&self, layout_box: &LayoutBox, render_pass: &mut wgpu::RenderPass, index_buffer: &wgpu::Buffer) {
        // Create vertices for this text box (simple background for now)
        let vertices = self.create_vertices(layout_box);

        // Skip empty boxes
        if vertices.is_empty() {
            return;
        }

        // Create vertex buffer
        let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Text Element Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Draw the text background
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);

        // TODO: Add actual text rendering with font support
    }

    fn create_vertices(&self, layout_box: &LayoutBox) -> Vec<Vertex> {
        let x = layout_box.x;
        let y = layout_box.y;
        let width = layout_box.width;
        let height = layout_box.height;
        let color = layout_box.style.color;

        // Only create vertices for visible elements
        if width <= 0.0 || height <= 0.0 {
            return Vec::new();
        }

        vec![
            Vertex { position: [x, y, 0.0], color },
            Vertex { position: [x + width, y, 0.0], color },
            Vertex { position: [x + width, y + height, 0.0], color },
            Vertex { position: [x, y + height, 0.0], color },
        ]
    }
}

// Factory for creating the appropriate renderer for a layout box
pub struct ElementRendererFactory {
    renderers: Vec<Box<dyn ElementRenderer>>,
}

impl ElementRendererFactory {
    pub fn new(device: &Arc<Device>) -> Self {
        let mut renderers: Vec<Box<dyn ElementRenderer>> = Vec::new();

        // Add renderers in order of priority
        renderers.push(Box::new(TextElementRenderer::new(device.clone())));
        renderers.push(Box::new(InlineElementRenderer::new(device.clone())));
        renderers.push(Box::new(BlockElementRenderer::new(device.clone())));

        Self { renderers }
    }

    pub fn get_renderer(&self, layout_box: &LayoutBox) -> Option<&dyn ElementRenderer> {
        for renderer in &self.renderers {
            if renderer.can_render(layout_box) {
                return Some(renderer.as_ref());
            }
        }
        None
    }
}
