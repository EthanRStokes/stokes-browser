use wgpu::util::DeviceExt;
use crate::Vertex;
use crate::dom::{Dom, DomNode, NodeType};
use std::rc::Rc;
use std::cell::RefCell;

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
    dom: Dom,
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
            dom: Dom::new(),
        }
    }

    pub fn set_html_content(&mut self, html: &str) {
        self.dom.parse_html(html);
        println!("DOM structure parsed:");
        self.dom.print_dom(); // Print DOM for debugging
    }

    pub fn create_content_boxes_from_dom(&mut self) {
        self.content_boxes.clear();
        let mut y_offset = 0.8f32; // Start from top
        
        self.process_node_for_layout(self.dom.document.clone(), &mut y_offset, 0);
    }
    
    fn process_node_for_layout(&mut self, node: Rc<RefCell<DomNode>>, y_offset: &mut f32, depth: usize) {
        let node_ref = node.borrow();
        let indent_factor = depth as f32 * 0.05; // Indentation based on depth
        
        match &node_ref.node_type {
            NodeType::Document => {
                // Process document's children
                for child in &node_ref.children {
                    self.process_node_for_layout(child.clone(), y_offset, depth);
                }
            },
            
            NodeType::Element(tag) => {
                // Create a box for this element
                let (color, height) = match tag.as_str() {
                    "title" => ([0.2, 0.4, 0.8], 0.12), // Blue for title
                    "h1" => ([0.1, 0.6, 0.1], 0.10),   // Green for h1
                    "h2" => ([0.1, 0.6, 0.1], 0.08),   // Green for h2
                    "h3" | "h4" | "h5" | "h6" => ([0.1, 0.6, 0.1], 0.06), // Green for other headers
                    "p" => ([0.8, 0.8, 0.8], 0.06),    // Light gray for paragraphs
                    "a" => ([0.8, 0.2, 0.2], 0.05),    // Red for links
                    "div" => ([0.6, 0.6, 0.6], 0.04),  // Gray for divs
                    _ => ([0.5, 0.5, 0.5], 0.04),      // Default gray for other elements
                };
                
                // Only create box if there's visible content
                let text = node_ref.get_text_content();
                if !text.trim().is_empty() {
                    self.content_boxes.push(ContentBox {
                        position: [-0.9 + indent_factor, *y_offset, 1.8 - indent_factor * 2.0, height],
                        color,
                    });
                    
                    *y_offset -= height + 0.02; // Add spacing between boxes
                }
                
                // Process children
                for child in &node_ref.children {
                    self.process_node_for_layout(child.clone(), y_offset, depth + 1);
                }
            },
            
            NodeType::Text(content) => {
                // Text nodes are typically rendered by their parent element
                // We won't create separate boxes for them
                // The content is already handled by the parent element
            },
            
            NodeType::Comment(_) => {
                // Comments are not rendered visually
            },
        }
        
        // Stop if we reach bottom of screen
        if *y_offset < -0.9 {
            return;
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
        device: &wgpu::Device,
    ) {
        // Update content boxes based on current DOM
        self.create_content_boxes_from_dom();

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
