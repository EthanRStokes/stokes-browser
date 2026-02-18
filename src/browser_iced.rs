//! Iced-based browser application using iced_winit
//!
//! This module provides an iced-based implementation of the browser UI,
//! designed to integrate with a Skia canvas for custom rendering.

use iced::widget::{button, canvas, column, container, row, text, text_input, Column, Container, Row};
use iced::{Color, Element, Length, Padding, Program, Size, Task, Theme};
use iced::widget::canvas::{Cache, Canvas, Geometry, Path, Stroke, Program as CanvasProgram};
use std::sync::{Arc, RwLock};
use iced::border::Radius;
use iced::widget::text::Alignment;

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

    // Tab process messages (to be expanded)
    TabMessageReceived { tab_id: String, content: String },

    // Canvas/Skia integration events
    CanvasRedraw,

    // Placeholder for external Skia frame updates
    FrameReady,
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

    /// Cache for the canvas (used for Skia integration placeholder)
    canvas_cache: Cache,

    /// Scale factor for HiDPI
    scale_factor: f32,

    /// Viewport size
    viewport_size: (u32, u32),
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
        let initial_tab = Tab::new("tab_0".to_string());

        Self {
            address_input: String::new(),
            tabs: vec![initial_tab],
            active_tab: 0,
            canvas_cache: Cache::new(),
            scale_factor: 1.0,
            viewport_size: (1280, 720),
        }
    }

    /// Update the browser state based on a message
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Navigate => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    tab.url = self.address_input.clone();
                    tab.is_loading = true;
                    // TODO: Send navigation message to tab process
                    println!("Navigating to: {}", self.address_input);
                }
            }

            Message::GoBack => {
                // TODO: Send GoBack to tab process
                println!("Go back");
            }

            Message::GoForward => {
                // TODO: Send GoForward to tab process
                println!("Go forward");
            }

            Message::Reload => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    tab.is_loading = true;
                    // TODO: Send Reload to tab process
                    println!("Reload");
                }
            }

            Message::AddressChanged(new_address) => {
                self.address_input = new_address;
            }

            Message::NewTab => {
                let new_id = format!("tab_{}", self.tabs.len());
                let new_tab = Tab::new(new_id);
                self.tabs.push(new_tab);
                self.active_tab = self.tabs.len() - 1;
                self.address_input.clear();
            }

            Message::CloseTab(index) => {
                if self.tabs.len() > 1 && index < self.tabs.len() {
                    self.tabs.remove(index);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len() - 1;
                    } else if index < self.active_tab {
                        self.active_tab -= 1;
                    }

                    // Update address bar to show current tab's URL
                    if let Some(tab) = self.tabs.get(self.active_tab) {
                        self.address_input = tab.url.clone();
                    }
                }
            }

            Message::SwitchTab(index) => {
                if index < self.tabs.len() {
                    self.active_tab = index;
                    if let Some(tab) = self.tabs.get(self.active_tab) {
                        self.address_input = tab.url.clone();
                    }
                }
            }

            Message::TabMessageReceived { tab_id, content } => {
                // TODO: Handle messages from tab processes
                println!("Tab {} message: {}", tab_id, content);
            }

            Message::CanvasRedraw => {
                self.canvas_cache.clear();
            }

            Message::FrameReady => {
                // Signal that a new frame from Skia is ready
                self.canvas_cache.clear();
            }
        }
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

                let tab_content = row![
                    button(text(title.clone()).size(12))
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

                container(tab_content)
                    .style(|theme| {
                        container::Style {
                            background: Some(Color::from_rgb(0.95, 0.95, 0.95).into()),
                            //background: Some(if is_active {
                            //    Color::from_rgb(0.95, 0.95, 0.95).into()
                            //} else {
                            //    Color::from_rgb(0.85, 0.85, 0.85).into()
                            //}),
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
        // This is where the Skia-rendered content will go
        // For now, we use an iced Canvas as a placeholder that can be
        // integrated with Skia later

        let canvas = Canvas::new(self)
            .width(Length::Fill)
            .height(Length::Fill);

        container(canvas)
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
    }

    /// Set viewport size
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.viewport_size = (width, height);
        self.canvas_cache.clear();
    }

    /// Get the active tab
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }

    /// Get the active tab ID
    pub fn active_tab_id(&self) -> Option<&str> {
        self.tabs.get(self.active_tab).map(|t| t.id.as_str())
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

/// Implement the Canvas Program trait for Skia integration placeholder
impl CanvasProgram<Message> for IcedBrowserApp {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.canvas_cache.draw(renderer, bounds.size(), |frame| {
            // Draw a placeholder background
            let background = Path::rectangle(
                iced::Point::ORIGIN,
                frame.size(),
            );
            frame.fill(&background, Color::WHITE);

            // Draw a centered message indicating this is where Skia content goes
            let center = iced::Point::new(
                frame.size().width / 2.0,
                frame.size().height / 2.0,
            );

            // Draw placeholder text area
            let placeholder_text = "Skia Canvas Integration Point";
            let text = canvas::Text {
                content: placeholder_text.to_string(),
                position: center,
                color: Color::from_rgb(0.6, 0.6, 0.6),
                size: iced::Pixels(16.0),
                align_x: Alignment::Center,
                align_y: iced::alignment::Vertical::Center,
                ..Default::default()
            };
            frame.fill_text(text);

            // Draw a border to show the canvas bounds
            let border = Path::rectangle(
                iced::Point::new(1.0, 1.0),
                Size::new(frame.size().width - 2.0, frame.size().height - 2.0),
            );
            frame.stroke(
                &border,
                Stroke::default()
                    .with_color(Color::from_rgb(0.8, 0.8, 0.8))
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
        .window_size((1280.0, 720.0))
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
            // Add more event mappings as needed
            _ => None,
        }
    }
}
