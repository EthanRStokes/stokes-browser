use anyrender::PaintScene;
use blitz_traits::shell::Viewport;
use glutin::surface::GlSurface;
use kurbo::Affine;
use parley::{FontContext, LayoutContext};
use std::num::NonZeroU32;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Modifiers, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use crate::ipc::{ParentToTabMessage, TabToParentMessage};
use crate::renderer::text::TextPainter;
use crate::tab_manager::TabManager;
use crate::ui::{BrowserUI, TextBrush};
use crate::window::{create_surface, Env};
use crate::{input, ipc};

/// Result of closing a tab
#[derive(Debug, PartialEq)]
enum TabCloseResult {
    Closed,
    QuitApp,
    NoAction,
}

/// The main browser application (parent process)
pub(crate) struct BrowserApp {
    env: Env,
    modifiers: Modifiers,
    previous_frame_start: Instant,
    tab_manager: TabManager,
    active_tab_index: usize,
    ui: BrowserUI,
    viewport: Arc<RwLock<Viewport>>,
    page_viewport: Arc<RwLock<Viewport>>,
    cursor_position: (f64, f64),
    loading_spinner_angle: f32,
    last_spinner_update: Instant,
    tab_order: Vec<String>,
    font_ctx: FontContext,
    layout_ctx: LayoutContext<TextBrush>,
}



impl BrowserApp {
    pub(crate) async fn new(el: &EventLoop<()>) -> Self {
        let env = crate::window::create_window(el);

        let viewport = Arc::new(RwLock::new(Viewport {
            color_scheme: Default::default(),
            window_size: env.window.inner_size().into(),
            hidpi_scale: env.window.scale_factor() as f32,
            zoom: 0.0,
        }));
        let page_viewport = Arc::new(RwLock::new(Viewport {
            color_scheme: Default::default(),
            window_size: (
                env.window.inner_size().width,
                // Subtract the chrome height converted into physical pixels (logical chrome * scale)
                (env.window.inner_size().height as f32 - (BrowserUI::CHROME_HEIGHT as f32 * env.window.scale_factor() as f32))
                    .max(0.0)
                    .round() as u32,
            ),
            hidpi_scale: env.window.scale_factor() as f32,
            zoom: 0.0,
        }));

        // Initialize UI
        let mut ui = BrowserUI::new(&env.gr_context, &viewport.read().unwrap());
        ui.initialize_renderer();

        // Create tab manager
        let tab_manager = TabManager::new().expect("Failed to create tab manager");

        Self {
            env,
            modifiers: Modifiers::default(),
            previous_frame_start: Instant::now(),
            tab_manager,
            active_tab_index: 0,
            ui,
            cursor_position: (0.0, 0.0),
            viewport,
            page_viewport,
            loading_spinner_angle: 0.0,
            last_spinner_update: Instant::now(),
            tab_order: vec![],
            font_ctx: FontContext::new(),
            layout_ctx: LayoutContext::new(),
        }
    }

    fn set_viewport(&mut self, size: (u32, u32)) {
        let mut vp = self.viewport.write().unwrap();

        vp.window_size = size;
        drop(vp);

        self.update_page_viewport();
    }

    fn update_page_viewport(&mut self) {
        let vp = self.viewport.read().unwrap();
        // Calculate the page viewport height in physical pixels by subtracting the chrome height
        // converted to physical pixels using the current hidpi scale.
        let chrome_physical = (BrowserUI::CHROME_HEIGHT as f32 * vp.hidpi_scale).round() as u32;

        let mut pvp = self.page_viewport.write().unwrap();

        pvp.window_size = (vp.window_size.0, vp.window_size.1.saturating_sub(chrome_physical));
        pvp.hidpi_scale = vp.hidpi_scale;
        pvp.zoom = vp.zoom;
        pvp.color_scheme = vp.color_scheme;
    }

    #[inline]
    fn active_tab_id(&self) -> Option<&String> {
        self.tab_order.get(self.active_tab_index)
    }

    fn navigate_to_url(&mut self, url: &str) {
        if let Some(tab_id) = self.active_tab_id().cloned() {
            let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate(url.to_string()));
            self.env.window.set_title(&format!("Loading: {}", url));
        }
    }

    pub(crate) fn add_tab(&mut self) {
        if let Ok(new_tab_id) = self.tab_manager.create_tab() {
            self.ui.add_tab(&new_tab_id, "New Tab");
            self.tab_order.push(new_tab_id.clone());

            // Switch to new tab
            self.active_tab_index = self.tab_order.len() - 1;
            self.ui.set_active_tab(&new_tab_id);

            // Clear the address bar when opening a new tab
            self.ui.update_address_bar("");

            // Send initial configuration
            let (width, height) = self.page_viewport.read().unwrap().window_size;
            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::Resize {
                width: width as f32,
                height: height as f32
            });
            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::SetScaleFactor(self.viewport.read().unwrap().hidpi_scale));

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

        self.handle_input_action(&action, event_loop);

        // Only forward click to active tab process if UI didn't handle it
        if action == input::InputAction::None {
            if let Some(tab_id) = self.active_tab_id().cloned() {
                // Apply chrome offset to forwarded coordinates so tab sees coordinates relative to its page canvas
                let chrome_offset = BrowserUI::CHROME_HEIGHT * self.viewport.read().unwrap().hidpi_scale;
                let forwarded_y = (y - chrome_offset).max(0.0);

                let key_modifiers = ipc::KeyModifiers {
                    ctrl: self.modifiers.state().control_key(),
                    alt: self.modifiers.state().alt_key(),
                    shift: self.modifiers.state().shift_key(),
                    meta: self.modifiers.state().super_key(),
                };
                let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Click {
                    x,
                    y: forwarded_y,
                    modifiers: key_modifiers,
                });
            }
        }
    }

    fn handle_middle_click(&mut self, x: f32, y: f32, event_loop: &ActiveEventLoop) {
        // Get tab info for UI
        let tabs: Vec<(String, String)> = self.tab_order.iter()
            .filter_map(|id| {
                self.tab_manager.get_tab(id).map(|t| (id.clone(), t.title.clone()))
            })
            .collect();

        // Handle middle-click on UI elements (like tabs)
        let action = input::handle_middle_click(
            x, y, &mut self.ui, &tabs
        );

        self.handle_input_action(&action, event_loop);

        // Only forward middle-click to active tab process if UI didn't handle it
        // This will make links open in new tab
        if action == input::InputAction::None {
            if let Some(tab_id) = self.active_tab_id().cloned() {
                // Apply chrome offset to forwarded coordinates so tab sees coordinates relative to its page canvas
                let chrome_offset = BrowserUI::CHROME_HEIGHT as f32 * self.viewport.read().unwrap().hidpi_scale;
                let forwarded_y = (y - chrome_offset).max(0.0);

                let key_modifiers = ipc::KeyModifiers {
                    ctrl: true,  // Middle-click should behave like Ctrl+click
                    alt: self.modifiers.state().alt_key(),
                    shift: self.modifiers.state().shift_key(),
                    meta: self.modifiers.state().super_key(),
                };
                let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Click {
                    x,
                    y: forwarded_y,
                    modifiers: key_modifiers,
                });
            }
        }
    }

    fn handle_input_action(&mut self, action: &input::InputAction, event_loop: &ActiveEventLoop) {
        match action {
            input::InputAction::CloseTab(tab_index) => {
                if self.close_tab(*tab_index) == TabCloseResult::QuitApp {
                    event_loop.exit();
                }
            }
            input::InputAction::Navigate(url) => {
                self.navigate_to_url(&url);
            }
            input::InputAction::SwitchTab(tab_index) => {
                self.switch_to_tab(*tab_index);
            }
            input::InputAction::AddTab => {
                self.add_tab();
            }
            input::InputAction::ReorderTab { from_index, to_index } => {
                // Reorder tabs in UI
                self.ui.reorder_tabs(*from_index, *to_index);

                // Reorder tabs in tab_order
                if *from_index < self.tab_order.len() && *to_index < self.tab_order.len() {
                    let tab_id = self.tab_order.remove(*from_index);
                    self.tab_order.insert(*to_index, tab_id);

                    // Update active tab index if needed
                    if self.active_tab_index == *from_index {
                        self.active_tab_index = *to_index;
                    } else if *from_index < self.active_tab_index && *to_index >= self.active_tab_index {
                        self.active_tab_index -= 1;
                    } else if *from_index > self.active_tab_index && *to_index <= self.active_tab_index {
                        self.active_tab_index += 1;
                    }
                }
            }
            input::InputAction::ReloadPage => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Reload);
                }
            }
            input::InputAction::GoBack => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::GoBack);
                }
            }
            input::InputAction::GoForward => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::GoForward);
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
                TabToParentMessage::NavigateRequest(url) => {
                    // Handle navigation request from web content (e.g., link clicks)
                    println!("Handling navigation request to: {}", url);
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate(url.clone()));
                    if Some(&tab_id) == self.active_tab_id() {
                        self.ui.update_address_bar(&url);
                    }
                }
                TabToParentMessage::NavigateRequestInNewTab(url) => {
                    // Handle navigation request in a new tab (e.g., Ctrl+click on link)
                    println!("Handling navigation request in new tab to: {}", url);
                    let tab_index = self.active_tab_index;
                    self.add_tab();
                    self.navigate_to_url(&*url);

                    self.switch_to_tab(tab_index);
                }
                TabToParentMessage::Alert(message) => {
                    // Display alert dialog using native dialog
                    println!("Alert from tab {}: {}", tab_id, message);
                    self.show_alert(&message);
                }
                _ => {}
            }
        }
    }

    fn render(&mut self) -> Result<(), String> {
        // Process messages from tab processes
        self.process_tab_messages();

        // Check tooltip timeouts and request redraw if any tooltip should now be visible
        if self.ui.update_tooltip_visibility(Instant::now()) {
            self.env.window.request_redraw();
        }

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
            .map(|frame| &frame.image);

        let canvas = self.env.surface.canvas();

        let mut painter = TextPainter {
            inner: canvas,
            cache: &mut Default::default(),
        };

        painter.reset();

        // Render the active tab's frame from shared memory
        if let Some(image) = frame_to_render {
            // Offset the page content so it renders below the chrome
            let chrome_offset = BrowserUI::CHROME_HEIGHT as f32 * self.viewport.read().unwrap().hidpi_scale;
            painter.set_matrix(Affine::translate((0.0, 0.0)));

            canvas.draw_image(image, (0.0, chrome_offset), None);
        }

        // Render UI on top
        self.ui.render(canvas, &mut self.font_ctx, &mut self.layout_ctx, &mut painter);
        self.ui.render_loading_indicator(&mut painter, is_loading, self.loading_spinner_angle);

        self.env.gr_context.flush_and_submit();
        self.env.gl_surface.swap_buffers(&self.env.gl_context)
            .map_err(|e| format!("Failed to swap buffers: {}", e))?;

        Ok(())
    }

    /// Show an alert dialog with the given message
    fn show_alert(&self, message: &str) {
        // For now, use rfd (Rusty File Dialogs) for native dialogs
        // This will display a native OS dialog box
        use rfd::MessageDialog;
        use rfd::MessageLevel;

        MessageDialog::new()
            .set_level(MessageLevel::Info)
            .set_title("Alert")
            .set_description(message)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
}

impl ApplicationHandler for BrowserApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        self.env.window.request_redraw();
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.env.window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                self.env.surface = create_surface(
                    &self.env.window,
                    self.env.fb_info,
                    &mut self.env.gr_context,
                    self.env.num_samples,
                    self.env.stencil_size
                );

                let (width, height): (u32, u32) = new_size.into();
                self.env.gl_surface.resize(
                    &self.env.gl_context,
                    NonZeroU32::new(width.max(1)).unwrap(),
                    NonZeroU32::new(height.max(1)).unwrap()
                );
                // Update viewport size
                self.set_viewport(new_size.into());
                let (width, height) = self.page_viewport.read().unwrap().window_size;

                self.ui.update_layout(&*self.viewport.read().unwrap());

                // Notify all tabs of resize
                for tab_id in &self.tab_order {
                    let _ = self.tab_manager.send_to_tab(tab_id, ParentToTabMessage::Resize {
                        width: width as f32,
                        height: height as f32
                    });
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let scale_factor = scale_factor as f32;
                let mut viewport = self.viewport.write().unwrap();
                let old_scale = viewport.hidpi_scale;
                viewport.hidpi_scale = scale_factor;
                self.ui.update_scale(viewport.hidpi_scale, old_scale);

                drop(viewport);
                self.update_page_viewport();

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
                // Update hover state before handling click
                self.ui.update_mouse_hover(
                    self.cursor_position.0 as f32,
                    self.cursor_position.1 as f32,
                    Instant::now()
                );

                // Check if we're starting a tab drag (only start drag on tabs)
                let x = self.cursor_position.0 as f32;
                let y = self.cursor_position.1 as f32;
                let chrome_height = self.ui.chrome_height();

                // Only try to start drag if we're in the tab area (first row)
                if y < chrome_height / 2.0 {
                    // Check if this is a close button click first
                    if self.ui.check_close_button_click(x, y).is_some() {
                        // Let handle_click process the close button
                        self.handle_click(x, y, event_loop);
                    } else {
                        // Try to start dragging
                        self.ui.start_tab_drag(x, y);
                        self.env.window.request_redraw();
                    }
                } else {
                    self.handle_click(x, y, event_loop);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                let x = self.cursor_position.0 as f32;
                let y = self.cursor_position.1 as f32;

                // Check if we were dragging a tab
                if self.ui.is_dragging_tab() {
                    // End the drag and get reorder info
                    if let Some((from_index, to_index)) = self.ui.end_tab_drag() {
                        // Reorder tabs in UI
                        self.ui.reorder_tabs(from_index, to_index);

                        // Reorder tabs in tab_order
                        if from_index < self.tab_order.len() && to_index < self.tab_order.len() {
                            let tab_id = self.tab_order.remove(from_index);
                            self.tab_order.insert(to_index, tab_id);

                            // Update active tab index if needed
                            if self.active_tab_index == from_index {
                                self.active_tab_index = to_index;
                            } else if from_index < self.active_tab_index && to_index >= self.active_tab_index {
                                self.active_tab_index -= 1;
                            } else if from_index > self.active_tab_index && to_index <= self.active_tab_index {
                                self.active_tab_index += 1;
                            }
                        }
                    } else {
                        // It was a click without significant drag - switch to the tab
                        if let Some(component_id) = self.ui.handle_click(x, y) {
                            if component_id.starts_with("tab") {
                                if let Some(tab_index) = self.tab_order.iter().position(|id| id == &component_id) {
                                    self.switch_to_tab(tab_index);
                                }
                            }
                        }
                    }
                }

                // Update hover state after mouse release
                self.ui.update_mouse_hover(x, y, Instant::now());
                self.env.window.request_redraw();
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Middle, .. } => {
                // Handle middle-click (open link in new tab)
                self.handle_middle_click(self.cursor_position.0 as f32, self.cursor_position.1 as f32, event_loop);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = (position.x, position.y);

                // Update tab drag if dragging
                if self.ui.is_dragging_tab() {
                    self.ui.update_tab_drag(position.x as f32);
                    self.env.window.request_redraw();
                } else {
                    // Update UI hover state on cursor movement
                    self.ui.update_mouse_hover(position.x as f32, position.y as f32, Instant::now());

                    if let Some(tab_id) = self.active_tab_id().cloned() {
                        // Forward mouse move to tab with chrome offset applied
                        let chrome_offset = BrowserUI::CHROME_HEIGHT as f32 * self.viewport.read().unwrap().hidpi_scale;
                        let forwarded_y = (position.y as f32 - chrome_offset).max(0.0);
                        let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::MouseMove {
                            x: position.x as f32,
                            y: forwarded_y
                        });
                    }
                }

                // Request redraw to show hover effects
                self.env.window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let (mut delta_x, mut delta_y) = match delta {
                        winit::event::MouseScrollDelta::LineDelta(x, y) => (x * 20.0, y * 20.0),
                        winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32),
                    };

                    // If shift is held, convert vertical scroll to horizontal scroll with increased speed
                    if self.modifiers.state().shift_key() {
                        delta_x = -delta_y * 5.0;
                        delta_y = 0.0;
                    }

                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Scroll {
                        delta_x,
                        delta_y: -delta_y * 2.0
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
                        self.handle_input_action(&action, event_loop);
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
