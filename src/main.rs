use wgpu::{DeviceDescriptor, InstanceDescriptor, RequestAdapterOptions};
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
    window: Option<Window>,
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
    async fn new() -> Self {
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

        Self {
            browser,
            window: None,
        }
    }
}

impl ApplicationHandler for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = Window::default_attributes()
            .with_title("Stokes Browser")
            .with_inner_size(LogicalSize::new(800, 600));

        let window = event_loop.create_window(window_attributes).unwrap();

        // todo impl resizing

        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(window) = &self.window {
                    let size = window.inner_size();
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Hello, world!");
    let event_loop = EventLoop::new()?;
    let mut app = BrowserApp::new().await;

    event_loop.run_app(&mut app)?;

    Ok(())
}
