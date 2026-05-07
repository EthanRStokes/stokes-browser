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
    PageClick,
    PageMouseMove { x: f32, y: f32 },
    PageScroll { delta_x: f32, delta_y: f32 },
    PageButtonReleased,
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
    BookmarkEditFolderSelected(Option<String>),
    BookmarkEditToggleFolder(String),
    BookmarkEditNewFolder,
    BookmarkEditNewFolderNameChanged(String),
    BookmarkEditNewFolderConfirm,

    // Tab drag-and-drop
    TabBarMouseMove { x: f32 },
    TabBarEntered,
    TabBarLeft,

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

impl From<cosmic::iced::mouse::Button> for CosmicMouseButton {
    fn from(button: cosmic::iced::mouse::Button) -> Self {
        match button {
            cosmic::iced::mouse::Button::Left => CosmicMouseButton::Left,
            cosmic::iced::mouse::Button::Right => CosmicMouseButton::Right,
            cosmic::iced::mouse::Button::Middle => CosmicMouseButton::Middle,
            cosmic::iced::mouse::Button::Other(val) => CosmicMouseButton::Other(val),
            _ => CosmicMouseButton::Other(0),
        }
    }
}
