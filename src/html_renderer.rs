use wgpu::util::DeviceExt;
use crate::Vertex;
use crate::dom::{Dom, NodeType};
use markup5ever_rcdom::{Handle, NodeData};
use std::collections::HashMap;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ContentBox {
    pub position: [f32; 4], // x, y, width, height
    pub color: [f32; 3],
}

// Basic CSS styles for different HTML elements
struct ElementStyle {
    color: [f32; 3],
    height: f32,
    margin_top: f32,
    margin_bottom: f32,
    font_weight: f32, // 1.0 for normal, 1.5 for bold
}

pub struct HtmlRenderer {
    pub content_boxes: Vec<ContentBox>,
    render_pipeline: wgpu::RenderPipeline,
    index_buffer: wgpu::Buffer,
    dom: Dom,
    styles: HashMap<String, ElementStyle>,
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

        // Initialize element styles
        let mut styles = HashMap::new();

        // Default styles for common HTML elements
        styles.insert("html".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.0], height: 0.0, margin_top: 0.0, margin_bottom: 0.0, font_weight: 1.0
        });
        styles.insert("body".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.0], height: 0.0, margin_top: 0.0, margin_bottom: 0.0, font_weight: 1.0
        });
        styles.insert("div".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.0], height: 0.06, margin_top: 0.02, margin_bottom: 0.02, font_weight: 1.0
        });
        styles.insert("h1".to_string(), ElementStyle {
            color: [0.1, 0.1, 0.1], height: 0.12, margin_top: 0.05, margin_bottom: 0.03, font_weight: 1.5
        });
        styles.insert("h2".to_string(), ElementStyle {
            color: [0.1, 0.1, 0.1], height: 0.10, margin_top: 0.04, margin_bottom: 0.02, font_weight: 1.5
        });
        styles.insert("p".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.0], height: 0.06, margin_top: 0.02, margin_bottom: 0.02, font_weight: 1.0
        });
        styles.insert("a".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.8], height: 0.06, margin_top: 0.0, margin_bottom: 0.0, font_weight: 1.0
        });

        Self {
            content_boxes: Vec::new(),
            render_pipeline,
            index_buffer,
            dom: Dom::new(),
            styles,
        }
    }

    pub fn set_html_content(&mut self, html: &str) {
        self.dom.parse_html(html);
        println!("DOM structure parsed:");
        self.dom.print_dom(); // Print DOM for debugging
    }

    pub fn create_content_boxes_from_dom(&mut self) {
        self.content_boxes.clear();

        // Start layout from top of the screen
        let mut y_offset = 0.8f32;

        // Process the DOM starting from the document node
        self.process_node_for_layout(self.dom.document.clone(), &mut y_offset, 0);
    }
    
    fn process_node_for_layout(&mut self, node: Handle, y_offset: &f32, depth: usize) {
        let mut y_offset: f32 = *y_offset;
        // Calculate indentation based on depth
        let indent_factor = depth as f32 * 0.05;

        match &node.data {
            NodeData::Document => {
                // Process document's children
                for child in node.children.borrow().iter() {
                    self.process_node_for_layout(child.clone(), &y_offset, depth);
                }
            },

            NodeData::Element { name, .. } => {
                let tag_name = name.local.to_string();

                // Skip non-visible elements or containers
                if tag_name == "html" || tag_name == "head" || tag_name == "meta" || tag_name == "link" || tag_name == "script" {
                    // Just process their children without creating boxes
                    for child in node.children.borrow().iter() {
                        self.process_node_for_layout(child.clone(), &y_offset, depth);
                    }
                    return;
                }

                // Get element style or use a default
                let style = self.styles.get(&tag_name).unwrap_or(&ElementStyle {
                    color: [0.2, 0.2, 0.2],
                    height: 0.05,
                    margin_top: 0.01,
                    margin_bottom: 0.01,
                    font_weight: 1.0,
                });

                // Apply top margin
                y_offset -= style.margin_top;

                // Only create a box for elements with text content
                let text = Dom::get_text_content(&node);
                if !text.trim().is_empty() {
                    // Special coloring for specific elements
                    let color = match tag_name.as_str() {
                        "h1" => [0.2, 0.2, 0.7],  // Blue for h1
                        "h2" => [0.2, 0.5, 0.2],  // Green for h2
                        "a" => [0.0, 0.0, 0.8],   // Blue for links
                        _ => style.color,         // Default from style
                    };

                    // Create the content box
                    self.content_boxes.push(ContentBox {
                        position: [-0.9 + indent_factor, y_offset, 1.8 - indent_factor * 2.0, style.height],
                        color,
                    });
                    
                    // Move down by element height
                    y_offset -= style.height;
                }
                let margin_bottom = style.margin_bottom.clone();
                
                // Process children with increased depth
                for child in node.children.borrow().iter() {
                    self.process_node_for_layout(child.clone(), &y_offset, depth + 1);
                }

                // Apply bottom margin
                y_offset -= margin_bottom;
            },
            
            NodeData::Text { contents } => {
                // Check if parent is already handling this text
                let text = contents.borrow().to_string();
                if !text.trim().is_empty() && node.parent.take().is_none() {
                    // Only create a standalone text box if it's not inside a rendered element
                    self.content_boxes.push(ContentBox {
                        position: [-0.9 + indent_factor, y_offset, 1.8 - indent_factor * 2.0, 0.05],
                        color: [0.0, 0.0, 0.0],
                    });

                    y_offset -= 0.07; // height + spacing
                }
            },
            
            _ => {} // Skip other node types
        }
        
        // Stop if we reach bottom of screen
        if y_offset < -0.9 {
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
