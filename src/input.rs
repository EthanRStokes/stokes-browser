use winit::event::{ElementState, KeyEvent, Modifiers, MouseButton, MouseScrollDelta};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::Window;
use arboard::Clipboard;

use crate::engine::Engine;
use crate::ui::BrowserUI;

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
    ui: &BrowserUI,
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

/// Handles keyboard input events
pub fn handle_keyboard_input(
    event: &KeyEvent,
    modifiers: &Modifiers,
    ui: &mut BrowserUI,
    active_engine: &mut Engine,
    active_tab_index: usize,
    num_tabs: usize,
    has_focused_text_field: bool,
) -> InputAction {
    if event.state != ElementState::Pressed {
        return InputAction::None;
    }

    // Handle keyboard shortcuts with modifiers
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
                    }
                    "c" => {
                        // Ctrl+C: Copy selected text to clipboard
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
                                            // Delete the selected text after copying
                                            ui.delete_selection();
                                        }
                                    }
                                }
                            }
                            return InputAction::RequestRedraw;
                        }
                    }
                    "t" => {
                        // Ctrl+T: New tab
                        println!("New tab shortcut (Ctrl+T)");
                        return InputAction::AddTab;
                    }
                    "w" => {
                        // Ctrl+W: Close current tab
                        println!("Close tab shortcut (Ctrl+W)");
                        return InputAction::CloseTab(active_tab_index);
                    }
                    "l" => {
                        // Ctrl+L: Focus address bar
                        println!("Focus address bar shortcut (Ctrl+L)");
                        ui.set_focus("address_bar");
                        return InputAction::RequestRedraw;
                    }
                    "r" => {
                        // Ctrl+R: Reload page
                        println!("Reload shortcut (Ctrl+R)");
                        return InputAction::ReloadPage;
                    }
                    _ => {}
                }
            }
            Key::Named(NamedKey::Tab) => {
                // Ctrl+Tab: Switch to next tab
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
            // Clear focus from address bar when Escape is pressed
            ui.clear_focus();
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::Backspace) => {
            ui.handle_key_input("Backspace");
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::Delete) => {
            ui.handle_key_input("Delete");
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::ArrowLeft) => {
            // Check if we're in a text field first
            if !has_focused_text_field {
                active_engine.scroll_horizontal(-30.0);
            } else {
                ui.handle_key_input("ArrowLeft");
            }
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::ArrowRight) => {
            // Check if we're in a text field first
            if !has_focused_text_field {
                active_engine.scroll_horizontal(30.0);
            } else {
                ui.handle_key_input("ArrowRight");
            }
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::ArrowUp) => {
            active_engine.scroll_vertical(-30.0);
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::ArrowDown) => {
            active_engine.scroll_vertical(30.0);
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::Home) => {
            // Check if we're in a text field first
            if !has_focused_text_field {
                active_engine.set_scroll_position(0.0, 0.0);
            } else {
                ui.handle_key_input("Home");
            }
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::End) => {
            // Check if we're in a text field first
            if !has_focused_text_field {
                active_engine.set_scroll_position(0.0, f32::MAX); // Will be clamped to max
            } else {
                ui.handle_key_input("End");
            }
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::Enter) => {
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
        Key::Named(NamedKey::PageUp) => {
            active_engine.scroll_vertical(-300.0);
            return InputAction::RequestRedraw;
        }
        Key::Named(NamedKey::PageDown) => {
            active_engine.scroll_vertical(300.0);
            return InputAction::RequestRedraw;
        }
        Key::Character(text) => {
            // Handle regular character input
            ui.handle_text_input(text.as_str());
            return InputAction::RequestRedraw;
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
