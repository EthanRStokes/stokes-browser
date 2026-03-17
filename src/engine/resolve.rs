use crate::engine::Engine;
use crate::engine::js_provider::{JsProviderMessage, ScriptKind};

impl Engine {
    pub fn resolve(&mut self, now: f64) {
        let dom = self.dom_mut();

        dom.resolve(now);

        // TODO: Run JS tasks
        self.handle_messages();

        self.process_timers();
    }

    fn handle_messages(&mut self) {
        let rx = self.js_rx.take().unwrap();

        while let Ok(message) = rx.try_recv() {
            self.handle_message(message);
        }

        self.js_rx = Some(rx);
    }

    fn handle_message(&mut self, message: JsProviderMessage) {
        match message {
            JsProviderMessage::ExecuteScript { script, script_kind, source_url, node_id } => {
                println!("Executing script ({} bytes)", script.len());
                // Save the script to a local file in debug_js/
                if self.config.debug_js {
                    use std::fs;
                    use std::path::Path;

                    let debug_dir = Path::new("debug_js");
                    if !debug_dir.exists() {
                        if let Err(e) = fs::create_dir_all(debug_dir) {
                            eprintln!("Failed to create debug_js directory: {}", e);
                        }
                    }

                    // Use a unique filename for each inline script
                    // Here we just use a timestamp for simplicity
                    use std::time::{SystemTime, UNIX_EPOCH};
                    let start = SystemTime::now();
                    let since_the_epoch = start.duration_since(UNIX_EPOCH)
                        .expect("Time went backwards");
                    let filename = format!("inline_script_{}.js", since_the_epoch.as_millis());
                    let filepath = debug_dir.join(filename);
                    if let Err(e) = fs::write(&filepath, &script) {
                        eprintln!("Failed to write inline script to {}: {}", filepath.display(), e);
                    } else {
                        println!("Saved inline script to {}", filepath.display());
                    }
                }

                match script_kind {
                    ScriptKind::Classic => {
                        // Set document.currentScript to the executing <script> element
                        crate::js::bindings::dom_bindings::set_current_script(node_id);
                        self.execute_javascript(&script, self.config.debug_js);
                        // Reset document.currentScript to null
                        crate::js::bindings::dom_bindings::set_current_script(None);
                    }
                    ScriptKind::Module => {
                        // Browsers expose currentScript as null for module scripts.
                        crate::js::bindings::dom_bindings::set_current_script(None);
                        self.execute_module_javascript(&script, source_url.as_deref(), self.config.debug_js);
                    }
                }
            }
        }
    }
}