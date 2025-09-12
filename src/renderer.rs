// Renderer module for handling all visual aspects of web page rendering
use wgpu::util::DeviceExt;
use std::sync::Arc;
use crate::Vertex;
use crate::dom::{Dom, NodeType};
use markup5ever_rcdom::{Handle, NodeData};

/// Represents a rendered box for an HTML element
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ContentBox {
    pub position: [f32; 4], // x, y, width, height
    pub color: [f32; 3],
}

/// CSS styling information for elements
pub struct ElementStyle {
    pub color: [f32; 3],
    pub background_color: [f32; 3],
    pub height: f32,
    pub width: Option<f32>, // None means auto/100%
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub font_size: f32,
    pub font_weight: f32, // 1.0 for normal, 1.5 for bold
    pub display: DisplayType,
}

/// Display types for elements
pub enum DisplayType {
    Block,
    Inline,
    None,
}

/// The main renderer for web content
pub struct Renderer {
    pub content_boxes: Vec<ContentBox>,
    render_pipeline: wgpu::RenderPipeline,
    index_buffer: wgpu::Buffer,
    stylesheet: StyleSheet,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

/// A stylesheet that contains styling rules
pub struct StyleSheet {
    element_styles: std::collections::HashMap<String, ElementStyle>,
}

impl StyleSheet {
    pub fn new() -> Self {
        let mut element_styles = std::collections::HashMap::new();

        // Default styles for common HTML elements
        element_styles.insert("html".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.0],
            background_color: [1.0, 1.0, 1.0],
            height: 0.0,
            width: None,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            font_size: 1.0,
            font_weight: 1.0,
            display: DisplayType::Block,
        });

        element_styles.insert("body".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.0],
            background_color: [1.0, 1.0, 1.0],
            height: 0.0,
            width: None,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            font_size: 1.0,
            font_weight: 1.0,
            display: DisplayType::Block,
        });

        element_styles.insert("div".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.0],
            background_color: [1.0, 1.0, 1.0],
            height: 0.06,
            width: None,
            margin_top: 0.02,
            margin_right: 0.0,
            margin_bottom: 0.02,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            font_size: 1.0,
            font_weight: 1.0,
            display: DisplayType::Block,
        });

        element_styles.insert("h1".to_string(), ElementStyle {
            color: [0.1, 0.1, 0.1],
            background_color: [1.0, 1.0, 1.0],
            height: 0.12,
            width: None,
            margin_top: 0.05,
            margin_right: 0.0,
            margin_bottom: 0.03,
            margin_left: 0.0,
            padding_top: 0.01,
            padding_right: 0.01,
            padding_bottom: 0.01,
            padding_left: 0.01,
            font_size: 2.0,
            font_weight: 1.5,
            display: DisplayType::Block,
        });

        element_styles.insert("p".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.0],
            background_color: [1.0, 1.0, 1.0],
            height: 0.06,
            width: None,
            margin_top: 0.02,
            margin_right: 0.0,
            margin_bottom: 0.02,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            font_size: 1.0,
            font_weight: 1.0,
            display: DisplayType::Block,
        });

        element_styles.insert("a".to_string(), ElementStyle {
            color: [0.0, 0.0, 0.8],
            background_color: [1.0, 1.0, 1.0],
            height: 0.06,
            width: None,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            font_size: 1.0,
            font_weight: 1.0,
            display: DisplayType::Inline,
        });

        Self { element_styles }
    }

    pub fn get_style(&self, tag_name: &str) -> Option<&ElementStyle> {
        self.element_styles.get(tag_name)
    }
}

impl Renderer {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>, surface_format: wgpu::TextureFormat) -> Self {
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

        let box_indices: &[u16] = &[0, 1, 2, 2, 3, 0];

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HTML Content Index Buffer"),
            contents: bytemuck::cast_slice(box_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            content_boxes: Vec::new(),
            render_pipeline,
            index_buffer,
            stylesheet: StyleSheet::new(),
            device,
            queue,
        }
    }

    /// Layout and render the DOM
    pub fn layout_and_render(&mut self, document: &Handle, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        // Clear previous layout
        self.content_boxes.clear();

        // Start layout from top of the screen
        let mut y_offset = 0.8f32;

        // Process the DOM to create layout boxes
        self.process_node_for_layout(document.clone(), &mut y_offset, 0);

        // Render the boxes
        self.render_content_boxes(encoder, view);
    }

    /// Process a DOM node for layout
    fn process_node_for_layout(&mut self, node: Handle, y_offset: &mut f32, depth: usize) {
        // Calculate indentation based on depth
        let indent_factor = depth as f32 * 0.05;

        match &node.data {
            NodeData::Document => {
                // Process document's children
                for child in node.children.borrow().iter() {
                    self.process_node_for_layout(child.clone(), y_offset, depth);
                }
            },

            NodeData::Element { name, .. } => {
                let tag_name = name.local.to_string();

                // Skip non-visible elements or containers
                if tag_name == "html" || tag_name == "head" || tag_name == "meta" || tag_name == "link" || tag_name == "script" {
                    // Just process their children without creating boxes
                    for child in node.children.borrow().iter() {
                        self.process_node_for_layout(child.clone(), y_offset, depth);
                    }
                    return;
                }

                // Get element style or use a default
                let style = match self.stylesheet.get_style(&tag_name) {
                    Some(style) => style,
                    None => &ElementStyle {
                        color: [0.2, 0.2, 0.2],
                        background_color: [1.0, 1.0, 1.0],
                        height: 0.05,
                        width: None,
                        margin_top: 0.01,
                        margin_right: 0.0,
                        margin_bottom: 0.01,
                        margin_left: 0.0,
                        padding_top: 0.0,
                        padding_right: 0.0,
                        padding_bottom: 0.0,
                        padding_left: 0.0,
                        font_size: 1.0,
                        font_weight: 1.0,
                        display: DisplayType::Block,
                    }
                };

                // Apply top margin
                *y_offset -= style.margin_top;

                // Only create a box for elements with text content or with styling needs
                let text = Dom::get_text_content(&node);
                if !text.trim().is_empty() || matches!(style.display, DisplayType::Block) {
                    // Special coloring for specific elements
                    let color = match tag_name.as_str() {
                        "h1" => [0.2, 0.2, 0.7],  // Blue for h1
                        "h2" => [0.2, 0.5, 0.2],  // Green for h2
                        "a" => [0.0, 0.0, 0.8],   // Blue for links
                        _ => style.color,         // Default from style
                    };

                    // Create the content box
                    self.content_boxes.push(ContentBox {
                        position: [
                            -0.9 + indent_factor + style.margin_left,
                            *y_offset,
                            1.8 - indent_factor * 2.0 - style.margin_left - style.margin_right,
                            style.height
                        ],
                        color,
                    });

                    // Move down by element height plus padding
                    *y_offset -= style.height + style.padding_top + style.padding_bottom;
                }

                let margin_bottom = style.margin_bottom;

                // Process children with increased depth
                for child in node.children.borrow().iter() {
                    self.process_node_for_layout(child.clone(), y_offset, depth + 1);
                }

                // Apply bottom margin
                *y_offset -= margin_bottom;
            },

            NodeData::Text { contents } => {
                // Check if parent is already handling this text
                let text = contents.borrow().to_string();
                if !text.trim().is_empty() && node.parent.take().is_none() {
                    // Only create a standalone text box if it's not inside a rendered element
                    self.content_boxes.push(ContentBox {
                        position: [-0.9 + indent_factor, *y_offset, 1.8 - indent_factor * 2.0, 0.05],
                        color: [0.0, 0.0, 0.0],
                    });

                    *y_offset -= 0.07; // height + spacing
                }
            },

            _ => {} // Skip other node types
        }
    }

    /// Create vertices for a content box
    fn create_box_vertices(&self, content_box: &ContentBox) -> Vec<Vertex> {
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

    /// Render all content boxes
    fn render_content_boxes(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
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
            let box_vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("HTML Content Box Vertex Buffer"),
                contents: bytemuck::cast_slice(&box_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

            render_pass.set_vertex_buffer(0, box_vertex_buffer.slice(..));
            render_pass.draw_indexed(0..6, 0, 0..1);
        }
    }
}
