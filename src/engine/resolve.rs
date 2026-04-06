use crate::engine::Engine;

impl Engine {
    pub fn resolve(&mut self, now: f64) {
        let dom = self.dom_mut();

        dom.resolve(now);

        // Unified JS scheduler order: provider messages -> timers -> microtasks/rejections.
        self.tick_js_tasks();
    }

    fn tick_js_tasks(&mut self) {
        self.handle_messages();

        if let Some(runtime) = self.js_runtime.as_mut() {
            runtime.tick();
        }
    }
}