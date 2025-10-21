// The core browser engine that coordinates between components
mod config;

use crate::css::transition_manager::TransitionManager;
use crate::css::{ComputedValues, CssParser};
use crate::dom::{Dom, DomNode, ImageData, ImageLoadingState, NodeType};
use crate::dom::{EventDispatcher, EventType};
use crate::js::JsRuntime;
use crate::layout::{LayoutBox, LayoutEngine};
use crate::networking::{HttpClient, NetworkError};
use crate::renderer::HtmlRenderer;
use skia_safe::Canvas;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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
    // Flag to track when layout needs recomputation due to DOM changes
    layout_needs_recomputation: Rc<RefCell<bool>>,
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
            layout_needs_recomputation: Rc::new(RefCell::new(false)),
        }
    }

    /// Navigate to a new URL
    pub async fn navigate(&mut self, url: &str) -> Result<(), NetworkError> {
        println!("Navigating to: {}", url);
        self.is_loading = true;
        self.current_url = url.to_string();

        // Fetch the page content
        let result = async {
            let html = self.http_client.fetch(url).await?;

            // Parse the HTML into our DOM
            let mut dom = Dom::parse_html(&html);

            // Extract page title
            self.page_title = dom.get_title();

            // Set up layout invalidation callback on the DOM root
            let layout_flag = Rc::clone(&self.layout_needs_recomputation);
            let callback = Rc::new(Box::new(move || {
                *layout_flag.borrow_mut() = true;
            }) as Box<dyn Fn()>);
            dom.get_mut_root().set_layout_invalidation_callback(callback);

            // Store the DOM
            self.dom = Some(dom);

            // Reset scroll position
            self.scroll_x = 0.0;
            self.scroll_y = 0.0;

            // Parse and apply CSS styles from the document
            self.parse_document_styles().await;

            // Calculate layout with CSS styles applied
            self.recalculate_layout();

            // Start loading images after layout is calculated
            self.start_image_loading().await;

            // Execute JavaScript in the page after everything is loaded
            self.execute_document_scripts().await;

            Ok(())
        }.await;

        // Always reset loading state
        self.is_loading = false;
        result
    }

    /// Start loading all images found in the current DOM
    pub async fn start_image_loading(&mut self) {
        if let Some(dom) = &mut self.dom {
            // Find all image nodes
            let image_nodes = dom.find_nodes(|node| matches!(node.node_type, NodeType::Image(_)));

            // Collect image sources that need to be loaded
            let mut image_requests: Vec<(&Rc<RefCell<DomNode>>, Rc<String>)> = Vec::new();

            for image_node_rc in &image_nodes {
                if let Ok(mut image_node) = image_node_rc.try_borrow_mut() {
                    if let NodeType::Image(ref mut image_data) = image_node.node_type {
                        let image_data = image_data.get_mut();
                        // Only start loading if not already loaded or loading
                        if matches!(image_data.loading_state, ImageLoadingState::NotLoaded) {
                            // Set to loading state
                            image_data.loading_state = ImageLoadingState::Loading;

                            if !image_data.src.is_empty() {
                                image_requests.push((image_node_rc, image_data.src.clone()));
                            }
                        }
                    }
                }
            }

            // Fetch all images concurrently
            let mut fetch_futures = Vec::new();
            for (_, src) in &image_requests {
                // Resolve URL before creating the future
                let absolute_url = match self.resolve_url(src) {
                    Ok(url) => url,
                    Err(e) => {
                        eprintln!("Failed to resolve image URL {}: {}", src, e);
                        continue;
                    }
                };

                let http_client = &self.http_client;
                fetch_futures.push(async move {
                    let result = http_client.fetch_resource(&absolute_url).await;
                    (src, result)
                });
            }

            let results = futures::future::join_all(fetch_futures).await;

            // Process results and update image nodes
            for ((node_rc, src), (_, result)) in image_requests.iter().zip(results.into_iter()) {
                if let Ok(mut image_node) = node_rc.try_borrow_mut() {
                    if let NodeType::Image(ref mut image_data) = image_node.node_type {
                        let image_data = image_data.get_mut();
                        match result {
                            Ok(image_bytes) => {
                                // Decode and cache the image immediately after loading
                                if let Some(decoded_image) = ImageData::decode_image_data_static(&image_bytes) {
                                    image_data.cached_image = Some(decoded_image);
                                    println!("Successfully loaded and decoded image: {}", src);
                                } else {
                                    println!("Successfully loaded but failed to decode image: {}", src);
                                }
                                image_data.loading_state = ImageLoadingState::Loaded(image_bytes);
                            }
                            Err(err) => {
                                image_data.loading_state = ImageLoadingState::Failed(err.to_string());
                                println!("Failed to load image {}: {}", src, err);
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
    pub fn resolve_url(&self, url: &str) -> Result<String, NetworkError> {
        // If the URL is already absolute, return it as-is
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file://") {
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

        // Handle local file paths
        if self.current_url.starts_with("file://") || self.current_url.starts_with('/') ||
            (self.current_url.len() >= 3 && self.current_url.chars().nth(1) == Some(':')) {
            // Current URL is a local file path
            use std::path::Path;

            let current_path = if self.current_url.starts_with("file://") {
                &self.current_url[7..]
            } else {
                &self.current_url
            };

            // Get the directory of the current file
            let base_path = Path::new(current_path);
            let base_dir = base_path.parent().unwrap_or(Path::new("."));

            // Resolve the relative path
            let resolved_path = if url.starts_with('/') {
                // Absolute path on the file system
                Path::new(url).to_path_buf()
            } else {
                // Relative path
                base_dir.join(url)
            };

            // Convert to string and normalize
            return resolved_path.to_str()
                .map(|s| s.to_string())
                .ok_or_else(|| NetworkError::FileRead("Invalid path encoding".to_string()));
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
                        let image_data = image_data.get_mut();
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

            // Clear the recomputation flag since we just recomputed
            *self.layout_needs_recomputation.borrow_mut() = false;
        }
    }

    /// Check if layout needs recomputation and apply it if needed
    /// This should be called before rendering or when checking layout state
    pub fn apply_pending_layout_changes(&mut self) {
        if *self.layout_needs_recomputation.borrow() {
            self.recalculate_layout();
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
        // Apply any pending layout changes from DOM modifications
        self.apply_pending_layout_changes();

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
    #[inline]
    pub async fn load_external_stylesheet(&mut self, css_url: &str) -> Result<(), NetworkError> {
        let absolute_url = self.resolve_url(css_url)?;
        let css_content = self.http_client.fetch_resource(&absolute_url).await?;
        let css_content = String::from_utf8(css_content).expect("Failed to decode CSS content as UTF-8");
        self.add_stylesheet(&css_content);
        Ok(())
    }

    /// Extract and parse CSS from <style> tags and <link> tags in the current DOM
    pub async fn parse_document_styles(&mut self) {
        if let Some(dom) = &mut self.dom {
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

            let mut fetch_futures = Vec::new();
            for href in link_hrefs {
                let absolute_url = match self.resolve_url(&href) {
                    Ok(url) => url,
                    Err(e) => {
                        println!("Failed to resolve stylesheet URL {}: {}", href, e);
                        continue;
                    }
                };
                let http_client = &self.http_client;
                fetch_futures.push(async move {
                    http_client.fetch_resource(&absolute_url).await
                });
            }

            let results = futures::future::join_all(fetch_futures).await;

            for result in results {
                match result {
                    Ok(css_bytes) => {
                        if let Ok(css_content) = String::from_utf8(css_bytes) {
                            self.add_stylesheet(&css_content);
                        } else {
                            println!("Failed to decode fetched stylesheet as UTF-8");
                        }
                    }
                    Err(e) => {
                        println!("Failed to fetch external stylesheet: {}", e);
                    }
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

    /// Set the loading state manually (useful for UI updates)
    pub fn set_loading_state(&mut self, loading: bool) {
        self.is_loading = loading;
    }

    /// Extract domain from URL
    fn extract_domain_from_url(&self, url: &str) -> Option<String> {
        url.split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .map(|s| s.to_string())
    }

    /// Scroll the page by delta amounts
    pub fn scroll(&mut self, delta_x: f32, delta_y: f32) {
        self.scroll_horizontal(delta_x);
        self.scroll_vertical(delta_y);
    }

    /// Scroll vertically by the given delta
    pub fn scroll_vertical(&mut self, delta: f32) -> bool {
        let delta = delta * 3.5;
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
            let user_agent = self.config.user_agent.clone();
            match JsRuntime::new(root, user_agent) {
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

        if let Some(dom) = &mut self.dom {
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

                // Save the script to a local file in debug_js/
                #[cfg(debug_assertions)]
                {
                    use std::fs;
                    use std::path::Path;

                    let debug_dir = Path::new("debug_js");
                    if !debug_dir.exists() {
                        if let Err(e) = fs::create_dir_all(debug_dir) {
                            eprintln!("Failed to create debug_js directory: {}", e);
                        }
                    }

                    // Use a unique filename for each inline script
                    // Here we just use a timestamp for simplicity
                    use std::time::{SystemTime, UNIX_EPOCH};
                    let start = SystemTime::now();
                    let since_the_epoch = start.duration_since(UNIX_EPOCH)
                        .expect("Time went backwards");
                    let filename = format!("inline_script_{}.js", since_the_epoch.as_millis());
                    let filepath = debug_dir.join(filename);
                    if let Err(e) = fs::write(&filepath, &content) {
                        eprintln!("Failed to write inline script to {}: {}", filepath.display(), e);
                    } else {
                        println!("Saved inline script to {}", filepath.display());
                    }
                }

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

        // Save the script to a local file in debug_js/
        #[cfg(debug_assertions)]
        {
            use std::fs;
            use std::path::Path;

            let debug_dir = Path::new("debug_js");
            if !debug_dir.exists() {
                if let Err(e) = fs::create_dir_all(debug_dir) {
                    eprintln!("Failed to create debug_js directory: {}", e);
                }
            }

            let filename = script_url.split('/').last().unwrap_or("script.js");
            let filepath = debug_dir.join(filename);
            if let Err(e) = fs::write(&filepath, &script_content) {
                eprintln!("Failed to write script to {}: {}", filepath.display(), e);
            } else {
                println!("Saved external script to {}", filepath.display());
            }
        }
        Ok(script_content)
    }

    /// Handle a click at the given position (viewport coordinates)
    /// Returns the href of the clicked link, if any
    pub fn handle_click(&mut self, x: f32, y: f32) -> Option<String> {
        // Adjust position for scroll offset
        let adjusted_x = x + self.scroll_x;
        let adjusted_y = y + self.scroll_y;

        // Find the element at this position
        if let Some(layout) = &self.layout {
            if let Some(node_id) = self.find_element_at_position(layout, adjusted_x, adjusted_y) {
                // Fire click event on the element
                self.fire_click_event(node_id, x as f64, y as f64);

                // Check if this element or any parent is an anchor tag
                return self.find_link_href(node_id);
            }
        }

        None
    }

    /// Fire a click event on a DOM node
    fn fire_click_event(&mut self, node_id: usize, x: f64, y: f64) {
        if let Some(node_rc) = self.node_map.get(&node_id).cloned() {
            if let Some(runtime) = &mut self.js_runtime {
                let context = runtime.context_mut();

                println!("[Event] Firing click event at ({}, {}) on node {}", x, y, node_id);

                if let Err(e) = EventDispatcher::dispatch_mouse_event(
                    &node_rc,
                    EventType::Click,
                    x,
                    y,
                    context,
                ) {
                    eprintln!("Error dispatching click event: {}", e);
                }
            }
        }
    }

    /// Handle a mouse move at the given position (viewport coordinates)
    pub fn handle_mouse_move(&mut self, x: f32, y: f32) {
        // Adjust position for scroll offset
        let adjusted_x = x + self.scroll_x;
        let adjusted_y = y + self.scroll_y;

        // Find the element at this position
        if let Some(layout) = &self.layout {
            if let Some(node_id) = self.find_element_at_position(layout, adjusted_x, adjusted_y) {
                // Fire mouse move event on the element
                self.fire_mouse_move_event(node_id, x as f64, y as f64);
            }
        }
    }

    /// Fire a mouse move event on a DOM node
    fn fire_mouse_move_event(&mut self, node_id: usize, x: f64, y: f64) {
        if let Some(node_rc) = self.node_map.get(&node_id).cloned() {
            if let Some(runtime) = &mut self.js_runtime {
                let context = runtime.context_mut();

                if let Err(e) = EventDispatcher::dispatch_mouse_event(
                    &node_rc,
                    EventType::MouseMove,
                    x,
                    y,
                    context,
                ) {
                    eprintln!("Error dispatching mouse move event: {}", e);
                }
            }
        }
    }

    /// Handle a mouse down event at the given position
    pub fn handle_mouse_down(&mut self, x: f32, y: f32) {
        let adjusted_x = x + self.scroll_x;
        let adjusted_y = y + self.scroll_y;

        if let Some(layout) = &self.layout {
            if let Some(node_id) = self.find_element_at_position(layout, adjusted_x, adjusted_y) {
                self.fire_mouse_event(node_id, EventType::MouseDown, x as f64, y as f64);
            }
        }
    }

    /// Handle a mouse up event at the given position
    pub fn handle_mouse_up(&mut self, x: f32, y: f32) {
        let adjusted_x = x + self.scroll_x;
        let adjusted_y = y + self.scroll_y;

        if let Some(layout) = &self.layout {
            if let Some(node_id) = self.find_element_at_position(layout, adjusted_x, adjusted_y) {
                self.fire_mouse_event(node_id, EventType::MouseUp, x as f64, y as f64);
            }
        }
    }

    /// Fire a generic mouse event on a DOM node
    fn fire_mouse_event(&mut self, node_id: usize, event_type: EventType, x: f64, y: f64) {
        if let Some(node_rc) = self.node_map.get(&node_id).cloned() {
            if let Some(runtime) = &mut self.js_runtime {
                let context = runtime.context_mut();

                println!("[Event] Firing {:?} event at ({}, {}) on node {}", event_type, x, y, node_id);

                if let Err(e) = EventDispatcher::dispatch_mouse_event(
                    &node_rc,
                    event_type,
                    x,
                    y,
                    context,
                ) {
                    eprintln!("Error dispatching mouse event: {}", e);
                }
            }
        }
    }

    /// Handle a keyboard event
    pub fn handle_key_event(&mut self, event_type: EventType, key: String, key_code: u32) {
        // For keyboard events, we typically fire them on the focused element
        // For now, we'll fire on the document root
        if let Some(dom) = &self.dom {
            let root = dom.get_root();

            if let Some(runtime) = &mut self.js_runtime {
                let context = runtime.context_mut();

                println!("[Event] Firing {:?} event with key: {} (code: {})", event_type, key, key_code);

                if let Err(e) = EventDispatcher::dispatch_keyboard_event(
                    &root,
                    event_type,
                    key,
                    key_code,
                    context,
                ) {
                    eprintln!("Error dispatching keyboard event: {}", e);
                }
            }
        }
    }

    /// Handle a scroll event
    pub fn handle_scroll_event(&mut self) {
        if let Some(dom) = &self.dom {
            let root = dom.get_root();

            if let Some(runtime) = &mut self.js_runtime {
                let context = runtime.context_mut();

                println!("[Event] Firing scroll event");

                if let Err(e) = EventDispatcher::dispatch_simple_event(
                    &root,
                    EventType::Scroll,
                    context,
                ) {
                    eprintln!("Error dispatching scroll event: {}", e);
                }
            }
        }
    }

    /// Handle a resize event
    pub fn handle_resize_event(&mut self) {
        if let Some(dom) = &self.dom {
            let root = dom.get_root();

            if let Some(runtime) = &mut self.js_runtime {
                let context = runtime.context_mut();

                println!("[Event] Firing resize event");

                if let Err(e) = EventDispatcher::dispatch_simple_event(
                    &root,
                    EventType::Resize,
                    context,
                ) {
                    eprintln!("Error dispatching resize event: {}", e);
                }
            }
        }
    }

    /// Fire a load event (typically called after page is fully loaded)
    pub fn fire_load_event(&mut self) {
        if let Some(dom) = &self.dom {
            let root = dom.get_root();

            if let Some(runtime) = &mut self.js_runtime {
                let context = runtime.context_mut();

                println!("[Event] Firing load event");

                if let Err(e) = EventDispatcher::dispatch_simple_event(
                    &root,
                    EventType::Load,
                    context,
                ) {
                    eprintln!("Error dispatching load event: {}", e);
                }
            }
        }
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

    /// Process pending JavaScript timers (setTimeout/setInterval)
    /// Returns true if any timers were executed
    pub fn process_timers(&mut self) -> bool {
        if let Some(runtime) = &mut self.js_runtime {
            runtime.process_timers()
        } else {
            false
        }
    }

    /// Check if there are any active timers
    pub fn has_active_timers(&self) -> bool {
        if let Some(runtime) = &self.js_runtime {
            runtime.has_active_timers()
        } else {
            false
        }
    }

    /// Get the time until the next timer should fire
    pub fn time_until_next_timer(&self) -> Option<std::time::Duration> {
        if let Some(runtime) = &self.js_runtime {
            runtime.time_until_next_timer()
        } else {
            None
        }
    }
}
