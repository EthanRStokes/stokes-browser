// The core browser engine that coordinates between components
mod config;

use crate::dom::Dom;
use crate::networking::HttpClient;

pub use self::config::EngineConfig;

/// The core browser engine that coordinates all browser activities
pub struct Engine {
    pub config: EngineConfig,
    pub dom: Dom,
    http_client: HttpClient,
    current_url: String,
    page_title: String,
    is_loading: bool,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            dom: Dom::new(),
            http_client: HttpClient::new(),
            current_url: String::new(),
            page_title: "New Tab".to_string(),
            is_loading: false,
        }
    }

    /// Navigate to a new URL
    pub async fn navigate(&mut self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("Navigating to: {}", url);
        self.is_loading = true;
        self.current_url = url.to_string();

        // Fetch the page content
        let html = self.http_client.fetch(url).await?;

        // Parse the HTML into our DOM
        self.dom.parse_html(&html);

        // Extract page title
        self.update_page_title();

        self.is_loading = false;
        Ok(())
    }

    /// Get the current page title
    pub fn page_title(&self) -> &str {
        &self.page_title
    }

    /// Get the current URL
    pub fn current_url(&self) -> &str {
        &self.current_url
    }

    /// Check if the page is currently loading
    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    /// Update the page title from the DOM
    fn update_page_title(&mut self) {
        // Find the title element in the DOM
        let title = self.dom.get_title();
        if !title.is_empty() {
            self.page_title = title;
        } else {
            // Default to the domain if no title is found
            self.page_title = self.extract_domain_from_url(&self.current_url)
                .unwrap_or_else(|| "Untitled Page".to_string());
        }
    }

    /// Extract domain from URL
    fn extract_domain_from_url(&self, url: &str) -> Option<String> {
        url.split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .map(|s| s.to_string())
    }
}
