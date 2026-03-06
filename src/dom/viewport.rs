// Copyright DioxusLabs
// Licensed under the Apache License, Version 2.0 or the MIT license.

use std::ops::{Deref, DerefMut};
use blitz_traits::shell::{ShellProvider, Viewport};
use crate::dom::{device, Dom};

/// Type that allows mutable access to the viewport
/// And syncs it back to stylist on drop.
pub struct ViewportMut<'doc> {
    doc: &'doc mut Dom,
    initial_viewport: Viewport,
}
impl ViewportMut<'_> {
    pub fn new(doc: &mut Dom) -> ViewportMut<'_> {
        let initial_viewport = doc.viewport.clone();
        ViewportMut {
            doc,
            initial_viewport,
        }
    }
}
impl Deref for ViewportMut<'_> {
    type Target = Viewport;

    fn deref(&self) -> &Self::Target {
        &self.doc.viewport
    }
}
impl DerefMut for ViewportMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.doc.viewport
    }
}
impl Drop for ViewportMut<'_> {
    fn drop(&mut self) {
        if self.doc.viewport == self.initial_viewport {
            return;
        }

        self.doc
            .set_stylist_device(device(&self.doc.viewport, self.doc.font_ctx.clone()));
        self.doc.scroll_viewport_by(0.0, 0.0); // Clamp scroll offset

        let scale_has_changed =
            self.doc.viewport().scale_f64() != self.initial_viewport.scale_f64();
        if scale_has_changed {
            self.doc.invalidate_inline_contexts();
            self.doc.shell_provider.request_redraw();
        }
    }
}