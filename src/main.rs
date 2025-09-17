mod engine;
mod networking;
mod ui;
mod dom;

use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::engine::{Engine, EngineConfig};
use crate::ui::BrowserUI;
use skia_safe::{Surface, gpu, Color};

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
    size: winit::dpi::PhysicalSize<u32>,
    skia_context: gpu::DirectContext,
}

impl BrowserApp {
    async fn new(window: Arc<Window>) -> Self {
        // Create Skia context
        let mut gr_context = gpu::DirectContext::new_gl(None, None).expect("Failed to create Skia GL context");
        let size = window.inner_size();

        // Initialize UI
        let mut ui = BrowserUI::new(&gr_context);
        ui.initialize_renderer();

        // Create initial tab
        let config = EngineConfig::default();
        let initial_tab = Tab::new("tab1", config.clone());

        Self {
            tabs: vec![initial_tab],
            active_tab_index: 0,
            ui,
            window,
            size,
            skia_context: gr_context,
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

    fn render(&mut self) -> Result<(), String> {
        // Create Skia surface for the window
        let surface = Surface::new_raster_n32_premul((self.size.width as i32, self.size.height as i32))
            .ok_or("Failed to create Skia surface")?;
        let canvas = surface.canvas();
        canvas.clear(Color::WHITE);
        self.active_tab().engine.render(canvas);
        self.ui.render(canvas);
        // Present the surface to the window (platform-specific, may require integration)
        Ok(())
    }
}

impl ApplicationHandler for BrowserApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
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
                }
                self.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                match self.render() {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Render error: {}", e);
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
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
    app.navigate("https://example.com").await;
    event_loop.run_app(&mut app)?;
    Ok(())
}
