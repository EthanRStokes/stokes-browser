use crate::engine::Engine;
use crate::engine::js_provider::{JsProviderMessage, ScriptKind};

impl Engine {
    pub(crate) fn handle_messages(&mut self) {
        let rx = self.js_rx.take().unwrap();

        while let Ok(message) = rx.try_recv() {
            self.handle_message(message);
        }

        self.js_rx = Some(rx);
    }

    fn handle_message(&mut self, message: JsProviderMessage) {
        match message {
            JsProviderMessage::ExecuteScript {
                script,
                script_kind,
                source_url,
                node_id,
            } => {
                println!("Executing script ({} bytes)", script.len());
                if self.config.debug_js {
                    write_debug_script_copy(&script);
                }

                match script_kind {
                    ScriptKind::Classic => {
                        // Keep currentScript scoped to classic script execution only.
                        crate::js::bindings::dom_bindings::set_current_script(node_id);
                        self.execute_javascript(&script, self.config.debug_js);
                        crate::js::bindings::dom_bindings::set_current_script(None);
                    }
                    ScriptKind::Module => {
                        crate::js::bindings::dom_bindings::set_current_script(None);
                        self.execute_module_javascript(&script, source_url.as_deref(), self.config.debug_js);
                    }
                }
            }
        }
    }
}

fn write_debug_script_copy(script: &str) {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    let debug_dir = Path::new("debug_js");
    if !debug_dir.exists() {
        if let Err(e) = fs::create_dir_all(debug_dir) {
            eprintln!("Failed to create debug_js directory: {}", e);
            return;
        }
    }

    let now = SystemTime::now();
    let millis = now
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis();
    let filepath = debug_dir.join(format!("inline_script_{millis}.js"));

    if let Err(e) = fs::write(&filepath, script) {
        eprintln!("Failed to write inline script to {}: {}", filepath.display(), e);
    } else {
        println!("Saved inline script to {}", filepath.display());
    }
}

