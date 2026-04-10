use crate::renderer::painter::ScenePainter;
use crate::bookmarks::BookmarkNode;
use anyrender::PaintScene;
use base64::Engine;
use blitz_traits::shell::Viewport;
use color::{AlphaColor, Srgb};
use kurbo::Affine;
use parley::{Alignment, AlignmentOptions, FontContext, GenericFamily, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty};
use peniko::Fill;
use skia_safe::{Canvas, Color, Data, Font, FontStyle, Image, Paint, Rect, TextBlob};
use std::collections::HashMap;
use std::f32::consts::PI;
use std::time::{Duration, Instant};
use usvg::Tree;
use crate::browser::VERSION;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextBrush {
    pub id: usize
}

impl TextBrush {
    pub(crate) fn from_id(id: usize) -> Self {
        Self { id }
    }
}

/// Tooltip information
#[derive(Debug, Clone)]
pub struct Tooltip {
    pub text: String,
    pub show_after: Duration,
    pub hover_start: Option<Instant>,
    pub is_visible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BookmarkUiAction {
    Navigate(String),
    AddPage { parent_id: Option<String> },
    AddFolder { parent_id: Option<String> },
    Rename(String),
    EditUrl(String),
    Delete(String),
    Move { id: String, parent_id: Option<String>, index: Option<usize> },
    ToggleCurrentPageBookmark,
    UiChanged,
}

#[derive(Debug, Clone)]
struct BookmarkContextMenuState {
    x: f32,
    y: f32,
    target_id: Option<String>,
    parent_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct BookmarkDragState {
    active: bool,
    dragged_id: Option<String>,
    dragged_parent_id: Option<String>,
    drag_start_x: f32,
    drag_start_y: f32,
    over_id: Option<String>,
    drop_parent_id: Option<String>,
    drop_index: Option<usize>,
}

impl Tooltip {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            show_after: Duration::from_millis(500), // Show after 500ms
            hover_start: None,
            is_visible: false,
        }
    }
}

/// Represents a UI component in the browser chrome
#[derive(Debug, Clone)]
pub enum UiComponent {
    Button {
        id: String,
        label: String,
        x: f32,  // Absolute pixel position
        y: f32,
        width: f32,  // Fixed pixel width
        height: f32,  // Fixed pixel height
        color: [f32; 3],
        hover_color: [f32; 3],
        pressed_color: [f32; 3],
        is_hover: bool,
        is_pressed: bool,
        is_active: bool,
        tooltip: Tooltip,
        icon_type: IconType,
    },
    TextField {
        id: String,
        text: String,
        x: f32,  // Absolute pixel position
        y: f32,
        width: f32,  // Can be adjusted based on window size
        height: f32,  // Fixed pixel height
        color: [f32; 3],
        border_color: [f32; 3],
        has_focus: bool,
        cursor_position: usize,
        selection_start: Option<usize>,
        selection_end: Option<usize>,
        is_flexible: bool,  // Whether width adjusts to available space
    },
    TabButton {
        id: String,
        title: String,
        x: f32,  // Absolute pixel position
        y: f32,
        width: f32,  // Fixed pixel width
        height: f32,  // Fixed pixel height
        color: [f32; 3],
        hover_color: [f32; 3],
        is_active: bool,
        is_hover: bool,
        tooltip: Tooltip,
        close_button_hover: bool,
        close_button_tooltip: Tooltip,
        favicon: Option<Image>,
        is_loading: bool,
    }
}

/// Icon types for buttons
#[derive(Debug, Clone)]
pub enum IconType {
    Back,
    Forward,
    Refresh,
    Home,
    Bookmark,
    NewTab,
    Close,
    Settings,
}

impl UiComponent {
    /// Create a navigation button (back, forward, refresh)
    pub fn navigation_button(id: &str, label: &str, x: f32, icon_type: IconType, tooltip_text: &str, scale_factor: f32) -> Self {
        let scaled = |v: f32| v * scale_factor;
        UiComponent::Button {
            id: id.to_string(),
            label: label.to_string(),
            x,
            y: scaled(48.0),  // Move to second row
            width: scaled(32.0),
            height: scaled(32.0),
            color: [0.95, 0.95, 0.95],
            hover_color: [0.85, 0.9, 1.0],
            pressed_color: [0.75, 0.8, 0.95],
            is_hover: false,
            is_pressed: false,
            is_active: false,
            tooltip: Tooltip::new(tooltip_text),
            icon_type,
        }
    }

    /// Create an address bar
    pub fn address_bar(url: &str, x: f32, width: f32, scale_factor: f32) -> Self {
        let scaled = |v: f32| v * scale_factor;
        UiComponent::TextField {
            id: "address_bar".to_string(),
            text: url.to_string(),
            x,
            y: scaled(48.0),  // Move to second row
            width,
            height: scaled(32.0),
            color: [1.0, 1.0, 1.0],
            border_color: [0.7, 0.7, 0.7],
            has_focus: false,
            cursor_position: 0,
            selection_start: None,
            selection_end: None,
            is_flexible: true,
        }
    }

    /// Create a tab button
    pub fn tab(id: &str, title: &str, x: f32, scale_factor: f32) -> Self {
        let scaled = |v: f32| v * scale_factor;
        UiComponent::TabButton {
            id: id.to_string(),
            title: title.to_string(),
            x,
            y: scaled(8.0),  // Move to first row
            width: scaled(150.0),
            height: scaled(32.0),
            color: if title == "New Tab" { [0.95, 0.95, 0.95] } else { [0.8, 0.8, 0.8] },
            hover_color: [0.85, 0.9, 1.0],
            is_active: title == "New Tab",
            is_hover: false,
            tooltip: Tooltip::new(&format_tab_tooltip_text(title)),
            close_button_hover: false,
            close_button_tooltip: Tooltip::new("Close tab"),
            favicon: None,
            is_loading: false,
        }
    }

    /// Check if a point is inside this component
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        match self {
            UiComponent::Button { x: bx, y: by, width, height, .. } |
            UiComponent::TabButton { x: bx, y: by, width, height, .. } => {
                x >= *bx && x <= *bx + *width && y >= *by && y <= *by + *height
            }
            UiComponent::TextField { x: bx, y: by, width, height, .. } => {
                x >= *bx && x <= *bx + *width && y >= *by && y <= *by + *height
            }
        }
    }

    /// Get component ID
    pub fn id(&self) -> &str {
        match self {
            UiComponent::Button { id, .. } |
            UiComponent::TextField { id, .. } |
            UiComponent::TabButton { id, .. } => id,
        }
    }

    /// Get component width
    fn width(&self) -> f32 {
        match self {
            UiComponent::Button { width, .. } |
            UiComponent::TextField { width, .. } |
            UiComponent::TabButton { width, .. } => *width,
        }
    }
}

/// Load and parse an SVG file
fn load_svg(svg_data: &str) -> Option<Tree> {
    let options = usvg::Options::default();
    Tree::from_str(svg_data, &options).ok()
}

fn format_tab_tooltip_text(title: &str) -> String {
    let normalized_title = title.split_whitespace().collect::<Vec<_>>().join(" ");

    if normalized_title.is_empty() {
        "Switch to tab".to_string()
    } else {
        format!("Switch to\n{}", normalized_title)
    }
}

/// State for tab dragging
#[derive(Debug, Clone, Default)]
pub struct TabDragState {
    /// Whether a tab is currently being dragged
    pub is_dragging: bool,
    /// The ID of the tab being dragged
    pub dragged_tab_id: Option<String>,
    /// The starting X position of the drag
    pub drag_start_x: f32,
    /// The original X position of the tab when drag started
    pub original_tab_x: f32,
    /// The current drag offset
    pub drag_offset: f32,
    /// The original index of the dragged tab
    pub original_index: usize,
    /// Whether the drag threshold has been exceeded (to distinguish from click)
    pub drag_threshold_exceeded: bool,
}

/// Represents the browser UI (chrome)
pub struct BrowserUI {
    pub components: Vec<UiComponent>,
    pub viewport: Viewport,
    tab_scroll_offset: f32,  // Horizontal scroll offset for tabs
    /// State for tab dragging
    pub tab_drag_state: TabDragState,
    // Preloaded SVGs for icons
    pub back_svg: Tree,
    pub forward_svg: Tree,
    pub reload_svg: Tree,
    pub home_svg: Tree,
    pub new_tab_svg: Tree,
    pub close_tab_svg: Tree,
    pub settings_svg: Tree,
    pub folder_svg: Tree,
    /// Whether the settings panel is open
    pub show_settings: bool,
    /// Whether we are currently dragging a text selection in a chrome text field.
    text_selection_drag_active: bool,
    /// Anchor byte-position used while extending selection during a drag.
    text_selection_drag_anchor: Option<usize>,
    /// Cached typeface for UI rendering so we don't recreate FontMgr every frame
    pub ui_typeface: skia_safe::Typeface,
    bookmarks: Vec<BookmarkNode>,
    bookmark_favicons: HashMap<String, Option<Image>>,
    open_bookmark_folder: Option<String>,
    selected_bookmark_id: Option<String>,
    bookmark_context_menu: Option<BookmarkContextMenuState>,
    bookmark_drag: BookmarkDragState,
    bookmark_button_active: bool,
    bookmark_hover_id: Option<String>,
    bookmark_pressed_id: Option<String>,
    mouse_pos: (f32, f32),
}

impl BrowserUI {
    // UI layout constants
    pub const CHROME_HEIGHT: f32 = 124.0;
    const BUTTON_SIZE: f32 = 32.0;
    const BUTTON_MARGIN: f32 = 8.0;
    const ADDRESS_BAR_HEIGHT: f32 = 32.0;
    const ADDRESS_BAR_MARGIN: f32 = 8.0;
    const MIN_ADDRESS_BAR_WIDTH: f32 = 200.0;
    const MAX_TAB_WIDTH: f32 = 200.0;  // Maximum width for a tab
    const MIN_TAB_WIDTH: f32 = 80.0;   // Minimum width before scrolling kicks in
    const TAB_SPACING: f32 = 4.0;       // Spacing between tabs
    const BOOKMARKS_ROW_Y: f32 = 88.0;
    const BOOKMARKS_ROW_HEIGHT: f32 = 32.0;
    const BOOKMARK_ITEM_WIDTH: f32 = 150.0;
    const BOOKMARK_ITEM_SPACING: f32 = 6.0;
    const BOOKMARK_CONTEXT_ROW_HEIGHT: f32 = 28.0;
    const BOOKMARK_CONTEXT_WIDTH: f32 = 190.0;

    pub fn new(_skia_context: &skia_safe::gpu::DirectContext, viewport: &Viewport) -> Self {
        // Default window width, will be updated on first resize
        let window_width = viewport.window_size.0 as f32;
        let scale_factor = viewport.hidpi_scale;
        let scaled = |v: f32| v * scale_factor;

        let font_mgr = skia_safe::FontMgr::new();
        let ui_typeface = font_mgr.match_family_style("DejaVu Sans", FontStyle::default())
            .or_else(|| font_mgr.match_family_style("Noto Sans", FontStyle::default()))
            .or_else(|| font_mgr.match_family_style("Arial Unicode MS", FontStyle::default()))
            .or_else(|| font_mgr.match_family_style("Segoe UI Symbol", FontStyle::default()))
            .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::default()))
            .unwrap_or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::default()).unwrap());

        Self {
            components: vec![
                UiComponent::navigation_button("back", "<", scaled(Self::BUTTON_MARGIN), IconType::Back, "Back", scale_factor),
                UiComponent::navigation_button("forward", ">", scaled(Self::BUTTON_MARGIN * 2.0 + Self::BUTTON_SIZE), IconType::Forward, "Forward", scale_factor),
                UiComponent::navigation_button("refresh", "⟳", scaled(Self::BUTTON_MARGIN * 3.0 + Self::BUTTON_SIZE * 2.0), IconType::Refresh, "Refresh", scale_factor),
                UiComponent::navigation_button("home", "H", scaled(Self::BUTTON_MARGIN * 4.0 + Self::BUTTON_SIZE * 3.0), IconType::Home, "Home", scale_factor),
                UiComponent::address_bar("",
                    scaled(Self::BUTTON_MARGIN * 5.0 + Self::BUTTON_SIZE * 4.0),
                    window_width - scaled(Self::BUTTON_MARGIN * 8.0 + Self::BUTTON_SIZE * 6.0), scale_factor),
                UiComponent::Button {
                    id: "bookmark_toggle".to_string(),
                    label: "*".to_string(),
                    x: window_width - scaled(Self::BUTTON_MARGIN * 2.0 + Self::BUTTON_SIZE * 2.0),
                    y: scaled(48.0),
                    width: scaled(Self::BUTTON_SIZE),
                    height: scaled(Self::BUTTON_SIZE),
                    color: [0.95, 0.95, 0.95],
                    hover_color: [0.97, 0.9, 0.65],
                    pressed_color: [0.95, 0.84, 0.45],
                    is_hover: false,
                    is_pressed: false,
                    is_active: false,
                    tooltip: Tooltip::new("Bookmark page"),
                    icon_type: IconType::Bookmark,
                },
                // Settings button - positioned to the right of the address bar
                UiComponent::Button {
                    id: "settings".to_string(),
                    label: "⚙".to_string(),
                    x: window_width - scaled(Self::BUTTON_MARGIN + Self::BUTTON_SIZE),
                    y: scaled(48.0),
                    width: scaled(Self::BUTTON_SIZE),
                    height: scaled(Self::BUTTON_SIZE),
                    color: [0.95, 0.95, 0.95],
                    hover_color: [0.85, 0.9, 1.0],
                    pressed_color: [0.75, 0.8, 0.95],
                    is_hover: false,
                    is_pressed: false,
                    is_active: false,
                    tooltip: Tooltip::new("Settings"),
                    icon_type: IconType::Settings,
                },
                // New Tab button - positioned in the tab row, will be updated in update_tab_layout
                UiComponent::Button {
                    id: "new_tab".to_string(),
                    label: "+".to_string(),
                    x: scaled(Self::BUTTON_MARGIN),
                    y: scaled(8.0),  // Tab row
                    width: scaled(Self::BUTTON_SIZE),
                    height: scaled(Self::BUTTON_SIZE),
                    color: [0.95, 0.95, 0.95],
                    hover_color: [0.85, 0.9, 1.0],
                    pressed_color: [0.75, 0.8, 0.95],
                    is_hover: false,
                    is_pressed: false,
                    is_active: false,
                    tooltip: Tooltip::new("New Tab"),
                    icon_type: IconType::NewTab,
                },
            ],
            viewport: viewport.clone(),
            tab_scroll_offset: 0.0,
            tab_drag_state: TabDragState::default(),
            back_svg: load_svg(include_str!("../assets/left_arrow.svg")).unwrap(),
            forward_svg: load_svg(include_str!("../assets/right_arrow.svg")).unwrap(),
            reload_svg: load_svg(include_str!("../assets/reload.svg")).unwrap(),
            home_svg: load_svg(include_str!("../assets/home.svg")).unwrap(),
            new_tab_svg: load_svg(include_str!("../assets/plus.svg")).unwrap(),
            close_tab_svg: load_svg(include_str!("../assets/close.svg")).unwrap(),
            settings_svg: load_svg(include_str!("../assets/settings.svg")).unwrap(),
            folder_svg: load_svg(include_str!("../assets/folder.svg")).unwrap(),
            show_settings: false,
            text_selection_drag_active: false,
            text_selection_drag_anchor: None,
            ui_typeface,
            bookmarks: Vec::new(),
            bookmark_favicons: HashMap::new(),
            open_bookmark_folder: None,
            selected_bookmark_id: None,
            bookmark_context_menu: None,
            bookmark_drag: BookmarkDragState::default(),
            bookmark_button_active: false,
            bookmark_hover_id: None,
            bookmark_pressed_id: None,
            mouse_pos: (0.0, 0.0),
        }
    }

    pub fn tab_row_height(&self) -> f32 {
        48.0 * self.viewport.hidpi_scale
    }

    pub fn set_bookmarks(&mut self, bookmarks: Vec<BookmarkNode>) {
        self.bookmarks = bookmarks;
        self.bookmark_favicons.clear();
        Self::cache_bookmark_favicons(&self.bookmarks, &mut self.bookmark_favicons);

        if let Some(selected) = self.selected_bookmark_id.clone() {
            if Self::find_bookmark(&self.bookmarks, &selected).is_none() {
                self.selected_bookmark_id = None;
            }
        }

        if let Some(open_folder) = self.open_bookmark_folder.clone() {
            if Self::find_bookmark(&self.bookmarks, &open_folder)
                .is_none_or(|bookmark| !bookmark.is_folder())
            {
                self.open_bookmark_folder = None;
            }
        }

        if let Some(menu) = self.bookmark_context_menu.as_ref() {
            if let Some(target_id) = menu.target_id.as_deref() {
                if Self::find_bookmark(&self.bookmarks, target_id).is_none() {
                    self.bookmark_context_menu = None;
                }
            }
        }
    }

    pub fn set_current_page_bookmarked(&mut self, is_bookmarked: bool) {
        self.bookmark_button_active = is_bookmarked;
        for comp in &mut self.components {
            if let UiComponent::Button { id, is_active, tooltip, .. } = comp {
                if id == "bookmark_toggle" {
                    *is_active = is_bookmarked;
                    tooltip.text = if is_bookmarked {
                        "Edit bookmark".to_string()
                    } else {
                        "Bookmark page".to_string()
                    };
                }
            }
        }
    }

    pub fn selected_bookmark_id(&self) -> Option<&str> {
        self.selected_bookmark_id.as_deref()
    }

    pub fn selected_bookmark_is_folder(&self) -> bool {
        self.selected_bookmark_id
            .as_ref()
            .and_then(|id| Self::find_bookmark(&self.bookmarks, id))
            .is_some_and(|bookmark| bookmark.is_folder())
    }

    pub fn handle_bookmark_click(&mut self, x: f32, y: f32) -> Option<BookmarkUiAction> {
        self.bookmark_pressed_id = None;
        if let Some(action) = self.handle_bookmark_context_menu_click(x, y) {
            self.bookmark_context_menu = None;
            return Some(action);
        }

        self.bookmark_context_menu = None;
        let (row_x, row_y, row_w, row_h) = self.bookmark_row_rect();

        if let Some(folder_id) = self.open_bookmark_folder.clone() {
            if let Some(folder) = Self::find_bookmark(&self.bookmarks, &folder_id).cloned() {
                if let Some((menu_x, menu_y, menu_w, menu_h)) = self.bookmark_folder_menu_rect(&folder) {
                    if x >= menu_x && x <= menu_x + menu_w && y >= menu_y && y <= menu_y + menu_h {
                        if let Some(action) = self.handle_folder_menu_click(&folder, x, y) {
                            return Some(action);
                        }
                    } else if !(x >= row_x && x <= row_x + row_w && y >= row_y && y <= row_y + row_h) {
                        self.open_bookmark_folder = None;
                        return Some(BookmarkUiAction::UiChanged);
                    }
                }
            }
        }

        if !(x >= row_x && x <= row_x + row_w && y >= row_y && y <= row_y + row_h) {
            return None;
        }

        let mut clicked: Option<(String, bool, Option<String>)> = None;
        for (bookmark, _idx, rect) in self.visible_root_bookmark_layout() {
            if x >= rect.left() && x <= rect.right() && y >= rect.top() && y <= rect.bottom() {
                clicked = Some((bookmark.id.clone(), bookmark.is_folder(), bookmark.url.clone()));
                break;
            }
        }

        if let Some((bookmark_id, is_folder, bookmark_url)) = clicked {
            self.selected_bookmark_id = Some(bookmark_id.clone());
            if is_folder {
                if self.open_bookmark_folder.as_deref() == Some(bookmark_id.as_str()) {
                    self.open_bookmark_folder = None;
                } else {
                    self.open_bookmark_folder = Some(bookmark_id);
                }
                return Some(BookmarkUiAction::UiChanged);
            }

            self.open_bookmark_folder = None;
            if let Some(url) = bookmark_url {
                return Some(BookmarkUiAction::Navigate(url));
            }
        }

        Some(BookmarkUiAction::UiChanged)
    }

    pub fn handle_bookmark_right_click(&mut self, x: f32, y: f32) -> Option<BookmarkUiAction> {
        self.bookmark_pressed_id = None;
        if let Some(action) = self.handle_bookmark_context_menu_click(x, y) {
            self.bookmark_context_menu = None;
            return Some(action);
        }

        if let Some((id, parent_id)) = self.bookmark_at_point(x, y) {
            self.selected_bookmark_id = Some(id.clone());
            self.bookmark_context_menu = Some(BookmarkContextMenuState {
                x,
                y,
                target_id: Some(id),
                parent_id,
            });
            return Some(BookmarkUiAction::UiChanged);
        }

        let (row_x, row_y, row_w, row_h) = self.bookmark_row_rect();
        if x >= row_x && x <= row_x + row_w && y >= row_y && y <= row_y + row_h {
            self.bookmark_context_menu = Some(BookmarkContextMenuState {
                x,
                y,
                target_id: None,
                parent_id: None,
            });
            return Some(BookmarkUiAction::UiChanged);
        }

        self.bookmark_context_menu = None;
        None
    }

    pub fn begin_bookmark_drag(&mut self, x: f32, y: f32) -> bool {
        if let Some((id, parent_id)) = self.bookmark_at_point(x, y) {
            self.bookmark_pressed_id = Some(id.clone());
            self.bookmark_drag = BookmarkDragState {
                active: false,
                dragged_id: Some(id),
                dragged_parent_id: parent_id,
                drag_start_x: x,
                drag_start_y: y,
                over_id: None,
                drop_parent_id: None,
                drop_index: None,
            };
            return true;
        }
        false
    }

    pub fn update_bookmark_drag(&mut self, x: f32, y: f32) -> bool {
        let Some(dragged_id) = self.bookmark_drag.dragged_id.clone() else {
            return false;
        };

        if !self.bookmark_drag.active {
            let dx = (x - self.bookmark_drag.drag_start_x).abs();
            let dy = (y - self.bookmark_drag.drag_start_y).abs();
            if dx < 6.0 * self.viewport.hidpi_scale && dy < 6.0 * self.viewport.hidpi_scale {
                return false;
            }
            self.bookmark_drag.active = true;
            self.bookmark_context_menu = None;
        }

        let mut next_over_id: Option<String> = None;
        let mut next_parent_id: Option<String> = None;
        let mut next_index: Option<usize> = None;

        let root_layout = self.visible_root_bookmark_layout();
        for (bookmark, index, rect) in &root_layout {
            if bookmark.id == dragged_id {
                continue;
            }
            if x >= rect.left() && x <= rect.right() && y >= rect.top() && y <= rect.bottom() {
                if bookmark.is_folder() {
                    next_over_id = Some(bookmark.id.clone());
                    next_parent_id = Some(bookmark.id.clone());
                    self.bookmark_drag.over_id = next_over_id;
                    self.bookmark_drag.drop_parent_id = next_parent_id;
                    self.bookmark_drag.drop_index = None;
                    return true;
                }

                let drop_before = x < rect.center_x();
                next_parent_id = None;
                next_index = Some(if drop_before { *index } else { index + 1 });
                self.bookmark_drag.over_id = None;
                self.bookmark_drag.drop_parent_id = next_parent_id;
                self.bookmark_drag.drop_index = next_index;
                return true;
            }
        }

        if let Some((open_folder_id, rows)) = self.visible_open_folder_layout() {
            for (bookmark, row_index, row_rect) in &rows {
                if bookmark.id == dragged_id {
                    continue;
                }
                if x >= row_rect.left() && x <= row_rect.right() && y >= row_rect.top() && y <= row_rect.bottom() {
                    if bookmark.is_folder() {
                        next_over_id = Some(bookmark.id.clone());
                        next_parent_id = Some(bookmark.id.clone());
                        self.bookmark_drag.over_id = next_over_id;
                        self.bookmark_drag.drop_parent_id = next_parent_id;
                        self.bookmark_drag.drop_index = None;
                        return true;
                    }
                    let drop_before = y < row_rect.center_y();
                    next_parent_id = Some(open_folder_id.clone());
                    next_index = Some(if drop_before { *row_index } else { *row_index + 1 });
                    self.bookmark_drag.over_id = None;
                    self.bookmark_drag.drop_parent_id = next_parent_id;
                    self.bookmark_drag.drop_index = next_index;
                    return true;
                }
            }

            if let Some((_, menu_y, _, menu_h)) = self.bookmark_folder_menu_rect(
                Self::find_bookmark(&self.bookmarks, &open_folder_id).expect("open folder should still exist"),
            ) {
                if y >= menu_y && y <= menu_y + menu_h {
                    next_parent_id = Some(open_folder_id);
                    next_index = Some(rows.len());
                    self.bookmark_drag.over_id = None;
                    self.bookmark_drag.drop_parent_id = next_parent_id;
                    self.bookmark_drag.drop_index = next_index;
                    return true;
                }
            }
        }

        let (_, row_y, _, row_h) = self.bookmark_row_rect();
        if y >= row_y && y <= row_y + row_h {
            let idx = root_layout
                .iter()
                .position(|(_, _, rect)| x < rect.center_x())
                .unwrap_or(root_layout.len());
            next_parent_id = None;
            next_index = Some(idx);
        }

        self.bookmark_drag.over_id = next_over_id;
        self.bookmark_drag.drop_parent_id = next_parent_id;
        self.bookmark_drag.drop_index = next_index;

        true
    }

    pub fn finish_bookmark_drag(&mut self) -> Option<BookmarkUiAction> {
        let dragged_id = self.bookmark_drag.dragged_id.take()?;
        let active = self.bookmark_drag.active;
        let over_id = self.bookmark_drag.over_id.take();
        let drop_parent_id = self.bookmark_drag.drop_parent_id.take();
        let drop_index = self.bookmark_drag.drop_index.take();
        let dragged_parent_id = self.bookmark_drag.dragged_parent_id.take();
        self.bookmark_drag.active = false;
        self.bookmark_pressed_id = None;

        if !active {
            return None;
        }

        if let Some(folder_id) = over_id {
            if folder_id != dragged_id {
                return Some(BookmarkUiAction::Move {
                    id: dragged_id,
                    parent_id: Some(folder_id),
                    index: None,
                });
            }
            return Some(BookmarkUiAction::UiChanged);
        }

        if dragged_parent_id == drop_parent_id && drop_index.is_none() {
            return Some(BookmarkUiAction::UiChanged);
        }

        Some(BookmarkUiAction::Move {
            id: dragged_id,
            parent_id: drop_parent_id,
            index: drop_index,
        })
    }

    pub fn is_dragging_bookmark(&self) -> bool {
        self.bookmark_drag.dragged_id.is_some()
    }

    fn bookmark_row_rect(&self) -> (f32, f32, f32, f32) {
        let scale = self.viewport.hidpi_scale;
        (
            0.0,
            Self::BOOKMARKS_ROW_Y * scale,
            self.window_width(),
            Self::BOOKMARKS_ROW_HEIGHT * scale,
        )
    }

    fn visible_root_bookmark_layout(&self) -> Vec<(&BookmarkNode, usize, Rect)> {
        let scale = self.viewport.hidpi_scale;
        let (row_x, row_y, row_w, row_h) = self.bookmark_row_rect();
        let mut cursor_x = row_x + 8.0 * scale;
        let item_w = Self::BOOKMARK_ITEM_WIDTH * scale;
        let item_h = row_h - (4.0 * scale);
        let spacing = Self::BOOKMARK_ITEM_SPACING * scale;
        let item_max_x = row_x + row_w - (8.0 * scale);
        let mut result = Vec::new();

        for (index, bookmark) in self.bookmarks.iter().enumerate() {
            if cursor_x + item_w > item_max_x {
                break;
            }
            let rect = Rect::from_xywh(cursor_x, row_y + 2.0 * scale, item_w, item_h);
            result.push((bookmark, index, rect));
            cursor_x += item_w + spacing;
        }

        result
    }

    fn visible_open_folder_layout(&self) -> Option<(String, Vec<(&BookmarkNode, usize, Rect)>)> {
        let folder_id = self.open_bookmark_folder.clone()?;
        let folder = Self::find_bookmark(&self.bookmarks, &folder_id)?;
        let (x, y, w, h) = self.bookmark_folder_menu_rect(folder)?;
        let row_h = 28.0 * self.viewport.hidpi_scale;
        let mut rows = Vec::new();

        for (index, bookmark) in folder.children.iter().enumerate() {
            let row_y = y + index as f32 * row_h;
            if row_y + row_h > y + h {
                break;
            }
            let rect = Rect::from_xywh(x + 2.0 * self.viewport.hidpi_scale, row_y + 1.0 * self.viewport.hidpi_scale, w - 4.0 * self.viewport.hidpi_scale, row_h - 2.0 * self.viewport.hidpi_scale);
            rows.push((bookmark, index, rect));
        }

        Some((folder_id, rows))
    }

    fn bookmark_at_point(&self, x: f32, y: f32) -> Option<(String, Option<String>)> {
        for (bookmark, _, rect) in self.visible_root_bookmark_layout() {
            if x >= rect.left() && x <= rect.right() && y >= rect.top() && y <= rect.bottom() {
                return Some((bookmark.id.clone(), None));
            }
        }

        if let Some((open_folder_id, rows)) = self.visible_open_folder_layout() {
            for (item, _, rect) in rows {
                if x >= rect.left() && x <= rect.right() && y >= rect.top() && y <= rect.bottom() {
                    return Some((item.id.clone(), Some(open_folder_id.clone())));
                }
            }
        }

        None
    }

    fn context_menu_entries(&self, target_id: Option<&str>) -> Vec<(&'static str, &'static str)> {
        match target_id.and_then(|id| Self::find_bookmark(&self.bookmarks, id)) {
            Some(node) if node.is_folder() => vec![
                ("add_page", "Add page"),
                ("add_folder", "Add folder"),
                ("rename", "Rename"),
                ("delete", "Remove"),
            ],
            Some(_) => vec![
                ("open", "Open"),
                ("rename", "Rename"),
                ("edit_url", "Edit URL"),
                ("delete", "Remove"),
            ],
            None => vec![
                ("add_page", "Add page"),
                ("add_folder", "Add folder"),
            ],
        }
    }

    fn bookmark_context_menu_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let menu = self.bookmark_context_menu.as_ref()?;
        let scale = self.viewport.hidpi_scale;
        let entries = self.context_menu_entries(menu.target_id.as_deref());
        let width = Self::BOOKMARK_CONTEXT_WIDTH * scale;
        let height = entries.len() as f32 * (Self::BOOKMARK_CONTEXT_ROW_HEIGHT * scale);
        let mut x = menu.x;
        let mut y = menu.y;

        if x + width > self.window_width() {
            x = (self.window_width() - width - 4.0 * scale).max(0.0);
        }
        if y + height > self.viewport.window_size.1 as f32 {
            y = (self.viewport.window_size.1 as f32 - height - 4.0 * scale).max(0.0);
        }

        Some((x, y, width, height))
    }

    fn handle_bookmark_context_menu_click(&self, x: f32, y: f32) -> Option<BookmarkUiAction> {
        let menu = self.bookmark_context_menu.as_ref()?;
        let (menu_x, menu_y, menu_w, menu_h) = self.bookmark_context_menu_rect()?;
        if x < menu_x || x > menu_x + menu_w || y < menu_y || y > menu_y + menu_h {
            return None;
        }

        let scale = self.viewport.hidpi_scale;
        let row_h = Self::BOOKMARK_CONTEXT_ROW_HEIGHT * scale;
        let idx = ((y - menu_y) / row_h).floor() as usize;
        let entries = self.context_menu_entries(menu.target_id.as_deref());
        let (command, _) = *entries.get(idx)?;

        match command {
            "open" => {
                let target_id = menu.target_id.as_deref()?;
                let bookmark = Self::find_bookmark(&self.bookmarks, target_id)?;
                Some(BookmarkUiAction::Navigate(bookmark.url.clone()?))
            }
            "add_page" => {
                let parent_id = menu
                    .target_id
                    .as_deref()
                    .and_then(|id| Self::find_bookmark(&self.bookmarks, id))
                    .and_then(|node| if node.is_folder() { Some(node.id.clone()) } else { None });
                Some(BookmarkUiAction::AddPage { parent_id })
            }
            "add_folder" => {
                let parent_id = menu
                    .target_id
                    .as_deref()
                    .and_then(|id| Self::find_bookmark(&self.bookmarks, id))
                    .and_then(|node| if node.is_folder() { Some(node.id.clone()) } else { None });
                Some(BookmarkUiAction::AddFolder { parent_id })
            }
            "rename" => Some(BookmarkUiAction::Rename(menu.target_id.clone()?)),
            "edit_url" => Some(BookmarkUiAction::EditUrl(menu.target_id.clone()?)),
            "delete" => Some(BookmarkUiAction::Delete(menu.target_id.clone()?)),
            _ => Some(BookmarkUiAction::UiChanged),
        }
    }

    fn cache_bookmark_favicons(bookmarks: &[BookmarkNode], cache: &mut HashMap<String, Option<Image>>) {
        for bookmark in bookmarks {
            let image = bookmark
                .favicon
                .as_ref()
                .and_then(|encoded| base64::engine::general_purpose::STANDARD.decode(encoded).ok())
                .and_then(|bytes| Image::from_encoded(Data::new_copy(&bytes)));
            cache.insert(bookmark.id.clone(), image);
            Self::cache_bookmark_favicons(&bookmark.children, cache);
        }
    }

    fn bookmark_folder_menu_rect(&self, folder: &BookmarkNode) -> Option<(f32, f32, f32, f32)> {
        if folder.children.is_empty() {
            return None;
        }

        let scale = self.viewport.hidpi_scale;
        let row_h = 28.0 * scale;
        let width = 280.0 * scale;
        let count = folder.children.len() as f32;
        let height = (count * row_h).min(10.0 * row_h);
        let (row_x, row_y, row_w, row_h_bar) = self.bookmark_row_rect();
        let mut x = row_x + 8.0 * scale;
        let y = row_y + row_h_bar + (2.0 * scale);

        // Position dropdown anchored to the selected top-level folder when possible.
        let item_w = Self::BOOKMARK_ITEM_WIDTH * scale;
        let spacing = Self::BOOKMARK_ITEM_SPACING * scale;
        for bookmark in &self.bookmarks {
            if bookmark.id == folder.id {
                break;
            }
            x += item_w + spacing;
        }

        if x + width > row_w {
            x = (row_w - width - 8.0 * scale).max(0.0);
        }

        Some((x, y, width, height))
    }

    fn handle_folder_menu_click(&mut self, folder: &BookmarkNode, x: f32, y: f32) -> Option<BookmarkUiAction> {
        let Some((menu_x, menu_y, menu_w, menu_h)) = self.bookmark_folder_menu_rect(folder) else {
            return None;
        };

        if x < menu_x || x > menu_x + menu_w || y < menu_y || y > menu_y + menu_h {
            return None;
        }

        let row_h = 28.0 * self.viewport.hidpi_scale;
        let idx = ((y - menu_y) / row_h).floor() as usize;
        let Some(bookmark) = folder.children.get(idx) else {
            return Some(BookmarkUiAction::UiChanged);
        };

        self.selected_bookmark_id = Some(bookmark.id.clone());
        if bookmark.is_folder() {
            self.open_bookmark_folder = Some(bookmark.id.clone());
            return Some(BookmarkUiAction::UiChanged);
        }

        if let Some(url) = &bookmark.url {
            self.open_bookmark_folder = None;
            return Some(BookmarkUiAction::Navigate(url.clone()));
        }

        Some(BookmarkUiAction::UiChanged)
    }

    fn find_bookmark<'a>(bookmarks: &'a [BookmarkNode], id: &str) -> Option<&'a BookmarkNode> {
        for bookmark in bookmarks {
            if bookmark.id == id {
                return Some(bookmark);
            }
            if let Some(found) = Self::find_bookmark(&bookmark.children, id) {
                return Some(found);
            }
        }
        None
    }

    /// Update UI layout when window is resized
    pub fn update_layout(&mut self, viewport: &Viewport) {
        self.viewport = viewport.clone();
        let scaled = |v: f32| v * self.viewport.hidpi_scale;
        let window_width = self.window_width();

        // Update address bar width and settings button position
        for comp in &mut self.components {
            match comp {
                UiComponent::TextField { id, width, is_flexible: true, .. } if id == "address_bar" => {
                    let available_width = window_width - scaled(Self::BUTTON_MARGIN * 8.0 + Self::BUTTON_SIZE * 6.0);
                    *width = available_width.max(scaled(Self::MIN_ADDRESS_BAR_WIDTH));
                }
                UiComponent::Button { id, x, .. } if id == "bookmark_toggle" => {
                    *x = window_width - scaled(Self::BUTTON_MARGIN * 2.0 + Self::BUTTON_SIZE * 2.0);
                }
                UiComponent::Button { id, x, .. } if id == "settings" => {
                    *x = window_width - scaled(Self::BUTTON_MARGIN + Self::BUTTON_SIZE);
                }
                _ => {}
            }
        }

        // Update tab layout with dynamic sizing (this will also position the new tab button)
        self.update_tab_layout();
    }

    /// Get the height of the chrome bar
    pub fn chrome_height(&self) -> f32 {
        Self::CHROME_HEIGHT * self.viewport.hidpi_scale
    }

    #[inline]
    fn window_width(&self) -> f32 {
        self.viewport.window_size.0 as f32
    }

    /// Initialize rendering resources
    pub fn initialize_renderer(&mut self) {
        // No-op for Skia
    }

    /// Calculate the appropriate width for each tab based on the number of tabs
    fn calculate_tab_width(&self) -> f32 {
        let tab_count = self.components.iter()
            .filter(|c| matches!(c, UiComponent::TabButton { .. }))
            .count();

        if tab_count == 0 {
            return Self::MAX_TAB_WIDTH * self.viewport.hidpi_scale;
        }

        // Available width for tabs (use scaled margin and reserve space for new tab button)
        let scaled_margin = Self::BUTTON_MARGIN * self.viewport.hidpi_scale;
        let new_tab_button_width = Self::BUTTON_SIZE * self.viewport.hidpi_scale;
        let available_width = self.window_width() - (scaled_margin * 3.0) - new_tab_button_width;

        // Calculate width that would fit all tabs (use scaled spacing)
        let scaled_spacing = Self::TAB_SPACING * self.viewport.hidpi_scale;
        let total_spacing = (tab_count - 1) as f32 * scaled_spacing;
        let width_per_tab = (available_width - total_spacing) / tab_count as f32;

        // Clamp between MIN and MAX (scaled), if it goes below MIN we'll use scrolling
        let scaled_min = Self::MIN_TAB_WIDTH * self.viewport.hidpi_scale;
        let scaled_max = Self::MAX_TAB_WIDTH * self.viewport.hidpi_scale;
        width_per_tab.max(scaled_min).min(scaled_max)
    }

    /// Update all tab positions and widths based on current state
    fn update_tab_layout(&mut self) {
        let scaled_margin = Self::BUTTON_MARGIN * self.viewport.hidpi_scale;
        let scaled_spacing = Self::TAB_SPACING * self.viewport.hidpi_scale;
        let new_tab_button_width = Self::BUTTON_SIZE * self.viewport.hidpi_scale;

        // Calculate available width for tabs (reserve space for the new tab button)
        let available_width_for_tabs = self.window_width() - (scaled_margin * 3.0) - new_tab_button_width;

        let tab_width = self.calculate_tab_width();
        let tab_count = self.components.iter()
            .filter(|c| matches!(c, UiComponent::TabButton { .. }))
            .count();

        // Calculate total width needed for all tabs (use scaled spacing)
        let total_tab_width = if tab_count > 0 {
            tab_count as f32 * tab_width + (tab_count.saturating_sub(1)) as f32 * scaled_spacing
        } else {
            0.0
        };

        // Update scroll offset bounds
        let max_scroll = (total_tab_width - available_width_for_tabs).max(0.0);
        self.tab_scroll_offset = self.tab_scroll_offset.min(max_scroll).max(0.0);

        // Update each tab's position and width
        let mut tab_x = scaled_margin - self.tab_scroll_offset;
        for comp in &mut self.components {
            if let UiComponent::TabButton { x, width, .. } = comp {
                *x = tab_x;
                *width = tab_width;
                tab_x += tab_width + scaled_spacing;
            }
        }

        // Position the "New Tab" button to the right of all tabs
        let new_tab_button_x = scaled_margin + total_tab_width - self.tab_scroll_offset + scaled_spacing;
        for comp in &mut self.components {
            if let UiComponent::Button { id, x, .. } = comp {
                if id == "new_tab" {
                    *x = new_tab_button_x;
                }
            }
        }
    }

    /// Handle mouse wheel scrolling for tabs
    pub fn handle_scroll(&mut self, delta_y: f32) {
        let tab_count = self.components.iter()
            .filter(|c| matches!(c, UiComponent::TabButton { .. }))
            .count();

        if tab_count == 0 {
            return;
        }

        let tab_width = self.calculate_tab_width();
        let scaled_spacing = Self::TAB_SPACING * self.viewport.hidpi_scale;
        let total_tab_width = tab_count as f32 * tab_width +
                              (tab_count.saturating_sub(1)) as f32 * scaled_spacing;

        // Only allow scrolling if tabs overflow (use scaled margin and account for new tab button)
        let scaled_margin = Self::BUTTON_MARGIN * self.viewport.hidpi_scale;
        let new_tab_button_width = Self::BUTTON_SIZE * self.viewport.hidpi_scale;
        let available_width = self.window_width() - (scaled_margin * 3.0) - new_tab_button_width;

        if total_tab_width > available_width {
            // Scroll by a portion of a tab width
            let scroll_amount = delta_y * 30.0; // Adjust sensitivity
            self.tab_scroll_offset -= scroll_amount;

            // Clamp scroll offset
            let max_scroll = total_tab_width - available_width;
            self.tab_scroll_offset = self.tab_scroll_offset.clamp(0.0, max_scroll);

            // Update tab positions
            self.update_tab_layout();
        }
    }

    /// Truncate text with ellipsis to fit within a given width
    fn truncate_text_to_width(text: &str, max_width: f32, font: &Font) -> String {
        if text.is_empty() {
            return String::new();
        }

        // Measure full text
        if let Some(blob) = TextBlob::new(text, font) {
            if blob.bounds().width() <= max_width {
                return text.to_string();
            }
        }

        // Binary search for the right length
        let ellipsis = "...";
        let ellipsis_width = TextBlob::new(ellipsis, font)
            .map(|b| b.bounds().width())
            .unwrap_or(20.0);

        let available_width = max_width - ellipsis_width;

        let mut low = 0;
        let mut high = text.len();
        let mut best_len = 0;

        while low <= high {
            let mut mid = (low + high) / 2;

            // Ensure mid is at a valid UTF-8 character boundary
            while mid > 0 && mid < text.len() && !text.is_char_boundary(mid) {
                mid -= 1;
            }

            if mid == 0 && low > 0 {
                // Can't make progress, break out
                break;
            }

            let substr = &text[..mid];

            if let Some(blob) = TextBlob::new(substr, font) {
                if blob.bounds().width() <= available_width {
                    best_len = mid;
                    low = mid + 1;
                } else {
                    high = mid.saturating_sub(1);
                }
            } else {
                break;
            }
        }

        // Make sure we don't cut in the middle of a UTF-8 character
        let mut truncate_at = best_len.min(text.len());
        while truncate_at > 0 && !text.is_char_boundary(truncate_at) {
            truncate_at -= 1;
        }

        format!("{}{}", &text[..truncate_at], ellipsis)
    }
}

impl BrowserUI {
    /// Add a new tab
    pub fn add_tab(&mut self, id: &str, title: &str) {
        let tab_count = self.components.iter().filter(|c| matches!(c, UiComponent::TabButton { .. })).count();

        // Set all existing tabs to inactive
        for comp in &mut self.components {
            if let UiComponent::TabButton { is_active, color, .. } = comp {
                *is_active = false;
                *color = [0.8, 0.8, 0.8]; // Inactive color
            }
        }

        // Add the new tab as active
        let x = Self::BUTTON_MARGIN + (tab_count as f32 * 158.0); // 150 width + 8 spacing
        let mut new_tab = UiComponent::tab(id, title, x, self.viewport.hidpi_scale);
        if let UiComponent::TabButton { is_active, color, .. } = &mut new_tab {
            *is_active = true;
            *color = [0.95, 0.95, 0.95];
        }
        self.components.push(new_tab);

        // Update tab layout after adding
        self.update_tab_layout();
    }

    /// Set active tab
    pub fn set_active_tab(&mut self, tab_id: &str) {
        for comp in &mut self.components {
            if let UiComponent::TabButton { id, is_active, color, .. } = comp {
                if id == tab_id {
                    *is_active = true;
                    *color = [0.95, 0.95, 0.95]; // Active color
                } else {
                    *is_active = false;
                    *color = [0.8, 0.8, 0.8]; // Inactive color
                }
            }
        }
    }

    /// Remove a tab
    pub fn remove_tab(&mut self, tab_id: &str) -> bool {
        let initial_count = self.components.len();
        self.components.retain(|comp| {
            if let UiComponent::TabButton { id, .. } = comp {
                id != tab_id
            } else {
                true
            }
        });

        let removed = self.components.len() < initial_count;

        if removed {
            // Update tab layout after removing
            self.update_tab_layout();
        }

        removed
    }

    /// Get all tab IDs
    pub fn get_tab_ids(&self) -> Vec<String> {
        self.components.iter()
            .filter_map(|comp| {
                if let UiComponent::TabButton { id, .. } = comp {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get active tab ID
    pub fn get_active_tab_id(&self) -> Option<String> {
        for comp in &self.components {
            if let UiComponent::TabButton { id, is_active: true, .. } = comp {
                return Some(id.clone());
            }
        }
        None
    }

    /// Update the address bar with a new URL
    pub fn update_address_bar(&mut self, url: &str) {
        for comp in &mut self.components {
            if let UiComponent::TextField { id, text, .. } = comp {
                if id == "address_bar" {
                    *text = url.to_string();
                }
            }
        }
    }

    /// Update tab title
    pub fn update_tab_title(&mut self, tab_id: &str, title: &str) {
        for comp in &mut self.components {
            if let UiComponent::TabButton { id, title: tab_title, tooltip, .. } = comp {
                if id == tab_id {
                    *tab_title = title.to_string();
                    tooltip.text = format_tab_tooltip_text(title);
                }
            }
        }
    }

    pub fn update_tab_loading(&mut self, tab_id: &str, is_loading: bool) {
        for comp in &mut self.components {
            if let UiComponent::TabButton { id, is_loading: tab_loading, .. } = comp {
                if id == tab_id {
                    *tab_loading = is_loading;
                    break;
                }
            }
        }
    }

    pub fn update_tab_favicon(&mut self, tab_id: &str, favicon: Option<&[u8]>) {
        for comp in &mut self.components {
            if let UiComponent::TabButton { id, favicon: tab_favicon, .. } = comp {
                if id == tab_id {
                    *tab_favicon = favicon
                        .and_then(|bytes| Image::from_encoded(Data::new_copy(bytes)));
                    break;
                }
            }
        }
    }

    /// Handle mouse click
    pub fn handle_click(&mut self, x: f32, y: f32) -> Option<String> {
        for comp in &self.components {
            if comp.contains_point(x, y) {
                return Some(comp.id().to_string());
            }
        }
        None
    }

    /// Check if the mouse is over any interactive UI element
    pub fn is_mouse_over_interactive_element(&self, x: f64, y: f64) -> bool {
        let x = x as f32;
        let y = y as f32;

        for comp in &self.components {
            match comp {
                UiComponent::Button { x: bx, y: by, width, height, .. } => {
                    if x >= *bx && x <= bx + width && y >= *by && y <= by + height {
                        return true;
                    }
                }
                UiComponent::TextField { x: fx, y: fy, width, height, .. } => {
                    if x >= *fx && x <= fx + width && y >= *fy && y <= fy + height {
                        return true;
                    }
                }
                UiComponent::TabButton { x: tx, y: ty, width, height, .. } => {
                    if x >= *tx && x <= tx + width && y >= *ty && y <= ty + height {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if the mouse is over a text field (for cursor display)
    pub fn is_mouse_over_text_field(&self, x: f64, y: f64) -> bool {
        let x = x as f32;
        let y = y as f32;

        for comp in &self.components {
            if let UiComponent::TextField { x: fx, y: fy, width, height, .. } = comp {
                if x >= *fx && x <= fx + width && y >= *fy && y <= fy + height {
                    return true;
                }
            }
        }
        false
    }

    /// Check if click is on close button of active tab, returns tab ID if so
    pub fn check_close_button_click(&self, x: f32, y: f32) -> Option<String> {
        for comp in &self.components {
            if let UiComponent::TabButton { id, x: tab_x, y: tab_y, width, height, is_active, .. } = comp {
                if *is_active {
                    // Calculate close button bounds
                    let close_button_size = 16.0 * self.viewport.hidpi_scale;
                    let close_button_x = tab_x + width - close_button_size - (4.0 * self.viewport.hidpi_scale);
                    let close_button_y = tab_y + (height / 2.0) - (close_button_size / 2.0);

                    // Check if click is within close button
                    if x >= close_button_x && x <= close_button_x + close_button_size &&
                       y >= close_button_y && y <= close_button_y + close_button_size {
                        return Some(id.clone());
                    }
                }
            }
        }
        None
    }

    /// Check if a point is over the close button of an active tab
    fn is_point_over_close_button(&self, x: f32, y: f32, tab_x: f32, tab_y: f32, tab_width: f32, tab_height: f32, is_active: bool) -> bool {
        if !is_active {
            return false;
        }

        let close_button_size = 16.0 * self.viewport.hidpi_scale;
        let close_button_x = tab_x + tab_width - close_button_size - (4.0 * self.viewport.hidpi_scale);
        let close_button_y = tab_y + (tab_height / 2.0) - (close_button_size / 2.0);

        x >= close_button_x && x <= close_button_x + close_button_size &&
        y >= close_button_y && y <= close_button_y + close_button_size
    }

    /// Set focus to a specific component
    pub fn set_focus(&mut self, component_id: &str) {
        for comp in &mut self.components {
            match comp {
                UiComponent::TextField { id, has_focus, cursor_position, text, selection_start, selection_end, .. } => {
                    if id == component_id {
                        // If already focused, just keep focus
                        if !*has_focus {
                            *has_focus = true;
                            // First focus: select all text
                            if !text.is_empty() {
                                *selection_start = Some(0);
                                *selection_end = Some(text.len());
                                *cursor_position = text.len();
                            } else {
                                *cursor_position = 0;
                            }
                        }
                    } else {
                        *has_focus = false;
                    }
                }
                _ => {}
            }
        }
    }

    /// Set focus to a text field at a specific click position
    /// First click: selects all text
    /// Subsequent clicks: positions cursor at click location
    pub fn set_focus_at_click(&mut self, component_id: &str, click_x: f32, shift_held: bool) {
        // First check if the text field already has focus and collect data needed for cursor calculation
        let mut already_focused = false;
        let mut field_data: Option<(String, f32)> = None;

        for comp in &self.components {
            if let UiComponent::TextField { id, has_focus, text, x, .. } = comp {
                if id == component_id {
                    already_focused = *has_focus;
                    if already_focused {
                        field_data = Some((text.clone(), *x));
                    }
                    break;
                }
            }
        }

        // Calculate cursor position if already focused (before mutable borrow)
        let new_cursor_pos = if already_focused {
            if let Some((text, field_x)) = field_data {
                Some(self.calculate_cursor_position_from_click(&text, field_x, click_x))
            } else {
                None
            }
        } else {
            None
        };

        // Now apply the changes with mutable borrow
        for comp in &mut self.components {
            match comp {
                UiComponent::TextField { id, has_focus, cursor_position, text, selection_start, selection_end, .. } => {
                    if id == component_id {
                        if already_focused {
                            // Already focused: position cursor at click location
                            if let Some(pos) = new_cursor_pos {
                                if shift_held {
                                    let anchor = selection_start.unwrap_or(*cursor_position);
                                    *selection_start = Some(anchor);
                                    *selection_end = Some(pos);
                                } else {
                                    *selection_start = None;
                                    *selection_end = None;
                                }
                                *cursor_position = pos;
                            }
                        } else {
                            // First focus: select all text
                            *has_focus = true;
                            if !text.is_empty() {
                                *selection_start = Some(0);
                                *selection_end = Some(text.len());
                                *cursor_position = text.len();
                            } else {
                                *cursor_position = 0;
                            }
                        }
                    } else {
                        *has_focus = false;
                    }
                }
                _ => {}
            }
        }
    }

    /// Calculate the cursor position (character index) from a click x-coordinate
    fn calculate_cursor_position_from_click(&self, text: &str, field_x: f32, click_x: f32) -> usize {
        Self::calculate_cursor_position_from_click_with_scale(text, field_x, click_x, self.viewport.hidpi_scale, self.ui_typeface.clone())
    }

    fn calculate_cursor_position_from_click_with_scale(text: &str, field_x: f32, click_x: f32, hidpi_scale: f32, typeface: skia_safe::Typeface) -> usize {
        if text.is_empty() {
            return 0;
        }

        let text_padding = 5.0 * hidpi_scale;
        let text_start_x = field_x + text_padding;

        // Click is before the text
        if click_x <= text_start_x {
            return 0;
        }

        let base_font_size = 14.0;
        let scaled_font_size = base_font_size * hidpi_scale;
        let font = Font::new(typeface, scaled_font_size);

        // Calculate relative click position from text start
        let relative_click_x = click_x - text_start_x;

        // Find the character position by measuring text width progressively
        let mut best_pos = text.len();
        let mut prev_width = 0.0;

        for (i, _) in text.char_indices() {
            let text_slice = &text[..i];
            let (width, _) = font.measure_str(text_slice, None);

            // Check if the click is between previous char and current char
            let midpoint = (prev_width + width) / 2.0;
            if relative_click_x < midpoint {
                best_pos = if i > 0 {
                    // Find the start of the previous character
                    text[..i].char_indices().last().map(|(idx, _)| idx).unwrap_or(0)
                } else {
                    0
                };
                return best_pos;
            }
            prev_width = width;
        }

        // If we get here, click is at or after the last character
        // Check if it's closer to the last character or the end
        let (full_width, _) = font.measure_str(text, None);
        let midpoint = (prev_width + full_width) / 2.0;
        if relative_click_x < midpoint {
            // Closer to the start of the last char
            text.char_indices().last().map(|(idx, _)| idx).unwrap_or(0)
        } else {
            text.len()
        }
    }

    /// Handle text input for focused component
    pub fn handle_text_input(&mut self, text: &str) {
        self.insert_text_at_cursor(text);
    }

    /// Handle key input for text editing
    pub fn handle_key_input(&mut self, key: &str, shift_held: bool, action_mod: bool) -> Option<String> {
        for comp in &mut self.components {
            if let UiComponent::TextField {
                id,
                has_focus: true,
                text: field_text,
                cursor_position,
                selection_start,
                selection_end,
                ..
            } = comp {
                // Ensure cursor_position is within valid bounds and aligned to a char boundary
                if *cursor_position > field_text.len() {
                    *cursor_position = field_text.len();
                }
                while *cursor_position > 0 && !field_text.is_char_boundary(*cursor_position) {
                    *cursor_position -= 1;
                }

                let selected_range = Self::selection_range(field_text, *selection_start, *selection_end);

                match key {
                    "Backspace" => {
                        if let Some((start, end)) = selected_range {
                            field_text.replace_range(start..end, "");
                            *cursor_position = start;
                            *selection_start = None;
                            *selection_end = None;
                        } else if action_mod {
                            let start = Self::prev_word_boundary(field_text, *cursor_position);
                            if start < *cursor_position {
                                field_text.replace_range(start..*cursor_position, "");
                                *cursor_position = start;
                            }
                        } else if *cursor_position > 0 {
                            let prev = Self::prev_char_boundary(field_text, *cursor_position);
                            if prev < *cursor_position && *cursor_position <= field_text.len() {
                                field_text.replace_range(prev..*cursor_position, "");
                                *cursor_position = prev;
                            }
                        }
                    }
                    "Delete" => {
                        if let Some((start, end)) = selected_range {
                            field_text.replace_range(start..end, "");
                            *cursor_position = start;
                            *selection_start = None;
                            *selection_end = None;
                        } else if action_mod {
                            let end = Self::next_word_boundary(field_text, *cursor_position);
                            if *cursor_position < end {
                                field_text.replace_range(*cursor_position..end, "");
                            }
                        } else if *cursor_position < field_text.len() {
                            let next = Self::next_char_boundary(field_text, *cursor_position);
                            if *cursor_position < next && next <= field_text.len() {
                                field_text.replace_range(*cursor_position..next, "");
                            }
                        }
                    }
                    "ArrowLeft" => {
                        let next_pos = if action_mod {
                            Self::prev_word_boundary(field_text, *cursor_position)
                        } else {
                            Self::prev_char_boundary(field_text, *cursor_position)
                        };
                        if shift_held {
                            let anchor = selection_start.unwrap_or(*cursor_position);
                            *selection_start = Some(anchor);
                            *selection_end = Some(next_pos);
                        } else {
                            *selection_start = None;
                            *selection_end = None;
                        }
                        *cursor_position = next_pos;
                    }
                    "ArrowRight" => {
                        let next_pos = if action_mod {
                            Self::next_word_boundary(field_text, *cursor_position)
                        } else {
                            Self::next_char_boundary(field_text, *cursor_position)
                        };
                        if shift_held {
                            let anchor = selection_start.unwrap_or(*cursor_position);
                            *selection_start = Some(anchor);
                            *selection_end = Some(next_pos);
                        } else {
                            *selection_start = None;
                            *selection_end = None;
                        }
                        *cursor_position = next_pos;
                    }
                    "Home" => {
                        if shift_held {
                            let anchor = selection_start.unwrap_or(*cursor_position);
                            *selection_start = Some(anchor);
                            *selection_end = Some(0);
                        } else {
                            *selection_start = None;
                            *selection_end = None;
                        }
                        *cursor_position = 0;
                    }
                    "End" => {
                        let end_pos = field_text.len();
                        if shift_held {
                            let anchor = selection_start.unwrap_or(*cursor_position);
                            *selection_start = Some(anchor);
                            *selection_end = Some(end_pos);
                        } else {
                            *selection_start = None;
                            *selection_end = None;
                        }
                        *cursor_position = field_text.len();
                        while *cursor_position > 0 && !field_text.is_char_boundary(*cursor_position) {
                            *cursor_position -= 1;
                        }
                    }
                    "Enter" => {
                        // Return the field content for navigation
                        if id == "address_bar" {
                            return Some(field_text.clone());
                        }
                    }
                    _ => {}
                }
                break;
            }
        }
        None
    }

    /// Get the current text of a text field
    pub fn get_text_field_content(&self, field_id: &str) -> Option<String> {
        for comp in &self.components {
            if let UiComponent::TextField { id, text, .. } = comp {
                if id == field_id {
                    return Some(text.clone());
                }
            }
        }
        None
    }

    /// Update scale factor for DPI changes
    pub fn update_scale(&mut self, hidpi_scale: f32, old_hidpi_scale: f32) {
        // Rescale all components
        let scale_ratio = hidpi_scale / old_hidpi_scale;

        for comp in &mut self.components {
            match comp {
                UiComponent::Button { x, y, width, height, .. } => {
                    *x *= scale_ratio;
                    *y *= scale_ratio;
                    *width *= scale_ratio;
                    *height *= scale_ratio;
                }
                UiComponent::TextField { x, y, width, height, .. } => {
                    *x *= scale_ratio;
                    *y *= scale_ratio;
                    *width *= scale_ratio;
                    *height *= scale_ratio;
                }
                UiComponent::TabButton { x, y, width, height, .. } => {
                    *x *= scale_ratio;
                    *y *= scale_ratio;
                    *width *= scale_ratio;
                    *height *= scale_ratio
                }
            }
        }

        // Update layout to recalculate positions properly
        self.update_layout(&self.viewport.clone());
    }

    /// Clear focus from all components
    pub fn clear_focus(&mut self) {
        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus, selection_start, selection_end, .. } = comp {
                *has_focus = false;
                *selection_start = None;
                *selection_end = None;
            }
        }
        self.end_text_selection_drag();
    }

    /// Check if any text field has focus
    pub fn is_text_field_focused(&self) -> bool {
        for comp in &self.components {
            if let UiComponent::TextField { has_focus: true, .. } = comp {
                return true;
            }
        }
        false
    }

    /// Select all text in the focused text field
    pub fn select_all(&mut self) {
        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus: true, text, selection_start, selection_end, cursor_position, .. } = comp {
                if !text.is_empty() {
                    *selection_start = Some(0);
                    *selection_end = Some(text.len());
                    *cursor_position = text.len();
                }
                break;
            }
        }
    }

    /// Get selected text from the focused text field
    pub fn get_selected_text(&self) -> Option<String> {
        for comp in &self.components {
            if let UiComponent::TextField { has_focus: true, text, selection_start, selection_end, .. } = comp {
                return Self::selected_text_for_range(text, *selection_start, *selection_end);
            }
        }
        None
    }

    /// Delete selected text in the focused text field
    pub fn delete_selection(&mut self) -> bool {
        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus: true, text, selection_start, selection_end, cursor_position, .. } = comp {
                if let (Some(&start), Some(&end)) = (selection_start.as_ref(), selection_end.as_ref()) {
                    let mut s = start.min(end);
                    let mut e = start.max(end);
                    if s > text.len() { s = text.len(); }
                    if e > text.len() { e = text.len(); }
                    // Align to char boundaries
                    if !text.is_char_boundary(s) {
                        s = Self::prev_char_boundary(text, s);
                    }
                    if !text.is_char_boundary(e) {
                        e = Self::next_char_boundary(text, e);
                    }
                    if s < e && e <= text.len() {
                        text.replace_range(s..e, "");
                        *cursor_position = s;
                        *selection_start = None;
                        *selection_end = None;
                        return true;
                    }
                }
                break;
            }
        }
        false
    }

    /// Insert text at cursor position (replacing selection if any)
    pub fn insert_text_at_cursor(&mut self, insert_text: &str) {
        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus: true, text, selection_start, selection_end, cursor_position, .. } = comp {
                // Delete selection if any
                if let (Some(&start), Some(&end)) = (selection_start.as_ref(), selection_end.as_ref()) {
                    let mut s = start.min(end);
                    let mut e = start.max(end);
                    if s > text.len() { s = text.len(); }
                    if e > text.len() { e = text.len(); }
                    // Align to char boundaries
                    if !text.is_char_boundary(s) {
                        s = Self::prev_char_boundary(text, s);
                    }
                    if !text.is_char_boundary(e) {
                        e = Self::next_char_boundary(text, e);
                    }
                    if s < e && e <= text.len() {
                        text.replace_range(s..e, "");
                        *cursor_position = s;
                        *selection_start = None;
                        *selection_end = None;
                    }
                }

                // Insert text at cursor position
                if *cursor_position > text.len() {
                    *cursor_position = text.len();
                }
                if !text.is_char_boundary(*cursor_position) {
                    *cursor_position = Self::prev_char_boundary(text, *cursor_position);
                }
                if *cursor_position <= text.len() {
                    text.insert_str(*cursor_position, insert_text);
                    *cursor_position += insert_text.len();
                }
                break;
            }
        }
    }

    /// Begin a mouse-driven selection gesture for a text field.
    pub fn begin_text_selection_drag(&mut self, component_id: &str, click_x: f32, shift_held: bool) {
        self.set_focus_at_click(component_id, click_x, shift_held);

        let mut anchor = None;
        for comp in &self.components {
            if let UiComponent::TextField { id, has_focus: true, cursor_position, selection_start, .. } = comp {
                if id == component_id {
                    anchor = Some(selection_start.unwrap_or(*cursor_position));
                    break;
                }
            }
        }

        self.text_selection_drag_active = true;
        self.text_selection_drag_anchor = anchor;
    }

    /// Extend focused text-field selection to the current pointer x position.
    pub fn update_text_selection_drag(&mut self, pointer_x: f32) -> bool {
        if !self.text_selection_drag_active {
            return false;
        }

        let hidpi_scale = self.viewport.hidpi_scale;

        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus: true, text, x, cursor_position, selection_start, selection_end, .. } = comp {
                let next_pos = Self::calculate_cursor_position_from_click_with_scale(text, *x, pointer_x, hidpi_scale, self.ui_typeface.clone());
                let anchor = self.text_selection_drag_anchor.unwrap_or(*cursor_position);
                *selection_start = Some(anchor);
                *selection_end = Some(next_pos);
                *cursor_position = next_pos;
                return true;
            }
        }

        false
    }

    pub fn end_text_selection_drag(&mut self) {
        self.text_selection_drag_active = false;
        self.text_selection_drag_anchor = None;
    }

    pub fn is_text_selection_drag_active(&self) -> bool {
        self.text_selection_drag_active
    }

    /// Clear selection in the focused text field
    pub fn clear_selection(&mut self) {
        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus: true, selection_start, selection_end, .. } = comp {
                *selection_start = None;
                *selection_end = None;
                break;
            }
        }
    }

    /// Toggle the settings panel visibility
    pub fn toggle_settings(&mut self) {
        self.show_settings = !self.show_settings;
    }

    /// Check if a click lands inside the settings panel and return the action id
    pub fn handle_settings_panel_click(&self, x: f32, y: f32) -> Option<String> {
        if !self.show_settings {
            return None;
        }
        let panel = self.settings_panel_rect();
        // If click is outside the panel, close it (no action id but signal close)
        if x < panel.0 || x > panel.0 + panel.2 || y < panel.1 || y > panel.1 + panel.3 {
            return Some("settings_panel_close".to_string());
        }
        // Check "Set as Default Browser" button inside panel
        let btn = self.default_browser_button_rect();
        if x >= btn.0 && x <= btn.0 + btn.2 && y >= btn.1 && y <= btn.1 + btn.3 {
            return Some("set_default_browser".to_string());
        }
        // Click inside panel but not on any button — consume the event
        Some("settings_panel_noop".to_string())
    }

    /// Returns (x, y, width, height) for the settings panel
    fn settings_panel_rect(&self) -> (f32, f32, f32, f32) {
        let s = self.viewport.hidpi_scale;
        let panel_width = 260.0 * s;
        let panel_height = 120.0 * s;
        let window_width = self.window_width();
        let chrome_height = self.chrome_height();
        let x = (window_width - panel_width - 8.0 * s).max(0.0);
        let y = chrome_height + 4.0 * s;
        (x, y, panel_width, panel_height)
    }

    /// Returns (x, y, width, height) for the "Set as Default Browser" button inside the panel
    fn default_browser_button_rect(&self) -> (f32, f32, f32, f32) {
        let s = self.viewport.hidpi_scale;
        let (px, py, pw, _ph) = self.settings_panel_rect();
        let padding = 16.0 * s;
        let btn_height = 32.0 * s;
        let btn_width = pw - padding * 2.0;
        // Position below the "Settings" title
        let btn_x = px + padding;
        let btn_y = py + 52.0 * s;
        (btn_x, btn_y, btn_width, btn_height)
    }

    /// Render the settings panel overlay
    pub fn render_settings_panel(&self, canvas: &Canvas, font: &Font) {
        if !self.show_settings {
            return;
        }

        let s = self.viewport.hidpi_scale;
        let mut paint = Paint::default();
        let (px, py, pw, ph) = self.settings_panel_rect();
        let panel_rect = Rect::from_xywh(px, py, pw, ph);

        // Shadow
        paint.set_color(Color::from_argb(60, 0, 0, 0));
        canvas.draw_round_rect(Rect::from_xywh(px + 3.0 * s, py + 3.0 * s, pw, ph), 8.0 * s, 8.0 * s, &paint);

        // Panel background
        paint.set_color(Color::from_rgb(250, 250, 252));
        canvas.draw_round_rect(panel_rect, 8.0 * s, 8.0 * s, &paint);

        // Panel border
        paint.set_color(Color::from_rgb(200, 200, 210));
        paint.set_stroke(true);
        paint.set_stroke_width(1.0 * s);
        canvas.draw_round_rect(panel_rect, 8.0 * s, 8.0 * s, &paint);
        paint.set_stroke(false);

        // Title "Settings"
        paint.set_color(Color::from_rgb(40, 40, 40));
        let title = "Settings";
        if let Some(blob) = TextBlob::new(title, font) {
            let bounds = blob.bounds();
            let text_y = py + 16.0 * s - bounds.top;
            canvas.draw_text_blob(&blob, (px + 16.0 * s, text_y), &paint);
        }

        // Separator line
        paint.set_color(Color::from_rgb(220, 220, 220));
        paint.set_stroke(true);
        paint.set_stroke_width(1.0 * s);
        canvas.draw_line((px + 8.0 * s, py + 40.0 * s), (px + pw - 8.0 * s, py + 40.0 * s), &paint);
        paint.set_stroke(false);

        // "Set as Default Browser" button
        let (bx, by, bw, bh) = self.default_browser_button_rect();
        let btn_rect = Rect::from_xywh(bx, by, bw, bh);
        paint.set_color(Color::from_rgb(70, 130, 220));
        canvas.draw_round_rect(btn_rect, 6.0 * s, 6.0 * s, &paint);

        // Button label
        paint.set_color(Color::WHITE);
        let label = "Set as Default Browser";
        if let Some(blob) = TextBlob::new(label, font) {
            let bounds = blob.bounds();
            let text_x = bx + (bw - bounds.width()) / 2.0;
            let text_y = by + (bh / 2.0) - (bounds.top + bounds.height() / 2.0);
            canvas.draw_text_blob(&blob, (text_x, text_y), &paint);
        }
    }

    /// Render the UI
    pub fn render(&self, canvas: &Canvas, font_ctx: &mut FontContext, layout_ctx: &mut LayoutContext<TextBrush>, painter: &mut ScenePainter, loading_spinner_angle: f32) {
        let canvas_width = canvas.image_info().width() as f32;
        let canvas_height = canvas.image_info().height() as f32;
        let chrome_height = self.chrome_height();

        // Draw browser chrome background bar at the top
        let mut chrome_paint = Paint::default();
        chrome_paint.set_color(Color::from_rgb(240, 240, 240)); // Light gray background
        let chrome_rect = Rect::from_xywh(0.0, 0.0, canvas_width, chrome_height);
        canvas.draw_rect(chrome_rect, &chrome_paint);

        // Draw a bottom border for the chrome
        chrome_paint.set_color(Color::from_rgb(200, 200, 200));
        let border_rect = Rect::from_xywh(0.0, chrome_height - 1.0, canvas_width, 1.0);
        canvas.draw_rect(border_rect, &chrome_paint);

        let mut paint = Paint::default();

        // Apply scale factor to font size for proper DPI scaling
        let base_font_size = 14.0;
        let scaled_font_size = base_font_size * self.viewport.hidpi_scale;
        let font = Font::new(self.ui_typeface.clone(), scaled_font_size);

        self.render_bookmarks_bar(canvas, &font);

        // Draw BROWSING WITH STOKES text in the top-right corner
        {
            let text = &format!("STOKES BROWSER {VERSION}");
            let mut builder = layout_ctx.ranged_builder(font_ctx, text, self.viewport.hidpi_scale, true);

            builder.push_default(GenericFamily::SystemUi);
            builder.push_default(LineHeight::FontSizeRelative(1.3));
            builder.push_default(StyleProperty::FontSize(base_font_size));

            let mut layout = builder.build(text);

            layout.break_all_lines(Some(canvas_width));
            layout.align(Alignment::Start, AlignmentOptions::default());
            let width = layout.width().ceil() as u32;
            let height = layout.height().ceil() as u32;
            let padded_width = width + 40;
            let padded_height = height + 40;

            let text_x = canvas_width - width as f32 - (20.0 * self.viewport.hidpi_scale);
            let text_y = height as f32;
            let pos = kurbo::Point {
                x: text_x as f64,
                y: text_y as f64,
            };
            let transform = Affine::translate((pos.x, pos.y));

            for line in layout.lines() {
                for item in line.items() {
                    match item {
                        PositionedLayoutItem::GlyphRun(glyph_run) => {
                            let mut run_x = glyph_run.offset();
                            let run_y = glyph_run.baseline();

                            let run = glyph_run.run();
                            let font = run.font();
                            let font_size = run.font_size();
                            let metrics = run.metrics();
                            let style = glyph_run.style();
                            let synthesis = run.synthesis();
                            let glyph_xform = synthesis.skew().map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));



                            painter.draw_glyphs(
                                font,
                                font_size,
                                true,
                                run.normalized_coords(),
                                Fill::NonZero,
                                &anyrender::Paint::from(AlphaColor::BLACK),
                                1.0,
                                transform,
                                glyph_xform,
                                glyph_run.positioned_glyphs().map(|glyph| anyrender::Glyph {
                                    id: glyph.id as _,
                                    x: glyph.x,
                                    y: glyph.y,
                                })
                            );
                        }
                        PositionedLayoutItem::InlineBox(_) => {
                        }
                    }
                }
            }
        }

        // Reset the canvas matrix to identity before drawing UI components
        // (the text rendering above may have modified it)
        painter.set_matrix(Affine::IDENTITY);

        // Scale other text rendering properties
        let text_padding = 5.0 * self.viewport.hidpi_scale;
        let cursor_margin = 6.0 * self.viewport.hidpi_scale;
        let cursor_stroke_width = 1.5 * self.viewport.hidpi_scale;
        let shadow_offset = 2.0 * self.viewport.hidpi_scale;

        // Collect tooltips to render them above everything else at the end
        let mut tooltips_to_render: Vec<(&Tooltip, f32, f32)> = Vec::new();

        for comp in &self.components {
            match comp {
                UiComponent::Button { x, y, width, height, color, hover_color, pressed_color, is_pressed, is_hover, is_active, tooltip, icon_type, .. } => {
                    let rect = Rect::from_xywh(*x, *y, *width, *height);

                    // Draw button shadow for depth
                    let shadow_rect = Rect::from_xywh(*x + shadow_offset, *y + shadow_offset, *width, *height);
                    paint.set_color(Color::from_argb(50, 0, 0, 0)); // Semi-transparent shadow
                    canvas.draw_round_rect(shadow_rect, 4.0, 4.0, &paint);

                    // Choose color based on state
                    let current_color = if *is_pressed {
                        pressed_color
                    } else if *is_active {
                        &[0.98, 0.88, 0.45]
                    } else if *is_hover {
                        hover_color
                    } else {
                        color
                    };

                    // Draw button background with rounded corners
                    paint.set_color(Color::from_rgb(
                        (current_color[0] * 255.0) as u8,
                        (current_color[1] * 255.0) as u8,
                        (current_color[2] * 255.0) as u8,
                    ));
                    canvas.draw_round_rect(rect, 4.0, 4.0, &paint);

                    // Draw button border
                    paint.set_color(if *is_hover {
                        Color::from_rgb(100, 150, 255)
                    } else {
                        Color::from_rgb(180, 180, 180)
                    });
                    paint.set_stroke(true);
                    paint.set_stroke_width(1.0 * self.viewport.hidpi_scale);
                    canvas.draw_round_rect(rect, 4.0, 4.0, &paint);
                    paint.set_stroke(false);

                    // Draw custom icon instead of text
                    self.draw_icon(painter, icon_type, rect, *is_hover, self.viewport.hidpi_scale);

                    // Collect tooltip for later rendering (to render above everything)
                    if tooltip.is_visible {
                        tooltips_to_render.push((tooltip, *x, *y));
                    }
                }
                UiComponent::TextField { text, x, y, width, height, color, border_color, has_focus, cursor_position, selection_start, selection_end, .. } => {
                    let rect = Rect::from_xywh(*x, *y, *width, *height);

                    // Draw field shadow
                    let shadow_rect = Rect::from_xywh(*x + 1.0, *y + 1.0, *width, *height);
                    paint.set_color(Color::from_argb(30, 0, 0, 0));
                    canvas.draw_round_rect(shadow_rect, 2.0, 2.0, &paint);

                    // Draw field background (brighter when focused)
                    let bg_color = if *has_focus {
                        Color::WHITE
                    } else {
                        Color::from_rgb(250, 250, 250)
                    };
                    paint.set_color(bg_color);
                    canvas.draw_round_rect(rect, 2.0, 2.0, &paint);

                    // Draw field border (blue when focused) with scaled stroke width
                    let border_color = if *has_focus {
                        Color::from_rgb(100, 150, 255)
                    } else {
                        Color::from_rgb(
                            (border_color[0] * 255.0) as u8,
                            (border_color[1] * 255.0) as u8,
                            (border_color[2] * 255.0) as u8,
                        )
                    };
                    paint.set_color(border_color);
                    paint.set_stroke(true);
                    paint.set_stroke_width(if *has_focus { 2.0 * self.viewport.hidpi_scale } else { 1.0 * self.viewport.hidpi_scale });
                    canvas.draw_round_rect(rect, 2.0, 2.0, &paint);
                    paint.set_stroke(false);

                    // Draw text selection highlight first so glyphs render on top.
                    if *has_focus {
                        if let Some((sel_start, sel_end)) = Self::selection_range(text, *selection_start, *selection_end) {
                            let text_before_selection = &text[..sel_start];
                            let selected_text = &text[sel_start..sel_end];
                            let (prefix_width, _) = font.measure_str(text_before_selection, None);
                            let (selected_width, _) = font.measure_str(selected_text, None);

                            paint.set_color(Color::from_argb(140, 132, 185, 255));
                            let selection_rect = Rect::from_xywh(
                                rect.left() + text_padding + prefix_width,
                                rect.top() + (2.0 * self.viewport.hidpi_scale),
                                selected_width.max(1.0),
                                rect.height() - (4.0 * self.viewport.hidpi_scale),
                            );
                            canvas.draw_rect(selection_rect, &paint);
                        }
                    }

                    // Draw text content with scaled padding, centered vertically
                    paint.set_color(Color::BLACK);
                    if let Some(blob) = TextBlob::new(text, &font) {
                        let text_bounds = blob.bounds();
                        // Center the text vertically in the field
                        let text_y = rect.top() + (rect.height() / 2.0) - (text_bounds.top + text_bounds.height() / 2.0);
                        canvas.draw_text_blob(&blob, (rect.left() + text_padding, text_y), &paint);
                    }

                    // Draw cursor if focused
                    if *has_focus {
                        // Calculate cursor position in pixels
                        let text_before_cursor = if *cursor_position > 0 {
                            &text[..*cursor_position.min(&text.len())]
                        } else {
                            ""
                        };

                        // Measure the actual width of text before cursor
                        let text_width = if text_before_cursor.is_empty() {
                            0.0
                        } else {
                            // Use font.measure_text to get the actual advance width
                            let (width, _) = font.measure_str(text_before_cursor, None);
                            width
                        };

                        let cursor_x = rect.left() + text_padding + text_width;

                        // Draw cursor line with scaled stroke width and margins
                        paint.set_color(Color::BLACK);
                        paint.set_stroke(true);
                        paint.set_stroke_width(cursor_stroke_width);
                        canvas.draw_line(
                            (cursor_x, rect.top() + cursor_margin),
                            (cursor_x, rect.bottom() - cursor_margin),
                            &paint
                        );
                        paint.set_stroke(false);
                    }
                }
                UiComponent::TabButton { title, x, y, width, height, color, hover_color, is_active, is_hover, tooltip, close_button_hover, close_button_tooltip, favicon, is_loading, .. } => {
                    let rect = Rect::from_xywh(*x, *y, *width, *height);

                    // Draw tab shadow
                    let shadow_rect = Rect::from_xywh(*x + 1.0, *y + 1.0, *width, *height);
                    paint.set_color(Color::from_argb(30, 0, 0, 0));
                    canvas.draw_round_rect(shadow_rect, 4.0, 4.0, &paint);

                    // Choose color based on state
                    let current_color = if *is_hover {
                        hover_color
                    } else {
                        color
                    };

                    paint.set_color(Color::from_rgb(
                        (current_color[0] * 255.0) as u8,
                        (current_color[1] * 255.0) as u8,
                        (current_color[2] * 255.0) as u8,
                    ));
                    canvas.draw_round_rect(rect, 4.0, 4.0, &paint);

                    // Draw tab border (different for active tab)
                    paint.set_color(if *is_active {
                        Color::from_rgb(100, 150, 255)
                    } else if *is_hover {
                        Color::from_rgb(150, 180, 255)
                    } else {
                        Color::from_rgb(180, 180, 180)
                    });
                    paint.set_stroke(true);
                    paint.set_stroke_width(if *is_active { 2.0 * self.viewport.hidpi_scale } else { 1.0 * self.viewport.hidpi_scale });
                    canvas.draw_round_rect(rect, 4.0, 4.0, &paint);
                    paint.set_stroke(false);

                    let favicon_size = 16.0 * self.viewport.hidpi_scale;
                    let favicon_padding_left = 8.0 * self.viewport.hidpi_scale;
                    let favicon_rect = Rect::from_xywh(
                        rect.left() + favicon_padding_left,
                        rect.center_y() - (favicon_size / 2.0),
                        favicon_size,
                        favicon_size,
                    );

                    Self::draw_tab_favicon(canvas, &mut paint, favicon_rect, favicon.as_ref());

                    if *is_loading {
                        let spinner_radius = (favicon_size * 0.75).max(8.0 * self.viewport.hidpi_scale);
                        Self::draw_spinner(
                            painter,
                            favicon_rect.center_x(),
                            favicon_rect.center_y(),
                            spinner_radius,
                            loading_spinner_angle,
                            self.viewport.hidpi_scale,
                        );
                    }

                    // Calculate space needed for close button if active
                    let close_button_space = if *is_active { 20.0 * self.viewport.hidpi_scale } else { 0.0 };

                    // Truncate tab text to fit within the tab width (leaving space for favicon + close button)
                    let text_start_x = favicon_rect.right() + (6.0 * self.viewport.hidpi_scale);
                    let max_text_width = (rect.right() - close_button_space) - text_start_x - text_padding;
                    let display_text = Self::truncate_text_to_width(title, max_text_width, &font);

                    // Draw tab text with scaled padding, centered vertically
                    paint.set_color(Color::BLACK);
                    if let Some(blob) = TextBlob::new(&display_text, &font) {
                        let text_bounds = blob.bounds();
                        // Center the text vertically in the tab
                        let text_y = rect.top() + (rect.height() / 2.0) - (text_bounds.top + text_bounds.height() / 2.0);
                        canvas.draw_text_blob(&blob, (text_start_x, text_y), &paint);
                    }

                    // Draw close button for active tab
                    if *is_active {
                        let close_button_size = 16.0 * self.viewport.hidpi_scale;
                        let close_button_x = rect.right() - close_button_size - (4.0 * self.viewport.hidpi_scale);
                        let close_button_y = rect.center_y() - (close_button_size / 2.0);
                        let close_button_rect = Rect::from_xywh(close_button_x, close_button_y, close_button_size, close_button_size);

                        // Draw close button background with different color when hovering
                        if *close_button_hover {
                            paint.set_color(Color::from_argb(100, 255, 100, 100)); // Reddish highlight when hovering
                        } else {
                            paint.set_color(Color::from_argb(20, 0, 0, 0)); // Subtle background
                        }
                        canvas.draw_round_rect(close_button_rect, 2.0, 2.0, &paint);

                        // Draw X icon with different color when hovering
                        self.draw_icon(painter, &IconType::Close, close_button_rect, *close_button_hover, self.viewport.hidpi_scale);
                    }

                    // Collect tooltip for later rendering (to render above everything)
                    if tooltip.is_visible {
                        tooltips_to_render.push((tooltip, *x, *y));
                    }

                    // Collect close button tooltip if visible
                    if close_button_tooltip.is_visible {
                        let close_button_size = 16.0 * self.viewport.hidpi_scale;
                        let close_button_x = *x + *width - close_button_size - (4.0 * self.viewport.hidpi_scale);
                        tooltips_to_render.push((close_button_tooltip, close_button_x, *y));
                    }
                }
            }
        }

        // Render all tooltips last so they appear above everything else
        for (tooltip, x, y) in tooltips_to_render {
            Self::draw_tooltip(painter, tooltip, x, y, &font, self.viewport.hidpi_scale, canvas_width, canvas_height);
        }

        // Render settings panel on top of everything
        self.render_settings_panel(canvas, &font);
    }

    fn render_bookmarks_bar(&self, canvas: &Canvas, font: &Font) {
        let scale = self.viewport.hidpi_scale;
        let mut paint = Paint::default();
        let (row_x, row_y, row_w, row_h) = self.bookmark_row_rect();
        let row_rect = Rect::from_xywh(row_x, row_y, row_w, row_h);

        paint.set_color(Color::from_rgb(247, 247, 248));
        canvas.draw_rect(row_rect, &paint);
        paint.set_color(Color::from_rgb(214, 214, 214));
        canvas.draw_line((row_x, row_y), (row_x + row_w, row_y), &paint);

        for (bookmark, index, item_rect) in self.visible_root_bookmark_layout() {
            let is_selected = self.selected_bookmark_id.as_deref() == Some(bookmark.id.as_str());
            let is_hovered = self.bookmark_hover_id.as_deref() == Some(bookmark.id.as_str());
            let is_pressed = self.bookmark_pressed_id.as_deref() == Some(bookmark.id.as_str());
            let is_drop_folder_target = self.bookmark_drag.active
                && self.bookmark_drag.over_id.as_deref() == Some(bookmark.id.as_str());

            paint.set_color(if is_drop_folder_target {
                Color::from_rgb(192, 224, 255)
            } else if is_pressed {
                Color::from_rgb(206, 219, 239)
            } else if is_hovered {
                Color::from_rgb(228, 236, 248)
            } else if is_selected {
                Color::from_rgb(210, 228, 255)
            } else {
                Color::from_rgb(236, 236, 238)
            });
            canvas.draw_round_rect(item_rect, 7.0 * scale, 7.0 * scale, &paint);

            paint.set_color(Color::from_rgb(180, 180, 190));
            paint.set_stroke(true);
            paint.set_stroke_width(1.0 * scale);
            canvas.draw_round_rect(item_rect, 7.0 * scale, 7.0 * scale, &paint);
            paint.set_stroke(false);

            let favicon_rect = Rect::from_xywh(
                item_rect.left() + 6.0 * scale,
                item_rect.center_y() - (8.0 * scale),
                16.0 * scale,
                16.0 * scale,
            );
            if bookmark.is_folder() {
                Self::render_svg_on_canvas(canvas, &self.folder_svg, favicon_rect, Color::from_rgb(208, 166, 92));
            } else {
                let favicon = self
                    .bookmark_favicons
                    .get(&bookmark.id)
                    .and_then(|image| image.as_ref());
                Self::draw_tab_favicon(canvas, &mut paint, favicon_rect, favicon);
            }

            let label = if bookmark.is_folder() {
                format!("{} v", bookmark.title)
            } else {
                bookmark.title.clone()
            };
            let text = Self::truncate_text_to_width(&label, item_rect.width() - (32.0 * scale), font);
            if let Some(blob) = TextBlob::new(&text, font) {
                let bounds = blob.bounds();
                let text_y = item_rect.center_y() - (bounds.top + bounds.height() / 2.0);
                paint.set_color(Color::from_rgb(45, 45, 45));
                canvas.draw_text_blob(&blob, (favicon_rect.right() + 5.0 * scale, text_y), &paint);
            }

            if self.bookmark_drag.active && self.bookmark_drag.drop_index == Some(index) {
                paint.set_color(Color::from_rgb(70, 130, 240));
                paint.set_stroke(true);
                paint.set_stroke_width(2.0 * scale);
                canvas.draw_line(
                    (item_rect.left() - 2.0 * scale, item_rect.top() + 2.0 * scale),
                    (item_rect.left() - 2.0 * scale, item_rect.bottom() - 2.0 * scale),
                    &paint,
                );
                paint.set_stroke(false);
            }
        }

        if self.bookmark_drag.active && self.bookmark_drag.drop_index == Some(self.visible_root_bookmark_layout().len()) {
            paint.set_color(Color::from_rgb(70, 130, 240));
            paint.set_stroke(true);
            paint.set_stroke_width(2.0 * scale);
            let end_x = row_x + row_w - 8.0 * scale;
            canvas.draw_line(
                (end_x, row_y + 5.0 * scale),
                (end_x, row_y + row_h - 5.0 * scale),
                &paint,
            );
            paint.set_stroke(false);
        }

        if let Some(folder_id) = self.open_bookmark_folder.as_ref() {
            if let Some(folder) = Self::find_bookmark(&self.bookmarks, folder_id) {
                self.render_bookmark_folder_menu(canvas, font, folder);
            }
        }

        self.render_bookmark_context_menu(canvas, font);
    }

    fn render_bookmark_context_menu(&self, canvas: &Canvas, font: &Font) {
        let Some((x, y, w, h)) = self.bookmark_context_menu_rect() else {
            return;
        };

        let scale = self.viewport.hidpi_scale;
        let menu = self.bookmark_context_menu.as_ref().expect("context menu should exist when rect exists");
        let entries = self.context_menu_entries(menu.target_id.as_deref());
        let mut paint = Paint::default();
        let panel = Rect::from_xywh(x, y, w, h);
        paint.set_color(Color::from_rgb(252, 252, 252));
        canvas.draw_round_rect(panel, 6.0 * scale, 6.0 * scale, &paint);
        paint.set_color(Color::from_rgb(190, 190, 200));
        paint.set_stroke(true);
        paint.set_stroke_width(1.0 * scale);
        canvas.draw_round_rect(panel, 6.0 * scale, 6.0 * scale, &paint);
        paint.set_stroke(false);

        let row_h = Self::BOOKMARK_CONTEXT_ROW_HEIGHT * scale;
        for (index, (_, label)) in entries.iter().enumerate() {
            let row_y = y + index as f32 * row_h;
            let hover = self.pointer_is_in_rect(
                Rect::from_xywh(x + 1.0 * scale, row_y + 1.0 * scale, w - 2.0 * scale, row_h - 2.0 * scale),
            );
            if hover {
                paint.set_color(Color::from_rgb(227, 236, 251));
                canvas.draw_round_rect(
                    Rect::from_xywh(x + 2.0 * scale, row_y + 1.0 * scale, w - 4.0 * scale, row_h - 2.0 * scale),
                    4.0 * scale,
                    4.0 * scale,
                    &paint,
                );
            }
            if let Some(blob) = TextBlob::new(label, font) {
                let bounds = blob.bounds();
                let text_y = row_y + (row_h / 2.0) - (bounds.top + bounds.height() / 2.0);
                paint.set_color(Color::from_rgb(45, 45, 45));
                canvas.draw_text_blob(&blob, (x + 8.0 * scale, text_y), &paint);
            }
        }
    }

    fn render_bookmark_folder_menu(&self, canvas: &Canvas, font: &Font, folder: &BookmarkNode) {
        let scale = self.viewport.hidpi_scale;
        let Some((x, y, w, h)) = self.bookmark_folder_menu_rect(folder) else {
            return;
        };

        let mut paint = Paint::default();
        let panel = Rect::from_xywh(x, y, w, h);
        paint.set_color(Color::from_rgb(252, 252, 252));
        canvas.draw_round_rect(panel, 6.0 * scale, 6.0 * scale, &paint);
        paint.set_color(Color::from_rgb(190, 190, 200));
        paint.set_stroke(true);
        paint.set_stroke_width(1.0 * scale);
        canvas.draw_round_rect(panel, 6.0 * scale, 6.0 * scale, &paint);
        paint.set_stroke(false);

        let row_h = 28.0 * scale;
        for (index, bookmark) in folder.children.iter().enumerate() {
            let row_y = y + index as f32 * row_h;
            if row_y + row_h > y + h {
                break;
            }

            let is_selected = self.selected_bookmark_id.as_deref() == Some(bookmark.id.as_str());
            let is_hovered = self.bookmark_hover_id.as_deref() == Some(bookmark.id.as_str());
            let is_pressed = self.bookmark_pressed_id.as_deref() == Some(bookmark.id.as_str());
            if is_selected || is_hovered || is_pressed {
                let selected_row = Rect::from_xywh(x + 2.0 * scale, row_y + 1.0 * scale, w - 4.0 * scale, row_h - 2.0 * scale);
                paint.set_color(if is_pressed {
                    Color::from_rgb(203, 217, 240)
                } else {
                    Color::from_rgb(221, 235, 255)
                });
                canvas.draw_round_rect(selected_row, 4.0 * scale, 4.0 * scale, &paint);
            }

            let icon_rect = Rect::from_xywh(x + 8.0 * scale, row_y + 6.0 * scale, 16.0 * scale, 16.0 * scale);
            if bookmark.is_folder() {
                Self::render_svg_on_canvas(canvas, &self.folder_svg, icon_rect, Color::from_rgb(208, 166, 92));
            } else {
                let favicon = self.bookmark_favicons.get(&bookmark.id).and_then(|img| img.as_ref());
                Self::draw_tab_favicon(canvas, &mut paint, icon_rect, favicon);
            }

            let label = if bookmark.is_folder() {
                format!("{} v", bookmark.title)
            } else {
                bookmark.title.clone()
            };
            let text = Self::truncate_text_to_width(&label, w - (36.0 * scale), font);
            if let Some(blob) = TextBlob::new(&text, font) {
                let bounds = blob.bounds();
                let text_y = row_y + (row_h / 2.0) - (bounds.top + bounds.height() / 2.0);
                paint.set_color(Color::from_rgb(40, 40, 40));
                canvas.draw_text_blob(&blob, (icon_rect.right() + 6.0 * scale, text_y), &paint);
            }
        }
    }

    /// Update mouse hover state and handle tooltips
    pub fn update_mouse_hover(&mut self, x: f32, y: f32, current_time: Instant) {
        self.mouse_pos = (x, y);
        self.bookmark_hover_id = self.bookmark_at_point(x, y).map(|(id, _)| id);

        for comp in &mut self.components {
            let is_hovering = comp.contains_point(x, y);

            match comp {
                UiComponent::Button { is_hover, tooltip, .. } => {
                    if is_hovering && !*is_hover {
                        // Just started hovering
                        *is_hover = true;
                        tooltip.hover_start = Some(current_time);
                        tooltip.is_visible = false;
                    } else if !is_hovering && *is_hover {
                        // Stopped hovering
                        *is_hover = false;
                        tooltip.hover_start = None;
                        tooltip.is_visible = false;
                    } else if is_hovering && *is_hover {
                        // Continue hovering - check if tooltip should be shown
                        if let Some(hover_start) = tooltip.hover_start {
                            if current_time.duration_since(hover_start) >= tooltip.show_after {
                                tooltip.is_visible = true;
                            }
                        }
                    }
                }
                UiComponent::TabButton { x: tab_x, y: tab_y, width, height, is_active, is_hover, tooltip, close_button_hover, close_button_tooltip, .. } => {
                    // Check if hovering over close button specifically (inline calculation to avoid borrowing issues)
                    let is_over_close_button = if *is_active {
                        let close_button_size = 16.0 * self.viewport.hidpi_scale;
                        let close_button_x = *tab_x + *width - close_button_size - (4.0 * self.viewport.hidpi_scale);
                        let close_button_y = *tab_y + (*height / 2.0) - (close_button_size / 2.0);
                        x >= close_button_x && x <= close_button_x + close_button_size &&
                        y >= close_button_y && y <= close_button_y + close_button_size
                    } else {
                        false
                    };

                    // Handle close button hover state
                    if is_over_close_button && !*close_button_hover {
                        // Just started hovering over close button
                        *close_button_hover = true;
                        close_button_tooltip.hover_start = Some(current_time);
                        close_button_tooltip.is_visible = false;
                        // Clear parent tab tooltip when over close button
                        tooltip.is_visible = false;
                        tooltip.hover_start = None;
                    } else if !is_over_close_button && *close_button_hover {
                        // Stopped hovering over close button
                        *close_button_hover = false;
                        close_button_tooltip.hover_start = None;
                        close_button_tooltip.is_visible = false;
                    } else if is_over_close_button && *close_button_hover {
                        // Continue hovering over close button
                        if let Some(hover_start) = close_button_tooltip.hover_start {
                            if current_time.duration_since(hover_start) >= close_button_tooltip.show_after {
                                close_button_tooltip.is_visible = true;
                            }
                        }
                    }

                    // Handle parent tab button hover state (only if not over close button)
                    if is_hovering && !is_over_close_button && !*is_hover {
                        // Just started hovering over tab (but not close button)
                        *is_hover = true;
                        tooltip.hover_start = Some(current_time);
                        tooltip.is_visible = false;
                    } else if (!is_hovering || is_over_close_button) && *is_hover {
                        // Stopped hovering over tab area
                        *is_hover = false;
                        tooltip.hover_start = None;
                        tooltip.is_visible = false;
                    } else if is_hovering && !is_over_close_button && *is_hover {
                        // Continue hovering over tab - check if tooltip should be shown
                        if let Some(hover_start) = tooltip.hover_start {
                            if current_time.duration_since(hover_start) >= tooltip.show_after {
                                tooltip.is_visible = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn pointer_is_in_rect(&self, rect: Rect) -> bool {
        let x = self.mouse_pos.0;
        let y = self.mouse_pos.1;
        x >= rect.left() && x <= rect.right() && y >= rect.top() && y <= rect.bottom()
    }

    /// Check tooltip timeouts and update visibility (returns true if any tooltip visibility changed)
    pub fn update_tooltip_visibility(&mut self, current_time: Instant) -> bool {
        let mut changed = false;

        for comp in &mut self.components {
            match comp {
                UiComponent::Button { is_hover: true, tooltip, .. } => {
                    if !tooltip.is_visible {
                        if let Some(hover_start) = tooltip.hover_start {
                            if current_time.duration_since(hover_start) >= tooltip.show_after {
                                tooltip.is_visible = true;
                                changed = true;
                            }
                        }
                    }
                }
                UiComponent::TabButton { is_hover, tooltip, close_button_hover, close_button_tooltip, .. } => {
                    // Check parent tab tooltip
                    if *is_hover && !tooltip.is_visible {
                        if let Some(hover_start) = tooltip.hover_start {
                            if current_time.duration_since(hover_start) >= tooltip.show_after {
                                tooltip.is_visible = true;
                                changed = true;
                            }
                        }
                    }
                    // Check close button tooltip
                    if *close_button_hover && !close_button_tooltip.is_visible {
                        if let Some(hover_start) = close_button_tooltip.hover_start {
                            if current_time.duration_since(hover_start) >= close_button_tooltip.show_after {
                                close_button_tooltip.is_visible = true;
                                changed = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        changed
    }

    /// Handle mouse press
    pub fn handle_mouse_press(&mut self, x: f32, y: f32) -> Option<String> {
        for comp in &mut self.components {
            if comp.contains_point(x, y) {
                if let UiComponent::Button { is_pressed, .. } = comp {
                    *is_pressed = true;
                }
                return Some(comp.id().to_string());
            }
        }
        None
    }

    /// Handle mouse release
    pub fn handle_mouse_release(&mut self, x: f32, y: f32) -> Option<String> {
        let mut clicked_id = None;

        for comp in &mut self.components {
            if let UiComponent::Button { is_pressed, id, x: bx, y: by, width, height, .. } = comp {
                let was_pressed = *is_pressed;
                let contains_point = x >= *bx && x <= *bx + *width && y >= *by && y <= *by + *height;

                if was_pressed && contains_point {
                    clicked_id = Some(id.clone());
                }
                *is_pressed = false;
            }
        }

        clicked_id
    }

    /// Draw a custom icon based on icon type
    fn draw_icon(&self, painter: &mut ScenePainter, icon_type: &IconType, rect: Rect, is_hover: bool, hidpi_scale: f32) {
        let center_x = rect.center_x() as f64;
        let center_y = rect.center_y() as f64;
        let icon_size = (rect.width().min(rect.height()) * 0.6) as f64;
        let half_size = icon_size / 2.0;

        // Set up stroke style
        let stroke_width = 2.0 * hidpi_scale as f64;
        let stroke = kurbo::Stroke::new(stroke_width)
            .with_caps(kurbo::Cap::Round)
            .with_join(kurbo::Join::Round);

        // Icon color (dark gray for most icons)
        let icon_color = AlphaColor::from_rgba8(60, 60, 60, 255);
        let hover_color = AlphaColor::from_rgba8(200, 50, 50, 255); // Red for close icon when hovering

        match icon_type {
            IconType::Back => {
                Self::render_svg(painter, &self.back_svg, rect, icon_color, hidpi_scale);
            }
            IconType::Forward => {
                Self::render_svg(painter, &self.forward_svg, rect, icon_color, hidpi_scale);
            }
            IconType::Refresh => {
                Self::render_svg(painter, &self.reload_svg, rect, icon_color, hidpi_scale);
            }
            IconType::Home => {
                Self::render_svg(painter, &self.home_svg, rect, icon_color, hidpi_scale);
            }
            IconType::Bookmark => {
                let cx = rect.center_x() as f64;
                let top = rect.top() as f64 + rect.height() as f64 * 0.2;
                let bottom = rect.bottom() as f64 - rect.height() as f64 * 0.2;
                let left = rect.left() as f64 + rect.width() as f64 * 0.28;
                let right = rect.right() as f64 - rect.width() as f64 * 0.28;
                let tip_y = rect.bottom() as f64 - rect.height() as f64 * 0.2;

                let mut path = kurbo::BezPath::new();
                path.move_to((left, top));
                path.line_to((right, top));
                path.line_to((right, bottom));
                path.line_to((cx, tip_y));
                path.line_to((left, bottom));
                path.close_path();

                painter.stroke(&stroke, Affine::IDENTITY, icon_color, None, &path);
            }
            IconType::NewTab => {
                Self::render_svg(painter, &self.new_tab_svg, rect, icon_color, hidpi_scale);
            }
            IconType::Close => {
                let color = if is_hover { hover_color } else { icon_color };
                Self::render_svg(painter, &self.close_tab_svg, rect, color, hidpi_scale);
            }
            IconType::Settings => {
                Self::render_svg(painter, &self.settings_svg, rect, icon_color, hidpi_scale);
            }
        }
    }

    /// Render an SVG tree into a rect
    fn render_svg(painter: &mut ScenePainter, tree: &Tree, rect: Rect, color: AlphaColor<Srgb>, hidpi_scale: f32) {
        // Save canvas state before SVG rendering
        painter.inner.save();

        let svg_size = tree.size();

        // Calculate scale to fit the SVG into the rect
        let scale_x = (rect.width() as f64 * 0.8) / svg_size.width() as f64;
        let scale_y = (rect.height() as f64 * 0.8) / svg_size.height() as f64;
        let scale = scale_x.min(scale_y);

        // Center the SVG in the rect
        let offset_x = rect.left() as f64 + (rect.width() as f64 - svg_size.width() as f64 * scale) / 2.0;
        let offset_y = rect.top() as f64 + (rect.height() as f64 - svg_size.height() as f64 * scale) / 2.0;

        let transform = Affine::translate((offset_x, offset_y)) * Affine::scale(scale);

        // Render all paths in the SVG
        for node in tree.root().children() {
            Self::render_svg_node(painter, node, transform, color);
        }

        // Restore canvas state after SVG rendering
        painter.inner.restore();
    }

    /// Recursively render SVG nodes
    fn render_svg_node(painter: &mut ScenePainter, node: &usvg::Node, transform: Affine, color: AlphaColor<Srgb>) {
        match node {
            usvg::Node::Group(group) => {
                let group_transform = Self::usvg_transform_to_affine(&group.transform());
                let combined_transform = transform * group_transform;

                for child in group.children() {
                    Self::render_svg_node(painter, child, combined_transform, color);
                }
            }
            usvg::Node::Path(path) => {
                let path_transform = Self::usvg_transform_to_affine(&path.abs_transform());
                let combined_transform = transform * path_transform;

                // Convert usvg path to kurbo path
                let kurbo_path = Self::usvg_path_to_kurbo(path.data());

                // Render based on paint type
                if let Some(ref stroke) = path.stroke() {
                    let stroke_width = stroke.width().get() as f64;
                    let kurbo_stroke = kurbo::Stroke::new(stroke_width)
                        .with_caps(kurbo::Cap::Round)
                        .with_join(kurbo::Join::Round);

                    painter.stroke(&kurbo_stroke, combined_transform, color, None, &kurbo_path);
                }

                if path.fill().is_some() {
                    painter.fill(Fill::NonZero, combined_transform, color, None, &kurbo_path);
                }
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => {
                // We don't need to handle images or text for simple icons
            }
        }
    }

    /// Convert usvg Transform to kurbo Affine
    fn usvg_transform_to_affine(transform: &usvg::Transform) -> Affine {
        Affine::new([
            transform.sx as f64,
            transform.ky as f64,
            transform.kx as f64,
            transform.sy as f64,
            transform.tx as f64,
            transform.ty as f64,
        ])
    }

    /// Convert usvg path data to kurbo BezPath
    fn usvg_path_to_kurbo(path_data: &usvg::tiny_skia_path::Path) -> kurbo::BezPath {
        use usvg::tiny_skia_path::PathSegment;

        let mut kurbo_path = kurbo::BezPath::new();

        for segment in path_data.segments() {
            match segment {
                PathSegment::MoveTo(p) => {
                    kurbo_path.move_to((p.x as f64, p.y as f64));
                }
                PathSegment::LineTo(p) => {
                    kurbo_path.line_to((p.x as f64, p.y as f64));
                }
                PathSegment::QuadTo(p1, p2) => {
                    kurbo_path.quad_to((p1.x as f64, p1.y as f64), (p2.x as f64, p2.y as f64));
                }
                PathSegment::CubicTo(p1, p2, p3) => {
                    kurbo_path.curve_to(
                        (p1.x as f64, p1.y as f64),
                        (p2.x as f64, p2.y as f64),
                        (p3.x as f64, p3.y as f64),
                    );
                }
                PathSegment::Close => {
                    kurbo_path.close_path();
                }
            }
        }

        kurbo_path
    }

    fn draw_tab_favicon(canvas: &Canvas, paint: &mut Paint, rect: Rect, favicon: Option<&Image>) {
        if let Some(image) = favicon {
            let image_w = image.width() as f32;
            let image_h = image.height() as f32;
            if image_w > 0.0 && image_h > 0.0 {
                canvas.save();
                canvas.translate((rect.left(), rect.top()));
                canvas.scale((rect.width() / image_w, rect.height() / image_h));
                canvas.draw_image(image, (0.0, 0.0), None);
                canvas.restore();
            } else {
                Self::draw_default_favicon(canvas, paint, rect);
            }
        } else {
            Self::draw_default_favicon(canvas, paint, rect);
        }
    }

    fn draw_default_favicon(canvas: &Canvas, paint: &mut Paint, rect: Rect) {
        paint.set_color(Color::from_rgb(220, 226, 236));
        canvas.draw_round_rect(rect, 3.0, 3.0, paint);

        paint.set_stroke(true);
        paint.set_stroke_width(1.0);
        paint.set_color(Color::from_rgb(110, 125, 150));
        canvas.draw_round_rect(rect, 3.0, 3.0, paint);

        paint.set_stroke_width(0.9);
        let cx = rect.center_x();
        let cy = rect.center_y();
        let rx = rect.width() * 0.28;
        let ry = rect.height() * 0.28;
        canvas.draw_line((cx - rx, cy), (cx + rx, cy), paint);
        canvas.draw_line((cx, cy - ry), (cx, cy + ry), paint);

        paint.set_stroke(false);
    }

    fn render_svg_on_canvas(canvas: &Canvas, _tree: &Tree, rect: Rect, color: Color) {
        // The folder glyph is drawn from simple geometry but still uses the folder asset pipeline.
        let mut paint = Paint::default();
        paint.set_color(color);

        let tab_h = rect.height() * 0.34;
        let tab_w = rect.width() * 0.44;
        let tab = Rect::from_xywh(rect.left() + rect.width() * 0.06, rect.top() + rect.height() * 0.08, tab_w, tab_h);
        canvas.draw_round_rect(tab, 2.0, 2.0, &paint);

        let body = Rect::from_xywh(rect.left() + rect.width() * 0.02, rect.top() + rect.height() * 0.28, rect.width() * 0.96, rect.height() * 0.64);
        canvas.draw_round_rect(body, 2.5, 2.5, &paint);

        paint.set_stroke(true);
        paint.set_stroke_width((rect.width() * 0.06).max(1.0));
        paint.set_color(Color::from_rgb(145, 108, 50));
        canvas.draw_round_rect(body, 2.5, 2.5, &paint);
        paint.set_stroke(false);
    }


    /// Draw a tooltip
    fn draw_tooltip(painter: &mut ScenePainter, tooltip: &Tooltip, x: f32, y: f32, font: &Font, hidpi_scale: f32, canvas_width: f32, canvas_height: f32) {
        if !tooltip.is_visible {
            return;
        }

        let padding = 8.0 * hidpi_scale;
        let lines: Vec<&str> = tooltip.text.lines().collect();
        if lines.is_empty() {
            return;
        }

        let (_, sample_bounds) = font.measure_str("Ag", None);
        let line_height = sample_bounds.height().max(font.size());
        let baseline_offset = -sample_bounds.top;
        let line_spacing = 2.0 * hidpi_scale;

        let max_line_width = lines.iter()
            .map(|line| {
                if line.is_empty() {
                    0.0
                } else {
                    font.measure_str(line, None).0
                }
            })
            .fold(0.0_f32, f32::max);

        let tooltip_width = (max_line_width + padding * 2.0) as f64;
        let tooltip_height = (line_height * lines.len() as f32
            + line_spacing * lines.len().saturating_sub(1) as f32
            + padding * 2.0) as f64;

        let mut tooltip_x = x as f64;
        let mut tooltip_y = y as f64 - tooltip_height - 5.0;

        let margin = (4.0 * hidpi_scale) as f64;

        if tooltip_x + tooltip_width > canvas_width as f64 - margin {
            tooltip_x = canvas_width as f64 - tooltip_width - margin;
        }

        if tooltip_x < margin {
            tooltip_x = margin;
        }

        if tooltip_y < margin {
            tooltip_y = y as f64 + 32.0 * hidpi_scale as f64 + 5.0;
        }

        if tooltip_y + tooltip_height > canvas_height as f64 - margin {
            tooltip_y = canvas_height as f64 - tooltip_height - margin;
        }

        let transform = Affine::IDENTITY;

        // Draw tooltip background with shadow
        let shadow_rect = kurbo::RoundedRect::from_rect(
            kurbo::Rect::new(tooltip_x + 2.0, tooltip_y + 2.0, tooltip_x + tooltip_width + 2.0, tooltip_y + tooltip_height + 2.0),
            4.0
        );
        let shadow_color = AlphaColor::from_rgba8(0, 0, 0, 100); // Semi-transparent black shadow
        painter.fill(Fill::NonZero, transform, shadow_color, None, &shadow_rect);

        // Draw tooltip background
        let tooltip_rect = kurbo::RoundedRect::from_rect(
            kurbo::Rect::new(tooltip_x, tooltip_y, tooltip_x + tooltip_width, tooltip_y + tooltip_height),
            4.0
        );
        let bg_color = AlphaColor::from_rgba8(255, 255, 220, 255); // Light yellow background
        painter.fill(Fill::NonZero, transform, bg_color, None, &tooltip_rect);

        // Draw tooltip border
        let stroke = kurbo::Stroke::new(1.0 * hidpi_scale as f64);
        let border_color = AlphaColor::from_rgba8(180, 180, 140, 255);
        painter.stroke(&stroke, transform, border_color, None, &tooltip_rect);

        // Draw tooltip text using canvas directly (TextBlob is Skia-specific)
        painter.set_matrix(transform);
        let mut paint = Paint::default();
        paint.set_color(Color::BLACK);

        for (index, line) in lines.iter().enumerate() {
            let draw_line = if line.is_empty() { " " } else { line };

            if let Some(text_blob) = TextBlob::new(draw_line, font) {
                let text_y = tooltip_y as f32
                    + padding
                    + baseline_offset
                    + index as f32 * (line_height + line_spacing);
                painter.inner.draw_text_blob(&text_blob, (tooltip_x as f32 + padding, text_y), &paint);
            }
        }
    }

    /// Draw a loading spinner indicator
    /// `angle` is the current rotation angle in radians (0 to 2*PI)
    pub fn render_loading_indicator(&self, painter: &mut ScenePainter, is_loading: bool, angle: f32) {
        if !is_loading {
            return;
        }

        // Find the address bar to position the spinner
        for comp in &self.components {
            if let UiComponent::TextField { id, x, y, width, height, .. } = comp {
                if id == "address_bar" {
                    // Position spinner at the right side of the address bar
                    let spinner_size = 20.0 * self.viewport.hidpi_scale;
                    let spinner_x = x + width - spinner_size - (8.0 * self.viewport.hidpi_scale);
                    let spinner_y = y + (height / 2.0);

                    Self::draw_spinner(painter, spinner_x, spinner_y, spinner_size / 2.0, angle, self.viewport.hidpi_scale);
                    break;
                }
            }
        }
    }

    /// Draw an animated spinner
    fn draw_spinner(painter: &mut ScenePainter, center_x: f32, center_y: f32, radius: f32, angle: f32, hidpi_scale: f32) {
        let stroke_width = 2.5 * hidpi_scale as f64;
        let stroke = kurbo::Stroke::new(stroke_width).with_caps(kurbo::Cap::Round);

        // Draw multiple arcs with varying opacity for a smooth spinner effect
        let num_segments = 8;
        for i in 0..num_segments {
            let segment_angle = angle as f64 + (i as f64 * 2.0 * PI as f64 / num_segments as f64);
            let start_angle = segment_angle.to_degrees();

            // Fade out older segments
            let alpha = ((num_segments - i) as f32 / num_segments as f32 * 255.0) as u8;
            let color = AlphaColor::from_rgba8(50, 120, 255, alpha);

            let sweep_angle = 30.0_f64.to_radians(); // Convert to radians for kurbo

            // Create arc using kurbo
            let arc = kurbo::Arc {
                center: kurbo::Point::new(center_x as f64, center_y as f64),
                radii: kurbo::Vec2::new(radius as f64, radius as f64),
                start_angle: start_angle.to_radians(),
                sweep_angle,
                x_rotation: 0.0,
            };

            painter.stroke(&stroke, Affine::IDENTITY, color, None, &arc);
        }
    }

    /// Start dragging a tab
    pub fn start_tab_drag(&mut self, x: f32, y: f32) -> bool {
        // Find which tab was clicked
        let mut found_tab: Option<(String, f32, usize)> = None;
        let mut tab_index = 0;

        for comp in &self.components {
            if let UiComponent::TabButton { id, x: tab_x, y: tab_y, width, height, .. } = comp {
                if x >= *tab_x && x <= *tab_x + *width && y >= *tab_y && y <= *tab_y + *height {
                    found_tab = Some((id.clone(), *tab_x, tab_index));
                    break;
                }
                tab_index += 1;
            }
        }

        if let Some((tab_id, original_x, index)) = found_tab {
            self.tab_drag_state = TabDragState {
                is_dragging: true,
                dragged_tab_id: Some(tab_id),
                drag_start_x: x,
                original_tab_x: original_x,
                drag_offset: 0.0,
                original_index: index,
                drag_threshold_exceeded: false,
            };
            true
        } else {
            false
        }
    }

    /// Update tab position during drag
    pub fn update_tab_drag(&mut self, x: f32) -> Option<(usize, usize)> {
        if !self.tab_drag_state.is_dragging {
            return None;
        }

        let drag_offset = x - self.tab_drag_state.drag_start_x;
        self.tab_drag_state.drag_offset = drag_offset;

        // Check if drag threshold has been exceeded (5 pixels)
        const DRAG_THRESHOLD: f32 = 5.0;
        if !self.tab_drag_state.drag_threshold_exceeded && drag_offset.abs() > DRAG_THRESHOLD {
            self.tab_drag_state.drag_threshold_exceeded = true;
        }

        // Only update visual position if threshold has been exceeded
        if !self.tab_drag_state.drag_threshold_exceeded {
            return None;
        }

        // Calculate the new X position for the dragged tab
        let new_tab_x = self.tab_drag_state.original_tab_x + drag_offset;

        // Get tab info for reordering calculation
        let scaled_spacing = Self::TAB_SPACING * self.viewport.hidpi_scale;
        let tab_width = self.calculate_tab_width();

        // Find the dragged tab's current center
        let dragged_center_x = new_tab_x + tab_width / 2.0;

        // Find which position the tab should be swapped to
        let mut target_index: Option<usize> = None;
        let mut current_tab_index = 0;

        for comp in &self.components {
            if let UiComponent::TabButton { id, x: tab_x, width, .. } = comp {
                if Some(id.clone()) == self.tab_drag_state.dragged_tab_id {
                    current_tab_index += 1;
                    continue;
                }

                let tab_center = *tab_x + *width / 2.0;

                // Check if the dragged tab's center has crossed this tab's center
                if current_tab_index < self.tab_drag_state.original_index {
                    // Moving left: swap when we pass the center of a tab to the left
                    if dragged_center_x < tab_center + *width / 2.0 {
                        target_index = Some(current_tab_index);
                        break;
                    }
                } else {
                    // Moving right: swap when we pass the center of a tab to the right
                    if dragged_center_x > tab_center - *width / 2.0 {
                        target_index = Some(current_tab_index);
                    }
                }
                current_tab_index += 1;
            }
        }

        // Update the dragged tab's visual position
        if let Some(dragged_id) = &self.tab_drag_state.dragged_tab_id {
            for comp in &mut self.components {
                if let UiComponent::TabButton { id, x, .. } = comp {
                    if id == dragged_id {
                        *x = new_tab_x;
                        break;
                    }
                }
            }
        }

        // Return the swap info if needed
        if let Some(target) = target_index {
            if target != self.tab_drag_state.original_index {
                return Some((self.tab_drag_state.original_index, target));
            }
        }

        None
    }

    /// End tab dragging and return the final reorder info (from_index, to_index) if tabs were reordered
    pub fn end_tab_drag(&mut self) -> Option<(usize, usize)> {
        if !self.tab_drag_state.is_dragging {
            return None;
        }

        // If threshold wasn't exceeded, treat as a click
        let threshold_exceeded = self.tab_drag_state.drag_threshold_exceeded;

        let drag_offset = self.tab_drag_state.drag_offset;
        let tab_width = self.calculate_tab_width();
        let scaled_spacing = Self::TAB_SPACING * self.viewport.hidpi_scale;
        let tab_slot_width = tab_width + scaled_spacing;

        // Calculate how many positions to move based on drag distance
        let positions_moved = (drag_offset / tab_slot_width).round() as i32;
        let from_index = self.tab_drag_state.original_index;
        let tab_count = self.components.iter()
            .filter(|c| matches!(c, UiComponent::TabButton { .. }))
            .count();

        // Calculate target index with bounds checking
        let to_index = if positions_moved < 0 {
            from_index.saturating_sub((-positions_moved) as usize)
        } else {
            (from_index + positions_moved as usize).min(tab_count.saturating_sub(1))
        };

        // Reset drag state
        self.tab_drag_state = TabDragState::default();

        // Update tab layout to restore proper positions
        self.update_tab_layout();

        // Only return reorder info if threshold was exceeded and tabs actually moved
        if threshold_exceeded && from_index != to_index {
            Some((from_index, to_index))
        } else {
            None
        }
    }

    /// Cancel tab dragging without applying changes
    pub fn cancel_tab_drag(&mut self) {
        self.tab_drag_state = TabDragState::default();
        self.update_tab_layout();
    }

    /// Check if a tab is currently being dragged
    pub fn is_dragging_tab(&self) -> bool {
        self.tab_drag_state.is_dragging
    }

    /// Check if a tab drag is actively happening (threshold exceeded)
    pub fn is_actively_dragging_tab(&self) -> bool {
        self.tab_drag_state.is_dragging && self.tab_drag_state.drag_threshold_exceeded
    }

    /// Reorder tabs in the UI components list
    pub fn reorder_tabs(&mut self, from_index: usize, to_index: usize) {
        // Get all tab indices in the components vec
        let tab_indices: Vec<usize> = self.components.iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if matches!(c, UiComponent::TabButton { .. }) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        if from_index >= tab_indices.len() || to_index >= tab_indices.len() {
            return;
        }

        let from_comp_index = tab_indices[from_index];
        let to_comp_index = tab_indices[to_index];

        // Remove the tab from its original position
        let tab = self.components.remove(from_comp_index);

        // Adjust the target index if necessary
        let adjusted_to_index = if from_comp_index < to_comp_index {
            to_comp_index - 1
        } else {
            to_comp_index
        };

        // Insert at the new position
        self.components.insert(adjusted_to_index, tab);

        // Update layout to recalculate positions
        self.update_tab_layout();
    }

    // Helper: find previous character boundary strictly before or at byte_pos
    fn prev_char_boundary(s: &str, byte_pos: usize) -> usize {
        if byte_pos == 0 {
            return 0;
        }
        // Walk char_indices and keep the last index < byte_pos
        let mut prev = 0usize;
        for (i, _) in s.char_indices() {
            if i >= byte_pos {
                break;
            }
            prev = i;
        }
        prev
    }

    // Helper: find next character boundary strictly after byte_pos, or s.len()
    fn next_char_boundary(s: &str, byte_pos: usize) -> usize {
        if byte_pos >= s.len() {
            return s.len();
        }
        for (i, _) in s.char_indices() {
            if i > byte_pos {
                return i;
            }
        }
        s.len()
    }

    fn prev_word_boundary(s: &str, byte_pos: usize) -> usize {
        if s.is_empty() {
            return 0;
        }

        let mut idx = byte_pos.min(s.len());
        while idx > 0 && !s.is_char_boundary(idx) {
            idx -= 1;
        }

        while idx > 0 {
            let prev = Self::prev_char_boundary(s, idx);
            let ch = s[prev..idx].chars().next().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            idx = prev;
        }

        while idx > 0 {
            let prev = Self::prev_char_boundary(s, idx);
            let ch = s[prev..idx].chars().next().unwrap_or(' ');
            if ch.is_whitespace() {
                break;
            }
            idx = prev;
        }

        idx
    }

    fn next_word_boundary(s: &str, byte_pos: usize) -> usize {
        if s.is_empty() {
            return 0;
        }

        let mut idx = byte_pos.min(s.len());
        while idx < s.len() && !s.is_char_boundary(idx) {
            idx += 1;
        }

        while idx < s.len() {
            let next = Self::next_char_boundary(s, idx);
            let ch = s[idx..next].chars().next().unwrap_or(' ');
            if ch.is_whitespace() {
                break;
            }
            idx = next;
        }

        while idx < s.len() {
            let next = Self::next_char_boundary(s, idx);
            let ch = s[idx..next].chars().next().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            idx = next;
        }

        idx
    }

    fn selected_text_for_range(text: &str, start: Option<usize>, end: Option<usize>) -> Option<String> {
        let (s, e) = Self::selection_range(text, start, end)?;
        Some(text[s..e].to_string())
    }

    fn selection_range(text: &str, start: Option<usize>, end: Option<usize>) -> Option<(usize, usize)> {
        let (Some(start), Some(end)) = (start, end) else {
            return None;
        };

        let mut s = start.min(end).min(text.len());
        let mut e = start.max(end).min(text.len());

        if !text.is_char_boundary(s) {
            s = Self::prev_char_boundary(text, s);
        }
        if !text.is_char_boundary(e) {
            e = Self::next_char_boundary(text, e);
        }

        if s < e {
            Some((s, e))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BrowserUI;

    #[test]
    fn selected_text_for_range_handles_forward_selection() {
        assert_eq!(
            BrowserUI::selected_text_for_range("hello world", Some(0), Some(5)),
            Some("hello".to_string())
        );
    }

    #[test]
    fn selected_text_for_range_handles_reversed_selection() {
        assert_eq!(
            BrowserUI::selected_text_for_range("hello world", Some(5), Some(0)),
            Some("hello".to_string())
        );
    }

    #[test]
    fn selected_text_for_range_returns_none_for_empty_selection() {
        assert_eq!(
            BrowserUI::selected_text_for_range("hello world", Some(3), Some(3)),
            None
        );
    }

    #[test]
    fn selected_text_for_range_snaps_to_utf8_boundaries() {
        assert_eq!(
            BrowserUI::selected_text_for_range("a🙂b", Some(2), Some(4)),
            Some("🙂".to_string())
        );
    }
}

