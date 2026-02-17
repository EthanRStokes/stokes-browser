use std::path::PathBuf;
use bincode::{Decode, Encode};
use blitz_traits::shell::{ClipboardError, FileDialogFilter, ShellProvider};
use tokio::sync::mpsc::UnboundedSender;
use cursor_icon::CursorIcon;

/// Messages sent from child (tab process) to parent (main process) to control the shell
#[derive(Debug, Clone, Encode, Decode)]
pub enum ShellProviderMessage {
    RequestRedraw,
    SetCursor(String), // string is CursorIcon.name()
    SetWindowTitle(String),
    SetImeEnabled(bool),
    SetImeCursorArea { x: f32, y: f32, width: f32, height: f32 },
}

pub(crate) struct StokesShellProvider {
    pub(crate) sender: UnboundedSender<ShellProviderMessage>,
}

impl StokesShellProvider {
    pub(crate) fn new(sender: UnboundedSender<ShellProviderMessage>) -> Self {
        Self { sender }
    }
}

impl ShellProvider for StokesShellProvider {
    // Implement the commonly-used shell provider methods by sending a message to the tab main thread.
    fn request_redraw(&self) {
        let _ = self.sender.send(ShellProviderMessage::RequestRedraw);
    }

    fn set_cursor(&self, cursor: CursorIcon) {
        // Use CursorIcon name as a simple wire representation
        let name = cursor.name().to_string();
        let _ = self.sender.send(ShellProviderMessage::SetCursor(name));
    }

    fn set_window_title(&self, title: String) {
        let _ = self.sender.send(ShellProviderMessage::SetWindowTitle(title));
    }

    fn set_ime_enabled(&self, enabled: bool) {
        let _ = self.sender.send(ShellProviderMessage::SetImeEnabled(enabled));
    }

    fn set_ime_cursor_area(&self, x: f32, y: f32, width: f32, height: f32) {
        let _ = self.sender.send(ShellProviderMessage::SetImeCursorArea { x, y, width, height });
    }

    fn get_clipboard_text(&self) -> Result<String, ClipboardError> {
        let mut cb = arboard::Clipboard::new().unwrap();
        cb.get_text()
            .map_err(|_| ClipboardError)
    }

    fn set_clipboard_text(&self, text: String) -> Result<(), ClipboardError> {
        let mut cb = arboard::Clipboard::new().unwrap();
        cb.set_text(text)
            .map_err(|_| ClipboardError)
    }

    fn open_file_dialog(&self, multiple: bool, filter: Option<FileDialogFilter>) -> Vec<PathBuf> {
        let mut dialog = rfd::FileDialog::new();
        if let Some(FileDialogFilter { name, extensions }) = filter {
            dialog = dialog.add_filter(&name, &extensions);
        }
        let files = if multiple {
            dialog.pick_files()
        } else {
            dialog.pick_file().map(|file| vec![file])
        };
        files.unwrap_or_default()
    }
}