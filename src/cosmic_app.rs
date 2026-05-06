use std::time::Duration;
use cosmic::app::{Core, Task};
use cosmic::iced::widget::image as iced_image;
use cosmic::iced::{Length, Subscription};
use cosmic::iced::Alignment;
use cosmic::widget::{self, mouse_area};
use cosmic::{Application, Element};

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
    current_frame: Option<iced_image::Handle>,
    window_size: (u32, u32),

    spinner_angle: f32,
    settings_open: bool,
    startup_url: Option<String>,
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
    PageClick { x: f32, y: f32 },
    PageMouseMove { x: f32, y: f32 },
    PageScroll { delta_x: f32, delta_y: f32 },
    PageButtonReleased { x: f32, y: f32 },

    // Bookmarks
    OpenBookmark(String),
    AddBookmark,
    ToggleSettings,
    SetDefaultBrowser,
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

    fn add_tab_with_url(&mut self, url: Option<&str>) {
        if let Ok(new_tab_id) = self.tab_manager.create_tab() {
            self.tab_order.push(new_tab_id.clone());
            self.active_tab_index = self.tab_order.len() - 1;

            let (width, height) = self.window_size;
            let chrome_height: u32 = 120; // approximate chrome height in logical pixels
            let page_height = height.saturating_sub(chrome_height);

            let _ = self.tab_manager.send_to_tab(&new_tab_id, ParentToTabMessage::Resize {
                width: width as f32,
                height: page_height as f32,
            });

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
            // Last tab — just close the app (no-op here; cosmic handles window close)
            return;
        }
        if let Some(idx) = self.tab_order.iter().position(|id| id == tab_id) {
            self.tab_order.remove(idx);
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
                TabToParentMessage::NavigationStarted(_) => {}
                TabToParentMessage::NavigationCompleted { url, .. } => {
                    if Some(&tab_id) == self.active_tab_id() {
                        self.url_input = url.clone();
                    }
                    // Update bookmark favicon if available
                    if let Some(tab) = self.tab_manager.get_tab(&tab_id) {
                        if let Some(favicon_bytes) = tab.favicon.clone() {
                            self.bookmarks.set_favicon_for_url(&url, &favicon_bytes);
                        }
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
                                self.current_frame = Some(iced_image::Handle::from_rgba(width, height, pixels));
                            }
                        }
                    }
                }
                TabToParentMessage::FaviconUpdated(Some(bytes)) => {
                    let tab_url = self.tab_manager.get_tab(&tab_id)
                        .map(|t| t.url.clone())
                        .unwrap_or_default();
                    if !tab_url.is_empty() {
                        self.bookmarks.set_favicon_for_url(&tab_url, &bytes);
                    }
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

            let loading_indicator = if is_loading {
                Element::from(widget::text("⟳").size(14))
            } else {
                Element::from(widget::space::horizontal().width(Length::Fixed(14.0)))
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
        let mut row = widget::row::with_capacity(16);

        for node in self.bookmarks.items() {
            if node.is_folder() {
                let folder_btn = widget::button::custom(
                    widget::row![
                        widget::icon::from_name("folder-symbolic").size(14).icon(),
                        widget::text(&node.title).size(13),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center)
                )
                .class(cosmic::theme::Button::Text)
                .padding([2, 6]);
                // Folders shown as plain buttons for now (no popover in this iteration)
                row = row.push(folder_btn);
            } else if let Some(url) = &node.url {
                let bm_btn = widget::button::custom(
                    widget::text(&node.title).size(13)
                )
                .on_press(Message::OpenBookmark(url.clone()))
                .class(cosmic::theme::Button::Text)
                .padding([2, 6]);
                row = row.push(bm_btn);
            }
        }

        Element::from(
            row.spacing(2)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .height(Length::Fixed(32.0))
        )
    }

    fn page_content_view(&self) -> Element<'_, Message> {
        let image_widget: Element<'_, Message> = if let Some(handle) = &self.current_frame {
            Element::from(
                iced_image::Image::new(handle.clone())
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
                .on_press(Message::PageClick { x: 0.0, y: 0.0 })
                .on_release(Message::PageButtonReleased { x: 0.0, y: 0.0 })
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

        let mut app = Self {
            core,
            tab_manager,
            bookmarks,
            url_input: String::new(),
            active_tab_index: 0,
            tab_order: vec![],
            current_frame: None,
            window_size: (1280, 800),
            spinner_angle: 0.0,
            settings_open: false,
            startup_url: flags,
        };

        let startup_url = app.startup_url.clone();
        app.add_tab_with_url(startup_url.as_deref().or(Some(DEFAULT_HOMEPAGE)));
        app.startup_url = None;

        (app, Task::none())
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        cosmic::iced::time::every(Duration::from_millis(16))
            .map(|_| Message::Tick)
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::Tick => {
                self.process_tab_messages();

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

            Message::PageClick { x, y } => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    use keyboard_types::Modifiers;
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
                        mods: Modifiers::empty(),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::PageButtonReleased { x, y } => {
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails};
                    use keyboard_types::Modifiers;
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
                        mods: keyboard_types::Modifiers::empty(),
                        details: PointerDetails::default(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
            }

            Message::PageMouseMove { x, y } => {
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
                if let Some(tab_id) = self.active_tab_id().cloned() {
                    use crate::events::{BlitzWheelDelta, BlitzWheelEvent, MouseEventButtons, PointerCoords};
                    use keyboard_types::Modifiers;
                    let event = UiEvent::Wheel(BlitzWheelEvent {
                        delta: BlitzWheelDelta::Pixels(delta_x as f64, delta_y as f64),
                        coords: PointerCoords {
                            screen_x: 0.0,
                            screen_y: 0.0,
                            client_x: 0.0,
                            client_y: 0.0,
                            page_x: 0.0,
                            page_y: 0.0,
                        },
                        buttons: MouseEventButtons::None,
                        mods: Modifiers::empty(),
                    });
                    let _ = self.tab_manager.send_to_tab(&tab_id, ParentToTabMessage::UI(event));
                }
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
                                title, url, None, favicon,
                            );
                            self.bookmarks.save_to_disk();
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
        }

        Task::none()
    }

    fn on_window_resize(&mut self, _id: cosmic::iced::window::Id, width: f32, height: f32) {
        let chrome_height: f32 = 120.0;
        let page_height = (height - chrome_height).max(0.0);
        self.window_size = (width as u32, height as u32);

        for tab_id in &self.tab_order.clone() {
            let _ = self.tab_manager.send_to_tab(tab_id, ParentToTabMessage::Resize {
                width,
                height: page_height,
            });
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let tab_bar = self.tab_bar_view();
        let nav_bar = self.nav_bar_view();
        let bookmarks_bar = self.bookmarks_bar_view();
        let page = self.page_content_view();

        Element::from(
            widget::column![
                tab_bar,
                nav_bar,
                bookmarks_bar,
                page,
            ]
            .width(Length::Fill)
            .height(Length::Fill)
        )
    }
}
