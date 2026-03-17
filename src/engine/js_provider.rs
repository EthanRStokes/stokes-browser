use std::sync::mpsc::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Classic,
    Module,
}

#[derive(Debug, Clone)]
pub enum JsProviderMessage {
    ExecuteScript {
        script: String,
        script_kind: ScriptKind,
        /// Optional source URL used for module-script metadata such as import.meta.url.
        source_url: Option<String>,
        /// Node ID of the `<script>` element being executed, for `document.currentScript`.
        node_id: Option<usize>,
    },
}

pub struct StokesJsProvider {
    pub(crate) sender: Sender<JsProviderMessage>,
}

impl StokesJsProvider {
    pub(crate) fn new(sender: Sender<JsProviderMessage>) -> Self {
        Self { sender }
    }

    pub fn execute_script(&self, script: String) {
        let _ = self.sender.send(JsProviderMessage::ExecuteScript {
            script,
            script_kind: ScriptKind::Classic,
            source_url: None,
            node_id: None,
        });
    }

    pub fn execute_script_with_node_id(&self, script: String, node_id: usize) {
        let _ = self.sender.send(JsProviderMessage::ExecuteScript {
            script,
            script_kind: ScriptKind::Classic,
            source_url: None,
            node_id: Some(node_id),
        });
    }

    pub fn execute_module_script_with_node_id(&self, script: String, node_id: usize, source_url: Option<String>) {
        let _ = self.sender.send(JsProviderMessage::ExecuteScript {
            script,
            script_kind: ScriptKind::Module,
            source_url,
            node_id: Some(node_id),
        });
    }
}

