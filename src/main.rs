mod engine;
mod networking;
mod ui;
mod dom;
mod layout;
mod renderer;
mod css;
mod js;
mod input;
mod ipc;
mod tab_process;
mod tab_manager;

use std::ffi::CString;
use std::num::NonZeroU32;
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

use crate::tab_manager::TabManager;
use crate::ipc::{ParentToTabMessage, TabToParentMessage};
use crate::ui::BrowserUI;
use skia_safe::{gpu, Color, ColorType, Surface};
use skia_safe::gpu::{backend_render_targets, DirectContext};
use skia_safe::gpu::gl::{Format, FramebufferInfo, Interface};
use skia_safe::gpu::surfaces::wrap_backend_render_target;
use winit::raw_window_handle::HasWindowHandle;

/// Result of closing a tab
#[derive(Debug, PartialEq)]
enum TabCloseResult {
    Closed,
    QuitApp,
    NoAction,
}

/// The main browser application (parent process)
struct BrowserApp {
    env: Env,
    fb_info: FramebufferInfo,
    num_samples: usize,
    stencil_size: usize,
    modifiers: Modifiers,
    previous_frame_start: Instant,
    tab_manager: TabManager,
    active_tab_index: usize,
    ui: BrowserUI,
    cursor_position: (f64, f64),
    scale_factor: f64,
    loading_spinner_angle: f32,
    last_spinner_update: Instant,
    tab_order: Vec<String>,
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
        // Load and set window icon
        let icon_data = include_bytes!("../assets/icon.png");
        let icon = image::load_from_memory(icon_data)
            .expect("Failed to load icon")
            .into_rgba8();
        let (icon_width, icon_height) = icon.dimensions();
        let icon = winit::window::Icon::from_rgba(icon.into_raw(), icon_width, icon_height)
            .expect("Failed to create icon");

        // Create window
        let window_attrs = WindowAttributes::default()
            .with_title("Web Browser")
            .with_inner_size(LogicalSize::new(1024, 768))
            .with_min_inner_size(LogicalSize::new(500, 0))
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

        // Create GL context
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

        let scale_factor = env.window.scale_factor();

        // Initialize UI
        let mut ui = BrowserUI::new(&gr_context, scale_factor);
        ui.initialize_renderer();

        // Create tab manager
        let mut tab_manager = TabManager::new().expect("Failed to create tab manager");

        // Create initial tab
        let initial_tab_id = tab_manager.create_tab().expect("Failed to create initial tab");
        ui.add_tab(&initial_tab_id, "New Tab");

        // Send initial configuration to tab
        let (width, height) = (size.width as f32, size.height as f32);
        let _ = tab_manager.send_to_tab(&initial_tab_id, ParentToTabMessage::Resize { width, height });
        let _ = tab_manager.send_to_tab(&initial_tab_id, ParentToTabMessage::SetScaleFactor(scale_factor));

        Self {
            env,
            fb_info,
            num_samples,
            stencil_size,
            modifiers: Modifiers::default(),
            previous_frame_start: Instant::now(),
            tab_manager,
            active_tab_index: 0,
            ui,
            cursor_position: (0.0, 0.0),
            scale_factor,
            loading_spinner_angle: 0.0,
            last_spinner_update: Instant::now(),
            tab_order: vec![initial_tab_id],
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

    fn active_tab_id(&self) -> Option<&String> {
        self.tab_order.get(self.active_tab_index)
    }

    fn navigate_to_url(&mut self, url: &str) {
        if let Some(tab_id) = self.active_tab_id().cloned() {
            let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate(url.to_string()));
            self.env.window.set_title(&format!("Loading: {}", url));
        }
    }

    fn add_tab(&mut self) {
        if let Ok(new_tab_id) = self.tab_manager.create_tab() {
            self.ui.add_tab(&new_tab_id, "New Tab");
            self.tab_order.push(new_tab_id.clone());

            // Switch to new tab
            self.active_tab_index = self.tab_order.len() - 1;
            self.ui.set_active_tab(&new_tab_id);

            // Send initial configuration
            let size = self.env.window.inner_size();
            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::Resize {
                width: size.width as f32,
                height: size.height as f32
            });
            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::SetScaleFactor(self.scale_factor));

            self.ui.set_focus("address_bar");
        }
    }

    fn close_tab(&mut self, tab_index: usize) -> TabCloseResult {
        if self.tab_order.len() <= 1 {
            return TabCloseResult::QuitApp;
        }

        if tab_index < self.tab_order.len() {
            let tab_id = self.tab_order.remove(tab_index);
            let _ = self.tab_manager.close_tab(&tab_id);
            self.ui.remove_tab(&tab_id);

            // Adjust active tab index
            if self.active_tab_index >= self.tab_order.len() {
                self.active_tab_index = self.tab_order.len() - 1;
            } else if tab_index <= self.active_tab_index && self.active_tab_index > 0 {
                self.active_tab_index -= 1;
            }

            // Update UI
            if let Some(active_id) = self.active_tab_id().cloned() {
                self.ui.set_active_tab(&active_id);
                if let Some(tab) = self.tab_manager.get_tab(&active_id) {
                    self.ui.update_address_bar(&tab.url);
                    self.env.window.set_title(&format!("{} - Web Browser", tab.title));
                }
            }

            return TabCloseResult::Closed;
        }
        TabCloseResult::NoAction
    }

    fn switch_to_tab(&mut self, index: usize) {
        if index < self.tab_order.len() {
            self.active_tab_index = index;
            let tab_id = &self.tab_order[index];
            self.ui.set_active_tab(tab_id);

            if let Some(tab) = self.tab_manager.get_tab(tab_id) {
                self.ui.update_address_bar(&tab.url);
                self.env.window.set_title(&format!("{} - Web Browser", tab.title));
            }
            self.ui.clear_focus();
        }
    }

    fn handle_click(&mut self, x: f32, y: f32, event_loop: &ActiveEventLoop) {
        // Get tab info for UI
        let tabs: Vec<(String, String)> = self.tab_order.iter()
            .filter_map(|id| {
                self.tab_manager.get_tab(id).map(|t| (id.clone(), t.title.clone()))
            })
            .collect();

        // Handle UI clicks
        let action = input::handle_mouse_click_ui(
            x, y, &mut self.ui, &tabs, self.active_tab_index
        );

        self.handle_input_action(action, event_loop);

        // Forward click to active tab process
        if let Some(tab_id) = self.active_tab_id().cloned() {
            let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Click { x, y });
        }
    }

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
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Reload);
                }
            }
            input::InputAction::RequestRedraw => {}
            input::InputAction::QuitApp => {
                event_loop.exit();
            }
            input::InputAction::ForwardToTab(_) => {
                // This case is handled separately in the keyboard input handler
            }
            input::InputAction::None => {}
        }
        self.env.window.request_redraw();
    }

    fn process_tab_messages(&mut self) {
        let messages = self.tab_manager.poll_messages();

        for (tab_id, message) in messages {
            self.tab_manager.process_tab_message(&tab_id, message.clone());

            // Update UI based on messages
            match message {
                TabToParentMessage::TitleChanged(title) => {
                    self.ui.update_tab_title(&tab_id, &title);
                    if Some(&tab_id) == self.active_tab_id() {
                        self.env.window.set_title(&format!("{} - Web Browser", title));
                    }
                }
                TabToParentMessage::NavigationCompleted { url, title } => {
                    self.ui.update_tab_title(&tab_id, &title);
                    if Some(&tab_id) == self.active_tab_id() {
                        self.ui.update_address_bar(&url);
                        self.env.window.set_title(&format!("{} - Web Browser", title));
                    }
                }
                TabToParentMessage::LoadingStateChanged(_is_loading) => {
                    // Update loading indicator
                    self.env.window.request_redraw();
                }
                TabToParentMessage::FrameRendered { .. } => {
                    self.env.window.request_redraw();
                }
                _ => {}
            }
        }
    }

    fn render(&mut self) -> Result<(), String> {
        // Process messages from tab processes
        self.process_tab_messages();

        // Check if active tab is loading
        let is_loading = self.active_tab_id()
            .and_then(|id| self.tab_manager.get_tab(id))
            .map(|t| t.is_loading)
            .unwrap_or(false);

        if is_loading {
            let now = Instant::now();
            let elapsed = now.duration_since(self.last_spinner_update).as_secs_f32();
            self.loading_spinner_angle += elapsed * 4.0 * std::f32::consts::PI;
            if self.loading_spinner_angle >= 2.0 * std::f32::consts::PI {
                self.loading_spinner_angle -= 2.0 * std::f32::consts::PI;
            }
            self.last_spinner_update = now;
            self.env.window.request_redraw();
        }

        // Get the rendered frame before borrowing canvas
        let frame_to_render = self.active_tab_id()
            .and_then(|id| self.tab_manager.get_tab(id))
            .and_then(|tab| tab.rendered_frame.as_ref())
            .map(|frame| frame.image.clone());

        let canvas = self.env.surface.canvas();
        canvas.clear(Color::WHITE);

        // Render the active tab's frame from shared memory
        if let Some(image) = frame_to_render {
            canvas.draw_image(&image, (0.0, 0.0), None);
        }

        // Render UI on top
        self.ui.render(canvas);
        self.ui.render_loading_indicator(canvas, is_loading, self.loading_spinner_angle);

        self.env.gr_context.flush_and_submit();
        self.env.gl_surface.swap_buffers(&self.env.gl_context)
            .map_err(|e| format!("Failed to swap buffers: {}", e))?;

        Ok(())
    }
}

impl ApplicationHandler for BrowserApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        self.env.window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
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

                self.ui.update_layout(width as f32, height as f32);

                // Notify all tabs of resize
                for tab_id in &self.tab_order {
                    let _ = self.tab_manager.send_to_tab(tab_id, ParentToTabMessage::Resize {
                        width: width as f32,
                        height: height as f32
                    });
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = scale_factor;
                self.ui.set_scale_factor(scale_factor);

                // Notify all tabs of scale factor change
                for tab_id in &self.tab_order {
                    let _ = self.tab_manager.send_to_tab(tab_id, ParentToTabMessage::SetScaleFactor(scale_factor));
                }

                self.env.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.render() {
                    eprintln!("Render error: {}", e);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                self.handle_click(self.cursor_position.0 as f32, self.cursor_position.1 as f32, event_loop);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = (position.x, position.y);
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::MouseMove {
                        x: position.x as f32,
                        y: position.y as f32
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let (delta_x, delta_y) = match delta {
                        winit::event::MouseScrollDelta::LineDelta(x, y) => (x * 20.0, y * 20.0),
                        winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32),
                    };
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Scroll {
                        delta_x,
                        delta_y: -delta_y
                    });
                }
                self.env.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Handle keyboard input with the new multi-process architecture
                let action = input::handle_keyboard_input(
                    &event,
                    &self.modifiers,
                    &mut self.ui,
                    self.active_tab_index,
                    self.tab_order.len(),
                );

                // Handle browser-level actions
                match &action {
                    input::InputAction::ForwardToTab(keyboard_input) => {
                        // Forward keyboard input to active tab process
                        if let Some(tab_id) = self.active_tab_id().cloned() {
                            let key_modifiers = ipc::KeyModifiers {
                                ctrl: self.modifiers.state().control_key(),
                                alt: self.modifiers.state().alt_key(),
                                shift: self.modifiers.state().shift_key(),
                                meta: self.modifiers.state().super_key(),
                            };

                            let key_type = match keyboard_input {
                                input::KeyboardInput::Character(s) => {
                                    ipc::KeyInputType::Character(s.clone())
                                }
                                input::KeyboardInput::Named(s) => {
                                    ipc::KeyInputType::Named(s.clone())
                                }
                                input::KeyboardInput::Scroll { direction, amount } => {
                                    let ipc_direction = match direction {
                                        input::ScrollDirection::Up => ipc::ScrollDirection::Up,
                                        input::ScrollDirection::Down => ipc::ScrollDirection::Down,
                                        input::ScrollDirection::Left => ipc::ScrollDirection::Left,
                                        input::ScrollDirection::Right => ipc::ScrollDirection::Right,
                                    };
                                    ipc::KeyInputType::Scroll {
                                        direction: ipc_direction,
                                        amount: *amount,
                                    }
                                }
                            };

                            let _ = self.tab_manager.send_to_tab(
                                &tab_id,
                                ParentToTabMessage::KeyboardInput {
                                    key_type,
                                    modifiers: key_modifiers,
                                },
                            );
                        }
                        self.env.window.request_redraw();
                    }
                    _ => {
                        // Handle non-forwarding actions
                        self.handle_input_action(action, event_loop);
                    }
                }
            }
            _ => {}
        }

        let expected_frame_length_seconds = 1.0 / 60.0;
        let frame_duration = Duration::from_secs_f32(expected_frame_length_seconds);
        let frame_start = Instant::now();

        if frame_start - self.previous_frame_start > frame_duration {
            self.previous_frame_start = frame_start;
            self.env.window.request_redraw();
        }

        let next_frame_time = self.previous_frame_start + frame_duration;
        event_loop.set_control_flow(ControlFlow::WaitUntil(next_frame_time));
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if this is a tab process
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--tab-process" {
        let tab_id = args[2].clone();
        let socket_path = std::path::PathBuf::from(&args[3]);
        return tab_process::tab_process_main(tab_id, socket_path).await.map_err(|e| e.into());
    }

    // Main browser process
    println!("Starting Web Browser...");
    let event_loop = EventLoop::new()?;
    let mut app = BrowserApp::new(&event_loop).await;

    // Navigate initial tab
    if let Some(tab_id) = app.active_tab_id().cloned() {
        let _ = app.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate("https://example.com".to_string()));
    }

    event_loop.run_app(&mut app)?;
    Ok(())
}
