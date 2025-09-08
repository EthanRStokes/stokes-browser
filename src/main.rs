use std::sync::Arc;
use wgpu::{DeviceDescriptor, InstanceDescriptor, RequestAdapterOptions, TextureFormat};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

struct Browser {
    url: String,
}

struct BrowserApp {
    browser: Browser,
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: winit::dpi::PhysicalSize<u32>,
    surface: wgpu::Surface<'static>,
    surface_format: wgpu::TextureFormat,
}

impl Browser {
    fn new() -> Self {
        Self {
            url: String::new(),
        }
    }
}

impl BrowserApp {
    async fn new(window: Arc<Window>) -> Self {
        let mut browser = Browser::new();
        let instance = wgpu::Instance::new(&InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&RequestAdapterOptions::default())
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default())
            .await
            .unwrap();

        let surface = instance.create_surface(window.clone()).unwrap();
        let cap = surface.get_capabilities(&adapter);
        let surface_format = cap.formats[0];

        Self {
            browser,
            window,
            device,
            queue,
            size: Default::default(),
            surface,
            surface_format,
        }
    }
}

impl ApplicationHandler for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // todo impl resizing
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let size = self.window.inner_size();
                self.window.request_redraw();
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Hello, world!");
    let event_loop = EventLoop::new()?;
    let window = Arc::new(
        event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Stokes Browser")
                    .with_inner_size(LogicalSize::new(800, 600)),
            )
            .unwrap()
    );
    let mut app = BrowserApp::new(window).await;

    event_loop.run_app(&mut app)?;

    Ok(())
}
