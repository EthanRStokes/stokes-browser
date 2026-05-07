use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use cosmic::app::{Core, Task};
use cosmic::iced::widget::shader::Shader;
use cosmic::iced::{Length, Subscription};
use cosmic::iced::Alignment;
use cosmic::widget::{self, mouse_area};
use cosmic::{Application, Element};
use base64::Engine as _;

use crate::ui::bookmarks::BookmarkStore;
use crate::ui::libcosmic::views::browser_frame_primitive::{BrowserFramePrimitive, BrowserFrameProgram};
use crate::events::UiEvent;
use crate::ipc::{ParentToTabMessage, TabToParentMessage};
use crate::shell_provider::ShellProviderMessage;
use crate::tab_manager::TabManager;
use crate::ui::libcosmic::messages::{CosmicMouseButton, Message};
use std::collections::HashSet;
use crate::ui::libcosmic::state::{BookmarkClipboardEntry, BookmarkDragState, BookmarkEditState, FolderDropdownState, FolderLevel, PendingFolder, TabDragState};
use crate::ui::libcosmic::views;
use crate::ui::libcosmic::views::bookmarks::{bar_x_of_id, compute_drag_insert_index, find_bookmark_at_x, POPUP_WIDTH, BAR_TOP_OFFSET, FOLDER_ITEM_HEIGHT};
use crate::ui::libcosmic::views::tabs::{compute_tab_width, find_tab_at_x, tab_drag_insert_index};

const DEFAULT_HOMEPAGE: &str = "https://html.duckduckgo.com";

#[cfg(debug_assertions)]
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-dev");
#[cfg(not(debug_assertions))]
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct CosmicBrowserApp {
    pub(crate) core: Core,
    pub(crate) tab_manager: TabManager,
    pub(crate) bookmarks: BookmarkStore,

    pub(crate) url_input: String,
    pub(crate) active_tab_index: usize,
    pub(crate) tab_order: Vec<String>,
    pub(crate) current_frame: Option<BrowserFramePrimitive>,
    pub(crate) current_frame_size: Option<(u32, u32)>,
    pub(crate) window_size: (u32, u32),
    pub(crate) window_scale_factor: f32,

    pub(crate) spinner_angle: f32,
    pub(crate) settings_open: bool,
    pub(crate) startup_url: Option<String>,

    pub(crate) page_mouse_position: (f32, f32),
    pub(crate) keyboard_modifiers: cosmic::iced::keyboard::Modifiers,

    pub(crate) tab_favicon_handles: HashMap<String, widget::image::Handle>,
    pub(crate) bookmark_favicon_handles: HashMap<String, widget::image::Handle>,

    pub(crate) tab_drag: Option<TabDragState>,
    pub(crate) tab_bar_mouse_x: f32,
    pub(crate) cursor_over_tab_bar: bool,

    pub(crate) bookmark_clipboard: Option<BookmarkClipboardEntry>,
    pub(crate) bookmark_drag: Option<BookmarkDragState>,
    pub(crate) bookmark_edit: Option<BookmarkEditState>,
    pub(crate) bookmark_bar_mouse_x: f32,
    pub(crate) cursor_over_bar: bool,
    pub(crate) folder_dropdown: Option<FolderDropdownState>,
}

// --- Key conversion helpers ---

pub(crate) fn cosmic_key_to_kbt_key(key: &cosmic::iced::keyboard::Key) -> keyboard_types::Key {
    match key {
        cosmic::iced::keyboard::Key::Character(ch) => keyboard_types::Key::Character(ch.as_str().into()),
        cosmic::iced::keyboard::Key::Named(named) => match named {
            cosmic::iced::keyboard::key::Named::Enter => keyboard_types::Key::Enter,
            _ => keyboard_types::Key::Unidentified,
        },
        _ => keyboard_types::Key::Unidentified,
    }
}

pub(crate) fn cosmic_location_to_kbt_location(location: cosmic::iced::keyboard::Location) -> keyboard_types::Location {
    match location {
        cosmic::iced::keyboard::Location::Standard => keyboard_types::Location::Standard,
        cosmic::iced::keyboard::Location::Left => keyboard_types::Location::Left,
        cosmic::iced::keyboard::Location::Right => keyboard_types::Location::Right,
        cosmic::iced::keyboard::Location::Numpad => keyboard_types::Location::Numpad,
    }
}

pub(crate) fn cosmic_modifiers_to_kbt_modifiers(modifiers: cosmic::iced::keyboard::Modifiers) -> keyboard_types::Modifiers {
    let mut result = keyboard_types::Modifiers::empty();
    if modifiers.shift()   { result |= keyboard_types::Modifiers::SHIFT; }
    if modifiers.control() { result |= keyboard_types::Modifiers::CONTROL; }
    if modifiers.alt()     { result |= keyboard_types::Modifiers::ALT; }
    if modifiers.logo()    { result |= keyboard_types::Modifiers::META; }
    result
}

pub(crate) fn decode_favicon_to_handle(bytes: &[u8]) -> Option<cosmic::iced::widget::image::Handle> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = rgba.into_raw();
    Some(widget::image::Handle::from_rgba(width, height, pixels))
}

fn build_bookmark_favicon_handles(items: &[crate::ui::bookmarks::BookmarkNode]) -> HashMap<String, cosmic::iced::widget::image::Handle> {
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

// --- CosmicBrowserApp helpers ---

impl CosmicBrowserApp {
    pub(crate) fn active_tab_id(&self) -> Option<&String> {
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

    pub(crate) fn sync_scale_factor_from_core(&mut self) {
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
}

// --- Application impl ---

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
            tab_drag: None,
            tab_bar_mouse_x: 0.0,
            cursor_over_tab_bar: false,

            bookmark_clipboard: None,
            bookmark_drag: None,
            bookmark_edit: None,
            bookmark_bar_mouse_x: 0.0,
            cursor_over_bar: false,
            folder_dropdown: None,
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
                    cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonPressed(
                        cosmic::iced::mouse::Button::Left,
                    )) => Some(Message::LeftMousePressed),
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

                let is_any_loading = self.tab_order.iter().any(|id| {
                    self.tab_manager.get_tab(id).map(|t| t.is_loading).unwrap_or(false)
                });
                if is_any_loading {
                    self.spinner_angle += 0.1;
                    if self.spinner_angle >= std::f32::consts::TAU {
                        self.spinner_angle -= std::f32::consts::TAU;
                    }
                }

                // Subfolder hover timer: open nested popup after 750ms
                let mut open_subfolder: Option<(usize, String, f32, f32)> = None;
                if let Some(dd) = &self.folder_dropdown {
                    for i in 0..dd.levels.len() {
                        if let (Some(subfolder_id), Some(started)) = (
                            dd.levels[i].hovered_subfolder.clone(),
                            dd.levels[i].hover_started,
                        ) {
                            if started.elapsed().as_millis() >= 500 {
                                let parent = &dd.levels[i];
                                let subfolder_idx = self.bookmarks.get(&parent.folder_id)
                                    .map(|f| f.children.iter().position(|c| c.id == subfolder_id).unwrap_or(0))
                                    .unwrap_or(0);
                                let item_y = parent.popup_y + subfolder_idx as f32 * FOLDER_ITEM_HEIGHT;
                                let nested_x = parent.popup_x + POPUP_WIDTH + 4.0;
                                let clamped_x = if nested_x + POPUP_WIDTH > self.window_size.0 as f32 {
                                    parent.popup_x - POPUP_WIDTH - 4.0
                                } else {
                                    nested_x
                                };
                                open_subfolder = Some((i, subfolder_id, clamped_x, item_y));
                                break;
                            }
                        }
                    }
                }
                if let Some((parent_level, subfolder_id, nested_x, nested_y)) = open_subfolder {
                    if let Some(dd) = &mut self.folder_dropdown {
                        dd.levels.truncate(parent_level + 1);
                        dd.levels[parent_level].hover_started = None;
                        dd.levels.push(FolderLevel {
                            folder_id: subfolder_id,
                            popup_x: nested_x,
                            popup_y: nested_y,
                            cursor_y: 0.0,
                            cursor_over: false,
                            hovered_subfolder: None,
                            hover_started: None,
                        });
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
                self.current_frame = None;
            }

            Message::PageClick => {
                let (x, y) = self.page_mouse_position;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    let event = UiEvent::PointerDown(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords { screen_x: x, screen_y: y, client_x: x, client_y: y, page_x: x, page_y: y },
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
                        coords: PointerCoords { screen_x: x, screen_y: y, client_x: x, client_y: y, page_x: x, page_y: y },
                        button: MouseEventButton::Main,
                        buttons: MouseEventButtons::None,
                        mods: cosmic_modifiers_to_kbt_modifiers(self.keyboard_modifiers),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::PageMouseMove { x, y } => {
                self.page_mouse_position = (x, y);
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    use keyboard_types::Modifiers;
                    let event = UiEvent::PointerMove(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords { screen_x: x, screen_y: y, client_x: x, client_y: y, page_x: x, page_y: y },
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
                        coords: PointerCoords { screen_x: x, screen_y: y, client_x: x, client_y: y, page_x: x, page_y: y },
                        buttons: MouseEventButtons::None,
                        mods: Modifiers::empty(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::PagePointerPressed { button } => {
                let (x, y) = self.page_mouse_position;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    let (blitz_button, buttons) = match button {
                        CosmicMouseButton::Left   => (MouseEventButton::Main,      MouseEventButtons::Primary),
                        CosmicMouseButton::Right  => (MouseEventButton::Secondary, MouseEventButtons::Secondary),
                        CosmicMouseButton::Middle => (MouseEventButton::Auxiliary, MouseEventButtons::Auxiliary),
                        CosmicMouseButton::Other(_) => (MouseEventButton::Main,    MouseEventButtons::Primary),
                    };
                    let event = UiEvent::PointerDown(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords { screen_x: x, screen_y: y, client_x: x, client_y: y, page_x: x, page_y: y },
                        button: blitz_button,
                        buttons,
                        mods: cosmic_modifiers_to_kbt_modifiers(self.keyboard_modifiers),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::PagePointerReleased { button } => {
                let (x, y) = self.page_mouse_position;
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    let (blitz_button, buttons) = match button {
                        CosmicMouseButton::Left   => (MouseEventButton::Main,      MouseEventButtons::None),
                        CosmicMouseButton::Right  => (MouseEventButton::Secondary, MouseEventButtons::None),
                        CosmicMouseButton::Middle => (MouseEventButton::Auxiliary, MouseEventButtons::None),
                        CosmicMouseButton::Other(_) => (MouseEventButton::Main,    MouseEventButtons::None),
                    };
                    let event = UiEvent::PointerUp(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords { screen_x: x, screen_y: y, client_x: x, client_y: y, page_x: x, page_y: y },
                        button: blitz_button,
                        buttons,
                        mods: cosmic_modifiers_to_kbt_modifiers(self.keyboard_modifiers),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::KeyPressed { key, modified_key, location, modifiers, text, repeat } => {
                self.keyboard_modifiers = modifiers;

                if modifiers.control() {
                    use cosmic::iced::keyboard::Key;
                    use cosmic::iced::keyboard::key::Named;
                    match &key {
                        Key::Character(ch) if ch.as_str() == "t" => {
                            self.add_tab_with_url(None);
                            return Task::none();
                        }
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
                self.folder_dropdown = None;
            }

            Message::AddBookmark => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    if let Some(tab) = self.tab_manager.get_tab(&tab_id) {
                        let title = if tab.title.trim().is_empty() { tab.url.clone() } else { tab.title.clone() };
                        let url = tab.url.clone();
                        let favicon = tab.favicon.clone();
                        if !url.trim().is_empty() {
                            let _ = self.bookmarks.add_bookmark_with_favicon(title, url.clone(), None, favicon.clone());
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
                if let Some(node) = self.bookmarks.get(&id) {
                    if let Some(url) = node.url.clone() {
                        // Get the current executable path
                        let exe_path = std::env::current_exe().unwrap();

                        let _ = Command::new(exe_path)
                            .arg("--url")
                            .arg(&url)
                            .spawn();
                    }
                }
            }

            Message::BookmarkEdit(id) => {
                if let Some(node) = self.bookmarks.get(&id) {
                    let parent_id = self.bookmarks.parent_folder_id(&id);
                    let mut expanded = HashSet::new();
                    if let Some(ref pid) = parent_id {
                        expanded.insert(pid.clone());
                    }
                    self.bookmark_edit = Some(BookmarkEditState {
                        id,
                        title: node.title.clone(),
                        url: node.url.clone().unwrap_or_default(),
                        is_folder: node.is_folder(),
                        selected_folder_id: parent_id,
                        expanded_folders: expanded,
                        pending_folders: Vec::new(),
                        naming_folder_temp_id: None,
                        next_temp_id: 0,
                    });
                }
            }

            Message::BookmarkEditTitleChanged(s) => {
                if let Some(edit) = &mut self.bookmark_edit { edit.title = s; }
            }

            Message::BookmarkEditUrlChanged(s) => {
                if let Some(edit) = &mut self.bookmark_edit { edit.url = s; }
            }

            Message::BookmarkEditFolderSelected(id) => {
                if let Some(edit) = &mut self.bookmark_edit {
                    if let Some(ref real_id) = id {
                        edit.expanded_folders.insert(real_id.clone());
                    }
                    edit.selected_folder_id = id;
                }
            }

            Message::BookmarkEditToggleFolder(id) => {
                if let Some(edit) = &mut self.bookmark_edit {
                    if edit.expanded_folders.contains(&id) {
                        edit.expanded_folders.remove(&id);
                    } else {
                        edit.expanded_folders.insert(id);
                    }
                }
            }

            Message::BookmarkEditNewFolder => {
                if let Some(edit) = &mut self.bookmark_edit {
                    let temp_id = format!("pending_{}", edit.next_temp_id);
                    edit.next_temp_id += 1;
                    if let Some(ref pid) = edit.selected_folder_id.clone() {
                        edit.expanded_folders.insert(pid.clone());
                    }
                    edit.pending_folders.push(PendingFolder {
                        temp_id: temp_id.clone(),
                        parent_id: edit.selected_folder_id.clone(),
                        name: String::new(),
                    });
                    edit.naming_folder_temp_id = Some(temp_id);
                }
            }

            Message::BookmarkEditNewFolderNameChanged(s) => {
                if let Some(edit) = &mut self.bookmark_edit {
                    if let Some(tid) = edit.naming_folder_temp_id.clone() {
                        if let Some(pf) = edit.pending_folders.iter_mut().find(|p| p.temp_id == tid) {
                            pf.name = s;
                        }
                    }
                }
            }

            Message::BookmarkEditNewFolderConfirm => {
                if let Some(edit) = &mut self.bookmark_edit {
                    edit.naming_folder_temp_id = None;
                }
            }

            Message::BookmarkEditCommit => {
                if let Some(edit) = self.bookmark_edit.take() {
                    let _ = self.bookmarks.rename(&edit.id, edit.title);
                    if !edit.is_folder && !edit.url.is_empty() {
                        let _ = self.bookmarks.update_url(&edit.id, edit.url);
                    }
                    // Create pending folders in order, resolving temp_id → real_id
                    let mut temp_to_real: HashMap<String, String> = HashMap::new();
                    for pf in &edit.pending_folders {
                        if pf.name.is_empty() { continue; }
                        let resolved_parent: Option<String> = pf.parent_id.as_ref().map(|p| {
                            temp_to_real.get(p).cloned().unwrap_or_else(|| p.clone())
                        });
                        if let Ok(real_id) = self.bookmarks.add_folder(
                            pf.name.clone(),
                            resolved_parent.as_deref(),
                        ) {
                            temp_to_real.insert(pf.temp_id.clone(), real_id);
                        }
                    }
                    // Move bookmark to selected folder (resolving temp_id if needed)
                    let resolved_target: Option<String> = edit.selected_folder_id.as_ref().map(|id| {
                        temp_to_real.get(id).cloned().unwrap_or_else(|| id.clone())
                    });
                    let _ = self.bookmarks.move_node(&edit.id, resolved_target.as_deref(), None);
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
                let target_idx = self.bookmarks.items().iter().position(|n| n.id == target_id);
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
                }
                self.bookmarks.save_to_disk();
            }

            Message::BookmarkDelete(id) => {
                let _ = self.bookmarks.delete(&id);
                self.bookmarks.save_to_disk();
                self.bookmark_favicon_handles.remove(&id);
            }

            Message::TabBarMouseMove { x } => {
                self.tab_bar_mouse_x = x;
                if let Some(drag) = &mut self.tab_drag {
                    drag.current_x = x;
                    if !drag.active && (x - drag.start_x).abs() > 8.0 {
                        drag.active = true;
                    }
                }
            }

            Message::TabBarEntered => {
                self.cursor_over_tab_bar = true;
            }

            Message::TabBarLeft => {
                self.cursor_over_tab_bar = false;
            }

            Message::BookmarkBarMouseMove { x } => {
                self.bookmark_bar_mouse_x = x;
                if let Some(drag) = &mut self.bookmark_drag {
                    drag.current_x = x;
                    drag.over_folder_level = None;
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
                if self.cursor_over_tab_bar {
                    let tw = compute_tab_width(self.window_size.0 as f32, self.tab_order.len());
                    if let Some(idx) = find_tab_at_x(self.tab_order.len(), tw, self.tab_bar_mouse_x) {
                        self.tab_drag = Some(TabDragState {
                            index: idx,
                            start_x: self.tab_bar_mouse_x,
                            current_x: self.tab_bar_mouse_x,
                            active: false,
                        });
                    }
                }
                if let Some(dd) = &self.folder_dropdown {
                    let mut drag_init: Option<BookmarkDragState> = None;
                    for (i, level) in dd.levels.iter().enumerate() {
                        if level.cursor_over {
                            let item_idx = (level.cursor_y / FOLDER_ITEM_HEIGHT) as usize;
                            if let Some(folder) = self.bookmarks.get(&level.folder_id) {
                                if let Some(child) = folder.children.get(item_idx) {
                                    drag_init = Some(BookmarkDragState {
                                        id: child.id.clone(),
                                        source_folder_id: Some(level.folder_id.clone()),
                                        start_x: self.bookmark_bar_mouse_x,
                                        start_y: level.cursor_y,
                                        current_x: self.bookmark_bar_mouse_x,
                                        current_y: level.cursor_y,
                                        active: false,
                                        over_folder_level: Some(i),
                                    });
                                }
                            }
                            break;
                        }
                    }
                    if let Some(d) = drag_init {
                        self.bookmark_drag = Some(d);
                    }
                }
            }

            Message::BookmarkMousePressed { id } => {
                self.bookmark_drag = Some(BookmarkDragState {
                    id,
                    source_folder_id: None,
                    start_x: self.bookmark_bar_mouse_x,
                    start_y: 0.0,
                    current_x: self.bookmark_bar_mouse_x,
                    current_y: 0.0,
                    active: false,
                    over_folder_level: None,
                });
            }

            Message::BookmarkFolderOpen { id, bar_x } => {
                let clamped_x = bar_x.min(self.window_size.0 as f32 - POPUP_WIDTH - 8.0).max(0.0);
                self.folder_dropdown = Some(FolderDropdownState {
                    levels: vec![FolderLevel {
                        folder_id: id,
                        popup_x: clamped_x,
                        popup_y: BAR_TOP_OFFSET,
                        cursor_y: 0.0,
                        cursor_over: false,
                        hovered_subfolder: None,
                        hover_started: None,
                    }],
                });
            }

            Message::BookmarkFolderClose => {
                self.folder_dropdown = None;
            }

            Message::FolderLevelMouseMove { level, x: _, y } => {
                if let Some(dd) = &mut self.folder_dropdown {
                    if let Some(lv) = dd.levels.get_mut(level) {
                        lv.cursor_y = y;
                    }
                }
                if let Some(drag) = &mut self.bookmark_drag {
                    drag.over_folder_level = Some(level);
                    drag.current_y = y;
                    if !drag.active && (y - drag.start_y).abs() > 8.0 {
                        drag.active = true;
                    }
                }
            }

            Message::FolderLevelEntered(level) => {
                if let Some(dd) = &mut self.folder_dropdown {
                    if let Some(lv) = dd.levels.get_mut(level) {
                        lv.cursor_over = true;
                    }
                }
                if let Some(drag) = &mut self.bookmark_drag {
                    drag.over_folder_level = Some(level);
                }
            }

            Message::FolderLevelLeft(level) => {
                if let Some(dd) = &mut self.folder_dropdown {
                    if let Some(lv) = dd.levels.get_mut(level) {
                        lv.cursor_over = false;
                        lv.hovered_subfolder = None;
                        lv.hover_started = None;
                    }
                    // Pop any deeper levels when leaving a parent level
                    if dd.levels.len() > level + 1 {
                        dd.levels.truncate(level + 1);
                    }
                }
            }

            Message::FolderSubfolderHovered { level, subfolder_id } => {
                if let Some(dd) = &mut self.folder_dropdown {
                    if let Some(lv) = dd.levels.get_mut(level) {
                        lv.hovered_subfolder = Some(subfolder_id);
                        lv.hover_started = Some(std::time::Instant::now());
                    }
                }
            }

            Message::FolderSubfolderLeft(level) => {
                if let Some(dd) = &mut self.folder_dropdown {
                    if let Some(lv) = dd.levels.get_mut(level) {
                        lv.hovered_subfolder = None;
                        lv.hover_started = None;
                    }
                }
            }

            Message::BookmarkDragReleased => {
                if let Some(drag) = self.tab_drag.take() {
                    if drag.active {
                        let tw = compute_tab_width(self.window_size.0 as f32, self.tab_order.len());
                        let insert_idx = tab_drag_insert_index(self.tab_order.len(), tw, drag.current_x);
                        let from = drag.index;
                        let to = if insert_idx > from { insert_idx - 1 } else { insert_idx };
                        if from != to {
                            let tab_id = self.tab_order.remove(from);
                            self.tab_order.insert(to, tab_id);
                            if self.active_tab_index == from {
                                self.active_tab_index = to;
                            } else if from < self.active_tab_index && to >= self.active_tab_index {
                                self.active_tab_index -= 1;
                            } else if from > self.active_tab_index && to <= self.active_tab_index {
                                self.active_tab_index += 1;
                            }
                        }
                    }
                }

                if let Some(drag) = self.bookmark_drag.take() {
                    if drag.active {
                        if let Some(level_idx) = drag.over_folder_level {
                            // Drop into an open folder popup
                            let (target_folder_id, insert_idx) = {
                                let dd = self.folder_dropdown.as_ref();
                                let level = dd.and_then(|d| d.levels.get(level_idx));
                                let folder_id = level.map(|l| l.folder_id.clone());
                                let cursor_y = level.map(|l| l.cursor_y).unwrap_or(0.0);
                                let idx = (cursor_y / FOLDER_ITEM_HEIGHT) as usize;
                                (folder_id, idx)
                            };
                            if let Some(fid) = target_folder_id {
                                let _ = self.bookmarks.move_node(&drag.id, Some(&fid), Some(insert_idx));
                                self.bookmarks.save_to_disk();
                            }
                        } else {
                            // Drop on root bar — check if dropped onto a folder button
                            let insert_idx = compute_drag_insert_index(self.bookmarks.items(), drag.current_x);
                            let drop_target_id = find_bookmark_at_x(self.bookmarks.items(), drag.current_x);
                            let is_drop_on_folder = drop_target_id.as_deref()
                                .and_then(|tid| self.bookmarks.get(tid))
                                .map(|n| n.is_folder())
                                .unwrap_or(false);
                            if is_drop_on_folder {
                                if let Some(folder_id) = drop_target_id {
                                    if folder_id != drag.id {
                                        let _ = self.bookmarks.move_node(&drag.id, Some(&folder_id), None);
                                        self.bookmarks.save_to_disk();
                                    }
                                }
                            } else {
                                let _ = self.bookmarks.move_node(&drag.id, None, Some(insert_idx));
                                self.bookmarks.save_to_disk();
                            }
                        }
                    } else if drag.source_folder_id.is_none() {
                        // Short click from root bar only — popup items handle their own navigation
                        if let Some(node) = self.bookmarks.get(&drag.id).cloned() {
                            if let Some(url) = node.url {
                                self.navigate_to_url(&url);
                                self.url_input = url;
                                self.folder_dropdown = None;
                            } else {
                                // Folder click — toggle dropdown
                                let already_open = self.folder_dropdown.as_ref()
                                    .and_then(|d| d.levels.first())
                                    .map(|l| l.folder_id == drag.id)
                                    .unwrap_or(false);
                                if already_open {
                                    self.folder_dropdown = None;
                                } else {
                                    let bx = bar_x_of_id(self.bookmarks.items(), &drag.id);
                                    let clamped_x = bx.min(self.window_size.0 as f32 - POPUP_WIDTH - 8.0).max(0.0);
                                    self.folder_dropdown = Some(FolderDropdownState {
                                        levels: vec![FolderLevel {
                                            folder_id: drag.id,
                                            popup_x: clamped_x,
                                            popup_y: BAR_TOP_OFFSET,
                                            cursor_y: 0.0,
                                            cursor_over: false,
                                            hovered_subfolder: None,
                                            hover_started: None,
                                        }],
                                    });
                                }
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
        let base = widget::column![
            views::tabs::tab_bar_view(self),
            views::nav::nav_bar_view(self),
            views::bookmarks::bookmarks_bar_view(self),
            views::page::page_content_view(self),
        ]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill);

        let mut layers: Vec<Element<'_, Message>> = vec![base.into()];
        if self.folder_dropdown.is_some() {
            layers.push(views::bookmarks::folder_dropdown_overlay_view(self));
        }
        if self.bookmark_edit.is_some() {
            layers.push(views::bookmarks::bookmark_edit_dialog_view(self));
        }
        if layers.len() == 1 {
            layers.remove(0)
        } else {
            cosmic::iced::widget::stack(layers).into()
        }
    }
}
