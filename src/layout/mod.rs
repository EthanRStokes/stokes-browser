// Layout engine for computing element positions and sizes
mod taffy;
mod inline;
pub(crate) mod table;
mod replaced;

use crate::dom::Dom;
use ::taffy::{compute_root_layout, round_layout, AvailableSpace, NodeId};

/// Layout engine responsible for computing element positions and sizes
pub struct LayoutEngine {
    viewport_width: f32,
    viewport_height: f32,
}

impl LayoutEngine {
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            viewport_width,
            viewport_height,
        }
    }

    /// Compute layout for a DOM tree using Taffy
    pub fn compute_layout(&mut self, dom: &mut Dom, scale_factor: f32) {
        // Reserve space for browser UI at the top (address bar, tabs, etc.)
        let ui_height = 0.0;
        let available_height = self.viewport_height - ui_height;

        let root_element_id = NodeId::from(dom.root_element().id);
        compute_root_layout(dom, root_element_id, taffy::Size {
            width: AvailableSpace::Definite(self.viewport_width),
            height: AvailableSpace::Definite(available_height),
        });
        round_layout(dom, root_element_id);
    }

    /// Update viewport size
    #[inline]
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
    }
}
