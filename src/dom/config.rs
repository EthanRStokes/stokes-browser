use crate::engine::nav_provider::StokesNavigationProvider;
use crate::engine::net_provider::StokesNetProvider;
use crate::shell_provider::StokesShellProvider;
use blitz_traits::shell::Viewport;
use parley::FontContext;
use std::sync::Arc;

#[derive(Default)]
pub struct DomConfig {
    pub viewport: Option<Viewport>,
    pub base_url: Option<String>,
    pub stylesheets: Option<Vec<String>>,
    pub net_provider: Option<Arc<StokesNetProvider>>,
    pub shell_provider: Option<Arc<StokesShellProvider>>,
    pub nav_provider: Option<Arc<StokesNavigationProvider>>,
    pub font_ctx: Option<FontContext>,
}