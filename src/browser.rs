use anyrender::PaintScene;
use blitz_traits::shell::Viewport;
use kurbo::Affine;
use parley::{FontContext, LayoutContext};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use bincode_next::serde::Compat;
use cursor_icon::CursorIcon;
use taffy::Point;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, Modifiers, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;
use winit_core::cursor::Cursor;
use winit_core::event::ButtonSource;
use winit_core::keyboard::{KeyCode, PhysicalKey};
use winit_core::window::{ImeCapabilities, ImeEnableRequest, ImeRequest, ImeRequestData};
use crate::ipc::{ParentToTabMessage, TabToParentMessage};
use crate::renderer::painter::ScenePainter;
use crate::tab_manager::TabManager;
use crate::ui::{BrowserUI, TextBrush};
use crate::window::Env;
use crate::{input, ipc};
use crate::convert_events::{button_source_to_blitz, pointer_source_to_blitz, pointer_source_to_blitz_details, winit_ime_to_blitz, winit_key_event_to_blitz, winit_modifiers_to_kbt_modifiers};
use crate::events::{BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, BlitzWheelEvent, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails, UiEvent};
use crate::shell_provider::ShellProviderMessage;

/// Result of closing a tab
#[derive(Debug, PartialEq)]
enum TabCloseResult {
    Closed,
    QuitApp,
    NoAction,
}

/// The main browser application (parent process)
pub(crate) struct BrowserApp {
    env: Option<Env>,
    modifiers: Modifiers,
    tab_manager: TabManager,
    active_tab_index: usize,
    ui: Option<BrowserUI>,
    viewport: Option<Viewport>,
    page_viewport: Option<Viewport>,
    pointer_position: (f64, f64),
    loading_spinner_angle: f32,
    last_spinner_update: Instant,
    tab_order: Vec<String>,
    font_ctx: FontContext,
    layout_ctx: LayoutContext<TextBrush>,
    startup_url: Option<String>,
    viewport_scroll: Point<f64>,
    buttons: MouseEventButtons,
}

impl BrowserApp {
    pub(crate) async fn new(el: &EventLoop, startup_url: Option<String>) -> Self {
        // Create tab manager
        let tab_manager = TabManager::new().expect("Failed to create tab manager");

        Self {
            env: None,
            modifiers: Modifiers::default(),
            tab_manager,
            active_tab_index: 0,
            ui: None,
            pointer_position: (0.0, 0.0),
            viewport: None,
            page_viewport: None,
            loading_spinner_angle: 0.0,
            last_spinner_update: Instant::now(),
            tab_order: vec![],
            font_ctx: FontContext::new(),
            layout_ctx: LayoutContext::new(),
            startup_url,
            viewport_scroll: Point::default(),
            buttons: MouseEventButtons::None,
        }
    }

    fn env(&self) -> &Env {
        self.env.as_ref().expect("Environment not initialized")
    }

    fn env_mut(&mut self) -> &mut Env {
        self.env.as_mut().expect("Environment not initialized")
    }

    fn ui(&self) -> &BrowserUI {
        self.ui.as_ref().expect("UI not initialized")
    }

    fn ui_mut(&mut self) -> &mut BrowserUI {
        self.ui.as_mut().expect("UI not initialized")
    }

    fn set_viewport(&mut self, size: (u32, u32)) {
        let mut vp = self.viewport.as_mut().unwrap();

        vp.window_size = size;

        self.update_page_viewport();
    }

    fn update_page_viewport(&mut self) {
        let vp = self.viewport.as_ref().unwrap();
        // Calculate the page viewport height in physical pixels by subtracting the chrome height
        // converted to physical pixels using the current hidpi scale.
        let chrome_physical = (BrowserUI::CHROME_HEIGHT as f32 * vp.hidpi_scale).round() as u32;

        let pvp = self.page_viewport.as_mut().unwrap();

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
            self.env.as_ref().unwrap().window.set_title(&format!("Loading: {}", url));
            self.ui.as_mut().unwrap().clear_focus();
        }
    }

    pub(crate) fn add_tab(&mut self) {
        self.add_tab_with_url(None);
    }

    pub(crate) fn add_tab_with_url(&mut self, url: Option<&str>) {
        let env = self.env.as_ref().unwrap();
        let ui = self.ui.as_mut().unwrap();
        if let Ok(new_tab_id) = self.tab_manager.create_tab() {
            ui.add_tab(&new_tab_id, "New Tab");
            self.tab_order.push(new_tab_id.clone());

            // Switch to new tab
            self.active_tab_index = self.tab_order.len() - 1;
            ui.set_active_tab(&new_tab_id);

            // Send initial configuration
            let (width, height) = self.page_viewport.as_ref().unwrap().window_size;
            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::Resize {
                width: width as f32,
                height: height as f32
            });
            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::SetScaleFactor(self.viewport.as_ref().unwrap().hidpi_scale));

            if let Some(u) = url {
                // Navigate to the provided URL immediately
                ui.update_address_bar(u);
                let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::Navigate(u.to_string()));
                env.window.set_title(&format!("Loading: {}", u));
            } else {
                // Clear the address bar when opening a blank new tab
                ui.update_address_bar("");
                ui.set_focus("address_bar");
            }
        }
    }

    fn close_tab(&mut self, tab_index: usize) -> TabCloseResult {
        if self.tab_order.len() <= 1 {
            return TabCloseResult::QuitApp;
        }

        if tab_index < self.tab_order.len() {
            let tab_id = self.tab_order.remove(tab_index);
            let _ = self.tab_manager.close_tab(&tab_id);
            self.ui.as_mut().unwrap().remove_tab(&tab_id);

            // Adjust active tab index
            if self.active_tab_index >= self.tab_order.len() {
                self.active_tab_index = self.tab_order.len() - 1;
            } else if tab_index <= self.active_tab_index && self.active_tab_index > 0 {
                self.active_tab_index -= 1;
            }

            // Update UI
            if let Some(active_id) = self.active_tab_id().cloned() {
                self.ui.as_mut().unwrap().set_active_tab(&active_id);
                if let Some(tab) = self.tab_manager.get_tab(&active_id) {
                    self.ui.as_mut().unwrap().update_address_bar(&tab.url);
                    self.env.as_ref().unwrap().window.set_title(&format!("{} - Web Browser", tab.title));
                }
            }

            return TabCloseResult::Closed;
        }
        TabCloseResult::NoAction
    }

    fn switch_to_tab(&mut self, index: usize) {
        let ui = self.ui.as_mut().unwrap();
        if index < self.tab_order.len() {
            self.active_tab_index = index;
            let tab_id = &self.tab_order[index];
            ui.set_active_tab(tab_id);

            if let Some(tab) = self.tab_manager.get_tab(tab_id) {
                ui.update_address_bar(&tab.url);
                self.env.as_ref().unwrap().window.set_title(&format!("{} - Web Browser", tab.title));
            }
            ui.clear_focus();
        }
    }

    fn handle_click(&mut self, x: f32, y: f32, event_loop: &dyn ActiveEventLoop) {
        // Get tab info for UI
        let tabs: Vec<(String, String)> = self.tab_order.iter()
            .filter_map(|id| {
                self.tab_manager.get_tab(id).map(|t| (id.clone(), t.title.clone()))
            })
            .collect();

        // Handle UI clicks
        let action = input::handle_mouse_click_ui(
            x, y, self.ui.as_mut().unwrap(), &tabs, self.active_tab_index
        );

        self.handle_input_action(&action, event_loop);

        // Only forward click to active tab process if UI didn't handle it
    }

    fn handle_middle_click(&mut self, x: f32, y: f32, event_loop: &dyn ActiveEventLoop) {
        // Get tab info for UI
        let tabs: Vec<(String, String)> = self.tab_order.iter()
            .filter_map(|id| {
                self.tab_manager.get_tab(id).map(|t| (id.clone(), t.title.clone()))
            })
            .collect();

        // Handle middle-click on UI elements (like tabs)
        let action = input::handle_middle_click(
            x, y, self.ui.as_mut().unwrap(), &tabs
        );

        self.handle_input_action(&action, event_loop);

        // Only forward middle-click to active tab process if UI didn't handle it
        // This will make links open in new tab
        if action == input::InputAction::None {
            if let Some(tab_id) = self.active_tab_id().cloned() {
                // Apply chrome offset to forwarded coordinates so tab sees coordinates relative to its page canvas
                let chrome_offset = BrowserUI::CHROME_HEIGHT as f32 * self.viewport.as_ref().unwrap().hidpi_scale;
                let forwarded_y = (y - chrome_offset).max(0.0);

                let key_modifiers = ipc::KeyModifiers {
                    ctrl: true,  // Middle-click should behave like Ctrl+click
                    alt: self.modifiers.state().alt_key(),
                    shift: self.modifiers.state().shift_key(),
                    meta: self.modifiers.state().meta_key(),
                };
                //let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Click {
                //    x,
                //    y: forwarded_y,
                //    modifiers: key_modifiers,
                //});
            }
        }
    }

    fn handle_input_action(&mut self, action: &input::InputAction, event_loop: &dyn ActiveEventLoop) {
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
                self.ui.as_mut().unwrap().reorder_tabs(*from_index, *to_index);

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
            input::InputAction::OpenSettings => {
                self.ui.as_mut().unwrap().toggle_settings();
            }
            input::InputAction::SetDefaultBrowser => {
                crate::default_browser::set_as_default_browser();
                self.show_alert("Stokes Browser has been set as your default browser.");
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
        self.env.as_ref().unwrap().window.request_redraw();
    }

    fn process_tab_messages(&mut self) {
        let messages = self.tab_manager.poll_messages();

        for (tab_id, message) in messages {
            {
                let gr_context = &mut self.env.as_mut().unwrap().gr_context;
                self.tab_manager.process_tab_message(&tab_id, message.clone(), gr_context);
            }

            let env = self.env.as_ref().unwrap();

            // Update UI based on messages
            match message {
                TabToParentMessage::TitleChanged(title) => {
                    self.ui.as_mut().unwrap().update_tab_title(&tab_id, &title);
                    if Some(&tab_id) == self.active_tab_id() {
                        env.window.set_title(&format!("{} - Web Browser", title));
                    }
                }
                TabToParentMessage::NavigationCompleted { url, title } => {
                    self.ui.as_mut().unwrap().update_tab_title(&tab_id, &title);
                    if Some(&tab_id) == self.active_tab_id() {
                        self.ui.as_mut().unwrap().update_address_bar(&url);
                        env.window.set_title(&format!("{} - Web Browser", title));
                    }
                }
                TabToParentMessage::LoadingStateChanged(_is_loading) => {
                    // Update loading indicator
                    env.window.request_redraw();
                }
                TabToParentMessage::FrameRendered { .. } => {
                    env.window.request_redraw();
                }
                TabToParentMessage::NavigateRequest(url) => {
                    // Handle navigation request from web content (e.g., link clicks)
                    println!("Handling navigation request to: {}", url);
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate(url.clone()));
                    if Some(&tab_id) == self.active_tab_id() {
                        self.ui.as_mut().unwrap().update_address_bar(&url);
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
                TabToParentMessage::ShellProvider(shell_msg) => {
                    match shell_msg {
                        ShellProviderMessage::RequestRedraw => {
                            env.window.request_redraw();
                        }
                        ShellProviderMessage::SetCursor(cursor) => {
                            env.window.set_cursor(Cursor::Icon(CursorIcon::from_str(&cursor).unwrap()));
                        }
                        ShellProviderMessage::SetWindowTitle(title) => {
                            env.window.set_title(&title);
                        }
                        ShellProviderMessage::SetImeEnabled(enabled) => {
                            if enabled {
                                let _ = env.window.request_ime_update(ImeRequest::Enable(
                                    ImeEnableRequest::new(ImeCapabilities::new(), ImeRequestData::default()).unwrap(),
                                ));
                            } else {
                                let _ = env.window.request_ime_update(ImeRequest::Disable);
                            }
                        }
                        ShellProviderMessage::SetImeCursorArea { x, y, width, height } => {
                            let _ = env.window.request_ime_update(ImeRequest::Update(
                                ImeRequestData::default().with_cursor_area(
                                    LogicalPosition::new(x, y).into(),
                                    LogicalSize::new(width, height).into(),
                                ),
                            ));
                        },
                        ShellProviderMessage::ViewportScroll((x, y)) => {
                            self.viewport_scroll = Point { x, y }
                        }
                    }
                },
                TabToParentMessage::UpdateButtons(buttons) => {
                    self.buttons = buttons;
                }
                _ => {}
            }
        }
    }

    pub fn pointer_coords(&self, position: PhysicalPosition<f64>) -> PointerCoords {
        let scale = self.viewport.as_ref().unwrap().scale_f64();
        let chrome_offset = BrowserUI::CHROME_HEIGHT;
        let LogicalPosition::<f32> {
            x: screen_x,
            y: mut screen_y,
        } = position.to_logical(scale);
        screen_y = screen_y - chrome_offset;
        let viewport_scroll_offset = self.viewport_scroll;


        let client_x = screen_x;
        let client_y = screen_y;
        let page_x = client_x + viewport_scroll_offset.x as f32;
        let page_y = client_y + viewport_scroll_offset.y as f32;

        PointerCoords {
            screen_x,
            screen_y,
            client_x,
            client_y,
            page_x,
            page_y,
        }
    }

    fn render(&mut self) -> Result<(), String> {
        // Process messages from tab processes
        self.process_tab_messages();

        let active_tab_id = self.active_tab_id().cloned();
        let env = self.env.as_mut().unwrap();
        let ui = self.ui.as_mut().unwrap();

        // Check tooltip timeouts and request redraw if any tooltip should now be visible
        if ui.update_tooltip_visibility(Instant::now()) {
            self.env.as_ref().unwrap().window.request_redraw();
        }

        // Check if active tab is loading
        let is_loading = active_tab_id.as_ref()
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
            self.env.as_ref().unwrap().window.request_redraw();
        }

        // Get the rendered frame before borrowing canvas
        let frame_to_render = active_tab_id.as_ref()
            .and_then(|id| self.tab_manager.get_tab(id))
            .and_then(|tab| tab.rendered_frame.as_ref())
            .map(|frame| &frame.image);

        let canvas = self.env.as_mut().unwrap().surface.canvas();

        let mut painter = ScenePainter {
            inner: canvas,
            cache: &mut Default::default(),
        };

        painter.reset();

        // Render the active tab's frame from shared memory
        if let Some(image) = frame_to_render {
            // Offset the page content so it renders below the chrome
            let chrome_offset = BrowserUI::CHROME_HEIGHT * self.viewport.as_ref().unwrap().hidpi_scale;
            painter.set_matrix(Affine::translate((0.0, 0.0)));

            canvas.draw_image(image, (0.0, chrome_offset), None);
        }

        // Render UI on top
        ui.render(canvas, &mut self.font_ctx, &mut self.layout_ctx, &mut painter);
        ui.render_loading_indicator(&mut painter, is_loading, self.loading_spinner_angle);

        self.env.as_mut().unwrap().gr_context.flush_and_submit();
        {
            let env = self.env.as_mut().unwrap();
            env.present()?;
        }

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

    fn request_redraw(&self) {
        self.env.as_ref().unwrap().window.request_redraw();
    }
}

impl ApplicationHandler for BrowserApp {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let event_loop = Box::new(event_loop);
        self.env = Some(crate::window::create_window_vk(&event_loop));

        let env = self.env.as_ref().unwrap();
        let viewport = Viewport {
            color_scheme: Default::default(),
            window_size: env.window.surface_size().into(),
            hidpi_scale: env.window.scale_factor() as f32,
            zoom: 1.0,
        };
        let page_viewport = Viewport {
            color_scheme: Default::default(),
            window_size: (
                env.window.surface_size().width,
                // Subtract the chrome height converted into physical pixels (logical chrome * scale)
                (env.window.surface_size().height as f32 - (BrowserUI::CHROME_HEIGHT as f32 * env.window.scale_factor() as f32))
                    .max(0.0)
                    .round() as u32,
            ),
            hidpi_scale: env.window.scale_factor() as f32,
            zoom: 1.0,
        };

        // Initialize UI
        let mut ui = BrowserUI::new(&env.gr_context, &viewport);
        ui.initialize_renderer();
        self.ui = Some(ui);
        self.viewport = Some(viewport);
        self.page_viewport = Some(page_viewport);

        // Supply the Vulkan context to the tab manager so it can import shared VkImages.
        // Clone the ash handles directly from VkState — they already have fully-populated
        // function tables from create_window_vk.  ash handle clones are cheap (just pointer copies).
        {
            let vk = &env.vk;
            let device_info = vk.device_info();
            // ash::Instance and ash::Device implement Clone.
            let ash_instance = vk.ash_instance.clone();
            let ash_physical_device = vk.ash_physical_device;
            let ash_device = vk.ash_device.clone();
            self.tab_manager.set_vulkan_context(device_info, ash_instance, ash_physical_device, ash_device);
        }

        // Create initial tab, navigating to the startup URL if one was provided
        if let Some(url) = self.startup_url.clone() {
            self.add_tab_with_url(Some(&url));
        } else {
            self.add_tab();
        }
        self.startup_url = None;
    }

    fn resumed(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.env.as_ref().unwrap().window.request_redraw();
    }

    fn about_to_wait(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.env.as_ref().unwrap().window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::SurfaceResized(new_size) => {
                let env = self.env.as_mut().unwrap();
                env.recreate_surface();

                // Update viewport size
                self.set_viewport(new_size.into());
                let (width, height) = self.page_viewport.as_ref().unwrap().window_size;

                self.ui.as_mut().unwrap().update_layout(&*self.viewport.as_ref().unwrap());

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
                let mut viewport = self.viewport.as_mut().unwrap();
                let old_scale = viewport.hidpi_scale;
                viewport.hidpi_scale = scale_factor;
                self.ui.as_mut().unwrap().update_scale(viewport.hidpi_scale, old_scale);

                self.update_page_viewport();

                // Notify all tabs of scale factor change
                for tab_id in &self.tab_order {
                    let _ = self.tab_manager.send_to_tab(tab_id, ParentToTabMessage::SetScaleFactor(scale_factor));
                }

                self.env.as_ref().unwrap().window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.render() {
                    eprintln!("Render error: {}", e);
                }
            }
            WindowEvent::PointerButton { state: ElementState::Pressed, button: ButtonSource::Mouse(MouseButton::Left), primary, position, .. } => {
                let ui = self.ui.as_mut().unwrap();
                // Update hover state before handling click
                ui.update_mouse_hover(
                    self.pointer_position.0 as f32,
                    self.pointer_position.1 as f32,
                    Instant::now()
                );

                // Check if we're starting a tab drag (only start drag on tabs)
                let x = self.pointer_position.0 as f32;
                let y = self.pointer_position.1 as f32;
                let chrome_height = ui.chrome_height();

                // Only try to start drag if we're in the tab area (first row)
                if y < chrome_height / 2.0 {
                    // Check if this is a close button click first
                    if ui.check_close_button_click(x, y).is_some() {
                        // Let handle_click process the close button
                        self.handle_click(x, y, event_loop);
                    } else {
                        // Try to start dragging - if it's not on a tab, handle as regular click
                        if !ui.start_tab_drag(x, y) {
                            // Not a tab, handle as regular click (e.g., new tab button)
                            self.handle_click(x, y, event_loop);
                        } else {
                            self.env.as_ref().unwrap().window.request_redraw();
                        }
                    }
                } else {
                    self.handle_click(x, y, event_loop);

                    let Some(tab_id) = self.active_tab_id().cloned() else {
                        return;
                    };

                    // Forward the click to the tab process so the engine can detect link clicks.
                    // Subtract the chrome height so coordinates are relative to the page canvas.
                    let chrome_offset = BrowserUI::CHROME_HEIGHT as f32 * self.viewport.as_ref().unwrap().hidpi_scale;
                    let page_y = (y - chrome_offset).max(0.0);
                    let click_modifiers = ipc::KeyModifiers {
                        ctrl: self.modifiers.state().control_key(),
                        alt: self.modifiers.state().alt_key(),
                        shift: self.modifiers.state().shift_key(),
                        meta: self.modifiers.state().meta_key(),
                    };
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Click {
                        x,
                        y: page_y,
                        modifiers: click_modifiers,
                    });
                    let id = button_source_to_blitz(&ButtonSource::Mouse(MouseButton::Left));
                    let coords = self.pointer_coords(position);
                    self.pointer_position = <(f64, f64)>::from(position);
                    let button = MouseEventButton::Main;

                    self.buttons |= button.into();

                    if id != BlitzPointerId::Mouse {
                        let event = UiEvent::PointerMove(BlitzPointerEvent {
                            id,
                            is_primary: primary,
                            coords,
                            button: Default::default(),
                            buttons: self.buttons,
                            mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                            details: PointerDetails::default()
                        });
                        let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                    }

                    let event = BlitzPointerEvent {
                        id,
                        is_primary: primary,
                        coords,
                        button,
                        buttons: self.buttons,
                        mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                        // TODO: details for pointer up/down events
                        details: PointerDetails::default(),
                    };

                    let event = UiEvent::PointerDown(event);

                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                    self.request_redraw();
                }
            }
            WindowEvent::PointerButton { state: ElementState::Released, button: ButtonSource::Mouse(MouseButton::Left), primary, position, .. } => {
                let x = self.pointer_position.0 as f32;
                let y = self.pointer_position.1 as f32;

                // Check if we were dragging a tab
                if self.ui().is_dragging_tab() {
                    // End the drag and get reorder info
                    if let Some((from_index, to_index)) = self.ui_mut().end_tab_drag() {
                        // Reorder tabs in UI
                        self.ui_mut().reorder_tabs(from_index, to_index);

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
                        if let Some(component_id) = self.ui_mut().handle_click(x, y) {
                            if component_id.starts_with("tab") {
                                if let Some(tab_index) = self.tab_order.iter().position(|id| id == &component_id) {
                                    self.switch_to_tab(tab_index);
                                }
                            }
                        }
                    }
                }

                // Update hover state after mouse release
                self.ui_mut().update_mouse_hover(x, y, Instant::now());


                let Some(tab_id) = self.active_tab_id().cloned() else {
                    return;
                };
                let id = button_source_to_blitz(&ButtonSource::Mouse(MouseButton::Left));
                let coords = self.pointer_coords(position);
                self.pointer_position = <(f64, f64)>::from(position);
                let button = MouseEventButton::Main;

                self.buttons -= button.into();

                if id != BlitzPointerId::Mouse {
                    let event = UiEvent::PointerMove(BlitzPointerEvent {
                        id,
                        is_primary: primary,
                        coords,
                        button: Default::default(),
                        buttons: self.buttons,
                        mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                        details: PointerDetails::default()
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }

                let event = BlitzPointerEvent {
                    id,
                    is_primary: primary,
                    coords,
                    button,
                    buttons: self.buttons,
                    mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                    // TODO: details for pointer up/down events
                    details: PointerDetails::default(),
                };

                let event = UiEvent::PointerUp(event);

                let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                self.request_redraw();
            }
            WindowEvent::PointerButton { state: ElementState::Pressed, button: ButtonSource::Mouse(MouseButton::Middle), primary, position, .. } => {
                // Handle middle-click (open link in new tab)
                self.handle_middle_click(self.pointer_position.0 as f32, self.pointer_position.1 as f32, event_loop);

                let Some(tab_id) = self.active_tab_id().cloned() else {
                    return;
                };
                let id = button_source_to_blitz(&ButtonSource::Mouse(MouseButton::Left));
                let coords = self.pointer_coords(position);
                self.pointer_position = <(f64, f64)>::from(position);
                let button = MouseEventButton::Auxiliary;

                self.buttons |= button.into();

                if id != BlitzPointerId::Mouse {
                    let event = UiEvent::PointerMove(BlitzPointerEvent {
                        id,
                        is_primary: primary,
                        coords,
                        button: Default::default(),
                        buttons: self.buttons,
                        mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                        details: PointerDetails::default()
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }

                let event = BlitzPointerEvent {
                    id,
                    is_primary: primary,
                    coords,
                    button,
                    buttons: self.buttons,
                    mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                    // TODO: details for pointer up/down events
                    details: PointerDetails::default(),
                };

                let event = UiEvent::PointerDown(event);

                let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                self.request_redraw();
            }
            WindowEvent::PointerButton { state, button, primary, position, .. } => {
                let Some(tab_id) = self.active_tab_id().cloned() else {
                    return;
                };
                let id = button_source_to_blitz(&ButtonSource::Mouse(MouseButton::Left));
                let coords = self.pointer_coords(position);
                self.pointer_position = <(f64, f64)>::from(position);
                let button = MouseEventButton::Main;

                self.buttons |= button.into();

                if id != BlitzPointerId::Mouse {
                    let event = UiEvent::PointerMove(BlitzPointerEvent {
                        id,
                        is_primary: primary,
                        coords,
                        button: Default::default(),
                        buttons: self.buttons,
                        mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                        details: PointerDetails::default()
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }

                let event = BlitzPointerEvent {
                    id,
                    is_primary: primary,
                    coords,
                    button,
                    buttons: self.buttons,
                    mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                    // TODO: details for pointer up/down events
                    details: PointerDetails::default(),
                };

                let event = UiEvent::PointerDown(event);

                let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                self.request_redraw();
            }
            WindowEvent::PointerMoved { position, source, primary, .. } => {
                self.pointer_position = (position.x, position.y);

                let ui = self.ui.as_mut().unwrap();
                // Update tab drag if dragging
                if ui.is_dragging_tab() {
                    ui.update_tab_drag(position.x as f32);
                    self.env.as_ref().unwrap().window.request_redraw();
                } else {
                    // Update UI hover state on cursor movement
                    ui.update_mouse_hover(position.x as f32, position.y as f32, Instant::now());

                    if let Some(tab_id) = self.active_tab_id().cloned() {
                        let event = UiEvent::PointerMove(BlitzPointerEvent {
                            id: pointer_source_to_blitz(&source),
                            is_primary: primary,
                            coords: self.pointer_coords(position),
                            button: Default::default(),
                            buttons: self.buttons,
                            mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                            details: pointer_source_to_blitz_details(&source)
                        });
                        let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                    }
                }

                // Request redraw to show hover effects
                self.env.as_ref().unwrap().window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let blitz_delta = match delta {
                        winit::event::MouseScrollDelta::LineDelta(x, y) => BlitzWheelDelta::Lines(x as f64, y as f64),
                        winit::event::MouseScrollDelta::PixelDelta(pos) => BlitzWheelDelta::Pixels(pos.x, pos.y),
                    };

                    let event = BlitzWheelEvent {
                        delta: blitz_delta,
                        coords: self.pointer_coords(PhysicalPosition::from(self.pointer_position)),
                        buttons: self.buttons,
                        mods: winit_modifiers_to_kbt_modifiers(self.modifiers.state()),
                    };

                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(UiEvent::Wheel(event)));
                }
                self.env.as_ref().unwrap().window.request_redraw();
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers;
            }
            WindowEvent::Ime(ime) => {
                let active_tab_id = self.active_tab_id().cloned().unwrap();
                let _ = self.tab_manager.send_to_tab(&active_tab_id, ParentToTabMessage::UI(UiEvent::Ime(winit_ime_to_blitz(ime))));
                self.env.as_ref().unwrap().window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Handle keyboard input with the new multi-process architecture
                let action = input::handle_keyboard_input(
                    &event,
                    &self.modifiers,
                    self.ui.as_mut().unwrap(),
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
                                meta: self.modifiers.state().meta_key(),
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

                            let key_event_data = winit_key_event_to_blitz(&event, self.modifiers.state());
                            let event = if event.state.is_pressed() {
                                UiEvent::KeyDown(key_event_data)
                            } else {
                                UiEvent::KeyUp(key_event_data)
                            };

                            let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                        }
                        self.env.as_ref().unwrap().window.request_redraw();
                    }
                    _ => {
                        // Handle non-forwarding actions
                        self.handle_input_action(&action, event_loop);
                    }
                }
            }
            _ => {}
        }
    }
}
