// Networking module for handling HTTP requests
use reqwest::{Client, header};
use std::time::Duration;

/// A client for making HTTP requests and fetching web resources
pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    pub fn new() -> Self {
        // Create a custom client with appropriate settings
        let client = Client::builder()
            .user_agent("Stokes-Browser/1.0")
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self { client }
    }

    /// Fetch HTML content from a URL
    pub async fn fetch(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        println!("Fetching: {}", url);

        // Ensure URL starts with http:// or https://
        let url = if !url.starts_with("http://") && !url.starts_with("https://") {
            format!("https://{}", url)
        } else {
            url.to_string()
        };

        // Make the request
        let response = self.client.get(&url).send().await?;

        // Check if successful
        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()).into());
        }

        // Get content type
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/html");

        // Currently we only handle HTML
        if !content_type.contains("text/html") {
            println!("Warning: Content type is {}, not HTML", content_type);
        }

        // Get the text content
        let html = response.text().await?;
        Ok(html)
    }

    /// Fetch an image or other resource (stub for future implementation)
    pub async fn fetch_resource(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let response = self.client.get(url).send().await?;
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }
}
