mod engine;
mod networking;
mod ui;

use std::sync::Arc;
use wgpu::{DeviceDescriptor, InstanceDescriptor, RequestAdapterOptions};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::engine::{Engine, EngineConfig};
use crate::ui::BrowserUI;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

impl Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}

/// Tab structure representing a browser tab
struct Tab {
    id: String,
    engine: Engine,
}

impl Tab {
    fn new(id: &str, config: EngineConfig) -> Self {
        Self {
            id: id.to_string(),
            engine: Engine::new(config),
        }
    }
}

/// The main browser application
struct BrowserApp {
    tabs: Vec<Tab>,
    active_tab_index: usize,
    ui: BrowserUI,
    window: Arc<Window>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    size: winit::dpi::PhysicalSize<u32>,
    surface: wgpu::Surface<'static>,
    surface_format: wgpu::TextureFormat,
    surface_config: wgpu::SurfaceConfiguration,
}

impl BrowserApp {
    async fn new(window: Arc<Window>) -> Self {
        // Initialize wgpu
        let instance = wgpu::Instance::new(&InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&RequestAdapterOptions::default())
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default())
            .await
            .unwrap();

        // Share device and queue using Arc
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // Initialize surface
        let surface = instance.create_surface(window.clone()).unwrap();
        let cap = surface.get_capabilities(&adapter);
        let surface_format = cap.formats[0];

        let size = window.inner_size();
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: cap.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        // Initialize UI
        let mut ui = BrowserUI::new(Arc::clone(&device), Arc::clone(&queue));
        ui.initialize_renderer(surface_format);

        // Create initial tab
        let config = EngineConfig::default();
        let initial_tab = Tab::new("tab1", config.clone());

        Self {
            tabs: vec![initial_tab],
            active_tab_index: 0,
            ui,
            window,
            device,
            queue,
            size,
            surface,
            surface_format,
            surface_config,
        }
    }

    // Get the currently active tab
    fn active_tab(&self) -> &Tab {
        &self.tabs[self.active_tab_index]
    }

    // Get the currently active tab as mutable
    fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab_index]
    }

    // Navigate to a URL in the current tab
    async fn navigate(&mut self, url: &str) {
        // Update window title to show loading state
        self.window.set_title(&format!("Loading: {}", url));

        // First, navigate and store the result
        let tab_id = self.active_tab().id.clone();
        let navigation_result = self.active_tab_mut().engine.navigate(url).await;

        // Now process the result without holding multiple mutable borrows
        match navigation_result {
            Ok(_) => {
                // Get data from active tab with an immutable borrow
                let current_url = self.active_tab().engine.current_url().to_string();
                let title = self.active_tab().engine.page_title().to_string();

                // Update UI with the data we already collected
                self.ui.update_address_bar(&current_url);
                self.ui.update_tab_title(&tab_id, &title);

                // Update window title
                self.window.set_title(&format!("{} - Stokes Browser", title));
            }
            Err(e) => {
                println!("Navigation error: {}", e);
                self.window.set_title("Error - Stokes Browser");
            }
        }
    }

    // Add a new tab
    fn add_tab(&mut self) {
        let tab_id = format!("tab{}", self.tabs.len() + 1);
        let config = EngineConfig::default();
        let new_tab = Tab::new(&tab_id, config);

        // Add to tabs list
        self.tabs.push(new_tab);
        self.active_tab_index = self.tabs.len() - 1;

        // Update UI
        self.ui.add_tab(&tab_id, "New Tab");
    }

    // Switch to a tab by index
    fn switch_to_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab_index = index;

            // Update UI to reflect active tab
            let url = self.active_tab().engine.current_url();;
            let title = self.active_tab().engine.page_title();
        // TODO   self.ui.update_address_bar(url);
            self.window.set_title(&format!("{} - Stokes Browser", title));
        }
    }

    // Handle mouse click
    fn handle_click(&mut self, x: f32, y: f32) {
        // Convert window coordinates to normalized device coordinates
        let ndc_x = (x / self.size.width as f32) * 2.0 - 1.0;
        let ndc_y = -((y / self.size.height as f32) * 2.0 - 1.0);

        // Check for UI interaction
        if let Some(component_id) = self.ui.handle_click(ndc_x, ndc_y) {
            // Handle based on component
            if component_id == "back" {
                println!("Back button clicked");
                // Back navigation would go here
            } else if component_id == "forward" {
                println!("Forward button clicked");
                // Forward navigation would go here
            } else if component_id == "refresh" {
                println!("Refresh button clicked");
                // Page refresh would go here
            } else if component_id.starts_with("tab") {
                // Tab switching
                let tab_index = component_id[3..].parse::<usize>().unwrap_or(1) - 1;
                if tab_index < self.tabs.len() {
                    self.switch_to_tab(tab_index);
                }
            }
        }
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        // Create a Skia surface for rendering HTML content
        let surface_texture = output.texture.as_hal::<skia_safe::gpu::backend_render_targets::D3D12RenderTargetHandle>(
            &wgpu::hal::D3D12 {},
            &wgpu::TextureViewDescriptor::default(),
        ).expect("Failed to get texture handle");

        // Create Skia context and surface
        let context = skia_safe::gpu::DirectContext::new(self.device.as_ref(), self.queue.as_ref())
            .expect("Failed to create Skia context");

        let render_target_info = skia_safe::gpu::backend_render_targets::WGPURenderTargetInfo {
            width: self.size.width as i32,
            height: self.size.height as i32,
            sample_count: 1,
            color_type: skia_safe::ColorType::RGBA8888,
            surface_origin: skia_safe::gpu::SurfaceOrigin::TopLeft,
            srgb_encoded: false,
        };

        let render_target = skia_safe::gpu::backend_render_targets::make_d3d12(render_target_info, surface_texture);
        let surface = skia_safe::Surface::from_backend_render_target(
            &context,
            &render_target,
            skia_safe::SurfaceOrigin::TopLeft,
            skia_safe::ColorType::RGBA8888,
            None,
            None,
        ).expect("Failed to create Skia surface");

        // Get canvas from surface
        let canvas = surface.canvas();

        // Clear canvas with white background
        canvas.clear(skia_safe::Color::WHITE);

        // Render active tab content
        self.active_tab().engine.render(canvas);


        self.ui.render(&mut encoder, &view);

        Ok(())
    }
}

impl ApplicationHandler for BrowserApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Window is already created, trigger a redraw
        self.window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if new_size.width > 0 && new_size.height > 0 {
                    self.size = new_size;
                    self.surface_config.width = new_size.width;
                    self.surface_config.height = new_size.height;
                    self.surface.configure(&self.device, &self.surface_config);
                }
                self.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                match self.render() {
                    Ok(_) => {}
                    Err(wgpu::SurfaceError::Lost) => {
                        self.surface.configure(&self.device, &self.surface_config);
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        event_loop.exit();
                    }
                    Err(e) => {
                        eprintln!("{:?}", e);
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
            //    // Get cursor position
            //    if let Some(position) = self.window.cursor_position() {
            //        let PhysicalPosition { x, y } = position;
            //        self.handle_click(x as f32, y as f32);
            //    }
                self.window.request_redraw();
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting Stokes Browser...");
    let event_loop = EventLoop::new()?;

    let window = Arc::new(
        event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Stokes Browser")
                    .with_inner_size(LogicalSize::new(1024, 768)),
            )
            .unwrap()
    );

    let mut app = BrowserApp::new(window).await;
    println!("Browser initialized, navigating to homepage...");

    // Navigate to the default homepage
    app.navigate("https://example.com").await;

    event_loop.run_app(&mut app)?;

    Ok(())
}
