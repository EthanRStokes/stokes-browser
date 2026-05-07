pub mod legacy;
pub mod libcosmic;

// Re-export TextBrush at crate::ui so existing dom/renderer imports keep working
pub use legacy::ui::TextBrush;
