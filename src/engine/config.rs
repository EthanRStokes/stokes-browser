// Engine configuration

/// Configuration for the browser engine
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// User agent string to use for HTTP requests
    pub user_agent: String,
    /// Default homepage URL
    pub homepage: String,
    /// Whether to enable JavaScript
    pub enable_javascript: bool,
    /// Whether to block ads (stub for now)
    pub block_ads: bool,
    /// Cache size in MB
    pub cache_size_mb: u32,
    /// Debug: Show hitboxes for clickable elements
    pub debug_hitboxes: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            user_agent: format!("Mozilla/5.0 (Linux; x86_64) Stokes/1.0 Chrome/145.0.0.0 AppleWebKit/537.36 Safari/537.36"),
            homepage: "https://example.com".to_string(),
            enable_javascript: true,
            block_ads: false,         // Not implemented yet
            cache_size_mb: 50,
            debug_hitboxes: false, // Enable for debugging click issues
        }
    }
}
