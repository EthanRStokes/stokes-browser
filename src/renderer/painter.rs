// Painter module - responsible for rendering layout boxes to the screen
use wgpu::util::DeviceExt;
use std::sync::Arc;
use crate::Vertex;
use crate::renderer::layout::LayoutBox;

/// Painter that handles rendering layout boxes to the screen
pub struct Painter {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    render_pipeline: wgpu::RenderPipeline,
    index_buffer: wgpu::Buffer,
}

impl Painter {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>, surface_format: wgpu::TextureFormat) -> Self {
        // Set up the rendering pipeline
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("HTML Renderer Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shader.wgsl").into()),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HTML Renderer Pipeline Layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HTML Renderer Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Create index buffer for rectangle drawing
        let rect_indices: &[u16] = &[0, 1, 2, 2, 3, 0];
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Rectangle Index Buffer"),
            contents: bytemuck::cast_slice(rect_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            device,
            queue,
            render_pipeline,
            index_buffer,
        }
    }

    /// Clear the screen with a background color
    pub fn clear_screen(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, color: [f32; 3]) {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Clear Screen Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: color[0] as f64,
                        g: color[1] as f64,
                        b: color[2] as f64,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
    }

    /// Render a layout box and all its children
    pub fn render_layout_tree(
        &self,
        layout_root: &LayoutBox,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView
    ) {
        // In a real implementation, this would traverse the layout tree
        // and render each box according to its style and position
        todo!("Implement rendering of layout tree");
    }

    /// Render a single layout box
    fn render_box(&self, layout_box: &LayoutBox, encoder: &mut wgpu::CommandEncoder, render_pass: &mut wgpu::RenderPass) {
        // Create vertices for this box
        let vertices = self.create_box_vertices(layout_box);

        // Create vertex buffer
        let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Box Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Draw the box
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);
    }

    /// Create vertices for a layout box
    fn create_box_vertices(&self, layout_box: &LayoutBox) -> Vec<Vertex> {
        let x = layout_box.x;
        let y = layout_box.y;
        let width = layout_box.width;
        let height = layout_box.height;
        let color = layout_box.style.background_color;

        vec![
            Vertex { position: [x, y, 0.0], color },
            Vertex { position: [x + width, y, 0.0], color },
            Vertex { position: [x + width, y + height, 0.0], color },
            Vertex { position: [x, y + height, 0.0], color },
        ]
    }
}
