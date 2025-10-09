mod engine;
mod networking;
mod ui;
mod dom;
mod layout;
mod renderer;
mod css;
mod js;
mod input;

use std::ffi::CString;
use std::num::NonZeroU32;
use std::ops::DerefMut;
use std::time::{Duration, Instant};
use gl::types::GLint;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, SurfaceAttributesBuilder, WindowSurface};
use glutin::surface::Surface as GlutinSurface;
use glutin_winit::DisplayBuilder;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Modifiers, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::engine::{Engine, EngineConfig};
use crate::ui::BrowserUI;
use skia_safe::{gpu, Color, ColorType, Surface};
use skia_safe::gpu::{backend_render_targets, DirectContext};
use skia_safe::gpu::gl::{Format, FramebufferInfo, Interface};
use skia_safe::gpu::surfaces::wrap_backend_render_target;
use winit::raw_window_handle::HasWindowHandle;

/// Result of closing a tab
#[derive(Debug, PartialEq)]
enum TabCloseResult {
    Closed,    // Tab was closed successfully
    QuitApp,   // Last tab was closed, application should quit
    NoAction,  // Tab could not be closed (invalid index, etc.)
}

/// Tab structure representing a browser tab
struct Tab {
    id: String,
    engine: Engine,
}

impl Tab {
    fn new(id: &str, config: EngineConfig, scale_factor: f64) -> Self {
        Self {
            id: id.to_string(),
            engine: Engine::new(config, scale_factor),
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
    scale_factor: f64, // Track DPI scale factor
    loading_spinner_angle: f32, // Track loading spinner rotation angle
    last_spinner_update: Instant, // Track last time spinner was updated
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
            .with_min_inner_size(LogicalSize::new(500, 0))  // Set minimum window size
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

        // Get initial scale factor from the window
        let scale_factor = env.window.scale_factor();

        // Initialize UI
        let mut ui = BrowserUI::new(&gr_context, scale_factor);
        ui.initialize_renderer();

        // Create initial tab
        let config = EngineConfig::default();

        // Create initial tab with scale-aware layout
        let initial_tab = Tab::new("tab1", config, scale_factor);

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
            scale_factor, // Initialize with actual scale factor from window
            loading_spinner_angle: 0.0, // Initialize spinner angle
            last_spinner_update: Instant::now(), // Initialize spinner update time
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

    // Navigate to a URL synchronously (for event handlers)
    fn navigate_to_url(&mut self, url: &str) {
        let url = url.to_string();

        // Set loading state immediately before spawning task
        self.active_tab_mut().engine.set_loading_state(true);

        // Update window title to show loading state
        self.render().expect("Render failed");

        // We still need to block here because we can't restructure the entire app to be async-aware
        // But the key is that we've already set the loading state and requested a redraw BEFORE blocking

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.navigate(&url).await;
            })
        });

        // Request another redraw after navigation completes
        self.env.window.request_redraw();
    }

    // Add a new tab
    fn add_tab(&mut self) {
        let tab_id = format!("tab{}", self.tabs.len() + 1);
        let config = EngineConfig::default();
        let new_tab = Tab::new(&tab_id, config, self.scale_factor);

        // Add to tabs list
        self.tabs.push(new_tab);

        // Add to UI
        self.ui.add_tab(&tab_id, "New Tab");

        // Switch to the new tab
        let new_tab_index = self.tabs.len() - 1;
        self.switch_to_tab(new_tab_index);

        // Automatically focus the address bar for the new tab
        self.ui.set_focus("address_bar");
    }

    // Close a tab
    fn close_tab(&mut self, tab_index: usize) -> TabCloseResult {
        if self.tabs.len() <= 1 {
            // Close the last tab - signal to quit the application
            return TabCloseResult::QuitApp;
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

            return TabCloseResult::Closed;
        }
        TabCloseResult::NoAction
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

            // Clear focus from address bar when switching tabs
            self.ui.clear_focus();
        }
    }

    // Switch to tab by ID
    fn switch_to_tab_by_id(&mut self, tab_id: &str) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
            self.switch_to_tab(index);
        }
    }

    // Handle mouse click
    fn handle_click(&mut self, x: f32, y: f32, event_loop: &ActiveEventLoop) {
        let tabs: Vec<(String, String)> = self.tabs.iter()
            .map(|tab| (tab.id.clone(), tab.engine.page_title().to_string()))
            .collect();
        let ui = &mut self.ui;
        let active_tab_index = self.active_tab_index;
        let engine = &mut self.tabs[active_tab_index].engine;

        let action = input::handle_mouse_click(
            x,
            y,
            ui,
            engine,
            &tabs,
            active_tab_index,
        );

        self.handle_input_action(action, event_loop);
    }

    // Handle middle-click on tabs to close them
    fn handle_middle_click(&mut self, x: f32, y: f32, event_loop: &ActiveEventLoop) {
        let tabs: Vec<(String, String)> = self.tabs.iter()
            .map(|tab| (tab.id.clone(), tab.engine.page_title().to_string()))
            .collect();

        let action = input::handle_middle_click(x, y, &mut self.ui, &tabs);
        self.handle_input_action(action, event_loop);
    }

    // Check if any text field currently has focus
    fn has_focused_text_field(&self) -> bool {
        self.ui.components.iter().any(|comp| {
            matches!(comp, crate::ui::UiComponent::TextField { has_focus: true, .. })
        })
    }

    // Handle input actions returned by the input module
    fn handle_input_action(&mut self, action: input::InputAction, event_loop: &ActiveEventLoop) {
        match action {
            input::InputAction::CloseTab(tab_index) => {
                if self.close_tab(tab_index) == TabCloseResult::QuitApp {
                    event_loop.exit();
                }
            }
            input::InputAction::Navigate(url) => {
                self.navigate_to_url(&url);
            }
            input::InputAction::SwitchTab(tab_index) => {
                self.switch_to_tab(tab_index);
            }
            input::InputAction::AddTab => {
                self.add_tab();
            }
            input::InputAction::ReloadPage => {
                self.reload_current_page();
            }
            input::InputAction::RequestRedraw => {
                // Just request redraw, no other action needed
            }
            input::InputAction::QuitApp => {
                event_loop.exit();
            }
            input::InputAction::None => {}
        }

        // Request a redraw after handling the input action
        self.env.window.request_redraw();
    }

    /// Process timers for the active tab
    fn process_timers(&mut self) {
        let engine = &mut self.active_tab_mut().engine;
        if engine.process_timers() {
            // If any timers were executed, request a redraw
            self.env.window.request_redraw();
        }
    }

    /// Check if there are any active timers that need processing
    fn has_active_timers(&self) -> bool {
        self.active_tab().engine.has_active_timers()
    }

    /// Get the time until the next timer should fire
    fn time_until_next_timer(&self) -> Option<Duration> {
        self.active_tab().engine.time_until_next_timer()
    }

    fn render(&mut self) -> Result<(), String> {
        // Update loading spinner angle if the page is loading
        let is_loading = self.active_tab().engine.is_loading();
        if is_loading {
            let now = Instant::now();
            let elapsed = now.duration_since(self.last_spinner_update).as_secs_f32();
            // Rotate at about 2 full rotations per second
            self.loading_spinner_angle += elapsed * 4.0 * std::f32::consts::PI;
            // Keep angle within 0-2π range
            if self.loading_spinner_angle >= 2.0 * std::f32::consts::PI {
                self.loading_spinner_angle -= 2.0 * std::f32::consts::PI;
            }
            self.last_spinner_update = now;

            // Request another redraw to continue animation
            self.env.window.request_redraw();
        }

        // Get the canvas first
        let canvas = self.env.surface.canvas();
        canvas.clear(Color::WHITE);

        // Render the active tab's web content using mutable reference with scale factor
        let active_tab_index = self.active_tab_index;
        let engine = &mut self.tabs[active_tab_index].engine;
        engine.render(canvas, self.scale_factor);

        // Render UI on top of web content
        self.ui.render(canvas);

        // Render loading indicator if page is loading
        self.ui.render_loading_indicator(canvas, is_loading, self.loading_spinner_angle);

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

                // Update UI layout with new window size
                self.ui.update_layout(width as f32, height as f32);

                // Update engine viewport size
                self.active_tab_mut().engine.resize(width as f32, height as f32);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // Update the stored scale factor when DPI changes
                self.scale_factor = scale_factor;

                self.ui.set_scale_factor(scale_factor);

                // Update UI scale factor for proper text scaling
                let engine = &mut self.active_tab_mut().engine;
                engine.scale_factor = scale_factor;
                // Recalculate layout with new scale factor
                engine.recalculate_layout();

                // Force a redraw to apply the new scaling
                self.env.window.request_redraw();
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
                // Use the stored cursor position for click handling
                self.handle_click(self.cursor_position.0 as f32, self.cursor_position.1 as f32, event_loop);
                self.env.window.request_redraw();
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Middle, .. } => {
                // Handle middle-click to close tabs
                self.handle_middle_click(self.cursor_position.0 as f32, self.cursor_position.1 as f32, event_loop);
                self.env.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Update cursor position when mouse moves
                self.cursor_position = (position.x, position.y);

                // Update cursor based on the element under the mouse
                input::update_cursor_for_position(
                    self.cursor_position,
                    &self.ui,
                    &self.active_tab().engine,
                    &self.env.window,
                );
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let action = input::handle_mouse_wheel(
                    delta,
                    self.cursor_position,
                    &self.modifiers,
                    &mut self.ui,
                    &mut self.tabs[self.active_tab_index].engine,
                );
                self.handle_input_action(action, event_loop);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Compute values before the function call to avoid borrow conflicts
                let active_tab_index = self.active_tab_index;
                let tabs_len = self.tabs.len();
                let has_focused = self.has_focused_text_field();

                let action = input::handle_keyboard_input(
                    &event,
                    &self.modifiers,
                    &mut self.ui,
                    &mut self.tabs[self.active_tab_index].engine,
                    active_tab_index,
                    tabs_len,
                    has_focused,
                );
                self.handle_input_action(action, event_loop);
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers;
            }
            _ => {}
        }

        // Process timers for the active tab
        self.process_timers();

        let expected_frame_length_seconds = 1.0 / 60.0;
        let frame_duration = Duration::from_secs_f32(expected_frame_length_seconds);

        if frame_start - self.previous_frame_start > frame_duration {
            self.previous_frame_start = frame_start;
            self.env.window.request_redraw();
        }

        // Determine when to wake up next based on timers
        let next_frame_time = self.previous_frame_start + frame_duration;

        if let Some(timer_duration) = self.time_until_next_timer() {
            // We have active timers, wake up at the earlier of next frame or next timer
            let timer_wake_time = Instant::now() + timer_duration;
            let wake_time = next_frame_time.min(timer_wake_time);
            event_loop.set_control_flow(ControlFlow::WaitUntil(wake_time));
        } else {
            // No active timers, just wake up for next frame
            event_loop.set_control_flow(ControlFlow::WaitUntil(next_frame_time));
        }
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
