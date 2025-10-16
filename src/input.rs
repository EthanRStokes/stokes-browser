use crate::engine::Engine;
use crate::ui::BrowserUI;
use arboard::Clipboard;
use winit::event::{ElementState, KeyEvent, Modifiers, MouseScrollDelta};
use winit::keyboard::{Key, NamedKey};
use winit::window::Window;

/// Result of input action that may affect tabs
#[derive(Debug, PartialEq)]
pub enum InputAction {
    None,
    RequestRedraw,
    QuitApp,
    Navigate(String),
    AddTab,
    CloseTab(usize),
    SwitchTab(usize),
    ReloadPage,
    ForwardToTab(KeyboardInput),
}

/// Represents keyboard input to be forwarded to tab process
#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardInput {
    Character(String),
    Named(String),
    Scroll { direction: ScrollDirection, amount: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Handles mouse click events (UI only, for multi-process architecture)
pub fn handle_mouse_click_ui(
    x: f32,
    y: f32,
    ui: &mut BrowserUI,
    tabs: &[(String, String)], // (tab_id, tab_title) pairs
    active_tab_index: usize,
) -> InputAction {
    // Check if close button was clicked first
    if let Some(tab_id) = ui.check_close_button_click(x, y) {
        println!("Close button clicked for tab: {}", tab_id);
        if let Some(tab_index) = tabs.iter().position(|(id, _)| id == &tab_id) {
            return InputAction::CloseTab(tab_index);
        }
    }

    // UI now uses pixel coordinates directly
    if let Some(component_id) = ui.handle_click(x, y) {
        // Handle based on component
        if component_id == "back" {
            println!("Back button clicked");
            // Back navigation would go here
            return InputAction::RequestRedraw;
        } else if component_id == "forward" {
            println!("Forward button clicked");
            // Forward navigation would go here
            return InputAction::RequestRedraw;
        } else if component_id == "refresh" {
            println!("Refresh button clicked");
            return InputAction::ReloadPage;
        } else if component_id == "new_tab" {
            println!("New tab button clicked");
            return InputAction::AddTab;
        } else if component_id == "address_bar" {
            // Focus the address bar for typing
            ui.set_focus("address_bar");
            return InputAction::RequestRedraw;
        } else if component_id.starts_with("tab") {
            // Tab switching by clicking
            if let Some(tab_index) = tabs.iter().position(|(id, _)| id == &component_id) {
                return InputAction::SwitchTab(tab_index);
            }
        }
        return InputAction::RequestRedraw;
    }

    // For content clicks, we don't handle them here - they're forwarded to the tab process
    InputAction::None
}

/// Handles mouse click events
pub fn handle_mouse_click(
    x: f32,
    y: f32,
    ui: &mut BrowserUI,
    active_engine: &mut Engine,
    tabs: &[(String, String)], // (tab_id, tab_title) pairs
    active_tab_index: usize,
) -> InputAction {
    // Check if close button was clicked first
    if let Some(tab_id) = ui.check_close_button_click(x, y) {
        println!("Close button clicked for tab: {}", tab_id);
        if let Some(tab_index) = tabs.iter().position(|(id, _)| id == &tab_id) {
            return InputAction::CloseTab(tab_index);
        }
    }

    // UI now uses pixel coordinates directly
    if let Some(component_id) = ui.handle_click(x, y) {
        // Handle based on component
        if component_id == "back" {
            println!("Back button clicked");
            // Back navigation would go here
            return InputAction::RequestRedraw;
        } else if component_id == "forward" {
            println!("Forward button clicked");
            // Forward navigation would go here
            return InputAction::RequestRedraw;
        } else if component_id == "refresh" {
            println!("Refresh button clicked");
            return InputAction::ReloadPage;
        } else if component_id == "new_tab" {
            println!("New tab button clicked");
            return InputAction::AddTab;
        } else if component_id == "address_bar" {
            // Focus the address bar for typing
            ui.set_focus("address_bar");
            return InputAction::RequestRedraw;
        } else if component_id.starts_with("tab") {
            // Tab switching by clicking
            if let Some(tab_index) = tabs.iter().position(|(id, _)| id == &component_id) {
                return InputAction::SwitchTab(tab_index);
            }
        }
        return InputAction::RequestRedraw;
    }

    // Check if a hyperlink was clicked on the page content
    // Adjust y coordinate to account for the chrome (UI bar)
    let chrome_height = ui.chrome_height();
    if y >= chrome_height as f32 {
        let content_y = y - chrome_height as f32;

        // Check if the click hit a hyperlink
        if let Some(href) = active_engine.handle_click(x, content_y) {
            println!("Hyperlink clicked: {}", href);

            // Resolve the href against the current page URL before navigating
            match active_engine.resolve_url(&href) {
                Ok(resolved_url) => {
                    println!("Resolved to: {}", resolved_url);
                    return InputAction::Navigate(resolved_url);
                }
                Err(e) => {
                    eprintln!("Failed to resolve hyperlink URL '{}': {}", href, e);
                    // Try navigating with the raw href as fallback
                    return InputAction::Navigate(href);
                }
            }
        }
    }

    InputAction::None
}

/// Handles middle-click events (typically for closing tabs)
pub fn handle_middle_click(
    x: f32,
    y: f32,
    ui: &mut BrowserUI,
    tabs: &[(String, String)], // (tab_id, tab_title) pairs
) -> InputAction {
    // Check if a tab was clicked
    if let Some(component_id) = ui.handle_click(x, y) {
        if component_id.starts_with("tab") {
            // Find the tab index by ID
            if let Some(tab_index) = tabs.iter().position(|(id, _)| id == &component_id) {
                println!("Middle-click closing tab: {}", component_id);
                return InputAction::CloseTab(tab_index);
            }
        }
    }
    InputAction::None
}

/// Handles mouse wheel/scroll events
pub fn handle_mouse_wheel(
    delta: MouseScrollDelta,
    cursor_position: (f64, f64),
    modifiers: &Modifiers,
    ui: &mut BrowserUI,
    active_engine: &mut Engine,
) -> InputAction {
    // Check if mouse is over the tab bar (y < chrome height)
    let chrome_height = ui.chrome_height() as f64;
    let is_over_tabs = cursor_position.1 >= 48.0 && cursor_position.1 < chrome_height;

    if is_over_tabs {
        // Handle tab scrolling
        match delta {
            MouseScrollDelta::LineDelta(_x, y) => {
                ui.handle_scroll(y);
            }
            MouseScrollDelta::PixelDelta(pos) => {
                ui.handle_scroll(pos.y as f32 / 30.0);
            }
        }
        return InputAction::RequestRedraw;
    } else {
        // Handle page content scrolling
        let scroll_speed = 50.0;
        let shift_held = modifiers.state().shift_key();

        match delta {
            MouseScrollDelta::LineDelta(x, y) => {
                if shift_held {
                    // Shift+scroll: convert vertical scroll to horizontal
                    active_engine.scroll_horizontal(-y * scroll_speed);
                } else {
                    // Normal scrolling: both vertical and horizontal
                    active_engine.scroll_vertical(-y * scroll_speed);
                    active_engine.scroll_horizontal(-x * scroll_speed);
                }
            }
            MouseScrollDelta::PixelDelta(pos) => {
                if shift_held {
                    // Shift+scroll: convert vertical scroll to horizontal
                    active_engine.scroll_horizontal(-pos.y as f32);
                } else {
                    // Pixel-precise scrolling (trackpad)
                    active_engine.scroll_vertical(-pos.y as f32);
                    active_engine.scroll_horizontal(-pos.x as f32);
                }
            }
        }
        return InputAction::RequestRedraw;
    }
}

/// Handles keyboard input events (multi-process version)
pub fn handle_keyboard_input(
    event: &KeyEvent,
    modifiers: &Modifiers,
    ui: &mut BrowserUI,
    active_tab_index: usize,
    num_tabs: usize,
) -> InputAction {
    if event.state != ElementState::Pressed {
        return InputAction::None;
    }

    let has_focused_text_field = ui.is_text_field_focused();

    // Handle keyboard shortcuts with modifiers (browser-level)
    if modifiers.state().control_key() {
        match &event.logical_key {
            Key::Character(text) => {
                match text.as_str() {
                    "a" => {
                        // Ctrl+A: Select all text in address bar
                        if has_focused_text_field {
                            println!("Select all shortcut (Ctrl+A)");
                            ui.select_all();
                            return InputAction::RequestRedraw;
                        }
                        // Forward to tab for page content selection
                        return InputAction::ForwardToTab(KeyboardInput::Character("ctrl+a".to_string()));
                    }
                    "c" => {
                        // Ctrl+C: Copy selected text to clipboard (UI only)
                        if has_focused_text_field {
                            if let Some(selected_text) = ui.get_selected_text() {
                                if !selected_text.is_empty() {
                                    println!("Copy shortcut (Ctrl+C): {}", selected_text);
                                    if let Ok(mut clipboard) = Clipboard::new() {
                                        if let Err(e) = clipboard.set_text(&selected_text) {
                                            eprintln!("Failed to copy to clipboard: {}", e);
                                        }
                                    }
                                }
                            }
                            return InputAction::RequestRedraw;
                        }
                        // Forward to tab for page content copying
                        return InputAction::ForwardToTab(KeyboardInput::Character("ctrl+c".to_string()));
                    }
                    "v" => {
                        // Ctrl+V: Paste text from clipboard
                        if has_focused_text_field {
                            println!("Paste shortcut (Ctrl+V)");
                            match Clipboard::new() {
                                Ok(mut clipboard) => {
                                    match clipboard.get_text() {
                                        Ok(clipboard_text) => {
                                            println!("Pasted text: {}", clipboard_text);
                                            ui.insert_text_at_cursor(&clipboard_text);
                                        }
                                        Err(e) => {
                                            eprintln!("Failed to read from clipboard: {:?}", e);
                                        }
                                    }
                                }
                                Err(_e) => {}
                            }
                            return InputAction::RequestRedraw;
                        }
                        // Forward to tab for page content pasting
                        return InputAction::ForwardToTab(KeyboardInput::Character("ctrl+v".to_string()));
                    }
                    "x" => {
                        // Ctrl+X: Cut selected text to clipboard
                        if has_focused_text_field {
                            if let Some(selected_text) = ui.get_selected_text() {
                                if !selected_text.is_empty() {
                                    println!("Cut shortcut (Ctrl+X): {}", selected_text);
                                    if let Ok(mut clipboard) = Clipboard::new() {
                                        if let Err(e) = clipboard.set_text(&selected_text) {
                                            eprintln!("Failed to copy to clipboard: {}", e);
                                        } else {
                                            ui.delete_selection();
                                        }
                                    }
                                }
                            }
                            return InputAction::RequestRedraw;
                        }
                        // Forward to tab for page content cutting
                        return InputAction::ForwardToTab(KeyboardInput::Character("ctrl+x".to_string()));
                    }
                    "t" => {
                        // Ctrl+T: New tab (always browser-level)
                        println!("New tab shortcut (Ctrl+T)");
                        return InputAction::AddTab;
                    }
                    "w" => {
                        // Ctrl+W: Close current tab (always browser-level)
                        println!("Close tab shortcut (Ctrl+W)");
                        return InputAction::CloseTab(active_tab_index);
                    }
                    "l" => {
                        // Ctrl+L: Focus address bar (always browser-level)
                        println!("Focus address bar shortcut (Ctrl+L)");
                        ui.set_focus("address_bar");
                        return InputAction::RequestRedraw;
                    }
                    "r" => {
                        // Ctrl+R: Reload page (always browser-level)
                        println!("Reload shortcut (Ctrl+R)");
                        return InputAction::ReloadPage;
                    }
                    "f" => {
                        // Ctrl+F: Find in page (forward to tab)
                        println!("Find in page shortcut (Ctrl+F)");
                        return InputAction::ForwardToTab(KeyboardInput::Character("ctrl+f".to_string()));
                    }
                    _ => {}
                }
            }
            Key::Named(NamedKey::Tab) => {
                // Ctrl+Tab: Switch to next tab (always browser-level)
                println!("Switch tab shortcut (Ctrl+Tab)");
                let next_index = (active_tab_index + 1) % num_tabs;
                return InputAction::SwitchTab(next_index);
            }
            _ => {}
        }
    }

    // Handle number keys for tab switching (Ctrl+1, Ctrl+2, etc.)
    if modifiers.state().control_key() {
        if let Key::Character(text) = &event.logical_key {
            if let Ok(num) = text.parse::<usize>() {
                if num >= 1 && num <= 9 {
                    let tab_index = num - 1;
                    if tab_index < num_tabs {
                        println!("Switch to tab {} shortcut (Ctrl+{})", tab_index + 1, num);
                        return InputAction::SwitchTab(tab_index);
                    }
                }
            }
        }
    }

    // Handle text input and navigation keys
    match &event.logical_key {
        Key::Named(NamedKey::Escape) => {
            if has_focused_text_field {
                // Clear focus from address bar when Escape is pressed
                ui.clear_focus();
                return InputAction::RequestRedraw;
            }
            // Forward to tab (e.g., for closing modals, stopping animations)
            return InputAction::ForwardToTab(KeyboardInput::Named("Escape".to_string()));
        }
        Key::Named(NamedKey::Backspace) => {
            if has_focused_text_field {
                ui.handle_key_input("Backspace");
                return InputAction::RequestRedraw;
            }
            return InputAction::ForwardToTab(KeyboardInput::Named("Backspace".to_string()));
        }
        Key::Named(NamedKey::Delete) => {
            if has_focused_text_field {
                ui.handle_key_input("Delete");
                return InputAction::RequestRedraw;
            }
            return InputAction::ForwardToTab(KeyboardInput::Named("Delete".to_string()));
        }
        Key::Named(NamedKey::ArrowLeft) => {
            if has_focused_text_field {
                ui.handle_key_input("ArrowLeft");
                return InputAction::RequestRedraw;
            }
            // Forward to tab for page scrolling
            return InputAction::ForwardToTab(KeyboardInput::Scroll {
                direction: ScrollDirection::Left,
                amount: 30.0,
            });
        }
        Key::Named(NamedKey::ArrowRight) => {
            if has_focused_text_field {
                ui.handle_key_input("ArrowRight");
                return InputAction::RequestRedraw;
            }
            return InputAction::ForwardToTab(KeyboardInput::Scroll {
                direction: ScrollDirection::Right,
                amount: 30.0,
            });
        }
        Key::Named(NamedKey::ArrowUp) => {
            if !has_focused_text_field {
                return InputAction::ForwardToTab(KeyboardInput::Scroll {
                    direction: ScrollDirection::Up,
                    amount: 30.0,
                });
            }
        }
        Key::Named(NamedKey::ArrowDown) => {
            if !has_focused_text_field {
                return InputAction::ForwardToTab(KeyboardInput::Scroll {
                    direction: ScrollDirection::Down,
                    amount: 30.0,
                });
            }
        }
        Key::Named(NamedKey::Home) => {
            if has_focused_text_field {
                ui.handle_key_input("Home");
                return InputAction::RequestRedraw;
            }
            return InputAction::ForwardToTab(KeyboardInput::Named("Home".to_string()));
        }
        Key::Named(NamedKey::End) => {
            if has_focused_text_field {
                ui.handle_key_input("End");
                return InputAction::RequestRedraw;
            }
            return InputAction::ForwardToTab(KeyboardInput::Named("End".to_string()));
        }
        Key::Named(NamedKey::Enter) => {
            if has_focused_text_field {
                if let Some(url) = ui.handle_key_input("Enter") {
                    // Navigate to the URL from the address bar
                    let url_to_navigate = if url.starts_with("http://")
                        || url.starts_with("https://")
                        || url.starts_with("file://")
                        || url.starts_with('/')
                        || url.ends_with(".html")
                        || url.ends_with(".htm")
                    {
                        url
                    } else {
                        format!("https://{}", url)
                    };
                    return InputAction::Navigate(url_to_navigate);
                }
                return InputAction::RequestRedraw;
            }
            return InputAction::ForwardToTab(KeyboardInput::Named("Enter".to_string()));
        }
        Key::Named(NamedKey::PageUp) => {
            return InputAction::ForwardToTab(KeyboardInput::Scroll {
                direction: ScrollDirection::Up,
                amount: 300.0,
            });
        }
        Key::Named(NamedKey::PageDown) => {
            return InputAction::ForwardToTab(KeyboardInput::Scroll {
                direction: ScrollDirection::Down,
                amount: 300.0,
            });
        }
        Key::Named(NamedKey::Space) => {
            if !has_focused_text_field {
                // Space bar scrolls down when not in a text field
                let scroll_amount: f32 = if modifiers.state().shift_key() {
                    -300.0 // Shift+Space scrolls up
                } else {
                    300.0
                };
                return InputAction::ForwardToTab(KeyboardInput::Scroll {
                    direction: if scroll_amount > 0.0 { ScrollDirection::Down } else { ScrollDirection::Up },
                    amount: scroll_amount.abs(),
                });
            }
            // Space in text field is handled as regular character input
            ui.handle_text_input(" ");
            return InputAction::RequestRedraw;
        }
        Key::Character(text) => {
            if has_focused_text_field {
                // Handle regular character input in UI text fields
                ui.handle_text_input(text.as_str());
                return InputAction::RequestRedraw;
            }
            // Forward other character input to tab (e.g., for in-page search)
            return InputAction::ForwardToTab(KeyboardInput::Character(text.to_string()));
        }
        Key::Named(NamedKey::Tab) => {
            // Tab key for focus navigation (forward to tab)
            if modifiers.state().shift_key() {
                return InputAction::ForwardToTab(KeyboardInput::Named("ShiftTab".to_string()));
            } else {
                return InputAction::ForwardToTab(KeyboardInput::Named("Tab".to_string()));
            }
        }
        _ => {}
    }

    InputAction::None
}

/// Updates cursor shape based on the current position
pub fn update_cursor_for_position(
    cursor_position: (f64, f64),
    ui: &BrowserUI,
    active_engine: &Engine,
    window: &Window,
) {
    let (x, y) = cursor_position;

    // Check if the mouse is over a text field first (UI takes priority)
    if ui.is_mouse_over_text_field(x, y) {
        // Change cursor to I-beam when over text fields
        window.set_cursor(winit::window::CursorIcon::Text);
    } else if ui.is_mouse_over_interactive_element(x, y) {
        // Change cursor to pointer (hand) when over other interactive elements like buttons
        window.set_cursor(winit::window::CursorIcon::Pointer);
    } else {
        // Check if the mouse is over page content
        let chrome_height = ui.chrome_height() as f64;
        if y > chrome_height {
            // Mouse is over page content, check CSS cursor property
            let content_y = (y - chrome_height) as f32;
            let css_cursor = active_engine.get_cursor_at_position(x as f32, content_y);
            let winit_cursor = css_cursor.to_winit_cursor();
            window.set_cursor(winit_cursor);
        } else {
            // Mouse is over chrome area but not an interactive element
            window.set_cursor(winit::window::CursorIcon::Default);
        }
    }
}
