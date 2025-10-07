// CSS cursor property

/// CSS cursor property
#[derive(Debug, Clone, PartialEq)]
pub enum Cursor {
    Auto,
    Default,
    Pointer,
    Text,
    Move,
    Wait,
    Help,
    NotAllowed,
    Crosshair,
    Grab,
    Grabbing,
    EResize,
    WResize,
    NResize,
    SResize,
    NEResize,
    NWResize,
    SEResize,
    SWResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
    Copy,
    Alias,
    ContextMenu,
    NoDrop,
    Progress,
    Cell,
    VerticalText,
}

impl Cursor {
    /// Parse cursor value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "auto" => Cursor::Auto,
            "default" => Cursor::Default,
            "pointer" => Cursor::Pointer,
            "text" => Cursor::Text,
            "move" => Cursor::Move,
            "wait" => Cursor::Wait,
            "help" => Cursor::Help,
            "not-allowed" => Cursor::NotAllowed,
            "crosshair" => Cursor::Crosshair,
            "grab" => Cursor::Grab,
            "grabbing" => Cursor::Grabbing,
            "e-resize" => Cursor::EResize,
            "w-resize" => Cursor::WResize,
            "n-resize" => Cursor::NResize,
            "s-resize" => Cursor::SResize,
            "ne-resize" => Cursor::NEResize,
            "nw-resize" => Cursor::NWResize,
            "se-resize" => Cursor::SEResize,
            "sw-resize" => Cursor::SWResize,
            "col-resize" => Cursor::ColResize,
            "row-resize" => Cursor::RowResize,
            "all-scroll" => Cursor::AllScroll,
            "zoom-in" => Cursor::ZoomIn,
            "zoom-out" => Cursor::ZoomOut,
            "copy" => Cursor::Copy,
            "alias" => Cursor::Alias,
            "context-menu" => Cursor::ContextMenu,
            "no-drop" => Cursor::NoDrop,
            "progress" => Cursor::Progress,
            "cell" => Cursor::Cell,
            "vertical-text" => Cursor::VerticalText,
            _ => Cursor::Auto, // Default to auto
        }
    }

    /// Convert to winit CursorIcon
    pub fn to_winit_cursor(&self) -> winit::window::CursorIcon {
        match self {
            Cursor::Auto => winit::window::CursorIcon::Default,
            Cursor::Default => winit::window::CursorIcon::Default,
            Cursor::Pointer => winit::window::CursorIcon::Pointer,
            Cursor::Text => winit::window::CursorIcon::Text,
            Cursor::Move => winit::window::CursorIcon::Move,
            Cursor::Wait => winit::window::CursorIcon::Wait,
            Cursor::Help => winit::window::CursorIcon::Help,
            Cursor::NotAllowed => winit::window::CursorIcon::NotAllowed,
            Cursor::Crosshair => winit::window::CursorIcon::Crosshair,
            Cursor::Grab => winit::window::CursorIcon::Grab,
            Cursor::Grabbing => winit::window::CursorIcon::Grabbing,
            Cursor::EResize => winit::window::CursorIcon::EResize,
            Cursor::WResize => winit::window::CursorIcon::WResize,
            Cursor::NResize => winit::window::CursorIcon::NResize,
            Cursor::SResize => winit::window::CursorIcon::SResize,
            Cursor::NEResize => winit::window::CursorIcon::NeResize,
            Cursor::NWResize => winit::window::CursorIcon::NwResize,
            Cursor::SEResize => winit::window::CursorIcon::SeResize,
            Cursor::SWResize => winit::window::CursorIcon::SwResize,
            Cursor::ColResize => winit::window::CursorIcon::ColResize,
            Cursor::RowResize => winit::window::CursorIcon::RowResize,
            Cursor::AllScroll => winit::window::CursorIcon::AllScroll,
            Cursor::ZoomIn => winit::window::CursorIcon::ZoomIn,
            Cursor::ZoomOut => winit::window::CursorIcon::ZoomOut,
            Cursor::Copy => winit::window::CursorIcon::Copy,
            Cursor::Alias => winit::window::CursorIcon::Alias,
            Cursor::ContextMenu => winit::window::CursorIcon::ContextMenu,
            Cursor::NoDrop => winit::window::CursorIcon::NoDrop,
            Cursor::Progress => winit::window::CursorIcon::Progress,
            Cursor::Cell => winit::window::CursorIcon::Cell,
            Cursor::VerticalText => winit::window::CursorIcon::VerticalText,
        }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor::Auto
    }
}

