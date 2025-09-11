// Engine configuration

/// Configuration for the browser engine
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// User agent string to use for HTTP requests
    pub user_agent: String,
    /// Default homepage URL
    pub homepage: String,
    /// Whether to enable JavaScript (stub for now)
    pub enable_javascript: bool,
    /// Whether to block ads (stub for now)
    pub block_ads: bool,
    /// Cache size in MB
    pub cache_size_mb: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            user_agent: format!("Stokes-Browser/1.0"),
            homepage: "https://example.com".to_string(),
            enable_javascript: false, // Not implemented yet
            block_ads: false,         // Not implemented yet
            cache_size_mb: 50,
        }
    }
}
