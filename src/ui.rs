use blitz_traits::shell::Viewport;
use skia_safe::{Canvas, Color, Font, FontStyle, Paint, Path, Rect, TextBlob};
use std::f32::consts::PI;
use std::time::{Duration, Instant};

/// Tooltip information
#[derive(Debug, Clone)]
pub struct Tooltip {
    pub text: String,
    pub show_after: Duration,
    pub hover_start: Option<Instant>,
    pub is_visible: bool,
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
    }
}

/// Icon types for buttons
#[derive(Debug, Clone)]
pub enum IconType {
    Back,
    Forward,
    Refresh,
    NewTab,
    Close,
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
            tooltip: Tooltip::new(&format!("Switch to {}", title)),
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

/// Represents the browser UI (chrome)
pub struct BrowserUI {
    pub components: Vec<UiComponent>,
    pub viewport: Viewport,
    tab_scroll_offset: f32,  // Horizontal scroll offset for tabs
}

impl BrowserUI {
    // UI layout constants
    const CHROME_HEIGHT: f32 = 88.0;
    const BUTTON_SIZE: f32 = 32.0;
    const BUTTON_MARGIN: f32 = 8.0;
    const ADDRESS_BAR_HEIGHT: f32 = 32.0;
    const ADDRESS_BAR_MARGIN: f32 = 8.0;
    const MIN_ADDRESS_BAR_WIDTH: f32 = 200.0;
    const MAX_TAB_WIDTH: f32 = 200.0;  // Maximum width for a tab
    const MIN_TAB_WIDTH: f32 = 80.0;   // Minimum width before scrolling kicks in
    const TAB_SPACING: f32 = 4.0;       // Spacing between tabs

    pub fn new(_skia_context: &skia_safe::gpu::DirectContext, viewport: &Viewport) -> Self {
        // Default window width, will be updated on first resize
        let window_width = viewport.window_size.0 as f32;
        let scale_factor = viewport.hidpi_scale;
        let scaled = |v: f32| v * scale_factor;

        Self {
            components: vec![
                UiComponent::navigation_button("back", "<", scaled(Self::BUTTON_MARGIN), IconType::Back, "Back", scale_factor),
                UiComponent::navigation_button("forward", ">", scaled(Self::BUTTON_MARGIN * 2.0 + Self::BUTTON_SIZE), IconType::Forward, "Forward", scale_factor),
                UiComponent::navigation_button("refresh", "⟳", scaled(Self::BUTTON_MARGIN * 3.0 + Self::BUTTON_SIZE * 2.0), IconType::Refresh, "Refresh", scale_factor),
                UiComponent::navigation_button("new_tab", "+", window_width - scaled(Self::BUTTON_MARGIN + Self::BUTTON_SIZE), IconType::NewTab, "New Tab", scale_factor),
                UiComponent::address_bar("",
                    scaled(Self::BUTTON_MARGIN * 4.0 + Self::BUTTON_SIZE * 3.0),
                    window_width - scaled(Self::BUTTON_MARGIN * 6.0 + Self::BUTTON_SIZE * 4.0), scale_factor)
            ],
            viewport: viewport.clone(),
            tab_scroll_offset: 0.0,
        }
    }

    /// Update UI layout when window is resized
    pub fn update_layout(&mut self, viewport: &Viewport) {
        self.viewport = viewport.clone();
        let scaled = |v: f32| v * self.viewport.hidpi_scale;

        // Update new tab button position (always on the right)
        let window_width = self.viewport.window_size.0 as f32;
        for comp in &mut self.components {
            if let UiComponent::Button { id, x, .. } = comp {
                if id == "new_tab" {
                    *x = window_width - scaled(Self::BUTTON_MARGIN + Self::BUTTON_SIZE);
                }
            }
        }

        // Update address bar width
        for comp in &mut self.components {
            if let UiComponent::TextField { id, width, is_flexible: true, .. } = comp {
                if id == "address_bar" {
                    let available_width = window_width - scaled(Self::BUTTON_MARGIN * 6.0 + Self::BUTTON_SIZE * 4.0);
                    *width = available_width.max(scaled(Self::MIN_ADDRESS_BAR_WIDTH));
                }
            }
        }

        // Update tab layout with dynamic sizing
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

        // Available width for tabs (use scaled margin)
        let window_width = self.window_width();
        let scaled_margin = Self::BUTTON_MARGIN * self.viewport.hidpi_scale;
        let available_width = window_width - (scaled_margin * 2.0);

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
        let tab_width = self.calculate_tab_width();
        let tab_count = self.components.iter()
            .filter(|c| matches!(c, UiComponent::TabButton { .. }))
            .count();

        // Calculate total width needed for all tabs (use scaled spacing)
        let scaled_spacing = Self::TAB_SPACING * self.viewport.hidpi_scale;
        let total_tab_width = tab_count as f32 * tab_width +
                              (tab_count.saturating_sub(1)) as f32 * scaled_spacing;

        // Update scroll offset bounds (use scaled margin)
        let scaled_margin = Self::BUTTON_MARGIN * self.viewport.hidpi_scale;
        let max_scroll = (total_tab_width - self.window_width() + scaled_margin * 2.0).max(0.0);
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

        // Only allow scrolling if tabs overflow (use scaled margin)
        let scaled_margin = Self::BUTTON_MARGIN * self.viewport.hidpi_scale;
        if total_tab_width > self.window_width() - scaled_margin * 2.0 {
            // Scroll by a portion of a tab width
            let scroll_amount = delta_y * 30.0; // Adjust sensitivity
            self.tab_scroll_offset -= scroll_amount;

            // Clamp scroll offset
            let max_scroll = total_tab_width - self.window_width() + scaled_margin * 2.0;
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
            if let UiComponent::TabButton { id, title: tab_title, .. } = comp {
                if id == tab_id {
                    *tab_title = title.to_string();
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

    /// Set focus to a specific component
    pub fn set_focus(&mut self, component_id: &str) {
        for comp in &mut self.components {
            match comp {
                UiComponent::TextField { id, has_focus, cursor_position, text, .. } => {
                    if id == component_id {
                        *has_focus = true;
                        *cursor_position = text.len(); // Move cursor to end
                    } else {
                        *has_focus = false;
                    }
                }
                _ => {}
            }
        }
    }

    /// Handle text input for focused component
    pub fn handle_text_input(&mut self, text: &str) {
        for comp in &mut self.components {
            if let UiComponent::TextField { has_focus: true, text: field_text, cursor_position, .. } = comp {
                // Ensure cursor is within bounds and at a char boundary
                if *cursor_position > field_text.len() {
                    *cursor_position = field_text.len();
                }
                if !field_text.is_char_boundary(*cursor_position) {
                    *cursor_position = Self::prev_char_boundary(field_text, *cursor_position);
                }

                // Insert text at cursor position (safe because we are at a char boundary)
                field_text.insert_str(*cursor_position, text);
                // Advance cursor to after inserted text
                *cursor_position = (*cursor_position).saturating_add(text.len());
                break;
            }
        }
    }

    /// Handle key input for text editing
    pub fn handle_key_input(&mut self, key: &str) -> Option<String> {
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

                match key {
                    "Backspace" => {
                        if *cursor_position > 0 {
                            let prev = Self::prev_char_boundary(field_text, *cursor_position);
                            // Remove the previous character (range from prev..cursor)
                            if prev < *cursor_position && *cursor_position <= field_text.len() {
                                field_text.replace_range(prev..*cursor_position, "");
                                *cursor_position = prev;
                            }
                        }
                    }
                    "Delete" => {
                        if *cursor_position < field_text.len() {
                            let next = Self::next_char_boundary(field_text, *cursor_position);
                            if *cursor_position < next && next <= field_text.len() {
                                field_text.replace_range(*cursor_position..next, "");
                            }
                        }
                    }
                    "ArrowLeft" => {
                        if *cursor_position > 0 {
                            *cursor_position = Self::prev_char_boundary(field_text, *cursor_position);
                        }
                    }
                    "ArrowRight" => {
                        if *cursor_position < field_text.len() {
                            *cursor_position = Self::next_char_boundary(field_text, *cursor_position);
                        }
                    }
                    "Home" => {
                        *cursor_position = 0;
                    }
                    "End" => {
                        *cursor_position = field_text.len();
                        // make sure it's on a char boundary
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
            if let UiComponent::TextField { has_focus, .. } = comp {
                *has_focus = false;
            }
        }
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
                if let (Some(&start), Some(&end)) = (selection_start.as_ref(), selection_end.as_ref()) {
                    let start = start.min(end);
                    let end = start.max(end);
                    if start < end && end <= text.len() {
                        return Some(text[start..end].to_string());
                    }
                }
                break;
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
                if *cursor_position <= text.len() {
                    text.insert_str(*cursor_position, insert_text);
                    *cursor_position += insert_text.len();
                }
                break;
            }
        }
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

    /// Render the UI
    pub fn render(&self, canvas: &Canvas) {
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
        let font_mgr = skia_safe::FontMgr::new();

        // Try to create a font that supports Unicode symbols
        let typeface = font_mgr.match_family_style("DejaVu Sans", FontStyle::default())
            .or_else(|| font_mgr.match_family_style("Noto Sans", FontStyle::default()))
            .or_else(|| font_mgr.match_family_style("Arial Unicode MS", FontStyle::default()))
            .or_else(|| font_mgr.match_family_style("Segoe UI Symbol", FontStyle::default()))
            .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::default()))
            .expect("Failed to create any typeface");

        // Apply scale factor to font size for proper DPI scaling
        let base_font_size = 14.0;
        let scaled_font_size = base_font_size * self.viewport.hidpi_scale;
        let font = Font::new(typeface.clone(), scaled_font_size);

        // Draw BROWSING WITH STOKES text in the top-right corner
        {
            // Draw "browsing the web" text in small-caps 18px Times New Roman
            let times_typeface = font_mgr.match_family_style("Times New Roman", FontStyle::bold_italic())
                .or_else(|| font_mgr.match_family_style("Liberation Serif", FontStyle::default()))
                .or_else(|| font_mgr.match_family_style("Times", FontStyle::default()))
                .or_else(|| font_mgr.match_family_style("serif", FontStyle::default()))
                .unwrap_or(typeface);

            let custom_font_size = 18.0 * self.viewport.hidpi_scale;
            let mut custom_font = Font::new(times_typeface, custom_font_size);

            // Enable small-caps by setting font features
            custom_font.set_subpixel(true);

            // Render text in small-caps style (manually convert to uppercase with smaller caps)
            let custom_text = "BROWSING WITH STOKES";
            paint.set_color(Color::from_rgb(60, 60, 60));

            if let Some(text_blob) = TextBlob::new(custom_text, &custom_font) {
                let text_bounds = text_blob.bounds();
                let text_x = canvas_width - text_bounds.width() - (20.0 * self.viewport.hidpi_scale);
                let text_y = 22.0 * self.viewport.hidpi_scale;
                canvas.draw_text_blob(&text_blob, (text_x, text_y), &paint);
            }
        }

        // Scale other text rendering properties
        let text_padding = 5.0 * self.viewport.hidpi_scale;
        let cursor_margin = 6.0 * self.viewport.hidpi_scale;
        let cursor_stroke_width = 1.5 * self.viewport.hidpi_scale;
        let shadow_offset = 2.0 * self.viewport.hidpi_scale;

        for comp in &self.components {
            match comp {
                UiComponent::Button { x, y, width, height, color, hover_color, pressed_color, is_pressed, is_hover, tooltip, icon_type, .. } => {
                    let rect = Rect::from_xywh(*x, *y, *width, *height);

                    // Draw button shadow for depth
                    let shadow_rect = Rect::from_xywh(*x + shadow_offset, *y + shadow_offset, *width, *height);
                    paint.set_color(Color::from_argb(50, 0, 0, 0)); // Semi-transparent shadow
                    canvas.draw_round_rect(shadow_rect, 4.0, 4.0, &paint);

                    // Choose color based on state
                    let current_color = if *is_pressed {
                        pressed_color
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
                    Self::draw_icon(canvas, icon_type, rect, self.viewport.hidpi_scale);

                    // Draw tooltip if visible
                    if tooltip.is_visible {
                        Self::draw_tooltip(canvas, tooltip, *x, *y, &font, self.viewport.hidpi_scale);
                    }
                }
                UiComponent::TextField { text, x, y, width, height, color, border_color, has_focus, cursor_position, .. } => {
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
                UiComponent::TabButton { title, x, y, width, height, color, hover_color, is_active, is_hover, tooltip, .. } => {
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

                    // Calculate space needed for close button if active
                    let close_button_space = if *is_active { 20.0 * self.viewport.hidpi_scale } else { 0.0 };

                    // Truncate tab text to fit within the tab width (leaving space for close button)
                    let max_text_width = *width - (text_padding * 2.0) - close_button_space;
                    let display_text = Self::truncate_text_to_width(title, max_text_width, &font);

                    // Draw tab text with scaled padding, centered vertically
                    paint.set_color(Color::BLACK);
                    if let Some(blob) = TextBlob::new(&display_text, &font) {
                        let text_bounds = blob.bounds();
                        // Center the text vertically in the tab
                        let text_y = rect.top() + (rect.height() / 2.0) - (text_bounds.top + text_bounds.height() / 2.0);
                        canvas.draw_text_blob(&blob, (rect.left() + text_padding, text_y), &paint);
                    }

                    // Draw close button for active tab
                    if *is_active {
                        let close_button_size = 16.0 * self.viewport.hidpi_scale;
                        let close_button_x = rect.right() - close_button_size - (4.0 * self.viewport.hidpi_scale);
                        let close_button_y = rect.center_y() - (close_button_size / 2.0);
                        let close_button_rect = Rect::from_xywh(close_button_x, close_button_y, close_button_size, close_button_size);

                        // Draw close button background (subtle)
                        paint.set_color(Color::from_argb(20, 0, 0, 0));
                        canvas.draw_round_rect(close_button_rect, 2.0, 2.0, &paint);

                        // Draw X icon
                        Self::draw_icon(canvas, &IconType::Close, close_button_rect, self.viewport.hidpi_scale);
                    }

                    // Draw tooltip if visible
                    if tooltip.is_visible {
                        Self::draw_tooltip(canvas, tooltip, *x, *y, &font, self.viewport.hidpi_scale);
                    }
                }
            }
        }
    }

    /// Update mouse hover state and handle tooltips
    pub fn update_mouse_hover(&mut self, x: f32, y: f32, current_time: Instant) {
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
                UiComponent::TabButton { is_hover, tooltip, .. } => {
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
                _ => {}
            }
        }
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
    fn draw_icon(canvas: &Canvas, icon_type: &IconType, rect: Rect, hidpi_scale: f32) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgb(60, 60, 60)); // Dark gray for icons
        paint.set_stroke(true);
        paint.set_stroke_width(2.0 * hidpi_scale);
        paint.set_style(skia_safe::PaintStyle::Stroke);
        paint.set_stroke_cap(skia_safe::paint::Cap::Round);
        paint.set_stroke_join(skia_safe::paint::Join::Round);

        let center_x = rect.center_x();
        let center_y = rect.center_y();
        let icon_size = rect.width().min(rect.height()) * 0.6;
        let half_size = icon_size / 2.0;

        match icon_type {
            IconType::Back => {
                // Draw left-pointing arrow
                let mut path = Path::new();
                path.move_to((center_x + half_size * 0.3, center_y - half_size * 0.6));
                path.line_to((center_x - half_size * 0.3, center_y));
                path.line_to((center_x + half_size * 0.3, center_y + half_size * 0.6));
                canvas.draw_path(&path, &paint);
            }
            IconType::Forward => {
                // Draw right-pointing arrow
                let mut path = Path::new();
                path.move_to((center_x - half_size * 0.3, center_y - half_size * 0.6));
                path.line_to((center_x + half_size * 0.3, center_y));
                path.line_to((center_x - half_size * 0.3, center_y + half_size * 0.6));
                canvas.draw_path(&path, &paint);
            }
            IconType::Refresh => {
                // Draw circular arrow
                let radius = half_size * 0.7;
                let mut path = Path::new();
                path.add_arc(Rect::from_xywh(center_x - radius, center_y - radius, radius * 2.0, radius * 2.0),
                           45.0, 270.0);
                canvas.draw_path(&path, &paint);

                // Draw arrow head
                let arrow_x = center_x + radius * 0.7;
                let arrow_y = center_y - radius * 0.7;
                let mut arrow_path = Path::new();
                arrow_path.move_to((arrow_x - 4.0, arrow_y - 4.0));
                arrow_path.line_to((arrow_x, arrow_y));
                arrow_path.line_to((arrow_x + 4.0, arrow_y - 4.0));
                canvas.draw_path(&arrow_path, &paint);
            }
            IconType::NewTab => {
                // Draw plus sign
                canvas.draw_line(
                    (center_x - half_size * 0.6, center_y),
                    (center_x + half_size * 0.6, center_y),
                    &paint
                );
                canvas.draw_line(
                    (center_x, center_y - half_size * 0.6),
                    (center_x, center_y + half_size * 0.6),
                    &paint
                );
            }
            IconType::Close => {
                // Draw X
                canvas.draw_line(
                    (center_x - half_size * 0.5, center_y - half_size * 0.5),
                    (center_x + half_size * 0.5, center_y + half_size * 0.5),
                    &paint
                );
                canvas.draw_line(
                    (center_x + half_size * 0.5, center_y - half_size * 0.5),
                    (center_x - half_size * 0.5, center_y + half_size * 0.5),
                    &paint
                );
            }
        }
    }

    /// Draw a tooltip
    fn draw_tooltip(canvas: &Canvas, tooltip: &Tooltip, x: f32, y: f32, font: &Font, hidpi_scale: f32) {
        if !tooltip.is_visible {
            return;
        }

        let mut paint = Paint::default();
        let padding = 8.0 * hidpi_scale;

        // Measure text
        if let Some(text_blob) = TextBlob::new(&tooltip.text, font) {
            let text_bounds = text_blob.bounds();
            let tooltip_width = text_bounds.width() + padding * 2.0;
            let tooltip_height = text_bounds.height() + padding * 2.0;

            // Position tooltip above the component
            let tooltip_x = x;
            let tooltip_y = y - tooltip_height - 5.0;

            // Draw tooltip background with shadow
            let shadow_rect = Rect::from_xywh(tooltip_x + 2.0, tooltip_y + 2.0, tooltip_width, tooltip_height);
            paint.set_color(Color::from_argb(100, 0, 0, 0)); // Semi-transparent black shadow
            canvas.draw_round_rect(shadow_rect, 4.0, 4.0, &paint);

            // Draw tooltip background
            let tooltip_rect = Rect::from_xywh(tooltip_x, tooltip_y, tooltip_width, tooltip_height);
            paint.set_color(Color::from_rgb(255, 255, 220)); // Light yellow background
            canvas.draw_round_rect(tooltip_rect, 4.0, 4.0, &paint);

            // Draw tooltip border
            paint.set_color(Color::from_rgb(180, 180, 140));
            paint.set_stroke(true);
            paint.set_stroke_width(1.0 * hidpi_scale);
            canvas.draw_round_rect(tooltip_rect, 4.0, 4.0, &paint);
            paint.set_stroke(false);

            // Draw tooltip text
            paint.set_color(Color::BLACK);
            canvas.draw_text_blob(&text_blob, (tooltip_x + padding, tooltip_y + padding - text_bounds.top), &paint);
        }
    }

    /// Draw a loading spinner indicator
    /// `angle` is the current rotation angle in radians (0 to 2*PI)
    pub fn render_loading_indicator(&self, canvas: &Canvas, is_loading: bool, angle: f32) {
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

                    Self::draw_spinner(canvas, spinner_x, spinner_y, spinner_size / 2.0, angle, self.viewport.hidpi_scale);
                    break;
                }
            }
        }
    }

    /// Draw an animated spinner
    fn draw_spinner(canvas: &Canvas, center_x: f32, center_y: f32, radius: f32, angle: f32, hidpi_scale: f32) {
        let mut paint = Paint::default();
        paint.set_stroke(true);
        paint.set_stroke_width(2.5 * hidpi_scale);
        paint.set_style(skia_safe::PaintStyle::Stroke);
        paint.set_stroke_cap(skia_safe::paint::Cap::Round);
        paint.set_anti_alias(true);

        // Draw multiple arcs with varying opacity for a smooth spinner effect
        let num_segments = 8;
        for i in 0..num_segments {
            let segment_angle = angle + (i as f32 * 2.0 * PI / num_segments as f32);
            let start_angle = segment_angle * 180.0 / PI;

            // Fade out older segments
            let alpha = ((num_segments - i) as f32 / num_segments as f32 * 255.0) as u8;
            paint.set_color(Color::from_argb(alpha, 50, 120, 255));

            let sweep_angle = 30.0; // degrees
            let rect = Rect::from_xywh(
                center_x - radius,
                center_y - radius,
                radius * 2.0,
                radius * 2.0
            );

            let mut path = Path::new();
            path.add_arc(rect, start_angle, sweep_angle);
            canvas.draw_path(&path, &paint);
        }
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
}
