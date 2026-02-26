use bincode_next::{Decode, Encode};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use tokio::sync::mpsc::UnboundedSender;
use crate::shell_provider::ShellProviderMessage;

#[derive(Debug, Clone)]
pub enum NavigationProviderMessage {
    NavigateTo(NavigationOptions),
    Navigate {
        url: String,
        contents: String,
        retain_scroll_position: bool,
        is_md: bool,
    }
}

pub struct StokesNavigationProvider {
    pub(crate) sender: UnboundedSender<NavigationProviderMessage>,
}

impl StokesNavigationProvider {
    pub(crate) fn new(sender: UnboundedSender<NavigationProviderMessage>) -> Self {
        Self { sender }
    }
}

impl NavigationProvider for StokesNavigationProvider {
    fn navigate_to(&self, options: NavigationOptions) {
        let _ = self.sender.send(NavigationProviderMessage::NavigateTo(options));
    }
}