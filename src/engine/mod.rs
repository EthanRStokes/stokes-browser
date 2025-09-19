// The core browser engine that coordinates between components
mod config;

use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use skia_safe::Canvas;
use crate::networking::HttpClient;
use crate::dom::{Dom, DomNode, NodeType, ImageData, ImageLoadingState};
use crate::layout::{LayoutEngine, LayoutBox};
use crate::renderer::HtmlRenderer;
use crate::css::{CssParser, Stylesheet, ComputedValues};

pub use self::config::EngineConfig;

/// The core browser engine that coordinates all browser activities
pub struct Engine {
    pub config: EngineConfig,
    http_client: HttpClient,
    current_url: String,
    page_title: String,
    is_loading: bool,
    dom: Option<Dom>,
    layout: Option<LayoutBox>,
    layout_engine: LayoutEngine,
    node_map: HashMap<usize, Rc<RefCell<DomNode>>>,
    scroll_y: f32,
    scroll_x: f32,
    content_height: f32,
    content_width: f32,
    viewport_height: f32,
    viewport_width: f32,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            http_client: HttpClient::new(),
            current_url: String::new(),
            page_title: "New Tab".to_string(),
            is_loading: false,
            dom: None,
            layout: None,
            layout_engine: LayoutEngine::new(800.0, 600.0), // Default viewport size
            node_map: HashMap::new(),
            scroll_y: 0.0,
            scroll_x: 0.0,
            content_height: 0.0,
            content_width: 0.0,
            viewport_height: 600.0,
            viewport_width: 800.0,
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
        let dom = Dom::parse_html(&html);
        
        // Extract page title
        self.page_title = dom.get_title();
        
        // Store the DOM
        self.dom = Some(dom);

        // Parse and apply CSS styles from the document
        self.parse_document_styles().await;

        // Calculate layout with CSS styles applied
        self.recalculate_layout();
        
        // Start loading images after layout is calculated
        self.start_image_loading().await;

        self.is_loading = false;
        Ok(())
    }

    /// Start loading all images found in the current DOM
    pub async fn start_image_loading(&mut self) {
        if let Some(dom) = &mut self.dom {
            // Find all image nodes
            let image_nodes = dom.find_nodes(|node| matches!(node.node_type, NodeType::Image(_)));

            for image_node_rc in image_nodes {
                if let Ok(mut image_node) = image_node_rc.try_borrow_mut() {
                    if let NodeType::Image(ref mut image_data) = image_node.node_type {
                        // Only start loading if not already loaded or loading
                        if matches!(image_data.loading_state, ImageLoadingState::NotLoaded) {
                            // Set to loading state
                            image_data.loading_state = ImageLoadingState::Loading;

                            // Start the async fetch (we'll need to handle this differently in practice)
                            let src = image_data.src.clone();
                            if !src.is_empty() {
                                // For now, we'll fetch synchronously in this method
                                // In a real browser, this would be done with proper async handling
                                match self.fetch_image(&src).await {
                                    Ok(image_bytes) => {
                                        image_data.loading_state = ImageLoadingState::Loaded(image_bytes);
                                        println!("Successfully loaded image: {}", src);
                                    }
                                    Err(err) => {
                                        image_data.loading_state = ImageLoadingState::Failed(err.to_string());
                                        println!("Failed to load image {}: {}", src, err);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Recalculate layout after images are loaded (dimensions may have changed)
            self.recalculate_layout();
        }
    }

    /// Fetch a single image from a URL
    async fn fetch_image(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // Resolve relative URLs against the current page URL
        let absolute_url = self.resolve_url(url)?;

        println!("Fetching image: {}", absolute_url);

        // Use the HTTP client to fetch the image data
        let image_bytes = self.http_client.fetch_resource(&absolute_url).await?;

        // Validate that we got some data
        if image_bytes.is_empty() {
            return Err("Empty image data received".into());
        }

        Ok(image_bytes)
    }

    /// Resolve a potentially relative URL against the current page URL
    fn resolve_url(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        // If the URL is already absolute, return it as-is
        if url.starts_with("http://") || url.starts_with("https://") {
            return Ok(url.to_string());
        }

        // Handle protocol-relative URLs
        if url.starts_with("//") {
            // Use the same protocol as the current page
            let protocol = if self.current_url.starts_with("https://") {
                "https:"
            } else {
                "http:"
            };
            return Ok(format!("{}{}", protocol, url));
        }

        // For relative URLs, we need to resolve them against the current page URL
        if self.current_url.is_empty() {
            return Err("Cannot resolve relative URL: no current page URL".into());
        }

        // Parse the current URL to get the base
        let base_url = if let Some(domain_end) = self.current_url.find('/') { // Skip "https://"
            &self.current_url[..domain_end]
        } else {
            &self.current_url
        };

        // Handle different types of relative URLs
        if url.starts_with('/') {
            // Absolute path relative to domain root
            Ok(format!("{}{}", base_url, url))
        } else {
            // Relative path - for simplicity, treat as relative to domain root
            // In a real browser, this would be relative to the current page's path
            Ok(format!("{}/{}", base_url, url))
        }
    }

    /// Force reload images (useful for debugging or refresh)
    pub async fn reload_images(&mut self) {
        if let Some(dom) = &mut self.dom {
            // Find all image nodes and reset their loading state
            let image_nodes = dom.find_nodes(|node| matches!(node.node_type, NodeType::Image(_)));

            for image_node_rc in image_nodes {
                if let Ok(mut image_node) = image_node_rc.try_borrow_mut() {
                    if let NodeType::Image(ref mut image_data) = image_node.node_type {
                        image_data.loading_state = ImageLoadingState::NotLoaded;
                    }
                }
            }

            // Start loading again
            self.start_image_loading().await;
        }
    }

    /// Recalculate layout based on current DOM
    pub fn recalculate_layout(&mut self) {
        if let Some(dom) = &self.dom {
            // Convert DOM root to Rc<RefCell<DomNode>> for layout engine
            let root_node = Rc::new(RefCell::new(dom.root.clone()));
            
            // Calculate layout
            self.layout = Some(self.layout_engine.compute_layout(&root_node));

            // Get node map for renderer
            self.node_map = self.layout_engine.get_node_map().clone();

            // Update content dimensions after layout calculation
            self.update_content_dimensions();
        }
    }
    
    /// Resize the viewport
    pub fn resize(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.layout_engine = LayoutEngine::new(width, height);
        self.recalculate_layout();

        // Update content dimensions after layout recalculation
        self.update_content_dimensions();
    }
    
    /// Render the current page to a canvas
    pub fn render(&self, canvas: &Canvas) {
        if let Some(layout) = &self.layout {
            // Create a renderer
            let renderer = HtmlRenderer::new();

            // Get computed styles from the layout engine for CSS-aware rendering
            let style_map: HashMap<usize, ComputedValues> = self.node_map.keys()
                .filter_map(|&node_id| {
                    self.layout_engine.get_computed_styles(node_id)
                        .map(|styles| (node_id, styles.clone()))
                })
                .collect();

            // Use CSS-aware rendering with styles
            renderer.render_with_styles(canvas, layout, &self.node_map, &style_map, self.scroll_x, self.scroll_y);
        }
    }

    /// Add a CSS stylesheet to the engine
    pub fn add_stylesheet(&mut self, css_content: &str) {
        let parser = CssParser::new();
        let stylesheet = parser.parse(css_content);
        self.layout_engine.add_stylesheet(stylesheet);

        // Recalculate layout with new styles if DOM exists
        if self.dom.is_some() {
            self.recalculate_layout();
        }
    }

    /// Add a CSS stylesheet from a URL
    pub async fn load_external_stylesheet(&mut self, css_url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let absolute_url = self.resolve_url(css_url)?;
        let css_content = self.http_client.fetch(&absolute_url).await?;
        self.add_stylesheet(&css_content);
        Ok(())
    }

    /// Render the current page to a canvas with CSS styling
    pub fn render_with_styles(&self, canvas: &Canvas) {
        if let Some(layout) = &self.layout {
            // Create a renderer
            let renderer = HtmlRenderer::new();

            // Get computed styles from the layout engine
            let style_map: HashMap<usize, ComputedValues> = self.node_map.keys()
                .filter_map(|&node_id| {
                    self.layout_engine.get_computed_styles(node_id)
                        .map(|styles| (node_id, styles.clone()))
                })
                .collect();

            // Render the layout with CSS styles and scroll offset
            renderer.render_with_styles(canvas, layout, &self.node_map, &style_map, self.scroll_x, self.scroll_y);
        }
    }

    /// Extract and parse CSS from <style> tags and <link> tags in the current DOM
    pub async fn parse_document_styles(&mut self) {
        if let Some(dom) = &self.dom {
            // Collect style contents first
            let mut style_contents = Vec::new();
            let style_elements = dom.query_selector("style");
            for style_element in style_elements {
                let style_node = style_element.borrow();
                let css_content = style_node.text_content();
                if !css_content.trim().is_empty() {
                    style_contents.push(css_content);
                }
            }

            // Collect link hrefs first
            let mut link_hrefs = Vec::new();
            let link_elements = dom.query_selector("link");
            for link_element in link_elements {
                let link_node = link_element.borrow();
                if let crate::dom::NodeType::Element(element_data) = &link_node.node_type {
                    if let (Some(rel), Some(href)) = (
                        element_data.attributes.get("rel"),
                        element_data.attributes.get("href")
                    ) {
                        if rel.to_lowercase() == "stylesheet" {
                            link_hrefs.push(href.clone());
                        }
                    }
                }
            }

            // Now process the collected data without holding DOM borrows
            for css_content in style_contents {
                self.add_stylesheet(&css_content);
            }

            for href in link_hrefs {
                if let Err(e) = self.load_external_stylesheet(&href).await {
                    println!("Failed to load stylesheet {}: {}", href, e);
                }
            }
        }
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

    /// Extract domain from URL
    fn extract_domain_from_url(&self, url: &str) -> Option<String> {
        url.split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .map(|s| s.to_string())
    }

    /// Scroll vertically by the given delta
    pub fn scroll_vertical(&mut self, delta: f32) -> bool {
        let old_scroll_y = self.scroll_y;
        self.scroll_y = (self.scroll_y + delta).max(0.0);

        // Don't scroll past the bottom of the content
        let max_scroll = (self.content_height - self.viewport_height).max(0.0);
        self.scroll_y = self.scroll_y.min(max_scroll);

        // Return whether scroll position actually changed
        old_scroll_y != self.scroll_y
    }

    /// Scroll horizontally by the given delta
    pub fn scroll_horizontal(&mut self, delta: f32) -> bool {
        let old_scroll_x = self.scroll_x;
        self.scroll_x = (self.scroll_x + delta).max(0.0);

        // Don't scroll past the right edge of the content
        let max_scroll = (self.content_width - self.viewport_width).max(0.0);
        self.scroll_x = self.scroll_x.min(max_scroll);

        // Return whether scroll position actually changed
        old_scroll_x != self.scroll_x
    }

    /// Get current scroll position
    pub fn scroll_position(&self) -> (f32, f32) {
        (self.scroll_x, self.scroll_y)
    }

    /// Set scroll position directly
    pub fn set_scroll_position(&mut self, x: f32, y: f32) {
        self.scroll_x = x.max(0.0).min((self.content_width - self.viewport_width).max(0.0));
        self.scroll_y = y.max(0.0).min((self.content_height - self.viewport_height).max(0.0));
    }

    /// Update content dimensions based on layout
    fn update_content_dimensions(&mut self) {
        if let Some(layout) = &self.layout {
            // Calculate total content dimensions from the layout tree
            let (width, height) = self.calculate_content_bounds(layout);
            self.content_width = width;
            self.content_height = height;
        }
    }

    /// Recursively calculate content bounds
    fn calculate_content_bounds(&self, layout_box: &LayoutBox) -> (f32, f32) {
        let mut max_width = layout_box.dimensions.content.right();
        let mut max_height = layout_box.dimensions.content.bottom();

        for child in &layout_box.children {
            let (child_width, child_height) = self.calculate_content_bounds(child);
            max_width = max_width.max(child_width);
            max_height = max_height.max(child_height);
        }

        (max_width, max_height)
    }
}
