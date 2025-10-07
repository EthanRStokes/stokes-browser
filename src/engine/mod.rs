// The core browser engine that coordinates between components
mod config;

use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use skia_safe::Canvas;
use crate::networking::{HttpClient, NetworkError};
use crate::dom::{Dom, DomNode, NodeType, ImageData, ImageLoadingState};
use crate::layout::{LayoutEngine, LayoutBox};
use crate::renderer::HtmlRenderer;
use crate::css::{CssParser, Stylesheet, ComputedValues};
use crate::css::transition_manager::TransitionManager;
use crate::js::JsRuntime;

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
    // Add cached renderer and style map
    renderer: HtmlRenderer,
    cached_style_map: HashMap<usize, ComputedValues>,
    style_map_dirty: bool,
    scroll_y: f32,
    scroll_x: f32,
    content_height: f32,
    content_width: f32,
    viewport_height: f32,
    viewport_width: f32,
    pub(crate) scale_factor: f64,
    // Add transition manager for CSS animations
    transition_manager: TransitionManager,
    // Store previous style map to detect changes for transitions
    previous_style_map: HashMap<usize, ComputedValues>,
    // JavaScript runtime
    js_runtime: Option<JsRuntime>,
}

impl Engine {
    pub fn new(config: EngineConfig, scale_factor: f64) -> Self {
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
            renderer: HtmlRenderer::new(),
            cached_style_map: HashMap::new(),
            style_map_dirty: false,
            scroll_y: 0.0,
            scroll_x: 0.0,
            content_height: 0.0,
            content_width: 0.0,
            viewport_height: 600.0,
            viewport_width: 800.0,
            scale_factor,
            transition_manager: TransitionManager::new(),
            previous_style_map: HashMap::new(),
            js_runtime: None,
        }
    }

    /// Navigate to a new URL
    pub async fn navigate(&mut self, url: &str) -> Result<(), NetworkError> {
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

        // Execute JavaScript in the page after everything is loaded
        self.execute_document_scripts().await;

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
                                        image_data.loading_state = ImageLoadingState::Loaded(image_bytes.clone());

                                        // Decode and cache the image immediately after loading
                                        if let Some(decoded_image) = ImageData::decode_image_data_static(&image_bytes) {
                                            image_data.cached_image = Some(decoded_image);
                                            println!("Successfully loaded and decoded image: {}", src);
                                        } else {
                                            println!("Successfully loaded but failed to decode image: {}", src);
                                        }
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
    async fn fetch_image(&self, url: &str) -> Result<Vec<u8>, NetworkError> {
        // Resolve relative URLs against the current page URL
        let absolute_url = self.resolve_url(url)?;

        println!("Fetching image: {}", absolute_url);

        // Use the HTTP client to fetch the image data
        let image_bytes = self.http_client.fetch_resource(&absolute_url).await?;

        // Validate that we got some data
        if image_bytes.is_empty() {
            return Err(NetworkError::Empty);
        }

        Ok(image_bytes)
    }

    /// Resolve a potentially relative URL against the current page URL
    fn resolve_url(&self, url: &str) -> Result<String, NetworkError> {
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
            return Err(NetworkError::Curl("Cannot resolve relative URL: no current page URL".to_string()));
        }

        // Parse the current URL to get the base domain
        // Find the domain part by looking for the third slash (after protocol://)
        let base_url = if self.current_url.starts_with("http://") || self.current_url.starts_with("https://") {
            // Find the end of the domain part
            let protocol_end = if self.current_url.starts_with("https://") { 8 } else { 7 }; // Length of "https://" or "http://"

            if let Some(path_start) = self.current_url[protocol_end..].find('/') {
                // Domain ends at the first slash after the protocol
                &self.current_url[..protocol_end + path_start]
            } else {
                // No path, use the entire URL as the domain
                &self.current_url
            }
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


    /// Recalculate layout with current DOM and styles
    pub fn recalculate_layout(&mut self) {
        if let Some(dom) = &self.dom {
            let root = dom.get_root();
            self.layout = Some(self.layout_engine.compute_layout(&root, self.scale_factor));

            // Update node map from layout engine
            self.node_map = self.layout_engine.get_node_map().clone();

            self.style_map_dirty = true;

            // Update content dimensions
            self.update_content_dimensions();
        }
    }

    /// Update the viewport size
    pub fn set_viewport_size(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.layout_engine.set_viewport(width, height);

        // Recalculate layout with new viewport
        self.recalculate_layout();
    }

    /// Get the viewport size
    pub fn viewport_size(&self) -> (f32, f32) {
        (self.viewport_width, self.viewport_height)
    }

    /// Get the content dimensions
    pub fn content_size(&self) -> (f32, f32) {
        (self.content_width, self.content_height)
    }

    /// Resize the viewport
    pub fn resize(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.layout_engine.set_viewport(width, height);
        self.recalculate_layout();

        // Update content dimensions after layout recalculation
        self.update_content_dimensions();
    }

    /// Render the current page to a canvas with transition support
    pub fn render(&mut self, canvas: &Canvas, scale_factor: f64) {
        if let Some(layout) = &self.layout {
            // Update style map only if it's dirty
            if self.style_map_dirty {
                let new_style_map: HashMap<usize, ComputedValues> = self.node_map.keys()
                    .filter_map(|&node_id| {
                        self.layout_engine.get_computed_styles(node_id)
                            .map(|styles| (node_id, styles.clone()))
                    })
                    .collect();

                // Check for style changes and update transitions
                for (node_id, new_styles) in &new_style_map {
                    if let Some(old_styles) = self.previous_style_map.get(node_id) {
                        // Update transitions for this element if styles changed
                        self.transition_manager.update_element_styles(*node_id, old_styles, new_styles);
                    }
                }

                // Update the cached style maps
                self.previous_style_map = self.cached_style_map.clone();
                self.cached_style_map = new_style_map;
                self.style_map_dirty = false;
            }

            // Clean up completed transitions
            self.transition_manager.cleanup_completed_transitions();

            // Use the renderer with transition support
            let transition_manager_ref = if self.transition_manager.has_active_transitions() {
                Some(&self.transition_manager)
            } else {
                None
            };

            self.renderer.render(
                canvas,
                layout,
                &self.node_map,
                &self.cached_style_map,
                transition_manager_ref,
                self.scroll_x,
                self.scroll_y,
                scale_factor
            );
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
    pub async fn load_external_stylesheet(&mut self, css_url: &str) -> Result<(), NetworkError> {
        let absolute_url = self.resolve_url(css_url)?;
        let css_content = self.http_client.fetch_resource(&absolute_url).await?;
        let css_content = String::from_utf8(css_content).expect("Failed to decode CSS content as UTF-8");
        self.add_stylesheet(&css_content);
        Ok(())
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
                if let NodeType::Element(element_data) = &link_node.node_type {
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

    /// Get the cursor style for the element at the given position
    pub fn get_cursor_at_position(&self, x: f32, y: f32) -> crate::css::Cursor {
        // Adjust position for scroll offset
        let adjusted_x = x + self.scroll_x;
        let adjusted_y = y + self.scroll_y;

        // Find the topmost element at this position
        if let Some(layout) = &self.layout {
            if let Some(node_id) = self.find_element_at_position(layout, adjusted_x, adjusted_y) {
                // Get the computed styles for this element
                if let Some(styles) = self.cached_style_map.get(&node_id) {
                    return styles.cursor.clone();
                }
            }
        }

        // Default cursor
        crate::css::Cursor::Auto
    }

    /// Recursively find the element at the given position (returns the deepest/topmost element)
    fn find_element_at_position(&self, layout_box: &LayoutBox, x: f32, y: f32) -> Option<usize> {
        let border_box = layout_box.dimensions.border_box();

        // Check if position is within this box
        if x >= border_box.left && x <= border_box.right &&
           y >= border_box.top && y <= border_box.bottom {

            // Check children first (they are on top)
            for child in &layout_box.children {
                if let Some(child_node_id) = self.find_element_at_position(child, x, y) {
                    return Some(child_node_id);
                }
            }

            // If no child matched, return this node
            return Some(layout_box.node_id);
        }

        None
    }

    /// Check if there are active transitions that require continuous rendering
    pub fn has_active_transitions(&self) -> bool {
        self.transition_manager.has_active_transitions()
    }

    /// Initialize JavaScript runtime for the current document
    pub fn initialize_js_runtime(&mut self) {
        if let Some(dom) = &self.dom {
            let root = dom.get_root();
            match JsRuntime::new(root) {
                Ok(runtime) => {
                    println!("JavaScript runtime initialized successfully");
                    self.js_runtime = Some(runtime);
                }
                Err(e) => {
                    eprintln!("Failed to initialize JavaScript runtime: {}", e);
                }
            }
        }
    }

    /// Execute JavaScript code in the current context
    pub fn execute_javascript(&mut self, code: &str) {
        if let Some(runtime) = &mut self.js_runtime {
            if let Err(e) = runtime.execute_script(code) {
                eprintln!("JavaScript execution error: {}", e);
            }
        } else {
            eprintln!("JavaScript runtime not initialized");
        }
    }

    /// Extract and execute JavaScript from <script> tags in the current DOM
    pub async fn execute_document_scripts(&mut self) {
        // Initialize JS runtime if not already done
        if self.js_runtime.is_none() {
            self.initialize_js_runtime();
        }

        // Collect script contents and external URLs first to avoid borrow issues
        let mut script_items = Vec::new();

        if let Some(dom) = &self.dom {
            let script_elements = dom.query_selector("script");

            for script_element in script_elements {
                let script_node = script_element.borrow();
                if let NodeType::Element(element_data) = &script_node.node_type {
                    // Check for external scripts
                    if let Some(src) = element_data.attributes.get("src") {
                        println!("Found external script: {}", src);
                        script_items.push((true, src.clone()));
                    } else {
                        // Get inline script content
                        let script_content = script_node.text_content();
                        if !script_content.trim().is_empty() {
                            script_items.push((false, script_content));
                        }
                    }
                }
            }
        }

        // Execute scripts in order (inline and external)
        for (is_external, content) in script_items {
            if is_external {
                // Fetch external script
                match self.load_external_script(&content).await {
                    Ok(script_content) => {
                        println!("Executing external script from {} ({} bytes)", content, script_content.len());
                        self.execute_javascript(&script_content);
                    }
                    Err(e) => {
                        eprintln!("Failed to load external script {}: {}", content, e);
                    }
                }
            } else {
                // Execute inline script
                println!("Executing inline script ({} bytes)", content.len());
                self.execute_javascript(&content);
            }
        }
    }

    /// Load an external JavaScript file from a URL
    async fn load_external_script(&self, script_url: &str) -> Result<String, NetworkError> {
        let absolute_url = self.resolve_url(script_url)?;
        let script_bytes = self.http_client.fetch_resource(&absolute_url).await?;
        let script_content = String::from_utf8(script_bytes)
            .map_err(|_| NetworkError::Utf8("Failed to decode script as UTF-8".to_string()))?;
        Ok(script_content)
    }

    /// Handle a click at the given position (viewport coordinates)
    /// Returns the href of the clicked link, if any
    pub fn handle_click(&self, x: f32, y: f32) -> Option<String> {
        // Adjust position for scroll offset
        let adjusted_x = x + self.scroll_x;
        let adjusted_y = y + self.scroll_y;

        // Find the element at this position
        if let Some(layout) = &self.layout {
            if let Some(node_id) = self.find_element_at_position(layout, adjusted_x, adjusted_y) {
                // Check if this element or any parent is an anchor tag
                return self.find_link_href(node_id);
            }
        }

        None
    }

    /// Find the href of a link element by checking the node and its ancestors
    fn find_link_href(&self, node_id: usize) -> Option<String> {
        // Walk up the DOM tree to find an anchor element
        let mut visited = std::collections::HashSet::new();
        let mut current_id = node_id;

        // Limit depth to prevent infinite loops
        let max_depth = 50;
        let mut depth = 0;

        loop {
            if depth >= max_depth || visited.contains(&current_id) {
                break;
            }
            visited.insert(current_id);
            depth += 1;

            if let Some(node_rc) = self.node_map.get(&current_id) {
                if let Ok(node) = node_rc.try_borrow() {
                    // Check if this is an anchor element with an href attribute
                    if let NodeType::Element(element_data) = &node.node_type {
                        if element_data.tag_name == "a" {
                            if let Some(href) = element_data.attributes.get("href") {
                                return Some(href.clone());
                            }
                        }
                    }

                    // Move to parent
                    if let Some(parent_weak) = &node.parent {
                        if let Some(parent_rc) = parent_weak.upgrade() {
                            // Find the parent's id in node_map
                            let mut found = false;
                            for (&pid, pnode_rc) in &self.node_map {
                                if Rc::ptr_eq(pnode_rc, &parent_rc) {
                                    current_id = pid;
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                break;
                            }
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        None
    }
}
