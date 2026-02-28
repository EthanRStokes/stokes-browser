use std::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub enum JsProviderMessage {
    ExecuteScript(String),
}

pub struct StokesJsProvider {
    pub(crate) sender: Sender<JsProviderMessage>,
}

impl StokesJsProvider {
    pub(crate) fn new(sender: Sender<JsProviderMessage>) -> Self {
        Self { sender }
    }

    pub fn execute_script(&self, script: String) {
        let _ = self.sender.send(JsProviderMessage::ExecuteScript(script));
    }
}