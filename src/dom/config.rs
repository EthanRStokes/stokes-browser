use blitz_traits::net::NetProvider;
use blitz_traits::shell::{ShellProvider, Viewport};
use parley::FontContext;
use std::sync::Arc;

#[derive(Default)]
pub struct DomConfig {
    pub viewport: Option<Viewport>,
    pub base_url: Option<String>,
    pub stylesheets: Option<Vec<String>>,
    pub net_provider: Option<Arc<dyn NetProvider>>,
    pub shell_provider: Option<Arc<dyn ShellProvider>>,
    pub font_ctx: Option<FontContext>,
}