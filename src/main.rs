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
}

impl Browser {
    fn new() -> Self {
        Self {
            url: String::new(),
        }
    }
}

impl BrowserApp {
    fn new() -> Self {
        let mut browser = Browser::new();
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Hello, world!");
    let event_loop = EventLoop::new()?;
    let mut app = BrowserApp::new();

    event_loop.run_app(&mut app)?;

    Ok(())
}
