use blitz_traits::shell::Viewport;
use parley::FontContext;

#[derive(Default)]
pub struct DomConfig {
    pub viewport: Option<Viewport>,
    pub base_url: Option<String>,
    pub stylesheets: Option<Vec<String>>,
    pub font_ctx: Option<FontContext>,
}