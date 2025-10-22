// Networking module for handling HTTP requests
use curl::easy::{Easy, List};
use std::path::Path;
use std::time::Duration;
use url::Url;

#[derive(Debug)]
pub enum NetworkError {
    Curl(String),
    Utf8(String),
    Http(u32),
    Empty,
    FileNotFound(String),
    FileRead(String),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            NetworkError::Curl(msg) => write!(f, "Curl error: {}", msg),
            NetworkError::Utf8(msg) => write!(f, "UTF-8 error: {}", msg),
            NetworkError::Http(code) => write!(f, "HTTP error: {}", code),
            NetworkError::Empty => write!(f, "Empty response body"),
            NetworkError::FileNotFound(path) => write!(f, "File not found: {}", path),
            NetworkError::FileRead(msg) => write!(f, "File read error: {}", msg),
        }
    }
}

impl std::error::Error for NetworkError {}

/// A client for making HTTP requests and fetching web resources
pub struct HttpClient {
    // curl::Easy is not Send/Sync, so we'll create it fresh for each request
}

impl HttpClient {
    pub fn new() -> Self {
        Self {}
    }


    /// Convert an input (which may be a file:// URL or a plain filesystem path)
    /// to a local file system path string.
    fn url_to_file_path(input: &str) -> String {
        if input.starts_with("file://") {
            // Url crate can normalize file URLs properly
            match Url::parse(input) {
                Ok(url) => {
                    if let Ok(path) = url.to_file_path() {
                        return path.to_string_lossy().into_owned();
                    }
                }
                Err(_) => {
                    // Fall through to manual stripping
                }
            }
            // Manual fallback: strip file:// prefix
            let path = &input[7..];
            if cfg!(windows) && path.starts_with('/') && path.len() > 2 && path.chars().nth(2) == Some(':') {
                return path[1..].to_string();
            }
            return path.to_string();
        }

        // If the input isn't a file:// URL but looks like a filesystem path,
        // return it as-is.
        input.to_string()
    }

    /// Read a local HTML file
    async fn read_local_file(path: &str) -> Result<String, NetworkError> {
        println!("Reading local file: {}", path);

        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            // Check if file exists
            let file_path = Path::new(&path);
            if !file_path.exists() {
                return Err(NetworkError::FileNotFound(path.clone()));
            }

            // Read the file
            std::fs::read_to_string(file_path)
                .map_err(|e| NetworkError::FileRead(e.to_string()))
        })
        .await
        .map_err(|e| NetworkError::FileRead(e.to_string()))?
    }

    /// Read a local resource file (for images, etc.)
    async fn read_local_resource(path: &str) -> Result<Vec<u8>, NetworkError> {
        println!("Reading local resource: {}", path);

        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            // Check if file exists
            let file_path = Path::new(&path);
            if !file_path.exists() {
                return Err(NetworkError::FileNotFound(path.clone()));
            }

            // Read the file as bytes
            std::fs::read(file_path)
                .map_err(|e| NetworkError::FileRead(e.to_string()))
        })
        .await
        .map_err(|e| NetworkError::FileRead(e.to_string()))?
    }

    /// Fetch HTML content from a URL or local file
    pub async fn fetch(&self, url: &str) -> Result<String, NetworkError> {
        println!("Fetching: {}", url);

        let url = match Url::parse(url) {
            Ok(u) => u,
            Err(err) => {
                return Err(NetworkError::Curl(err.to_string()))
            }
        };

        // Check if it's a local file
        if url.scheme() == "file" {
            let file_path = Self::url_to_file_path(url.as_str());
            return Self::read_local_file(&file_path).await;
        }

        // Normalize the URL: if it lacks a scheme, default to https://

        // Run curl operation in a blocking task since curl is synchronous
        let result = tokio::task::spawn_blocking(move || {
            let mut easy = Easy::new();
            let mut data = Vec::new();
            let mut headers = Vec::new();

            // Configure curl
            easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.useragent("Stokes-Browser/1.0").map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.timeout(Duration::from_secs(30)).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.follow_location(true).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.max_redirections(5).map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Set up data collection
            {
                let mut transfer = easy.transfer();
                transfer.write_function(|new_data| {
                    data.extend_from_slice(new_data);
                    Ok(new_data.len())
                }).map_err(|e| NetworkError::Curl(e.to_string()))?;
                
                transfer.header_function(|header| {
                    headers.push(String::from_utf8_lossy(header).to_string());
                    true
                }).map_err(|e| NetworkError::Curl(e.to_string()))?;
                
                transfer.perform().map_err(|e| NetworkError::Curl(e.to_string()))?;
            }

            // Check response code
            let response_code = easy.response_code().map_err(|e| NetworkError::Curl(e.to_string()))?;
            if response_code >= 400 {
                return Err(NetworkError::Http(response_code));
            }

            // Check content type
            let content_type = headers.iter()
                .find(|h| h.to_lowercase().starts_with("content-type:"))
                .and_then(|h| h.split(':').nth(1))
                .map(|s| s.trim())
                .unwrap_or("text/html");

            if !content_type.contains("text/html") {
                println!("Warning: Content type is {}, not HTML", content_type);
            }

            // Convert to string
            let html = String::from_utf8(data)
                .map_err(|_| NetworkError::Utf8("Response contains invalid UTF-8".to_string()))?;

            Ok::<String, NetworkError>(html)
        }).await.map_err(|e| NetworkError::Curl(e.to_string()))?;

        result
    }

    /// Fetch an image or other resource
    pub async fn fetch_resource(&self, url: &str) -> Result<Vec<u8>, NetworkError> {
        println!("Fetching resource: {}", url);

        let url = match Url::parse(url) {
            Ok(u) => u,
            Err(err) => {
                return Err(NetworkError::Curl(err.to_string()))
            }
        };

        // Check if it's a local file
        if url.scheme() == "file" {
            let file_path = Self::url_to_file_path(url.as_str());
            return Self::read_local_resource(&file_path).await;
        }

        // Run curl operation in a blocking task
        let result = tokio::task::spawn_blocking(move || {
            let mut easy = Easy::new();
            let mut data = Vec::new();
            let mut headers = Vec::new();

            // Configure curl
            easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.useragent("Stokes-Browser/1.0").map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.timeout(Duration::from_secs(30)).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.follow_location(true).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.max_redirections(5).map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Set Accept header for resources
            let mut header_list = List::new();
            header_list.append("Accept: image/*, */*").map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.http_headers(header_list).map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Set up data collection
            {
                let mut transfer = easy.transfer();
                transfer.write_function(|new_data| {
                    data.extend_from_slice(new_data);
                    Ok(new_data.len())
                }).map_err(|e| NetworkError::Curl(e.to_string()))?;
                
                transfer.header_function(|header| {
                    headers.push(String::from_utf8_lossy(header).to_string());
                    true
                }).map_err(|e| NetworkError::Curl(e.to_string()))?;
                
                transfer.perform().map_err(|e| NetworkError::Curl(e.to_string()))?;
            }

            // Check response code
            let response_code = easy.response_code().map_err(|e| NetworkError::Curl(e.to_string()))?;
            if response_code >= 400 {
                return Err(NetworkError::Http(response_code));
            }

            // Check content type
            if let Some(content_type_header) = headers.iter()
                .find(|h| h.to_lowercase().starts_with("content-type:")) {
                if let Some(content_type) = content_type_header.split(':').nth(1) {
                    let content_type = content_type.trim();
                    println!("Resource content type: {}", content_type);

                    if !content_type.starts_with("image/") && !content_type.contains("svg") {
                        println!("Warning: Expected image content type, got: {}", content_type);
                    }
                }
            }

            // Validate we got some data
            if data.is_empty() {
                return Err(NetworkError::Empty);
            }

            println!("Successfully fetched resource: {} bytes", data.len());
            Ok::<Vec<u8>, NetworkError>(data)
        }).await.map_err(|e| NetworkError::Curl(e.to_string()))?;

        result
    }

    /// Check if a URL is valid and reachable (for validation)
    pub async fn head(&self, url: &str) -> Result<bool, NetworkError> {
        let url = match Url::parse(url) {
            Ok(u) => u,
            Err(e) => return Err(NetworkError::Curl(e.to_string())),
        };

        let result = tokio::task::spawn_blocking(move || {
            let mut easy = Easy::new();

            // Configure curl for HEAD request
            easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.useragent("Stokes-Browser/1.0").map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.timeout(Duration::from_secs(10)).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.nobody(true).map_err(|e| NetworkError::Curl(e.to_string()))?; // This makes it a HEAD request
            easy.follow_location(true).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.max_redirections(5).map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Perform the request
            easy.perform().map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Check response code
            let response_code = easy.response_code().map_err(|e| NetworkError::Curl(e.to_string()))?;
            Ok::<bool, NetworkError>(response_code < 400)
        }).await.map_err(|e| NetworkError::Curl(e.to_string()))?;

        result
    }
}
