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

    /// Fetch an image or other resource
    pub async fn fetch_resource(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        println!("Fetching resource: {}", url);

        // Make the request with appropriate headers for resources
        let response = self.client
            .get(url)
            .header(header::ACCEPT, "image/*, */*")
            .send()
            .await?;

        // Check if successful
        if !response.status().is_success() {
            return Err(format!("HTTP error {}: {}", response.status().as_u16(), response.status().canonical_reason().unwrap_or("Unknown error")).into());
        }

        // Get content type to validate it's an image (optional validation)
        if let Some(content_type) = response.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type_str) = content_type.to_str() {
                println!("Resource content type: {}", content_type_str);

                // Log if it's not an image type (but still proceed)
                if !content_type_str.starts_with("image/") {
                    println!("Warning: Expected image content type, got: {}", content_type_str);
                }
            }
        }

        // Get the binary content
        let bytes = response.bytes().await?;

        // Validate we got some data
        if bytes.is_empty() {
            return Err("Empty response body".into());
        }

        println!("Successfully fetched resource: {} bytes", bytes.len());
        Ok(bytes.to_vec())
    }

    /// Check if a URL is valid and reachable (for validation)
    pub async fn head(&self, url: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let response = self.client.head(url).send().await?;
        Ok(response.status().is_success())
    }
}
