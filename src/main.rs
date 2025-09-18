mod engine;
mod networking;
mod ui;
mod dom;

use std::ffi::CString;
use std::num::NonZeroU32;
use std::sync::Arc;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::DisplayBuilder;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::engine::{Engine, EngineConfig};
use crate::ui::BrowserUI;
use skia_safe::{gpu, Color, Surface};
use skia_safe::gpu::DirectContext;
use skia_safe::gpu::gl::Interface;
use winit::raw_window_handle::HasWindowHandle;

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
    async fn new(el: &EventLoop<()>) -> Self {
        let window_attrs = WindowAttributes::default()
            .with_title("Stokes Browser")
            .with_inner_size(LogicalSize::new(1024, 768));

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_transparency(true);

        let display_builder = DisplayBuilder::new().with_window_attributes(window_attrs.into());
        let (window, gl_config) = display_builder
            .build(el, template, |configs| {
                configs
                    .reduce(|accum, config| {
                        let transparency_check = config.supports_transparency().unwrap_or(false)
                            & !accum.supports_transparency().unwrap_or(false);

                        if transparency_check || config.num_samples() < accum.num_samples() {
                            config
                        } else {
                            accum
                        }
                    })
                    .unwrap()
            })
            .unwrap();
        let window = window.expect("Could not create window with OpenGL context.");
        let window_handle = window.window_handle().expect("Failed to retrieve RawWindowHandle");
        let raw_window_handle = window_handle.as_raw();

        let context_attributes = ContextAttributesBuilder::new().build(Some(raw_window_handle));

        let fallback_context_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(None))
            .build(Some(raw_window_handle));
        let not_current_gl_context = unsafe {
            gl_config
                .display()
                .create_context(&gl_config, &context_attributes)
                .unwrap_or_else(|_| {
                    gl_config
                        .display()
                        .create_context(&gl_config, &fallback_context_attributes)
                        .expect("failed to create context")
                })
        };

        let (width, height) = window.inner_size().into();

        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw_window_handle,
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap()
        );

        let gl_surface = unsafe {
            gl_config
                .display()
                .create_window_surface(&gl_config, &attrs)
                .expect("Failed to create GL surface")
        };

        let gl_context = not_current_gl_context
            .make_current(&gl_surface)
            .expect("Failed to make GL context current");

        gl::load_with(|s| {
            gl_config
                .display()
                .get_proc_address(CString::new(s).unwrap().as_c_str())
        });
        let interface = Interface::new_load_with(|name| {
            if name == "eglGetCurrentDisplay" {
                return std::ptr::null();
            }
            gl_config
                .display()
                .get_proc_address(CString::new(name).unwrap().as_c_str())
        }).expect("Could not create interface");

        // Create Skia context - using the correct API for version 0.88.0
        let context_options = gpu::ContextOptions::default();
        let gr_context = gpu::DirectContext::new_gl(interface, Some(&context_options))
            .expect("Failed to create Skia GL context");

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
            window: Arc::new(window),
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
            let url = self.active_tab().engine.current_url(); // Fixed double semicolon
            let title = self.active_tab().engine.page_title();
            // TODO: self.ui.update_address_bar(url);
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
        // Create Skia surface for the window using updated API
        let info = skia_safe::ImageInfo::new(
            (self.size.width as i32, self.size.height as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );

        let mut surface = skia_safe::Surface::new_raster(&info, None, None)
            .ok_or("Failed to create Skia surface")?;

        // Get a mutable canvas for rendering
        let canvas = surface.canvas();
        canvas.clear(Color::WHITE);

        // Get mutable references to engine and UI for rendering
        let engine = &self.active_tab().engine;
        let ui = &self.ui;

        // Render the active tab and UI
        engine.render(canvas);
        ui.render(canvas);

        // Note: To display this on screen with winit, you'd need to convert
        // the surface pixels to a texture and present it to the window
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

    let mut app = BrowserApp::new(&event_loop).await;
    println!("Browser initialized, navigating to homepage...");
    app.navigate("https://example.com").await;
    event_loop.run_app(&mut app)?;
    Ok(())
}
