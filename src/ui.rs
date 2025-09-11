// Browser UI components
use wgpu::util::DeviceExt;
use winit::window::Window;
use std::sync::Arc;
use crate::Vertex;

/// Represents a UI component in the browser chrome
#[derive(Debug, Clone)]
pub enum UiComponent {
    Button {
        id: String,
        label: String,
        position: [f32; 2],
        size: [f32; 2],
        color: [f32; 3],
        hover_color: [f32; 3],
        is_hover: bool,
        is_active: bool,
    },
    TextField {
        id: String,
        text: String,
        position: [f32; 2],
        size: [f32; 2],
        color: [f32; 3],
        border_color: [f32; 3],
        has_focus: bool,
    },
    TabButton {
        id: String,
        title: String,
        position: [f32; 2],
        size: [f32; 2],
        color: [f32; 3],
        is_active: bool,
    }
}

impl UiComponent {
    /// Create a navigation button (back, forward, refresh)
    pub fn navigation_button(id: &str, label: &str, x_pos: f32) -> Self {
        UiComponent::Button {
            id: id.to_string(),
            label: label.to_string(),
            position: [x_pos, 0.9],
            size: [0.05, 0.05],
            color: [0.8, 0.8, 0.8],
            hover_color: [0.9, 0.9, 1.0],
            is_hover: false,
            is_active: false,
        }
    }

    /// Create an address bar
    pub fn address_bar(url: &str) -> Self {
        UiComponent::TextField {
            id: "address_bar".to_string(),
            text: url.to_string(),
            position: [0.15, 0.9],
            size: [0.7, 0.05],
            color: [1.0, 1.0, 1.0],
            border_color: [0.7, 0.7, 0.7],
            has_focus: false,
        }
    }

    /// Create a tab button
    pub fn tab(id: &str, title: &str, index: usize, is_active: bool) -> Self {
        let x_pos = -0.95 + (index as f32 * 0.15);
        UiComponent::TabButton {
            id: id.to_string(),
            title: title.to_string(),
            position: [x_pos, 0.95],
            size: [0.14, 0.04],
            color: if is_active { [0.9, 0.9, 0.9] } else { [0.7, 0.7, 0.7] },
            is_active,
        }
    }

    /// Check if a point is inside this component
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        match self {
            UiComponent::Button { position, size, .. } |
            UiComponent::TextField { position, size, .. } |
            UiComponent::TabButton { position, size, .. } => {
                x >= position[0] - size[0] / 2.0 &&
                x <= position[0] + size[0] / 2.0 &&
                y >= position[1] - size[1] / 2.0 &&
                y <= position[1] + size[1] / 2.0
            }
        }
    }

    /// Get component ID
    pub fn id(&self) -> &str {
        match self {
            UiComponent::Button { id, .. } |
            UiComponent::TextField { id, .. } |
            UiComponent::TabButton { id, .. } => id,
        }
    }
}

/// Represents the browser UI (chrome)
pub struct BrowserUI {
    components: Vec<UiComponent>,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl BrowserUI {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        // Initialize with standard browser UI components
        let mut components = Vec::new();

        // Navigation buttons
        components.push(UiComponent::navigation_button("back", "←", -0.95));
        components.push(UiComponent::navigation_button("forward", "→", -0.89));
        components.push(UiComponent::navigation_button("refresh", "↻", -0.83));

        // Address bar
        components.push(UiComponent::address_bar("https://example.com"));

        // Initial tab
        components.push(UiComponent::tab("tab1", "New Tab", 0, true));

        Self {
            components,
            vertex_buffer: None,
            index_buffer: None,
            render_pipeline: None,
            device,
            queue,
        }
    }

    /// Initialize rendering resources
    pub fn initialize_renderer(&mut self, surface_format: wgpu::TextureFormat) {
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UI Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let render_pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UI Pipeline Layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let render_pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UI Pipeline"),
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

        // Standard quad indices for UI elements
        let indices = &[0, 1, 2, 2, 3, 0];
        let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI Index Buffer"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        self.render_pipeline = Some(render_pipeline);
        self.index_buffer = Some(index_buffer);
    }

    /// Update the address bar with a new URL
    pub fn update_address_bar(&mut self, url: &str) {
        for component in &mut self.components {
            if let UiComponent::TextField { id, text, .. } = component {
                if id == "address_bar" {
                    *text = url.to_string();
                    break;
                }
            }
        }
    }

    /// Update tab title
    pub fn update_tab_title(&mut self, tab_id: &str, title: &str) {
        for component in &mut self.components {
            if let UiComponent::TabButton { id, title: tab_title, .. } = component {
                if id == tab_id {
                    // Truncate long titles
                    if title.len() > 15 {
                        *tab_title = format!("{}...", &title[0..12]);
                    } else {
                        *tab_title = title.to_string();
                    }
                    break;
                }
            }
        }
    }

    /// Add a new tab
    pub fn add_tab(&mut self, id: &str, title: &str) {
        // Count existing tabs
        let mut tab_count = 0;
        for component in &self.components {
            if let UiComponent::TabButton { .. } = component {
                tab_count += 1;
            }
        }

        // Set all tabs to inactive
        for component in &mut self.components {
            if let UiComponent::TabButton { is_active, .. } = component {
                *is_active = false;
            }
        }

        // Add the new tab
        self.components.push(UiComponent::tab(id, title, tab_count, true));
    }

    /// Handle mouse click
    pub fn handle_click(&mut self, x: f32, y: f32) -> Option<String> {
        for component in &mut self.components {
            if component.contains_point(x, y) {
                return Some(component.id().to_string());
            }
        }
        None
    }

    /// Render the UI
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        if self.render_pipeline.is_none() || self.index_buffer.is_none() {
            return;
        }

        let render_pipeline = self.render_pipeline.as_ref().unwrap();
        let index_buffer = self.index_buffer.as_ref().unwrap();

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI Render Pass"),
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

        render_pass.set_pipeline(render_pipeline);
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);

        // Render each UI component
        for component in &self.components {
            let vertices = self.create_component_vertices(component);

            let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("UI Vertex Buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.draw_indexed(0..6, 0, 0..1);
        }
    }

    /// Create vertices for a UI component
    fn create_component_vertices(&self, component: &UiComponent) -> Vec<Vertex> {
        match component {
            UiComponent::Button { position, size, color, is_hover, .. } => {
                let x = position[0];
                let y = position[1];
                let width = size[0];
                let height = size[1];
                let color = if *is_hover {
                    [color[0] + 0.1, color[1] + 0.1, color[2] + 0.1]
                } else {
                    *color
                };

                vec![
                    Vertex { position: [x - width/2.0, y - height/2.0, 0.0], color },
                    Vertex { position: [x + width/2.0, y - height/2.0, 0.0], color },
                    Vertex { position: [x + width/2.0, y + height/2.0, 0.0], color },
                    Vertex { position: [x - width/2.0, y + height/2.0, 0.0], color },
                ]
            },
            UiComponent::TextField { position, size, color, border_color, .. } => {
                let x = position[0];
                let y = position[1];
                let width = size[0];
                let height = size[1];

                // For simplicity, just render the background box
                vec![
                    Vertex { position: [x - width/2.0, y - height/2.0, 0.0], color: *color },
                    Vertex { position: [x + width/2.0, y - height/2.0, 0.0], color: *color },
                    Vertex { position: [x + width/2.0, y + height/2.0, 0.0], color: *color },
                    Vertex { position: [x - width/2.0, y + height/2.0, 0.0], color: *color },
                ]
            },
            UiComponent::TabButton { position, size, color, is_active, .. } => {
                let x = position[0];
                let y = position[1];
                let width = size[0];
                let height = size[1];

                // Use a slightly different color for active tabs
                let color = if *is_active {
                    [color[0] + 0.05, color[1] + 0.05, color[2] + 0.05]
                } else {
                    *color
                };

                vec![
                    Vertex { position: [x - width/2.0, y - height/2.0, 0.0], color },
                    Vertex { position: [x + width/2.0, y - height/2.0, 0.0], color },
                    Vertex { position: [x + width/2.0, y + height/2.0, 0.0], color },
                    Vertex { position: [x - width/2.0, y + height/2.0, 0.0], color },
                ]
            }
        }
    }
}
