//! Iced-based browser application using iced_winit
//!
//! This module provides an iced-based implementation of the browser UI,
//! designed to integrate with a Skia canvas for custom rendering.

use iced::widget::{button, canvas, column, container, row, text, text_input, Column, Row, image};
use iced::{Color, Element, Length, Padding, Size, Task, Theme, Subscription};
use iced::widget::canvas::{Cache, Canvas, Geometry, Path, Stroke, Program as CanvasProgram};
use iced::widget::image::Handle as ImageHandle;
use std::time::{Duration, Instant};
use iced::border::Radius;
use iced::widget::text::Alignment;

use crate::ipc::{ParentToTabMessage, TabToParentMessage, KeyModifiers, KeyInputType};
use crate::tab_manager::TabManager;
use crate::shell_provider::ShellProviderMessage;

/// Messages that drive the browser UI
#[derive(Debug, Clone)]
pub enum Message {
    // Navigation
    Navigate,
    GoBack,
    GoForward,
    Reload,

    // Address bar
    AddressChanged(String),

    // Tabs
    NewTab,
    CloseTab(usize),
    SwitchTab(usize),

    // Tab process messages
    TabMessageReceived { tab_id: String, message: TabToParentMessage },

    // Canvas/Skia integration events
    CanvasRedraw,

    // Frame updates
    FrameReady,

    // Polling tick for tab messages
    Tick(Instant),

    // Mouse events for content area
    ContentClick { x: f32, y: f32 },
    ContentMouseMove { x: f32, y: f32 },
    ContentScroll { delta_x: f32, delta_y: f32 },

    // Keyboard input
    KeyPressed { key: String, modifiers: KeyModifiers },

    // Window events
    WindowResized { width: u32, height: u32 },
    ScaleFactorChanged(f32),

    // Focus management
    FocusAddressBar,

    // Error handling
    Error(String),
}

/// Represents a browser tab
#[derive(Debug, Clone)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub url: String,
    pub is_loading: bool,
}

impl Tab {
    pub fn new(id: String) -> Self {
        Self {
            id,
            title: "New Tab".to_string(),
            url: String::new(),
            is_loading: false,
        }
    }
}

/// The main iced browser application state
pub struct IcedBrowserApp {
    /// Current address bar text
    address_input: String,

    /// List of open tabs
    tabs: Vec<Tab>,

    /// Index of the active tab
    active_tab: usize,

    /// Tab order (tab IDs in display order)
    tab_order: Vec<String>,

    /// Tab manager for IPC with tab processes
    tab_manager: Option<TabManager>,

    /// Cache for the canvas (used for Skia integration placeholder)
    canvas_cache: Cache,

    /// Scale factor for HiDPI
    scale_factor: f32,

    /// Viewport size
    viewport_size: (u32, u32),

    /// Page viewport size (excluding chrome)
    page_viewport_size: (u32, u32),

    /// Loading spinner angle
    loading_spinner_angle: f32,

    /// Last spinner update time
    last_spinner_update: Instant,

    /// Current cursor position
    cursor_position: (f32, f32),

    /// Current modifiers state
    modifiers: KeyModifiers,

    /// Error message to display
    last_error: Option<String>,
}

impl Default for IcedBrowserApp {
    fn default() -> Self {
        Self::new()
    }
}

impl IcedBrowserApp {
    /// Chrome height in logical pixels (tab bar + toolbar)
    pub const CHROME_HEIGHT: f32 = 80.0;

    pub fn new() -> Self {
        // Create tab manager
        let tab_manager = TabManager::new().ok();

        if tab_manager.is_none() {
            eprintln!("Warning: Failed to create tab manager");
        }

        Self {
            address_input: String::new(),
            tabs: vec![],
            active_tab: 0,
            tab_order: vec![],
            tab_manager,
            canvas_cache: Cache::new(),
            scale_factor: 1.0,
            viewport_size: (1280, 720),
            page_viewport_size: (1280, (720.0 - Self::CHROME_HEIGHT) as u32),
            loading_spinner_angle: 0.0,
            last_spinner_update: Instant::now(),
            cursor_position: (0.0, 0.0),
            modifiers: KeyModifiers {
                ctrl: false,
                alt: false,
                shift: false,
                meta: false,
            },
            last_error: None,
        }
    }

    /// Get the active tab ID
    fn active_tab_id(&self) -> Option<&String> {
        self.tab_order.get(self.active_tab)
    }

    /// Update the page viewport size based on window size
    fn update_page_viewport(&mut self) {
        let chrome_physical = (Self::CHROME_HEIGHT * self.scale_factor).round() as u32;
        self.page_viewport_size = (
            self.viewport_size.0,
            self.viewport_size.1.saturating_sub(chrome_physical),
        );
    }

    /// Create a new tab
    fn create_tab(&mut self) -> Task<Message> {
        if let Some(ref mut tab_manager) = self.tab_manager {
            match tab_manager.create_tab() {
                Ok(tab_id) => {
                    let tab = Tab::new(tab_id.clone());
                    self.tabs.push(tab);
                    self.tab_order.push(tab_id.clone());
                    self.active_tab = self.tab_order.len() - 1;
                    self.address_input.clear();

                    // Send initial configuration to the tab
                    let (width, height) = self.page_viewport_size;
                    let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Resize {
                        width: width as f32,
                        height: height as f32,
                    });
                    let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::SetScaleFactor(self.scale_factor));
                }
                Err(e) => {
                    self.last_error = Some(format!("Failed to create tab: {}", e));
                }
            }
        }
        Task::none()
    }

    /// Navigate the active tab to a URL
    fn navigate_to_url(&mut self, url: &str) {
        if let Some(tab_id) = self.active_tab_id().cloned() {
            if let Some(ref mut tab_manager) = self.tab_manager {
                let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate(url.to_string()));
            }

            // Update tab state
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.url = url.to_string();
                tab.is_loading = true;
            }
        }
    }

    /// Close a tab by index
    fn close_tab(&mut self, index: usize) -> bool {
        if self.tab_order.len() <= 1 {
            return true; // Signal to quit app
        }

        if index < self.tab_order.len() {
            let tab_id = self.tab_order.remove(index);

            // Close the tab process
            if let Some(ref mut tab_manager) = self.tab_manager {
                let _ = tab_manager.close_tab(&tab_id);
            }

            // Remove from tabs list
            self.tabs.retain(|t| t.id != tab_id);

            // Adjust active tab index
            if self.active_tab >= self.tab_order.len() {
                self.active_tab = self.tab_order.len().saturating_sub(1);
            } else if index < self.active_tab && self.active_tab > 0 {
                self.active_tab -= 1;
            }

            // Update address bar
            if let Some(tab_id) = self.active_tab_id() {
                if let Some(tab) = self.tabs.iter().find(|t| &t.id == tab_id) {
                    self.address_input = tab.url.clone();
                }
            }
        }
        false // Don't quit
    }

    /// Switch to a tab by index
    fn switch_to_tab(&mut self, index: usize) {
        if index < self.tab_order.len() {
            self.active_tab = index;
            if let Some(tab_id) = self.active_tab_id() {
                if let Some(tab) = self.tabs.iter().find(|t| &t.id == tab_id) {
                    self.address_input = tab.url.clone();
                }
            }
            self.canvas_cache.clear();
        }
    }

    /// Process messages from tab processes
    fn process_tab_messages(&mut self) -> Vec<Task<Message>> {
        let tasks = vec![];

        // First, collect all the messages
        let messages: Vec<(String, TabToParentMessage)> = if let Some(ref mut tab_manager) = self.tab_manager {
            let msgs = tab_manager.poll_messages();
            // Process each message in tab_manager first
            for (tab_id, message) in &msgs {
                tab_manager.process_tab_message(tab_id, message.clone());
            }
            msgs
        } else {
            vec![]
        };

        // Now process the UI updates separately
        for (tab_id, message) in messages {
            // Handle UI updates
            match &message {
                TabToParentMessage::TitleChanged(title) => {
                    self.update_tab_title(&tab_id, &title);
                }
                TabToParentMessage::NavigationCompleted { url, title } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.title = title.clone();
                        tab.url = url.clone();
                        tab.is_loading = false;
                    }
                    let active_tab_id = self.tab_order.get(self.active_tab).cloned();
                    if active_tab_id.as_ref() == Some(&tab_id) {
                        self.address_input = url.clone();
                    }
                }
                TabToParentMessage::NavigationStarted(url) => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_loading = true;
                        tab.url = url.clone();
                    }
                }
                TabToParentMessage::NavigationFailed(error) => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_loading = false;
                    }
                    self.last_error = Some(format!("Navigation failed: {}", error));
                }
                TabToParentMessage::LoadingStateChanged(is_loading) => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_loading = *is_loading;
                    }
                }
                TabToParentMessage::FrameRendered { .. } => {
                    self.canvas_cache.clear();
                }
                TabToParentMessage::NavigateRequest(url) => {
                    // Handle navigation request from web content
                    let tab_id_clone = tab_id.clone();
                    let url_clone = url.clone();
                    if let Some(ref mut tab_manager) = self.tab_manager {
                        let _ = tab_manager.send_to_tab(&tab_id_clone, ParentToTabMessage::Navigate(url_clone.clone()));
                    }
                    let active_tab_id = self.tab_order.get(self.active_tab).cloned();
                    if active_tab_id.as_ref() == Some(&tab_id) {
                        self.address_input = url_clone;
                    }
                }
                TabToParentMessage::NavigateRequestInNewTab(url) => {
                    // Create new tab and navigate
                    let url_clone = url.clone();
                    let current_index = self.active_tab;
                    self.create_new_tab_internal();
                    self.navigate_to_url_internal(&url_clone);
                    // Switch back to original tab
                    self.switch_to_tab(current_index);
                }
                TabToParentMessage::Alert(msg) => {
                    // Display alert (simple console for now, could use native dialog)
                    println!("Alert from tab {}: {}", tab_id, msg);
                    // Could use rfd for native dialogs
                }
                TabToParentMessage::ShellProvider(shell_msg) => {
                    match shell_msg {
                        ShellProviderMessage::RequestRedraw => {
                            self.canvas_cache.clear();
                        }
                        ShellProviderMessage::SetCursor(_cursor) => {
                            // TODO: Set cursor via iced
                        }
                        ShellProviderMessage::SetWindowTitle(_title) => {
                            // TODO: Set window title
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        tasks
    }

    /// Internal helper to create a new tab without returning a Task
    fn create_new_tab_internal(&mut self) {
        if let Some(ref mut tab_manager) = self.tab_manager {
            match tab_manager.create_tab() {
                Ok(tab_id) => {
                    let tab = Tab::new(tab_id.clone());
                    self.tabs.push(tab);
                    self.tab_order.push(tab_id.clone());
                    self.active_tab = self.tab_order.len() - 1;
                    self.address_input.clear();

                    // Send initial configuration to the tab
                    let (width, height) = self.page_viewport_size;
                    let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Resize {
                        width: width as f32,
                        height: height as f32,
                    });
                    let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::SetScaleFactor(self.scale_factor));
                }
                Err(e) => {
                    self.last_error = Some(format!("Failed to create tab: {}", e));
                }
            }
        }
    }

    /// Internal helper to navigate without creating a Task
    fn navigate_to_url_internal(&mut self, url: &str) {
        let tab_id = self.tab_order.get(self.active_tab).cloned();
        if let Some(tab_id) = tab_id {
            if let Some(ref mut tab_manager) = self.tab_manager {
                let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate(url.to_string()));
            }

            // Update tab state
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.url = url.to_string();
                tab.is_loading = true;
            }
        }
    }

    /// Update the browser state based on a message
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate => {
                let url = self.address_input.clone();
                if !url.is_empty() {
                    self.navigate_to_url(&url);
                }
                Task::none()
            }

            Message::GoBack => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(ref mut tab_manager) = self.tab_manager {
                        let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::GoBack);
                    }
                }
                Task::none()
            }

            Message::GoForward => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(ref mut tab_manager) = self.tab_manager {
                        let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::GoForward);
                    }
                }
                Task::none()
            }

            Message::Reload => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(ref mut tab_manager) = self.tab_manager {
                        let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Reload);
                    }
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab.is_loading = true;
                    }
                }
                Task::none()
            }

            Message::AddressChanged(new_address) => {
                self.address_input = new_address;
                Task::none()
            }

            Message::NewTab => {
                self.create_tab()
            }

            Message::CloseTab(index) => {
                if self.close_tab(index) {
                    // Return a task that will exit the app
                    iced::exit()
                } else {
                    Task::none()
                }
            }

            Message::SwitchTab(index) => {
                self.switch_to_tab(index);
                Task::none()
            }

            Message::TabMessageReceived { tab_id, message } => {
                if let Some(ref mut tab_manager) = self.tab_manager {
                    tab_manager.process_tab_message(&tab_id, message);
                }
                self.canvas_cache.clear();
                Task::none()
            }

            Message::CanvasRedraw => {
                self.canvas_cache.clear();
                Task::none()
            }

            Message::FrameReady => {
                self.canvas_cache.clear();
                Task::none()
            }

            Message::Tick(_now) => {
                // Process tab messages
                let _tasks = self.process_tab_messages();

                // Update loading spinner
                let is_loading = self.active_tab_id()
                    .and_then(|id| self.tabs.iter().find(|t| &t.id == id))
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
                }

                Task::none()
            }

            Message::ContentClick { x, y } => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(ref mut tab_manager) = self.tab_manager {
                        let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Click {
                            x,
                            y,
                            modifiers: self.modifiers.clone(),
                        });
                    }
                }
                Task::none()
            }

            Message::ContentMouseMove { x, y } => {
                self.cursor_position = (x, y);
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(ref mut tab_manager) = self.tab_manager {
                        let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::MouseMove { x, y });
                    }
                }
                Task::none()
            }

            Message::ContentScroll { delta_x, delta_y } => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(ref mut tab_manager) = self.tab_manager {
                        let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Scroll {
                            delta_x,
                            delta_y,
                        });
                    }
                }
                self.canvas_cache.clear();
                Task::none()
            }

            Message::KeyPressed { key, modifiers } => {
                self.modifiers = modifiers.clone();

                // Forward keyboard input to tab
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(ref mut tab_manager) = self.tab_manager {
                        let key_type = KeyInputType::Character(key);
                        let _ = tab_manager.send_to_tab(&tab_id, ParentToTabMessage::KeyboardInput {
                            key_type,
                            modifiers,
                        });
                    }
                }
                Task::none()
            }

            Message::WindowResized { width, height } => {
                self.viewport_size = (width, height);
                self.update_page_viewport();

                // Notify all tabs of resize
                if let Some(ref mut tab_manager) = self.tab_manager {
                    let (w, h) = self.page_viewport_size;
                    for tab_id in &self.tab_order {
                        let _ = tab_manager.send_to_tab(tab_id, ParentToTabMessage::Resize {
                            width: w as f32,
                            height: h as f32,
                        });
                    }
                }
                self.canvas_cache.clear();
                Task::none()
            }

            Message::ScaleFactorChanged(scale) => {
                self.scale_factor = scale;
                self.update_page_viewport();

                // Notify all tabs
                if let Some(ref mut tab_manager) = self.tab_manager {
                    for tab_id in &self.tab_order {
                        let _ = tab_manager.send_to_tab(tab_id, ParentToTabMessage::SetScaleFactor(scale));
                    }
                }
                self.canvas_cache.clear();
                Task::none()
            }

            Message::FocusAddressBar => {
                // TODO: Focus management in iced
                Task::none()
            }

            Message::Error(error) => {
                self.last_error = Some(error);
                Task::none()
            }
        }
    }

    /// Subscription for polling tab messages
    pub fn subscription(&self) -> Subscription<Message> {
        // Poll every 16ms (~60 FPS) for tab messages
        iced::time::every(Duration::from_millis(16))
            .map(Message::Tick)
    }

    fn title(&self) -> String {
        if let Some(tab) = self.active_tab() {
            return tab.title.clone();
        }
        "Stokes Browser".to_string()
    }

    /// Build the view for the browser
    pub fn view(&self) -> Column<Message> {
        // Build tab bar
        let tab_bar = self.build_tab_bar();

        // Build toolbar (navigation buttons + address bar)
        let toolbar = self.build_toolbar();

        // Build content area (canvas placeholder for Skia)
        let content = self.build_content_area();

        // Stack everything vertically
        column![tab_bar, toolbar, content]
            .spacing(0)
            .into()
    }

    /// Build the tab bar row
    fn build_tab_bar(&self) -> Element<Message> {
        let mut tab_buttons: Vec<Element<Message>> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let is_active = index == self.active_tab;
                let title = if tab.title.len() > 15 {
                    format!("{}...", &tab.title[..12])
                } else {
                    tab.title.clone()
                };

                // Add loading indicator to title if loading
                let display_title = if tab.is_loading {
                    format!("⏳ {}", title)
                } else {
                    title
                };

                let tab_content = row![
                    button(text(display_title.clone()).size(12))
                        .on_press(Message::SwitchTab(index))
                        .style(if is_active {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .padding(Padding::from([4, 8])),
                    button(text("×").size(12))
                        .on_press(Message::CloseTab(index))
                        .style(button::text)
                        .padding(Padding::from([4, 6])),
                ]
                .spacing(2);

                let tab_bg = if is_active {
                    Color::from_rgb(0.95, 0.95, 0.95)
                } else {
                    Color::from_rgb(0.85, 0.85, 0.85)
                };

                container(tab_content)
                    .style(move |_theme| {
                        container::Style {
                            background: Some(tab_bg.into()),
                            border: iced::Border {
                                radius: Radius {
                                    top_left: 4.0,
                                    top_right: 4.0,
                                    bottom_left: 0.0,
                                    bottom_right: 0.0
                                },
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    })
                    .padding(2)
                    .into()
            })
            .collect();

        // Add new tab button
        tab_buttons.push(
            button(text("+").size(14))
                .on_press(Message::NewTab)
                .style(button::secondary)
                .padding(Padding::from([4, 10]))
                .into(),
        );

        container(
            Row::with_children(tab_buttons)
                .spacing(2)
                .padding(Padding::from([4, 8])),
        )
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(Color::from_rgb(0.9, 0.9, 0.9).into()),
            ..Default::default()
        })
        .into()
    }

    /// Build the toolbar (navigation + address bar)
    fn build_toolbar(&self) -> Element<Message> {
        let back_btn = button(text("←").size(16))
            .on_press(Message::GoBack)
            .style(button::secondary)
            .padding(Padding::from([6, 10]));

        let forward_btn = button(text("→").size(16))
            .on_press(Message::GoForward)
            .style(button::secondary)
            .padding(Padding::from([6, 10]));

        let reload_btn = button(text("⟳").size(16))
            .on_press(Message::Reload)
            .style(button::secondary)
            .padding(Padding::from([6, 10]));

        let address_bar = text_input("Enter URL...", &self.address_input)
            .on_input(Message::AddressChanged)
            .on_submit(Message::Navigate)
            .padding(8)
            .width(Length::Fill);

        container(
            row![back_btn, forward_btn, reload_btn, address_bar]
                .spacing(4)
                .padding(Padding::from([4, 8]))
                .align_y(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(Color::from_rgb(0.95, 0.95, 0.95).into()),
            border: iced::Border {
                width: 0.0,
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    /// Build the content area (placeholder for Skia canvas integration)
    fn build_content_area(&self) -> Element<Message> {
        // Check if we have a rendered frame from the active tab
        let frame_data = self.active_tab_id()
            .and_then(|tab_id| {
                self.tab_manager.as_ref().and_then(|tm| {
                    tm.get_tab(tab_id).and_then(|tab| {
                        tab.rendered_frame.as_ref().map(|frame| {
                            (frame.raw_pixels.clone(), frame.width, frame.height)
                        })
                    })
                })
            });

        let is_loading = self.active_tab_id()
            .and_then(|id| self.tabs.iter().find(|t| &t.id == id))
            .map(|t| t.is_loading)
            .unwrap_or(false);

        let content: Element<Message> = if let Some((pixels, width, height)) = frame_data {
            // We have a rendered frame - display it as an image
            let handle = ImageHandle::from_rgba(width, height, pixels);
            image(handle)
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Fill)
                .into()
        } else if !self.tabs.is_empty() && is_loading {
            // Loading state - show canvas with spinner
            let canvas_state = CanvasState {
                loading: true,
                loading_angle: self.loading_spinner_angle,
                has_tabs: true,
                has_frame: false,
            };
            Canvas::new(canvas_state)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if !self.tabs.is_empty() {
            // Has tabs but no frame yet - show placeholder
            let canvas_state = CanvasState {
                loading: false,
                loading_angle: 0.0,
                has_tabs: true,
                has_frame: false,
            };
            Canvas::new(canvas_state)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            // No tabs - show welcome
            let canvas_state = CanvasState {
                loading: false,
                loading_angle: 0.0,
                has_tabs: false,
                has_frame: false,
            };
            Canvas::new(canvas_state)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(Color::WHITE.into()),
                ..Default::default()
            })
            .into()
    }

    /// Get the current theme
    pub fn theme(&self) -> Theme {
        Theme::Light
    }

    /// Set the scale factor
    pub fn set_scale_factor(&mut self, scale: f32) {
        self.scale_factor = scale;
        self.update_page_viewport();
    }

    /// Set viewport size
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.viewport_size = (width, height);
        self.update_page_viewport();
        self.canvas_cache.clear();
    }

    /// Get the active tab
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }

    /// Update tab title
    pub fn update_tab_title(&mut self, tab_id: &str, title: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.title = title.to_string();
        }
    }

    /// Update tab URL
    pub fn update_tab_url(&mut self, tab_id: &str, url: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.url = url.to_string();
            // Also update address bar if this is the active tab
            if self.tabs.get(self.active_tab).map(|t| &t.id) == Some(&tab_id.to_string()) {
                self.address_input = url.to_string();
            }
        }
    }

    /// Set tab loading state
    pub fn set_tab_loading(&mut self, tab_id: &str, is_loading: bool) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.is_loading = is_loading;
        }
    }
}

/// Canvas state for rendering - used for loading spinners and placeholders
struct CanvasState {
    loading: bool,
    loading_angle: f32,
    has_tabs: bool,
    has_frame: bool,
}

/// Implement the Canvas Program trait for rendering the content area
impl CanvasProgram<Message> for CanvasState {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut cache = Cache::new();

        let geometry = cache.draw(renderer, bounds.size(), |frame| {
            // Draw background
            let background = Path::rectangle(
                iced::Point::ORIGIN,
                frame.size(),
            );
            frame.fill(&background, Color::WHITE);

            let center = iced::Point::new(
                frame.size().width / 2.0,
                frame.size().height / 2.0,
            );

            if !self.has_tabs {
                // No tabs - show welcome message
                let welcome_text = canvas::Text {
                    content: "Welcome! Press + to open a new tab".to_string(),
                    position: center,
                    color: Color::from_rgb(0.4, 0.4, 0.4),
                    size: iced::Pixels(20.0),
                    align_x: Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..Default::default()
                };
                frame.fill_text(welcome_text);
            } else if self.loading {
                // Loading state - show spinner
                let loading_text = canvas::Text {
                    content: "Loading...".to_string(),
                    position: center,
                    color: Color::from_rgb(0.5, 0.5, 0.5),
                    size: iced::Pixels(18.0),
                    align_x: Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..Default::default()
                };
                frame.fill_text(loading_text);

                // Draw loading spinner
                let spinner_radius = 20.0;
                let spinner_center = iced::Point::new(center.x, center.y + 40.0);

                for i in 0..8 {
                    let angle = self.loading_angle + (i as f32 * std::f32::consts::PI / 4.0);
                    let x = spinner_center.x + spinner_radius * angle.cos();
                    let y = spinner_center.y + spinner_radius * angle.sin();

                    let alpha = 0.3 + (0.7 * (i as f32 / 8.0));
                    let dot = Path::circle(iced::Point::new(x, y), 3.0);
                    frame.fill(&dot, Color::from_rgba(0.3, 0.5, 0.8, alpha));
                }
            } else {
                // No content yet - show placeholder
                let placeholder_text = canvas::Text {
                    content: "Enter a URL to browse".to_string(),
                    position: center,
                    color: Color::from_rgb(0.6, 0.6, 0.6),
                    size: iced::Pixels(16.0),
                    align_x: Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..Default::default()
                };
                frame.fill_text(placeholder_text);
            }

            // Draw border around content area
            let border = Path::rectangle(
                iced::Point::new(1.0, 1.0),
                Size::new(frame.size().width - 2.0, frame.size().height - 2.0),
            );
            frame.stroke(
                &border,
                Stroke::default()
                    .with_color(Color::from_rgb(0.85, 0.85, 0.85))
                    .with_width(1.0),
            );
        });

        vec![geometry]
    }
}

// ============================================================================
// Entry point for running the iced browser
// ============================================================================

/// Run the iced browser application
pub fn run_iced_browser() -> iced::Result {
    iced::application(IcedBrowserApp::default, IcedBrowserApp::update, IcedBrowserApp::view)
        .theme(IcedBrowserApp::theme)
        .subscription(IcedBrowserApp::subscription)
        //.scale_factor(IcedBrowserApp::scale_factor)
        .title(IcedBrowserApp::title)
        .run()
}

// ============================================================================
// iced_winit integration helpers
// ============================================================================

/// Module for iced_winit specific integration
///
/// This provides hooks for integrating iced with a custom winit event loop
/// and Skia rendering pipeline.
pub mod winit_integration {
    use super::*;

    /// Configuration for iced_winit integration
    pub struct IcedWinitConfig {
        /// Whether to use hardware acceleration
        pub hardware_acceleration: bool,
        /// Target frame rate
        pub target_fps: u32,
        /// Enable vsync
        pub vsync: bool,
    }

    impl Default for IcedWinitConfig {
        fn default() -> Self {
            Self {
                hardware_acceleration: true,
                target_fps: 60,
                vsync: true,
            }
        }
    }

    /// Represents a point where Skia can render into the iced scene
    ///
    /// This struct holds the information needed to composite Skia-rendered
    /// content into the iced UI.
    pub struct SkiaIntegrationPoint {
        /// X offset in logical pixels
        pub x: f32,
        /// Y offset in logical pixels (accounts for chrome height)
        pub y: f32,
        /// Width in logical pixels
        pub width: f32,
        /// Height in logical pixels
        pub height: f32,
    }

    impl SkiaIntegrationPoint {
        /// Create a new Skia integration point for the content area
        pub fn for_content_area(viewport_width: f32, viewport_height: f32) -> Self {
            Self {
                x: 0.0,
                y: IcedBrowserApp::CHROME_HEIGHT,
                width: viewport_width,
                height: viewport_height - IcedBrowserApp::CHROME_HEIGHT,
            }
        }

        /// Get the physical pixel bounds (accounting for scale factor)
        pub fn physical_bounds(&self, scale: f32) -> (f32, f32, f32, f32) {
            (
                self.x * scale,
                self.y * scale,
                self.width * scale,
                self.height * scale,
            )
        }
    }

    /// Helper to convert winit events to iced messages
    ///
    /// This can be expanded to handle more event types as needed.
    pub fn winit_event_to_message(
        event: &winit::event::WindowEvent,
    ) -> Option<Message> {
        match event {
            winit::event::WindowEvent::RedrawRequested => Some(Message::CanvasRedraw),
            winit::event::WindowEvent::Resized(size) => Some(Message::WindowResized {
                width: size.width,
                height: size.height,
            }),
            winit::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                Some(Message::ScaleFactorChanged(*scale_factor as f32))
            }
            // Add more event mappings as needed
            _ => None,
        }
    }
}
