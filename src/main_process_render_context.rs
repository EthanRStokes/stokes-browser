use crate::display_list::{DisplayFont, DisplayFontData};
use crate::ipc::TabFontSync;
use crate::ui::TextBrush;
use parley::{FontContext, LayoutContext};
use peniko::{Blob, FontData};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct MainProcessRenderContextSync {
    fonts: HashMap<DisplayFont, FontData>,
    tab_generations: HashMap<String, u64>,
    active_layout_sync: Option<(String, u64)>,
}

impl MainProcessRenderContextSync {
    pub fn sync_display_fonts(
        &mut self,
        fonts: Vec<DisplayFontData>,
        font_ctx: &Arc<Mutex<FontContext>>,
    ) {
        self.register_font_payloads(fonts, font_ctx);
    }

    pub fn sync_tab_fonts(
        &mut self,
        tab_id: &str,
        sync: TabFontSync,
        font_ctx: &Arc<Mutex<FontContext>>,
    ) {
        if self
            .tab_generations
            .get(tab_id)
            .is_some_and(|generation| sync.generation < *generation)
        {
            return;
        }

        self.tab_generations
            .insert(tab_id.to_string(), sync.generation);
        self.register_font_payloads(sync.fonts, font_ctx);
    }

    pub fn sync_layout_ctx_for_fragment(
        &mut self,
        tab_id: &str,
        generation: u64,
        layout_ctx: &mut LayoutContext<TextBrush>,
    ) -> bool {
        let needs_reset = self
            .active_layout_sync
            .as_ref()
            .map(|(active_tab_id, active_generation)| {
                active_tab_id != tab_id || *active_generation != generation
            })
            .unwrap_or(true);

        if needs_reset {
            *layout_ctx = LayoutContext::new();
            self.active_layout_sync = Some((tab_id.to_string(), generation));
        }

        needs_reset
    }

    pub fn resolve_fonts(&self, fonts: &[DisplayFont]) -> Vec<Option<FontData>> {
        fonts
            .iter()
            .map(|font| self.fonts.get(font).cloned())
            .collect()
    }

    pub fn remove_tab(&mut self, tab_id: &str) {
        self.tab_generations.remove(tab_id);
        if self
            .active_layout_sync
            .as_ref()
            .is_some_and(|(active_tab_id, _)| active_tab_id == tab_id)
        {
            self.active_layout_sync = None;
        }
    }

    fn register_font_payloads<I>(&mut self, fonts: I, font_ctx: &Arc<Mutex<FontContext>>)
    where
        I: IntoIterator<Item = DisplayFontData>,
    {
        let pending_fonts = fonts
            .into_iter()
            .filter(|font| !self.fonts.contains_key(&font.font))
            .collect::<Vec<_>>();

        if pending_fonts.is_empty() {
            return;
        }

        let mut font_ctx = font_ctx.lock().unwrap();
        for font in pending_fonts {
            let blob = Blob::new(Arc::new(font.bytes));
            font_ctx.collection.register_fonts(blob.clone(), None);
            self.fonts
                .insert(font.font.clone(), FontData::new(blob, font.font.index));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MainProcessRenderContextSync;
    use crate::display_list::{DisplayFont, DisplayFontData};
    use crate::ipc::TabFontSync;
    use crate::ui::TextBrush;
    use parley::{FontContext, LayoutContext};
    use std::sync::{Arc, Mutex};

    fn font(blob_id: u64) -> DisplayFontData {
        DisplayFontData {
            font: DisplayFont { blob_id, index: 0 },
            bytes: vec![blob_id as u8, blob_id as u8 + 1],
        }
    }

    #[test]
    fn ignores_stale_tab_font_sync_generations() {
        let font_ctx = Arc::new(Mutex::new(FontContext::new()));
        let mut sync = MainProcessRenderContextSync::default();

        sync.sync_tab_fonts(
            "tab1",
            TabFontSync {
                generation: 2,
                replace_existing: true,
                fonts: vec![font(2)],
            },
            &font_ctx,
        );
        sync.sync_tab_fonts(
            "tab1",
            TabFontSync {
                generation: 1,
                replace_existing: true,
                fonts: vec![font(1)],
            },
            &font_ctx,
        );

        let resolved = sync.resolve_fonts(&[
            DisplayFont { blob_id: 1, index: 0 },
            DisplayFont { blob_id: 2, index: 0 },
        ]);

        assert!(resolved[0].is_none());
        assert!(resolved[1].is_some());
    }

    #[test]
    fn layout_ctx_resets_when_tab_or_generation_changes() {
        let mut sync = MainProcessRenderContextSync::default();
        let mut layout_ctx = LayoutContext::<TextBrush>::new();

        assert!(sync.sync_layout_ctx_for_fragment("tab1", 1, &mut layout_ctx));
        assert!(!sync.sync_layout_ctx_for_fragment("tab1", 1, &mut layout_ctx));
        assert!(sync.sync_layout_ctx_for_fragment("tab1", 2, &mut layout_ctx));
        assert!(sync.sync_layout_ctx_for_fragment("tab2", 2, &mut layout_ctx));
    }

    #[test]
    fn deduplicates_repeated_font_payloads() {
        let font_ctx = Arc::new(Mutex::new(FontContext::new()));
        let mut sync = MainProcessRenderContextSync::default();

        sync.sync_display_fonts(vec![font(7)], &font_ctx);
        sync.sync_display_fonts(vec![font(7)], &font_ctx);

        let resolved = sync.resolve_fonts(&[DisplayFont { blob_id: 7, index: 0 }]);
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].is_some());
    }
}

