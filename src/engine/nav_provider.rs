use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use tokio::sync::mpsc::UnboundedSender;
use crate::shell_provider::ShellProviderMessage;

#[derive(Debug, Clone)]
pub enum NavigationProviderMessage {
    NavigateTo(NavigationOptions),
    Navigate {
        navigation_id: u64,
        url: String,
        contents: String,
        retain_scroll_position: bool,
        is_md: bool,
    },
    /// Like `NavigateTo`, but replaces the current history entry instead of pushing a new one.
    NavigateReplace(NavigationOptions),
    /// Commit the fetched contents for a replace-navigation (no history push).
    NavigateReplaceCommit {
        navigation_id: u64,
        url: String,
        contents: String,
    },
}

pub struct StokesNavigationProvider {
    pub(crate) sender: UnboundedSender<NavigationProviderMessage>,
}

impl StokesNavigationProvider {
    pub(crate) fn new(sender: UnboundedSender<NavigationProviderMessage>) -> Self {
        Self { sender }
    }

    /// Navigate to `options.url`, replacing the current history entry rather than pushing a new one.
    pub fn navigate_replace(&self, options: NavigationOptions) {
        let _ = self.sender.send(NavigationProviderMessage::NavigateReplace(options));
    }
}

impl NavigationProvider for StokesNavigationProvider {
    fn navigate_to(&self, options: NavigationOptions) {
        let _ = self.sender.send(NavigationProviderMessage::NavigateTo(options));
    }
}