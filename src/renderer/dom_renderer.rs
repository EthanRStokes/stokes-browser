// DOM Renderer - main renderer that coordinates styling, layout, and painting
use markup5ever_rcdom::Handle;
use std::sync::Arc;
use crate::renderer::style::StyleResolver;
use crate::renderer::layout::LayoutEngine;
use crate::renderer::painter::Painter;

/// Main renderer that coordinates the rendering pipeline
pub struct DomRenderer {
    style_resolver: StyleResolver,
    layout_engine: LayoutEngine,
    painter: Painter,
    viewport_width: f32,
    viewport_height: f32,
}

impl DomRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        surface_format: wgpu::TextureFormat,
        viewport_width: f32,
        viewport_height: f32
    ) -> Self {
        let style_resolver = StyleResolver::new();
        let layout_engine = LayoutEngine::new(viewport_width, viewport_height);
        let painter = Painter::new(device, queue, surface_format);

        Self {
            style_resolver,
            layout_engine,
            painter,
            viewport_width,
            viewport_height,
        }
    }

    /// Render the DOM tree to the screen
    pub fn render(&mut self, document: &Handle, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        // Phase 1: Style Resolution
        // Apply styles to all elements in the DOM
        // In a real implementation, this would traverse the DOM and compute styles

        // Phase 2: Layout
        // Convert the styled DOM into a layout tree
        let layout_tree = self.layout_engine.create_layout(document);

        // Phase 3: Painting
        // Render the layout tree to the screen

        // First, clear the screen with the background color
        // Get background color from HTML or BODY element
        let background_color = [1.0, 1.0, 1.0]; // Default white
        self.painter.clear_screen(encoder, view, background_color);

        // Then render the layout tree
        self.painter.render_layout_tree(&layout_tree, encoder, view);
    }

    /// Update the viewport dimensions
    pub fn resize(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.layout_engine = LayoutEngine::new(width, height);
    }
}
