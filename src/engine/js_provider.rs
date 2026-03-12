use std::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub enum JsProviderMessage {
    ExecuteScript {
        script: String,
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
        let _ = self.sender.send(JsProviderMessage::ExecuteScript { script, node_id: None });
    }

    pub fn execute_script_with_node_id(&self, script: String, node_id: usize) {
        let _ = self.sender.send(JsProviderMessage::ExecuteScript { script, node_id: Some(node_id) });
    }
}

