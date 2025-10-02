mod engine;
mod networking;
mod ui;
mod dom;
mod layout;
mod renderer;
mod css;

use std::ffi::CString;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use gl::types::GLint;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, SurfaceAttributesBuilder, WindowSurface};
use glutin::surface::Surface as GlutinSurface;
use glutin_winit::DisplayBuilder;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, Modifiers, MouseButton, WindowEvent, MouseScrollDelta, DeviceEvent, KeyEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::engine::{Engine, EngineConfig};
use crate::ui::BrowserUI;
use skia_safe::{gpu, Color, ColorType, Surface};
use skia_safe::gpu::{backend_render_targets, DirectContext};
use skia_safe::gpu::gl::{Format, FramebufferInfo, Interface};
use skia_safe::gpu::surfaces::wrap_backend_render_target;
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
    env: Env,
    fb_info: FramebufferInfo,
    num_samples: usize,
    stencil_size: usize,
    modifiers: Modifiers,
    frame: usize,
    previous_frame_start: Instant,
    tabs: Vec<Tab>,
    active_tab_index: usize,
    ui: BrowserUI,
    size: winit::dpi::PhysicalSize<u32>,
    skia_context: gpu::DirectContext,
    cursor_position: (f64, f64), // Track cursor position
}

struct Env {
    surface: Surface,
    gl_surface: GlutinSurface<WindowSurface>,
    gr_context: DirectContext,
    gl_context: PossiblyCurrentContext,
    window: Window,
}

impl BrowserApp {
    async fn new(el: &EventLoop<()>) -> Self {
        let icon_data = include_bytes!("../assets/icon.png");
        let icon = image::load_from_memory(icon_data)
            .expect("Failed to load icon")
            .into_rgba8();
        let (icon_width, icon_height) = icon.dimensions();
        let icon = winit::window::Icon::from_rgba(icon.into_raw(), icon_width, icon_height)
            .expect("Failed to create icon");

        let window_attrs = WindowAttributes::default()
            .with_title("Web Browser")
            .with_inner_size(LogicalSize::new(1024, 768))
            .with_window_icon(Some(icon));

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
        let mut gr_context = gpu::direct_contexts::make_gl(interface, Some(&context_options))
            .expect("Failed to create Skia GL context");

        let size = window.inner_size();

        let fb_info = {
            let mut fboid: GLint = 0;
            unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };

            FramebufferInfo {
                fboid: fboid.try_into().unwrap(),
                format: Format::RGBA8.into(),
                ..Default::default()
            }
        };

        let num_samples = gl_config.num_samples() as usize;
        let stencil_size = gl_config.stencil_size() as usize;

        let surface = Self::create_surface(&window, fb_info, &mut gr_context, num_samples, stencil_size);

        let env = Env {
            surface,
            gl_surface,
            gr_context: gr_context.clone(),
            gl_context,
            window
        };

        // Initialize UI
        let mut ui = BrowserUI::new(&gr_context);
        ui.initialize_renderer();

        // Create initial tab
        let config = EngineConfig::default();
        let initial_tab = Tab::new("tab1", config.clone());

        // Add the initial tab to the UI
        ui.add_tab("tab1", "New Tab");

        Self {
            env,
            fb_info,
            num_samples,
            stencil_size,
            modifiers: Modifiers::default(),
            frame: 0,
            previous_frame_start: Instant::now(),
            tabs: vec![initial_tab],
            active_tab_index: 0,
            ui,
            size,
            skia_context: gr_context,
            cursor_position: (0.0, 0.0), // Initialize cursor position
        }
    }

    fn create_surface(
        window: &Window,
        fb_info: FramebufferInfo,
        gr_context: &mut DirectContext,
        num_samples: usize,
        stencil_size: usize
    ) -> Surface {
        let size = window.inner_size();
        let size = (
            size.width.try_into().expect("Could not convert width"),
            size.height.try_into().expect("Could not convert height")
        );
        let backend_render_target = backend_render_targets::make_gl(size, num_samples, stencil_size, fb_info);

        wrap_backend_render_target(
            gr_context,
            &backend_render_target,
            gpu::SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            None,
            None
        ).expect("Failed to wrap backend render target")
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
        self.env.window.set_title(&format!("Loading: {}", url));

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
                self.env.window.set_title(&format!("{} - Web Browser", title));
            }
            Err(e) => {
                println!("Navigation error: {}", e);
                self.env.window.set_title("Error - Web Browser");
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

    // Close a tab
    fn close_tab(&mut self, tab_index: usize) -> bool {
        if self.tabs.len() <= 1 {
            // Don't close the last tab
            return false;
        }

        if tab_index < self.tabs.len() {
            let tab_id = self.tabs[tab_index].id.clone();

            // Remove from tabs list
            self.tabs.remove(tab_index);

            // Remove from UI
            self.ui.remove_tab(&tab_id);

            // Adjust active tab index
            if self.active_tab_index >= self.tabs.len() {
                self.active_tab_index = self.tabs.len() - 1;
            } else if tab_index <= self.active_tab_index && self.active_tab_index > 0 {
                self.active_tab_index -= 1;
            }

            // Update UI to show the new active tab
            let active_tab_id = &self.tabs[self.active_tab_index].id;
            self.ui.set_active_tab(active_tab_id);

            // Update window title and address bar
            let url = self.tabs[self.active_tab_index].engine.current_url().to_string();
            let title = self.tabs[self.active_tab_index].engine.page_title().to_string();
            self.ui.update_address_bar(&url);
            self.env.window.set_title(&format!("{} - Web Browser", title));

            return true;
        }
        false
    }

    // Switch to a tab by index
    fn switch_to_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab_index = index;

            // Update UI to reflect active tab
            let tab_id = self.tabs[index].id.clone();
            self.ui.set_active_tab(&tab_id);

            let url = self.tabs[index].engine.current_url().to_string();
            let title = self.tabs[index].engine.page_title().to_string();
            self.ui.update_address_bar(&url);
            self.env.window.set_title(&format!("{} - Web Browser", title));
        }
    }

    // Switch to tab by ID
    fn switch_to_tab_by_id(&mut self, tab_id: &str) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
            self.switch_to_tab(index);
        }
    }

    // Handle mouse click
    fn handle_click(&mut self, x: f32, y: f32) {
        // Convert window coordinates to normalized coordinates for UI
        let norm_x = x / self.size.width as f32;
        let norm_y = y / self.size.height as f32;

        // Check for UI interaction
        if let Some(component_id) = self.ui.handle_click(norm_x, norm_y) {
            // Handle based on component
            if component_id == "back" {
                println!("Back button clicked");
                // Back navigation would go here
            } else if component_id == "forward" {
                println!("Forward button clicked");
                // Forward navigation would go here
            } else if component_id == "refresh" {
                println!("Refresh button clicked");
                // Reload the current page
                self.reload_current_page();
            } else if component_id == "new_tab" {
                println!("New tab button clicked");
                self.add_tab();
                self.env.window.request_redraw();
            } else if component_id == "address_bar" {
                // Focus the address bar for typing
                self.ui.set_focus("address_bar");
                self.env.window.request_redraw();
            } else if component_id.starts_with("tab") {
                // Tab switching by clicking
                self.switch_to_tab_by_id(&component_id);
                self.env.window.request_redraw();
            }
        }
    }

    // Check if any text field currently has focus
    fn has_focused_text_field(&self) -> bool {
        self.ui.components.iter().any(|comp| {
            matches!(comp, crate::ui::UiComponent::TextField { has_focus: true, .. })
        })
    }

    fn render(&mut self) -> Result<(), String> {
        // Get the canvas first
        let canvas = self.env.surface.canvas();
        canvas.clear(Color::WHITE);

        // Render the active tab's web content using mutable reference
        let active_tab_index = self.active_tab_index;
        let engine = &mut self.tabs[active_tab_index].engine;
        engine.render(canvas);

        // Render UI on top of web content
        self.ui.render(canvas);

        // Flush to display
        self.env.gr_context.flush_and_submit();
        self.env.gl_surface.swap_buffers(&self.env.gl_context)
            .map_err(|e| format!("Failed to swap buffers: {}", e))?;

        Ok(())
    }

    // Reload the current page in the active tab
    fn reload_current_page(&mut self) {
        let current_url = self.active_tab().engine.current_url().to_string();
        if !current_url.is_empty() {
            println!("Reloading page: {}", current_url);

            // Update window title to show reloading state
            self.env.window.set_title(&format!("Reloading: {}", current_url));

            // Navigate to the current URL again to reload the page
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    self.navigate(&current_url).await;
                })
            });
        } else {
            println!("No URL to reload");
        }
    }
}

impl ApplicationHandler for BrowserApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        self.env.window.request_redraw();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let mut draw_frame = false;
        let frame_start = Instant::now();

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                self.size = new_size;
                self.env.surface = Self::create_surface(
                    &self.env.window,
                    self.fb_info,
                    &mut self.env.gr_context,
                    self.num_samples,
                    self.stencil_size
                );
                let (width, height): (u32, u32) = new_size.into();

                self.env.gl_surface.resize(
                    &self.env.gl_context,
                    NonZeroU32::new(width.max(1)).unwrap(),
                    NonZeroU32::new(height.max(1)).unwrap()
                );

                // Update engine viewport size
                self.active_tab_mut().engine.resize(width as f32, height as f32);
            }
            WindowEvent::RedrawRequested => {
                draw_frame = true;
                match self.render() {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Render error: {}", e);
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                // Use the stored cursor position for click handling
                self.handle_click(self.cursor_position.0 as f32, self.cursor_position.1 as f32);
                self.env.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Update cursor position when mouse moves
                self.cursor_position = (position.x, position.y);
                // No need to handle click on cursor move - only track position
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Handle mouse wheel scrolling
                let scroll_speed = 50.0;
                match delta {
                    MouseScrollDelta::LineDelta(_x, y) => {
                        // Vertical scrolling (most common)
                        self.active_tab_mut().engine.scroll_vertical(-y * scroll_speed);
                        self.env.window.request_redraw();
                    }
                    MouseScrollDelta::PixelDelta(pos) => {
                        // Pixel-precise scrolling (trackpad)
                        self.active_tab_mut().engine.scroll_vertical(-pos.y as f32);
                        self.active_tab_mut().engine.scroll_horizontal(-pos.x as f32);
                        self.env.window.request_redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    use winit::keyboard::{KeyCode, PhysicalKey, Key, NamedKey};

                    // Handle keyboard shortcuts with modifiers
                    if self.modifiers.state().control_key() {
                        match event.logical_key {
                            Key::Character(ref text) => {
                                match text.as_str() {
                                    "t" => {
                                        // Ctrl+T: New tab
                                        println!("New tab shortcut (Ctrl+T)");
                                        self.add_tab();
                                        self.env.window.request_redraw();
                                        return;
                                    }
                                    "w" => {
                                        // Ctrl+W: Close current tab
                                        println!("Close tab shortcut (Ctrl+W)");
                                        if self.close_tab(self.active_tab_index) {
                                            self.env.window.request_redraw();
                                        }
                                        return;
                                    }
                                    "l" => {
                                        // Ctrl+L: Focus address bar
                                        println!("Focus address bar shortcut (Ctrl+L)");
                                        self.ui.set_focus("address_bar");
                                        self.env.window.request_redraw();
                                        return;
                                    }
                                    "r" => {
                                        // Ctrl+R: Reload page
                                        println!("Reload shortcut (Ctrl+R)");
                                        self.reload_current_page();
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            Key::Named(NamedKey::Tab) => {
                                // Ctrl+Tab: Switch to next tab
                                println!("Switch tab shortcut (Ctrl+Tab)");
                                let next_index = (self.active_tab_index + 1) % self.tabs.len();
                                self.switch_to_tab(next_index);
                                self.env.window.request_redraw();
                                return;
                            }
                            _ => {}
                        }
                    }

                    // Handle number keys for tab switching (Ctrl+1, Ctrl+2, etc.)
                    if self.modifiers.state().control_key() {
                        if let Key::Character(ref text) = event.logical_key {
                            if let Ok(num) = text.parse::<usize>() {
                                if num >= 1 && num <= 9 {
                                    let tab_index = num - 1;
                                    if tab_index < self.tabs.len() {
                                        println!("Switch to tab {} shortcut (Ctrl+{})", tab_index + 1, num);
                                        self.switch_to_tab(tab_index);
                                        self.env.window.request_redraw();
                                        return;
                                    }
                                }
                            }
                        }
                    }

                    // Handle text input and navigation keys
                    match event.logical_key {
                        Key::Named(NamedKey::Backspace) => {
                            self.ui.handle_key_input("Backspace");
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::Delete) => {
                            self.ui.handle_key_input("Delete");
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            // Check if we're in a text field first
                            if !self.has_focused_text_field() {
                                self.active_tab_mut().engine.scroll_horizontal(-30.0);
                            } else {
                                self.ui.handle_key_input("ArrowLeft");
                            }
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            // Check if we're in a text field first
                            if !self.has_focused_text_field() {
                                self.active_tab_mut().engine.scroll_horizontal(30.0);
                            } else {
                                self.ui.handle_key_input("ArrowRight");
                            }
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            self.active_tab_mut().engine.scroll_vertical(-30.0);
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            self.active_tab_mut().engine.scroll_vertical(30.0);
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::Home) => {
                            // Check if we're in a text field first
                            if !self.has_focused_text_field() {
                                self.active_tab_mut().engine.set_scroll_position(0.0, 0.0);
                            } else {
                                self.ui.handle_key_input("Home");
                            }
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::End) => {
                            // Check if we're in a text field first
                            if !self.has_focused_text_field() {
                                let engine = &mut self.active_tab_mut().engine;
                                engine.set_scroll_position(0.0, f32::MAX); // Will be clamped to max
                            } else {
                                self.ui.handle_key_input("End");
                            }
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::Enter) => {
                            if let Some(url) = self.ui.handle_key_input("Enter") {
                                // Navigate to the URL from the address bar
                                let url_to_navigate = if url.starts_with("http://") || url.starts_with("https://") {
                                    url
                                } else {
                                    format!("https://{}", url)
                                };

                                // Navigate synchronously using block_in_place
                                tokio::task::block_in_place(|| {
                                    tokio::runtime::Handle::current().block_on(async {
                                        self.navigate(&url_to_navigate).await;
                                    })
                                });
                            }
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::PageUp) => {
                            self.active_tab_mut().engine.scroll_vertical(-300.0);
                            self.env.window.request_redraw();
                        }
                        Key::Named(NamedKey::PageDown) => {
                            self.active_tab_mut().engine.scroll_vertical(300.0);
                            self.env.window.request_redraw();
                        }
                        Key::Character(text) => {
                            // Handle regular character input
                            self.ui.handle_text_input(text.as_str());
                            self.env.window.request_redraw();
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers;
            }
            _ => {}
        }

        let expected_frame_length_seconds = 1.0 / 60.0;
        let frame_duration = Duration::from_secs_f32(expected_frame_length_seconds);

        if frame_start - self.previous_frame_start > frame_duration {
            self.previous_frame_start = frame_start;
            self.env.window.request_redraw();
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(
            self.previous_frame_start + frame_duration
        ))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting Web Browser...");
    let event_loop = EventLoop::new()?;

    let mut app = BrowserApp::new(&event_loop).await;
    println!("Browser initialized, navigating to homepage...");
    app.navigate("https://example.com").await;
    event_loop.run_app(&mut app)?;
    Ok(())
}
