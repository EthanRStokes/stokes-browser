use wgpu::util::DeviceExt;
use crate::Vertex;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ContentBox {
    pub position: [f32; 4], // x, y, width, height
    pub color: [f32; 3],
}

pub struct HtmlRenderer {
    pub content_boxes: Vec<ContentBox>,
    render_pipeline: wgpu::RenderPipeline,
    index_buffer: wgpu::Buffer,
}

const BOX_INDICES: &[u16] = &[
    0, 1, 2,
    2, 3, 0,
];

impl HtmlRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("HTML Content Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HTML Content Pipeline Layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HTML Content Pipeline"),
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

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HTML Content Index Buffer"),
            contents: bytemuck::cast_slice(BOX_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            content_boxes: Vec::new(),
            render_pipeline,
            index_buffer,
        }
    }

    pub fn create_content_boxes(&mut self, parsed_text: &[String]) {
        self.content_boxes.clear();
        let mut y_offset = 0.8f32; // Start from top

        for (i, text) in parsed_text.iter().enumerate() {
            if i >= 20 { break; } // Limit to first 20 elements

            let (color, height) = if text.starts_with("TITLE:") {
                ([0.2, 0.4, 0.8], 0.12) // Blue for title
            } else if text.starts_with("H1:") || text.starts_with("H2:") || text.starts_with("H3:") {
                ([0.1, 0.6, 0.1], 0.08) // Green for headers
            } else if text.starts_with("P:") {
                ([0.8, 0.8, 0.8], 0.06) // Light gray for paragraphs
            } else if text.starts_with("A:") {
                ([0.8, 0.2, 0.2], 0.05) // Red for links
            } else {
                ([0.6, 0.6, 0.6], 0.04) // Gray for other elements
            };

            self.content_boxes.push(ContentBox {
                position: [-0.9, y_offset, 1.8, height], // x, y, width, height
                color,
            });

            y_offset -= height + 0.02; // Add spacing between boxes
            if y_offset < -0.9 { break; } // Stop if we reach bottom
        }
    }

    pub fn create_box_vertices(&self, content_box: &ContentBox) -> Vec<Vertex> {
        let x = content_box.position[0];
        let y = content_box.position[1];
        let width = content_box.position[2];
        let height = content_box.position[3];
        let color = content_box.color;

        vec![
            Vertex { position: [x, y - height, 0.0], color },
            Vertex { position: [x + width, y - height, 0.0], color },
            Vertex { position: [x + width, y, 0.0], color },
            Vertex { position: [x, y, 0.0], color },
        ]
    }

    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        parsed_text: &[String],
        device: &wgpu::Device,
    ) {
        // Update content boxes based on current parsed text
        self.create_content_boxes(parsed_text);

        if self.content_boxes.is_empty() {
            return;
        }

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HTML Content Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

        // Render each content box
        for content_box in &self.content_boxes {
            let box_vertices = self.create_box_vertices(content_box);

            // Create a new vertex buffer for this box
            let box_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("HTML Content Box Vertex Buffer"),
                contents: bytemuck::cast_slice(&box_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

            render_pass.set_vertex_buffer(0, box_vertex_buffer.slice(..));
            render_pass.draw_indexed(0..BOX_INDICES.len() as u32, 0, 0..1);
        }
    }
}
