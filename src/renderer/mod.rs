// Module declaration for the renderer subsystem
pub mod layout;
pub mod style;
pub mod painter;
pub mod dom_renderer;
mod element_renderer;

// Re-exports
pub use dom_renderer::DomRenderer;
