use std::collections::HashMap;
use std::time::Duration;
use cosmic::app::{Core, Task};
use std::sync::Arc;
use cosmic::iced::widget::shader::Shader;
use crate::browser_frame_primitive::{BrowserFramePrimitive, BrowserFrameProgram};
use cosmic::iced::{Length, Subscription};
use cosmic::iced::Alignment;
use cosmic::widget::{self, mouse_area};
use cosmic::{Application, Element};

use base64::Engine as _;
use crate::bookmark_context_menu::{
    BookmarkClipboardEntry, BookmarkDragState, BookmarkEditState,
    build_bookmark_context_menu, compute_drag_insert_index, find_bookmark_at_x,
};
use crate::bookmarks::BookmarkStore;
use crate::events::UiEvent;
use crate::ipc::ParentToTabMessage;
use crate::tab_manager::TabManager;
use crate::ipc::TabToParentMessage;
use crate::shell_provider::ShellProviderMessage;

const DEFAULT_HOMEPAGE: &str = "https://html.duckduckgo.com";

#[cfg(debug_assertions)]
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-dev");
#[cfg(not(debug_assertions))]
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct CosmicBrowserApp {
    core: Core,
    tab_manager: TabManager,
    bookmarks: BookmarkStore,

    url_input: String,
    active_tab_index: usize,
    tab_order: Vec<String>,
    current_frame: Option<BrowserFramePrimitive>,
    current_frame_size: Option<(u32, u32)>,
    window_size: (u32, u32),
    window_scale_factor: f32,

    spinner_angle: f32,
    settings_open: bool,
    startup_url: Option<String>,

    // Track mouse position over the page for input events
    page_mouse_position: (f32, f32),
    // Track keyboard modifiers
    keyboard_modifiers: cosmic::iced::keyboard::Modifiers,

    tab_favicon_handles: HashMap<String, widget::image::Handle>,
    bookmark_favicon_handles: HashMap<String, widget::image::Handle>,

    bookmark_clipboard: Option<BookmarkClipboardEntry>,
    bookmark_drag: Option<BookmarkDragState>,
    bookmark_edit: Option<BookmarkEditState>,
    bookmark_bar_mouse_x: f32,
    cursor_over_bar: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    // URL bar
    UrlChanged(String),
    UrlSubmit,

    // Navigation
    GoBack,
    GoForward,
    Refresh,
    Home,

    // Tabs
    NewTab,
    CloseTab(String),
    SwitchTab(usize),

    // Frame polling
    Tick,

    // Page input forwarding
    PageClick,                    // Use tracked mouse position
    PageMouseMove { x: f32, y: f32 },
    PageScroll { delta_x: f32, delta_y: f32 },
    PageButtonReleased,            // Use tracked mouse position
    PagePointerPressed { button: CosmicMouseButton },
    PagePointerReleased { button: CosmicMouseButton },

    // Keyboard input
    KeyPressed {
        key: cosmic::iced::keyboard::Key,
        modified_key: cosmic::iced::keyboard::Key,
        location: cosmic::iced::keyboard::Location,
        modifiers: cosmic::iced::keyboard::Modifiers,
        text: Option<String>,
        repeat: bool,
    },
    KeyReleased {
        key: cosmic::iced::keyboard::Key,
        modified_key: cosmic::iced::keyboard::Key,
        location: cosmic::iced::keyboard::Location,
        modifiers: cosmic::iced::keyboard::Modifiers,
    },
    ModifiersChanged(cosmic::iced::keyboard::Modifiers),

    // Bookmarks
    OpenBookmark(String),
    AddBookmark,
    ToggleSettings,
    SetDefaultBrowser,

    // Bookmark context menu
    BookmarkOpenNewTab(String),
    BookmarkOpenNewWindow(String),
    BookmarkEdit(String),
    BookmarkEditTitleChanged(String),
    BookmarkEditUrlChanged(String),
    BookmarkEditCommit,
    BookmarkEditCancel,
    BookmarkCut(String),
    BookmarkCopy(String),
    BookmarkPasteAfter(String),
    BookmarkDelete(String),

    // Bookmark drag-and-drop
    BookmarkBarMouseMove { x: f32 },
    BookmarkBarEntered,
    BookmarkBarLeft,
    BookmarkMousePressed { id: String },
    LeftMousePressed,
    BookmarkDragReleased,
}

#[derive(Debug, Clone, Copy)]
pub enum CosmicMouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

// Conversion from cosmic::iced::mouse::Button
impl From<cosmic::iced::mouse::Button> for CosmicMouseButton {
    fn from(button: cosmic::iced::mouse::Button) -> Self {
        match button {
            cosmic::iced::mouse::Button::Left => CosmicMouseButton::Left,
            cosmic::iced::mouse::Button::Right => CosmicMouseButton::Right,
            cosmic::iced::mouse::Button::Middle => CosmicMouseButton::Middle,
            cosmic::iced::mouse::Button::Other(val) => CosmicMouseButton::Other(val),
            _ => CosmicMouseButton::Other(0), // Back, Forward, etc.
        }
    }
}

// Convert cosmic Key to keyboard_types::Key
fn cosmic_key_to_kbt_key(key: &cosmic::iced::keyboard::Key) -> keyboard_types::Key {
    match key {
        cosmic::iced::keyboard::Key::Character(ch) => keyboard_types::Key::Character(ch.as_str().into()),
        cosmic::iced::keyboard::Key::Named(named) => match named {
            cosmic::iced::keyboard::key::Named::Enter => keyboard_types::Key::Enter,
            _ => keyboard_types::Key::Unidentified,
        },
        _ => keyboard_types::Key::Unidentified,
    }
}

// Convert cosmic Location to keyboard_types::Location
fn cosmic_location_to_kbt_location(location: cosmic::iced::keyboard::Location) -> keyboard_types::Location {
    match location {
        cosmic::iced::keyboard::Location::Standard => keyboard_types::Location::Standard,
        cosmic::iced::keyboard::Location::Left => keyboard_types::Location::Left,
        cosmic::iced::keyboard::Location::Right => keyboard_types::Location::Right,
        cosmic::iced::keyboard::Location::Numpad => keyboard_types::Location::Numpad,
    }
}

// Convert cosmic Modifiers to keyboard_types::Modifiers
fn cosmic_modifiers_to_kbt_modifiers(modifiers: cosmic::iced::keyboard::Modifiers) -> keyboard_types::Modifiers {
    let mut result = keyboard_types::Modifiers::empty();
    if modifiers.shift() {
        result |= keyboard_types::Modifiers::SHIFT;
    }
    if modifiers.control() {
        result |= keyboard_types::Modifiers::CONTROL;
    }
    if modifiers.alt() {
        result |= keyboard_types::Modifiers::ALT;
    }
    if modifiers.logo() {
        result |= keyboard_types::Modifiers::META;
    }
    result
}

fn decode_favicon_to_handle(bytes: &[u8]) -> Option<cosmic::iced::widget::image::Handle> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = rgba.into_raw();
    Some(widget::image::Handle::from_rgba(width, height, pixels))
}

fn build_bookmark_favicon_handles(items: &[crate::bookmarks::BookmarkNode]) -> HashMap<String, cosmic::iced::widget::image::Handle> {
    let mut handles = HashMap::new();
    for node in items {
        if let Some(favicon_b64) = &node.favicon {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(favicon_b64) {
                if let Some(handle) = decode_favicon_to_handle(&bytes) {
                    handles.insert(node.id.clone(), handle);
                }
            }
        }
        handles.extend(build_bookmark_favicon_handles(&node.children));
    }
    handles
}

// Convert cosmic key event to BlitzKeyEvent for KeyDown
fn cosmic_key_to_blitz_key_down(
    key: cosmic::iced::keyboard::Key,
    _modified_key: cosmic::iced::keyboard::Key,
    location: cosmic::iced::keyboard::Location,
    modifiers: cosmic::iced::keyboard::Modifiers,
    text: Option<String>,
    repeat: bool,
) -> Option<crate::events::BlitzKeyEvent> {
    Some(crate::events::BlitzKeyEvent {
        key: cosmic_key_to_kbt_key(&key),
        code: keyboard_types::Code::Unidentified,
        modifiers: cosmic_modifiers_to_kbt_modifiers(modifiers),
        location: cosmic_location_to_kbt_location(location),
        is_auto_repeating: repeat,
        is_composing: false,
        state: crate::events::KeyState::Pressed,
        text: text.map(|t| t.into()),
    })
}

// Convert cosmic key event to BlitzKeyEvent for KeyUp
fn cosmic_key_to_blitz_key_up(
    key: cosmic::iced::keyboard::Key,
    _modified_key: cosmic::iced::keyboard::Key,
    location: cosmic::iced::keyboard::Location,
    modifiers: cosmic::iced::keyboard::Modifiers,
) -> Option<crate::events::BlitzKeyEvent> {
    Some(crate::events::BlitzKeyEvent {
        key: cosmic_key_to_kbt_key(&key),
        code: keyboard_types::Code::Unidentified,
        modifiers: cosmic_modifiers_to_kbt_modifiers(modifiers),
        location: cosmic_location_to_kbt_location(location),
        is_auto_repeating: false,
        is_composing: false,
        state: crate::events::KeyState::Released,
        text: None,
    })
}

impl CosmicBrowserApp {
    fn active_tab_id(&self) -> Option<&String> {
        self.tab_order.get(self.active_tab_index)
    }

    fn navigate_to_url(&mut self, url: &str) {
        if let Some(tab_id) = self.active_tab_id().cloned() {
            let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate(url.to_string()));
        }
    }

    fn window_scale(&self) -> f32 {
        self.window_scale_factor.max(0.1)
    }

    fn sync_scale_factor_from_core(&mut self) {
        let scale = self.core.scale_factor() as f32;
        if (scale - self.window_scale_factor).abs() > f32::EPSILON {
            self.window_scale_factor = scale;
            for tab_id in &self.tab_order {
                let _ = self.tab_manager.send_to_tab(tab_id, ParentToTabMessage::SetScaleFactor(scale));
            }
            self.send_resize_to_tabs();
        }
    }

    fn page_size_physical(&self, width: f32, height: f32) -> (f32, f32) {
        let chrome_height: f32 = 112.0;
        let page_height = (height - Self::COSMIC_HEADER_HEIGHT - chrome_height).max(0.0);
        let scale = self.window_scale();
        (width * scale, page_height * scale)
    }

    fn send_resize_to_tabs(&mut self) {
        let (width, height) = self.window_size;
        let (physical_width, physical_height) = self.page_size_physical(width as f32, height as f32);
        for tab_id in &self.tab_order.clone() {
            let _ = self.tab_manager.send_to_tab(tab_id, ParentToTabMessage::Resize {
                width: physical_width,
                height: physical_height,
            });
        }
    }

    // COSMIC header bar height (approximate: 32px base + padding)
    const COSMIC_HEADER_HEIGHT: f32 = 48.0;

    fn add_tab_with_url(&mut self, url: Option<&str>) {
        if let Ok(new_tab_id) = self.tab_manager.create_tab() {
            self.tab_order.push(new_tab_id.clone());
            self.active_tab_index = self.tab_order.len() - 1;

            self.sync_scale_factor_from_core();
            let (width, height) = self.window_size;
            let (physical_width, physical_height) = self.page_size_physical(width as f32, height as f32);

            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::Resize {
                width: physical_width,
                height: physical_height,
            });
            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::SetScaleFactor(self.window_scale()));

            if let Some(u) = url {
                self.url_input = u.to_string();
                let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::Navigate(u.to_string()));
            } else {
                self.url_input = String::new();
            }
        }
    }

    fn close_tab(&mut self, tab_id: &str) {
        if self.tab_order.len() <= 1 {
            // Last tab — exit the app process so the window actually closes.
            std::process::exit(0);
        }
        if let Some(idx) = self.tab_order.iter().position(|id| id == tab_id) {
            self.tab_order.remove(idx);
            self.tab_favicon_handles.remove(tab_id);
            let _ = self.tab_manager.close_tab(tab_id);
            if self.active_tab_index >= self.tab_order.len() {
                self.active_tab_index = self.tab_order.len() - 1;
            } else if idx < self.active_tab_index {
                self.active_tab_index -= 1;
            }
            // Update url bar for new active tab
            if let Some(id) = self.active_tab_id().cloned() {
                if let Some(tab) = self.tab_manager.get_tab(&id) {
                    self.url_input = tab.url.clone();
                }
            }
        }
    }

    fn switch_to_tab(&mut self, index: usize) {
        if index < self.tab_order.len() {
            self.active_tab_index = index;
            let tab_id = self.tab_order[index].clone();
            if let Some(tab) = self.tab_manager.get_tab(&tab_id) {
                self.url_input = tab.url.clone();
            }
        }
    }

    fn process_tab_messages(&mut self) {
        let messages = self.tab_manager.poll_messages();
        for (tab_id, message) in messages {
            self.tab_manager.process_tab_message(&tab_id, message.clone());
            match message {
                TabToParentMessage::NavigationStarted(_) => {
                    self.tab_favicon_handles.remove(&tab_id);
                }
                TabToParentMessage::NavigationCompleted { url, .. } => {
                    if Some(&tab_id) == self.active_tab_id() {
                        self.url_input = url.clone();
                    }
                }
                TabToParentMessage::NavigateRequest(url) => {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Navigate(url.clone()));
                    if Some(&tab_id) == self.active_tab_id() {
                        self.url_input = url;
                    }
                }
                TabToParentMessage::NavigateRequestInNewTab(url) => {
                    let prev_index = self.active_tab_index;
                    self.add_tab_with_url(Some(&url));
                    self.switch_to_tab(prev_index);
                }
                TabToParentMessage::FrameRendered { shmem_name, width, height } => {
                    if let Some(tab) = self.tab_manager.get_tab_mut(&tab_id) {
                        if let Ok(pixels) = TabManager::load_frame_pixels_from_shmem(tab, &shmem_name, width, height) {
                            if Some(&tab_id) == self.active_tab_id() {
                                self.current_frame = Some(BrowserFramePrimitive {
                                    pixels: Arc::new(pixels),
                                    width,
                                    height,
                                });
                                self.current_frame_size = Some((width, height));
                            }
                        }
                    }
                }
                TabToParentMessage::FaviconUpdated(Some(bytes)) => {
                    if let Some(handle) = decode_favicon_to_handle(&bytes) {
                        self.tab_favicon_handles.insert(tab_id.clone(), handle);
                    }
                    let tab_url = self.tab_manager.get_tab(&tab_id)
                        .map(|t| t.url.clone())
                        .unwrap_or_default();
                    if !tab_url.is_empty() {
                        self.bookmarks.set_favicon_for_url(&tab_url, &bytes);
                        if let Some(bm) = self.bookmarks.find_by_url(&tab_url) {
                            let bm_id = bm.id.clone();
                            if let Some(handle) = decode_favicon_to_handle(&bytes) {
                                self.bookmark_favicon_handles.insert(bm_id, handle);
                            }
                        }
                    }
                }
                TabToParentMessage::FaviconUpdated(None) => {
                    self.tab_favicon_handles.remove(&tab_id);
                }
                TabToParentMessage::Alert(msg) => {
                    eprintln!("Alert from tab {}: {}", tab_id, msg);
                }
                TabToParentMessage::ShellProvider(ShellProviderMessage::SetWindowTitle(_title)) => {}
                _ => {}
            }
        }
    }

    fn tab_bar_view(&self) -> Element<'_, Message> {
        let mut row = widget::row::with_capacity(self.tab_order.len() + 1);

        for (i, tab_id) in self.tab_order.iter().enumerate() {
            let tab = self.tab_manager.get_tab(tab_id);
            let title = tab.map(|t| t.title.as_str()).unwrap_or("New Tab");
            let is_loading = tab.map(|t| t.is_loading).unwrap_or(false);
            let is_active = i == self.active_tab_index;

            let title_text = widget::text(if title.len() > 20 {
                format!("{}…", &title[..20])
            } else {
                title.to_string()
            });

            let loading_indicator: Element<'_, Message> = if is_loading {
                Element::from(widget::text("⟳").size(14))
            } else if let Some(handle) = self.tab_favicon_handles.get(tab_id) {
                Element::from(
                    widget::image::Image::new(handle.clone())
                        .width(Length::Fixed(16.0))
                        .height(Length::Fixed(16.0))
                )
            } else {
                Element::from(widget::space::horizontal().width(Length::Fixed(16.0)))
            };

            let close_btn = widget::button::icon(
                widget::icon::from_name("window-close-symbolic").size(12)
            )
            .on_press(Message::CloseTab(tab_id.clone()))
            .padding(2);

            let tab_content = widget::row![
                loading_indicator,
                title_text,
                close_btn,
            ]
            .spacing(4)
            .align_y(Alignment::Center);

            let tab_btn = if is_active {
                widget::button::custom(tab_content)
                    .on_press(Message::SwitchTab(i))
                    .class(cosmic::theme::Button::Suggested)
            } else {
                widget::button::custom(tab_content)
                    .on_press(Message::SwitchTab(i))
                    .class(cosmic::theme::Button::Text)
            };

            row = row.push(tab_btn);
        }

        // New tab button
        let new_tab_btn = widget::button::icon(
            widget::icon::from_name("list-add-symbolic").size(16)
        )
        .on_press(Message::NewTab)
        .padding(4);

        row = row.push(new_tab_btn);

        Element::from(
            row.spacing(2)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .height(Length::Fixed(40.0))
        )
    }

    fn nav_bar_view(&self) -> Element<'_, Message> {
        let back_btn = widget::button::icon(
            widget::icon::from_name("go-previous-symbolic").size(16)
        )
        .on_press(Message::GoBack)
        .padding(6);

        let fwd_btn = widget::button::icon(
            widget::icon::from_name("go-next-symbolic").size(16)
        )
        .on_press(Message::GoForward)
        .padding(6);

        let refresh_btn = widget::button::icon(
            widget::icon::from_name("view-refresh-symbolic").size(16)
        )
        .on_press(Message::Refresh)
        .padding(6);

        let home_btn = widget::button::icon(
            widget::icon::from_name("go-home-symbolic").size(16)
        )
        .on_press(Message::Home)
        .padding(6);

        let url_input = widget::text_input("Enter URL...", &self.url_input)
            .on_input(Message::UrlChanged)
            .on_submit(|_| Message::UrlSubmit)
            .width(Length::Fill);

        let bookmark_btn = widget::button::icon(
            widget::icon::from_name("user-bookmarks-symbolic").size(16)
        )
        .on_press(Message::AddBookmark)
        .padding(6);

        let settings_btn = widget::button::icon(
            widget::icon::from_name("emblem-system-symbolic").size(16)
        )
        .on_press(Message::ToggleSettings)
        .padding(6);

        Element::from(
            widget::row![
                back_btn,
                fwd_btn,
                refresh_btn,
                home_btn,
                url_input,
                bookmark_btn,
                settings_btn,
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fixed(40.0))
        )
    }

    fn bookmarks_bar_view(&self) -> Element<'_, Message> {
        let drag_insert_idx = self.bookmark_drag.as_ref()
            .filter(|d| d.active)
            .map(|d| compute_drag_insert_index(self.bookmarks.items(), d.current_x));

        let mut row = widget::row::with_capacity(32);

        for (i, node) in self.bookmarks.items().iter().enumerate() {
            if drag_insert_idx == Some(i) {
                row = row.push(widget::divider::vertical::light());
            }

            let icon_elem: Element<'_, Message> = if node.is_folder() {
                Element::from(widget::icon::from_name("folder-symbolic").size(14).icon())
            } else if let Some(handle) = self.bookmark_favicon_handles.get(&node.id) {
                Element::from(
                    widget::image::Image::new(handle.clone())
                        .width(Length::Fixed(14.0))
                        .height(Length::Fixed(14.0))
                )
            } else {
                Element::from(widget::space::horizontal().width(Length::Fixed(14.0)))
            };

            let bm_btn = widget::button::custom(
                widget::row![
                    icon_elem,
                    widget::text(&node.title).size(13),
                ]
                .spacing(4)
                .align_y(Alignment::Center)
            )
            .on_press(Message::BookmarkMousePressed { id: node.id.clone() })
            .class(cosmic::theme::Button::Text)
            .padding([2, 6]);

            let cm = cosmic::widget::context_menu(
                bm_btn,
                build_bookmark_context_menu(
                    &node.id,
                    self.bookmark_clipboard.is_some(),
                    node.is_folder(),
                ),
            );

            let item: Element<'_, Message> =
                if self.bookmark_edit.as_ref().map_or(false, |e| e.id == node.id) {
                    let edit = self.bookmark_edit.as_ref().unwrap();
                    let mut form_col = widget::column::with_capacity(3).spacing(4);
                    form_col = form_col.push(
                        widget::text_input("Title", &edit.title)
                            .on_input(Message::BookmarkEditTitleChanged),
                    );
                    if !edit.is_folder {
                        form_col = form_col.push(
                            widget::text_input("URL", &edit.url)
                                .on_input(Message::BookmarkEditUrlChanged),
                        );
                    }
                    form_col = form_col.push(
                        widget::row![
                            widget::button::text("Save").on_press(Message::BookmarkEditCommit),
                            widget::button::text("Cancel").on_press(Message::BookmarkEditCancel),
                        ]
                        .spacing(4),
                    );
                    Element::from(
                        cosmic::widget::popover(cm)
                            .popup(widget::container(form_col).padding(8))
                            .position(cosmic::widget::popover::Position::Bottom)
                            .on_close(Message::BookmarkEditCancel),
                    )
                } else {
                    Element::from(cm)
                };

            row = row.push(item);
        }

        if drag_insert_idx == Some(self.bookmarks.items().len()) {
            row = row.push(widget::divider::vertical::light());
        }

        Element::from(
            mouse_area(
                row.spacing(2)
                    .align_y(Alignment::Center)
                    .width(Length::Fill)
                    .height(Length::Fixed(32.0)),
            )
            .on_move(|pos: cosmic::iced::Point| Message::BookmarkBarMouseMove { x: pos.x })
            .on_enter(Message::BookmarkBarEntered)
            .on_exit(Message::BookmarkBarLeft),
        )
    }

    fn page_content_view(&self) -> Element<'_, Message> {
        let image_widget: Element<'_, Message> = if let Some(primitive) = &self.current_frame {
            let program = BrowserFrameProgram { current: Some(primitive.clone()) };
            Element::from(
                Shader::new(program)
                    .width(Length::Fill)
                    .height(Length::Fill)
            )
        } else {
            Element::from(
                widget::container(widget::text("Loading..."))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
            )
        };

        Element::from(
            mouse_area(image_widget)
                .on_press(Message::PageClick)
                .on_release(Message::PageButtonReleased)
                .on_middle_press(Message::PagePointerPressed { button: CosmicMouseButton::Middle })
                .on_middle_release(Message::PagePointerReleased { button: CosmicMouseButton::Middle })
                .on_move(|pos: cosmic::iced::Point| Message::PageMouseMove { x: pos.x, y: pos.y })
                .on_scroll(|delta| {
                    use cosmic::iced::mouse::ScrollDelta;
                    let (dx, dy) = match delta {
                        ScrollDelta::Lines { x, y } => (x, y),
                        ScrollDelta::Pixels { x, y } => (x, y),
                    };
                    Message::PageScroll { delta_x: dx, delta_y: dy }
                })
        )
    }
}

impl Application for CosmicBrowserApp {
    type Executor = cosmic::executor::Default;
    type Flags = Option<String>;
    type Message = Message;

    const APP_ID: &'static str = "com.stokesbrowser.StokesBrowser";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let tab_manager = TabManager::new().expect("Failed to create tab manager");
        let bookmarks = BookmarkStore::load_from_disk();
        let bookmark_favicon_handles = build_bookmark_favicon_handles(bookmarks.items());

        let mut app = Self {
            core,
            tab_manager,
            bookmarks,
            url_input: String::new(),
            active_tab_index: 0,
            tab_order: vec![],
            current_frame: None,
            current_frame_size: None,
            window_size: (1280, 800),
            window_scale_factor: 1.0,
            spinner_angle: 0.0,
            settings_open: false,
            startup_url: flags,
            page_mouse_position: (0.0, 0.0),
            keyboard_modifiers: cosmic::iced::keyboard::Modifiers::empty(),
            tab_favicon_handles: HashMap::new(),
            bookmark_favicon_handles,
            bookmark_clipboard: None,
            bookmark_drag: None,
            bookmark_edit: None,
            bookmark_bar_mouse_x: 0.0,
            cursor_over_bar: false,
        };

        let initial_scale = app.core.scale_factor() as f32;
        app.window_scale_factor = initial_scale;

        let startup_url = app.startup_url.clone();
        app.add_tab_with_url(startup_url.as_deref().or(Some(DEFAULT_HOMEPAGE)));
        app.startup_url = None;

        (app, Task::none())
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch([
            cosmic::iced::time::every(Duration::from_millis(16))
                .map(|_| Message::Tick),

            // Listen to keyboard events
            cosmic::iced::event::listen_with(|event, _status, _id| {
                match event {
                    cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                        key,
                        modified_key,
                        physical_key: _,
                        location,
                        modifiers,
                        text,
                        repeat,
                    }) => Some(Message::KeyPressed {
                        key,
                        modified_key,
                        location,
                        modifiers,
                        text: text.map(|t| t.to_string()),
                        repeat,
                    }),
                    cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyReleased {
                        key,
                        modified_key,
                        physical_key: _,
                        location,
                        modifiers,
                    }) => Some(Message::KeyReleased {
                        key,
                        modified_key,
                        location,
                        modifiers,
                    }),
                    cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                        Some(Message::ModifiersChanged(modifiers))
                    }
                    // Global left-button press starts bookmark drag detection
                    cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonPressed(
                        cosmic::iced::mouse::Button::Left,
                    )) => Some(Message::LeftMousePressed),
                    // Global left-button release clears any active bookmark drag
                    cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
                        cosmic::iced::mouse::Button::Left,
                    )) => Some(Message::BookmarkDragReleased),
                    _ => None,
                }
            }),
        ])
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::Tick => {
                self.process_tab_messages();
                self.sync_scale_factor_from_core();

                // Animate spinner
                let is_any_loading = self.tab_order.iter().any(|id| {
                    self.tab_manager.get_tab(id).map(|t| t.is_loading).unwrap_or(false)
                });
                if is_any_loading {
                    self.spinner_angle += 0.1;
                    if self.spinner_angle >= std::f32::consts::TAU {
                        self.spinner_angle -= std::f32::consts::TAU;
                    }
                }
            }

            Message::UrlChanged(url) => {
                self.url_input = url;
            }

            Message::UrlSubmit => {
                let url = self.url_input.trim().to_string();
                if !url.is_empty() {
                    let nav_url = if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("about:") {
                        url
                    } else if url.contains('.') && !url.contains(' ') {
                        format!("https://{}", url)
                    } else {
                        format!("https://html.duckduckgo.com/html/?q={}", percent_encoding::utf8_percent_encode(&url, percent_encoding::NON_ALPHANUMERIC))
                    };
                    self.navigate_to_url(&nav_url);
                }
            }

            Message::GoBack => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::GoBack);
                }
            }

            Message::GoForward => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::GoForward);
                }
            }

            Message::Refresh => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::Reload);
                }
            }

            Message::Home => {
                self.navigate_to_url(DEFAULT_HOMEPAGE);
            }

            Message::NewTab => {
                self.add_tab_with_url(None);
            }

            Message::CloseTab(tab_id) => {
                self.close_tab(&tab_id);
            }

            Message::SwitchTab(index) => {
                self.switch_to_tab(index);
                // Reload current frame for newly active tab
                self.current_frame = None;
            }

            Message::PageClick => {
                let (x, y) = self.page_mouse_position;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    let event = UiEvent::PointerDown(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            screen_x: x,
                            screen_y: y,
                            client_x: x,
                            client_y: y,
                            page_x: x,
                            page_y: y,
                        },
                        button: MouseEventButton::Main,
                        buttons: MouseEventButtons::Primary,
                        mods: cosmic_modifiers_to_kbt_modifiers(self.keyboard_modifiers),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::PageButtonReleased => {
                let (x, y) = self.page_mouse_position;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    let event = UiEvent::PointerUp(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            screen_x: x,
                            screen_y: y,
                            client_x: x,
                            client_y: y,
                            page_x: x,
                            page_y: y,
                        },
                        button: MouseEventButton::Main,
                        buttons: MouseEventButtons::None,
                        mods: cosmic_modifiers_to_kbt_modifiers(self.keyboard_modifiers),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::PageMouseMove { x, y } => {
                // Track mouse position for click events
                self.page_mouse_position = (x, y);

                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    use keyboard_types::Modifiers;
                    let event = UiEvent::PointerMove(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            screen_x: x,
                            screen_y: y,
                            client_x: x,
                            client_y: y,
                            page_x: x,
                            page_y: y,
                        },
                        button: MouseEventButton::default(),
                        buttons: MouseEventButtons::None,
                        mods: Modifiers::empty(),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::PageScroll { delta_x, delta_y } => {
                let (x, y) = self.page_mouse_position;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzWheelDelta, BlitzWheelEvent, MouseEventButtons, PointerCoords};
                    use keyboard_types::Modifiers;
                    let event = UiEvent::Wheel(BlitzWheelEvent {
                        delta: BlitzWheelDelta::Pixels(delta_x as f64, delta_y as f64),
                        coords: PointerCoords {
                            screen_x: x,
                            screen_y: y,
                            client_x: x,
                            client_y: y,
                            page_x: x,
                            page_y: y,
                        },
                        buttons: MouseEventButtons::None,
                        mods: Modifiers::empty(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            // Handle pointer pressed with specific button
            Message::PagePointerPressed { button } => {
                let (x, y) = self.page_mouse_position;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};

                    let (blitz_button, buttons) = match button {
                        CosmicMouseButton::Left => (MouseEventButton::Main, MouseEventButtons::Primary),
                        CosmicMouseButton::Right => (MouseEventButton::Secondary, MouseEventButtons::Secondary),
                        CosmicMouseButton::Middle => (MouseEventButton::Auxiliary, MouseEventButtons::Auxiliary),
                        CosmicMouseButton::Other(_) => (MouseEventButton::Main, MouseEventButtons::Primary),
                    };

                    let event = UiEvent::PointerDown(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            screen_x: x,
                            screen_y: y,
                            client_x: x,
                            client_y: y,
                            page_x: x,
                            page_y: y,
                        },
                        button: blitz_button,
                        buttons,
                        mods: cosmic_modifiers_to_kbt_modifiers(self.keyboard_modifiers),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            // Handle pointer released with specific button
            Message::PagePointerReleased { button } => {
                let (x, y) = self.page_mouse_position;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};

                    let (blitz_button, buttons) = match button {
                        CosmicMouseButton::Left => (MouseEventButton::Main, MouseEventButtons::None),
                        CosmicMouseButton::Right => (MouseEventButton::Secondary, MouseEventButtons::None),
                        CosmicMouseButton::Middle => (MouseEventButton::Auxiliary, MouseEventButtons::None),
                        CosmicMouseButton::Other(_) => (MouseEventButton::Main, MouseEventButtons::None),
                    };

                    let event = UiEvent::PointerUp(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            screen_x: x,
                            screen_y: y,
                            client_x: x,
                            client_y: y,
                            page_x: x,
                            page_y: y,
                        },
                        button: blitz_button,
                        buttons,
                        mods: cosmic_modifiers_to_kbt_modifiers(self.keyboard_modifiers),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            // Keyboard input handling
            Message::KeyPressed { key, modified_key, location, modifiers, text, repeat } => {
                self.keyboard_modifiers = modifiers;

                // Browser-level keybinds — intercept before forwarding to tab
                if modifiers.control() {
                    use cosmic::iced::keyboard::Key;
                    use cosmic::iced::keyboard::key::Named;
                    match &key {
                        // New Tab
                        Key::Character(ch) if ch.as_str() == "t" => {
                            self.add_tab_with_url(None);
                            return Task::none();
                        }
                        // Close Tab
                        Key::Character(ch) if ch.as_str() == "w" => {
                            if let Some(tab_id) = self.active_tab_id().cloned() {
                                self.close_tab(&tab_id);
                            }
                            return Task::none();
                        }
                        Key::Named(Named::Tab) => {
                            let n = self.tab_order.len();
                            if n > 0 {
                                if modifiers.shift() {
                                    let prev = (self.active_tab_index + n - 1) % n;
                                    self.switch_to_tab(prev);
                                } else {
                                    let next = (self.active_tab_index + 1) % n;
                                    self.switch_to_tab(next);
                                }
                                self.current_frame = None;
                            }
                            return Task::none();
                        }
                        _ => {}
                    }
                }

                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(event) = cosmic_key_to_blitz_key_down(key, modified_key, location, modifiers, text, repeat) {
                        let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(UiEvent::KeyDown(event)));
                    }
                }
            }

            Message::KeyReleased { key, modified_key, location, modifiers } => {
                self.keyboard_modifiers = modifiers;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(event) = cosmic_key_to_blitz_key_up(key, modified_key, location, modifiers) {
                        let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(UiEvent::KeyUp(event)));
                    }
                }
            }

            Message::ModifiersChanged(modifiers) => {
                self.keyboard_modifiers = modifiers;
            }


            Message::OpenBookmark(url) => {
                self.navigate_to_url(&url);
                self.url_input = url;
            }

            Message::AddBookmark => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(tab) = self.tab_manager.get_tab(&tab_id) {
                        let title = if tab.title.trim().is_empty() {
                            tab.url.clone()
                        } else {
                            tab.title.clone()
                        };
                        let url = tab.url.clone();
                        let favicon = tab.favicon.clone();
                        if !url.trim().is_empty() {
                            let _ = self.bookmarks.add_bookmark_with_favicon(
                                title, url.clone(), None, favicon.clone(),
                            );
                            self.bookmarks.save_to_disk();
                            if let Some(favicon_bytes) = &favicon {
                                if let Some(bm) = self.bookmarks.find_by_url(&url) {
                                    let bm_id = bm.id.clone();
                                    if let Some(handle) = decode_favicon_to_handle(favicon_bytes) {
                                        self.bookmark_favicon_handles.insert(bm_id, handle);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Message::ToggleSettings => {
                self.settings_open = !self.settings_open;
            }

            Message::SetDefaultBrowser => {
                crate::default_browser::set_as_default_browser();
            }

            Message::BookmarkOpenNewTab(id) => {
                if let Some(node) = self.bookmarks.get(&id) {
                    if let Some(url) = node.url.clone() {
                        self.add_tab_with_url(Some(&url));
                    }
                }
            }

            Message::BookmarkOpenNewWindow(id) => {
                // Multi-window not yet implemented; open in new tab
                if let Some(node) = self.bookmarks.get(&id) {
                    if let Some(url) = node.url.clone() {
                        self.add_tab_with_url(Some(&url));
                    }
                }
            }

            Message::BookmarkEdit(id) => {
                if let Some(node) = self.bookmarks.get(&id) {
                    self.bookmark_edit = Some(BookmarkEditState {
                        id,
                        title: node.title.clone(),
                        url: node.url.clone().unwrap_or_default(),
                        is_folder: node.is_folder(),
                    });
                }
            }

            Message::BookmarkEditTitleChanged(s) => {
                if let Some(edit) = &mut self.bookmark_edit {
                    edit.title = s;
                }
            }

            Message::BookmarkEditUrlChanged(s) => {
                if let Some(edit) = &mut self.bookmark_edit {
                    edit.url = s;
                }
            }

            Message::BookmarkEditCommit => {
                if let Some(edit) = self.bookmark_edit.take() {
                    let _ = self.bookmarks.rename(&edit.id, edit.title);
                    if !edit.is_folder && !edit.url.is_empty() {
                        let _ = self.bookmarks.update_url(&edit.id, edit.url);
                    }
                    self.bookmarks.save_to_disk();
                }
            }

            Message::BookmarkEditCancel => {
                self.bookmark_edit = None;
            }

            Message::BookmarkCut(id) => {
                self.bookmark_clipboard = Some(BookmarkClipboardEntry { id, is_cut: true });
            }

            Message::BookmarkCopy(id) => {
                self.bookmark_clipboard = Some(BookmarkClipboardEntry { id, is_cut: false });
            }

            Message::BookmarkPasteAfter(target_id) => {
                let (entry_id, is_cut) = match &self.bookmark_clipboard {
                    Some(e) => (e.id.clone(), e.is_cut),
                    None => return Task::none(),
                };
                let target_idx = self.bookmarks.items()
                    .iter()
                    .position(|n| n.id == target_id);
                let insert_idx = target_idx.map(|i| i + 1);

                if is_cut {
                    self.bookmark_clipboard = None;
                    let _ = self.bookmarks.move_node(&entry_id, None, insert_idx);
                } else {
                    if let Some(node) = self.bookmarks.get(&entry_id) {
                        let title = node.title.clone();
                        let url_opt = node.url.clone();
                        let favicon_b64 = node.favicon.clone();
                        if let Some(url) = url_opt {
                            let favicon_bytes = favicon_b64.and_then(|b| {
                                base64::engine::general_purpose::STANDARD.decode(&b).ok()
                            });
                            if let Ok(new_id) = self.bookmarks.add_bookmark_with_favicon(
                                title, url, None, favicon_bytes.clone(),
                            ) {
                                let _ = self.bookmarks.move_node(&new_id, None, insert_idx);
                                if let Some(bytes) = favicon_bytes {
                                    if let Some(handle) = decode_favicon_to_handle(&bytes) {
                                        self.bookmark_favicon_handles.insert(new_id, handle);
                                    }
                                }
                            }
                        }
                    }
                    // clipboard kept for copy (not consumed)
                }
                self.bookmarks.save_to_disk();
            }

            Message::BookmarkDelete(id) => {
                let _ = self.bookmarks.delete(&id);
                self.bookmarks.save_to_disk();
                self.bookmark_favicon_handles.remove(&id);
            }

            Message::BookmarkBarMouseMove { x } => {
                self.bookmark_bar_mouse_x = x;
                if let Some(drag) = &mut self.bookmark_drag {
                    drag.current_x = x;
                    if !drag.active && (x - drag.start_x).abs() > 8.0 {
                        drag.active = true;
                    }
                }
            }

            Message::BookmarkBarEntered => {
                self.cursor_over_bar = true;
            }

            Message::BookmarkBarLeft => {
                self.cursor_over_bar = false;
            }

            Message::LeftMousePressed => {
                if self.cursor_over_bar {
                    if let Some(id) = find_bookmark_at_x(self.bookmarks.items(), self.bookmark_bar_mouse_x) {
                        self.bookmark_drag = Some(BookmarkDragState {
                            id,
                            start_x: self.bookmark_bar_mouse_x,
                            current_x: self.bookmark_bar_mouse_x,
                            active: false,
                        });
                    }
                }
            }

            Message::BookmarkMousePressed { id: _ } => {
                // No-op: drag start is handled by LeftMousePressed via global subscription.
                // This fires on ButtonReleased (iced button behavior) — too late for drag.
            }

            Message::BookmarkDragReleased => {
                if let Some(drag) = self.bookmark_drag.take() {
                    if drag.active {
                        let insert_idx = compute_drag_insert_index(
                            self.bookmarks.items(),
                            drag.current_x,
                        );
                        let _ = self.bookmarks.move_node(&drag.id, None, Some(insert_idx));
                        self.bookmarks.save_to_disk();
                    } else {
                        if let Some(node) = self.bookmarks.get(&drag.id) {
                            if let Some(url) = node.url.clone() {
                                self.navigate_to_url(&url);
                                self.url_input = url;
                            }
                        }
                    }
                }
            }
        }

        Task::none()
    }

    fn on_window_resize(&mut self, _id: cosmic::iced::window::Id, width: f32, height: f32) {
        self.sync_scale_factor_from_core();
        self.window_size = (width as u32, height as u32);
        let (physical_width, physical_height) = self.page_size_physical(width, height);

        for tab_id in &self.tab_order.clone() {
            let _ = self.tab_manager.send_to_tab(tab_id, ParentToTabMessage::Resize {
                width: physical_width,
                height: physical_height,
            });
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let tab_bar = self.tab_bar_view();
        let nav_bar = self.nav_bar_view();
        let bookmarks_bar = self.bookmarks_bar_view();
        let page = self.page_content_view();

        widget::column![
            tab_bar,
            nav_bar,
            bookmarks_bar,
            page,
        ]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}
