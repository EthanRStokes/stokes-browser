// The core browser engine that coordinates between components
mod config;
pub mod net_provider;

pub use self::config::EngineConfig;
use crate::dom::node::{RasterImageData, SpecialElementData};
use crate::dom::{Dom, ImageData, NodeData};
use crate::dom::{EventDispatcher, EventType};
use crate::js::JsRuntime;
use crate::networking::{resolve_url, NetworkError, NewHttpClient, Resource};
use crate::renderer::painter::ScenePainter;
use crate::renderer::HtmlRenderer;
use blitz_traits::shell::{ShellProvider, Viewport};
use markup5ever::local_name;
use selectors::Element;
use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::Arc;
use style::context::{RegisteredSpeculativePainter, RegisteredSpeculativePainters};
use style::dom::{TDocument, TNode};
use style::thread_state::ThreadState;
use style::traversal::DomTraversal;
use url::Url;
use crate::networking;

thread_local! {
    pub(crate) static ENGINE_REF: RefCell<Option<*mut Engine>> = RefCell::new(None);
    pub(crate) static USER_AGENT_REF: RefCell<Option<String>> = RefCell::new(None);
}

/// The core browser engine that coordinates all browser activities
pub struct Engine {
    pub config: EngineConfig,
    new_http_client: Option<NewHttpClient>,
    current_url: String,
    page_title: String,
    is_loading: bool,
    pub(crate) dom: Option<Dom>,
    style_map_dirty: bool,
    scroll_y: f32,
    scroll_x: f32,
    content_height: f32,
    content_width: f32,
    pub(crate) viewport: Viewport,
    // JavaScript runtime
    js_runtime: Option<JsRuntime>,
    // Navigation history
    history: Vec<String>,
    history_index: Option<usize>,
    shell_provider: Arc<dyn ShellProvider>,
}

impl Engine {
    pub fn new(config: EngineConfig, viewport: Viewport, shell_provider: Arc<dyn ShellProvider>) -> Self {
        Self {
            config,
            new_http_client: None,
            current_url: String::new(),
            page_title: "New Tab".to_string(),
            is_loading: false,
            dom: None,
            style_map_dirty: false,
            scroll_y: 0.0,
            scroll_x: 0.0,
            content_height: 0.0,
            content_width: 0.0,
            viewport,
            js_runtime: None,
            history: Vec::new(),
            history_index: None,
            shell_provider,
        }
    }

    pub(crate) fn dom(&self) -> &Dom {
        self.dom.as_ref().unwrap()
    }

    pub(crate) fn dom_mut(&mut self) -> &mut Dom {
        self.dom.as_mut().unwrap()
    }

    /// Navigate to a new URL
    pub async fn navigate(&mut self, url: &str, invalidate_js: bool, history: bool) -> Result<(), NetworkError> {
        println!("Navigating to: {}", url);
        self.is_loading = true;
        self.current_url = url.to_string();

        // Fetch the page content
        let result = async {
            let html = networking::fetch(url, &self.config.user_agent).unwrap_or_else(|err| {
                include_str!("../../assets/404.html").to_string()
            });

            // Parse the HTML into our DOM
            let mut dom = Dom::parse_html(url, &html, self.viewport.clone(), self.shell_provider.clone());

            // Extract page title
            self.page_title = dom.get_title();

            // set http client
            self.new_http_client = Some(NewHttpClient {
                tx: dom.tx.clone(),
                dom_id: dom.id,
                net_provider: dom.net_provider.clone(),
                shell_provider: dom.shell_provider.clone()
            });

            // Store the DOM
            self.dom = Some(dom);
            if invalidate_js {
                let js = self.js_runtime.take();
                drop(js);
                self.js_runtime = None;
            }

            // Reset scroll position
            self.scroll_x = 0.0;
            self.scroll_y = 0.0;

            // Parse and apply CSS styles from the document
            self.parse_document_styles().await;

            // TODO Execute JavaScript in the page after everything is loaded
            if self.config.enable_javascript {
                style::thread_state::enter(ThreadState::SCRIPT);
                self.execute_document_scripts().await;
                style::thread_state::exit(ThreadState::SCRIPT);
            }

            {
                let dom = self.dom.as_mut().unwrap();

                dom.resolve();
            }

            // Calculate layout with CSS styles applied
            self.update_content_dimensions();

            Ok(())
        }.await;

        // Always reset loading state
        self.is_loading = false;

        // Add to history if navigation was successful
        if history && result.is_ok() {
            self.add_to_history(url.to_string());
        }

        result
    }

    /// Fetch a single image from a URL
    async fn fetch_image(&self, url: &str) -> Result<Vec<u8>, NetworkError> {
        // Resolve relative URLs against the current page URL
        let absolute_url = self.resolve_url(url)?;

        println!("Fetching image: {}", absolute_url);

        // Use the HTTP client to fetch the image data
        let image_bytes = networking::fetch_resource(&absolute_url, &self.config.user_agent).await?;

        // Validate that we got some data
        if image_bytes.is_empty() {
            return Err(NetworkError::Empty);
        }

        Ok(image_bytes)
    }

    /// Resolve a potentially relative URL against the current page URL
    pub fn resolve_url(&self, url: &str) -> Result<String, NetworkError> {
        resolve_url(&self.current_url, url)
    }

    /// Force reload images (useful for debugging or refresh)
    pub async fn reload_images(&mut self) {
        let dom = self.dom_mut();
        // Find all image nodes and reset their loading state
        let image_nodes = dom.find_node_ids(|node| node.data.element().is_some_and(|data| {
            matches!(data.special_data, SpecialElementData::Image(_))
        }));

        for image_node in image_nodes {
            let image_node = dom.get_node_mut(image_node).unwrap();
            if let NodeData::Element(ref mut image_data) = image_node.data {
                if let SpecialElementData::Image(image_data) = &mut image_data.special_data {
                    // TODO
                    //image_data.loading_state = ImageLoadingState::NotLoaded;
                }
            }
        }
        println!("ASDJOAJ")
    }

    /// Update the viewport size
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport.clone();
        if let Some(dom) = &mut self.dom {
            dom.set_viewport(viewport);
        }

        // Recalculate layout with new viewport
        style::thread_state::enter(ThreadState::LAYOUT);
        self.update_content_dimensions();
        style::thread_state::exit(ThreadState::LAYOUT);
    }

    /// Get the viewport size
    #[inline]
    pub fn viewport_size(&self) -> (u32, u32) {
        self.viewport.window_size
    }

    #[inline]
    pub fn viewport_width(&self) -> f32 {
        self.viewport.window_size.0 as f32
    }

    #[inline]
    pub fn viewport_height(&self) -> f32 {
        self.viewport.window_size.1 as f32
    }

    /// Get the content dimensions
    pub fn content_size(&self) -> (f32, f32) {
        (self.content_width, self.content_height)
    }

    /// Resize the viewport
    pub fn resize(&mut self, width: f32, height: f32) {
        self.set_viewport(Viewport {
            window_size: (width as u32, height as u32),
            ..self.viewport
        });

        // Update content dimensions after layout recalculation
        self.update_content_dimensions();
    }

    /// Render the current page to a canvas
    pub fn render(&mut self, painter: &mut ScenePainter) {
        {
            let dom = self.dom.as_mut().unwrap();

            dom.resolve();
        }

        let dom = self.dom.as_ref().unwrap();
        let node = dom.root_node();

        let mut renderer = HtmlRenderer {
            dom: &dom,
            scale_factor: self.viewport.scale_f64(),
            width: self.viewport_width() as u32,
            height: self.viewport_height() as u32,
            debug_hitboxes: self.config.debug_hitboxes,
        };

        renderer.render(
            painter,
            node,
        );
    }

    /// Add a CSS stylesheet to the engine
    pub fn add_stylesheet(&mut self, css_content: &str) {
        self.dom_mut().add_stylesheet(css_content);
    }

    /// Add an author CSS stylesheet (from <style> or <link> tags) to the engine
    pub fn add_author_stylesheet(&mut self, css_content: &str) {
        self.dom_mut().add_author_stylesheet(css_content);
    }

    /// Add a CSS stylesheet from a URL
    #[inline]
    pub async fn load_external_stylesheet(&mut self, css_url: &str) -> Result<(), NetworkError> {
        let absolute_url = self.resolve_url(css_url)?;
        let css_content = networking::fetch_resource(&absolute_url, &self.config.user_agent).await?;
        let css_content = String::from_utf8(css_content).expect("Failed to decode CSS content as UTF-8");
        self.add_author_stylesheet(&css_content);
        Ok(())
    }

    /// Extract and parse CSS from <style> tags and <link> tags in the current DOM
    pub async fn parse_document_styles(&mut self) {
        let dom = self.dom.as_mut().unwrap();

        // Collect style contents and link hrefs before any processing
        let mut style_contents: Vec<String> = Vec::new();
        let style_elements = dom.query_selector("style");
        for style_element in style_elements {
            let css_content = style_element.text_content();
            if !css_content.trim().is_empty() {
                style_contents.push(css_content);
            }
        }

        // Add all inline stylesheets from <style> tags
        for css_content in style_contents {
            self.add_author_stylesheet(&css_content);
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
        let max_scroll = (self.content_height - (self.viewport_height() / self.viewport.hidpi_scale)).max(0.0);
        self.scroll_y = self.scroll_y.min(max_scroll);

        self.update_dom_scroll();

        // Return whether scroll position actually changed
        old_scroll_y != self.scroll_y
    }

    /// Scroll horizontally by the given delta
    pub fn scroll_horizontal(&mut self, delta: f32) -> bool {
        let old_scroll_x = self.scroll_x;
        self.scroll_x = (self.scroll_x + delta).max(0.0);

        // Don't scroll past the right edge of the content
        let max_scroll = (self.content_width - (self.viewport_width() / self.viewport.hidpi_scale)).max(0.0);
        self.scroll_x = self.scroll_x.min(max_scroll);

        self.update_dom_scroll();

        // Return whether scroll position actually changed
        old_scroll_x != self.scroll_x
    }

    /// Get current scroll position
    pub fn scroll_position(&self) -> taffy::Point<f64> {
        self.dom().viewport_scroll
    }

    fn update_dom_scroll(&mut self) {
        let x = self.scroll_x as f64;
        let y = self.scroll_y as f64;

        let dom = self.dom_mut();
        dom.viewport_scroll.x = x;
        dom.viewport_scroll.y = y;
    }

    /// Set scroll position directly
    pub fn set_scroll_position(&mut self, x: f32, y: f32) {
        self.scroll_x = x.max(0.0).min((self.content_width - self.viewport_width()).max(0.0));
        self.scroll_y = y.max(0.0).min((self.content_height - self.viewport_height()).max(0.0));

        self.update_dom_scroll();
    }

    /// Update content dimensions based on layout
    pub(crate) fn update_content_dimensions(&mut self) {
        if let Some(dom) = &self.dom {
            let root_element = dom.root_element();
            let layout = root_element.final_layout;
            self.content_width = layout.size.width;
            self.content_height = layout.size.height;
        }
    }

    /// Initialize JavaScript runtime for the current document
    pub fn initialize_js_runtime(&mut self) {
        let user_agent = self.config.user_agent.clone();
        let dom = self.dom_mut();
        let dom = dom as *mut Dom;
        // TODO reimplement JavaScript
        match JsRuntime::new(dom, user_agent) {
            Ok(runtime) => {
                println!("JavaScript runtime initialized successfully");
                self.js_runtime = Some(runtime);
            }
            Err(e) => {
                eprintln!("Failed to initialize JavaScript runtime: {}", e);
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
        let mut script_items: Vec<(bool, String)> = Vec::new();

        let dom = self.dom();
        let script_elements = dom.query_selector("script");

        for script_element in script_elements {
            if let NodeData::Element(element_data) = &script_element.data {
                // Check for external scripts
                if let Some(src) = element_data.attr(local_name!("src")) {
                    println!("Found external script: {}", src);
                    script_items.push((true, src.to_string()));
                } else {
                    // Get inline script content
                    let script_content = script_element.text_content();
                    if !script_content.trim().is_empty() {
                        script_items.push((false, script_content));
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
        if script_url.contains("cloudflare") {
            return Err(NetworkError::Utf8("Blocked Cloudflare script".to_string()));
        }

        let absolute_url = self.resolve_url(script_url)?;
        let script_bytes = networking::fetch_resource(&absolute_url, &self.config.user_agent).await?;
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
        // Find the element at this position starting from root
        if let Some(dom) = &self.dom {
            let root_id = dom.root_element().id;
            if let Some(node_id) = dom.hover_node_id {
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
        let dom = self.dom.as_ref().unwrap();
        let node = dom.root_node().get_node(node_id);
        if let Some(runtime) = &mut self.js_runtime {
            let context = runtime.cx();

            println!("[Event] Firing click event at ({}, {}) on node {}", x, y, node_id);

            if let Err(e) = EventDispatcher::dispatch_mouse_event(
                node,
                EventType::Click,
                x,
                y,
                context,
            ) {
                eprintln!("Error dispatching click event: {}", e);
            }
        }
    }

    /// Handle a mouse move at the given position (viewport coordinates)
    pub fn handle_mouse_move(&mut self, x: f32, y: f32) {
        // Adjust position for scroll offset
        let adjusted_x = x + self.scroll_x;
        let adjusted_y = y + self.scroll_y;

        // Find the element at this position starting from root
        if let Some(dom) = &mut self.dom {
            dom.set_hover(adjusted_x, adjusted_y);

            // Fire mouse move event on the element
            self.fire_mouse_move_event(x as f64, y as f64);
        }
    }

    /// Fire a mouse move event on a DOM node
    fn fire_mouse_move_event(&mut self, x: f64, y: f64) {
        let dom = self.dom.as_ref().unwrap();
        let hover_node_id = match dom.hover_node_id {
            Some(hover_node_id) => hover_node_id,
            None => return,
        };

        let dom = self.dom.as_mut().unwrap();
        let root = dom.root_node().get_node(hover_node_id);

        let node = root.get_node(hover_node_id);
        if let Some(runtime) = &mut self.js_runtime {
            let context = runtime.cx();

            if let Err(e) = EventDispatcher::dispatch_mouse_event(
                &node,
                EventType::MouseMove,
                x,
                y,
                context,
            ) {
                eprintln!("Error dispatching mouse move event: {}", e);
            }
        }
    }

    /// Handle a mouse down event at the given position
    pub fn handle_mouse_down(&mut self, x: f32, y: f32) {
        if let Some(dom) = &self.dom {
            if let Some(node_id) = dom.hover_node_id {
                self.fire_mouse_event(node_id, EventType::MouseDown, x as f64, y as f64);
            }
        }
    }

    /// Handle a mouse up event at the given position
    pub fn handle_mouse_up(&mut self, x: f32, y: f32) {
        if let Some(dom) = &self.dom {
            if let Some(node_id) = dom.hover_node_id {
                self.fire_mouse_event(node_id, EventType::MouseUp, x as f64, y as f64);
            }
        }
    }

    /// Fire a generic mouse event on a DOM node
    fn fire_mouse_event(&mut self, node_id: usize, event_type: EventType, x: f64, y: f64) {
        let dom = self.dom.as_ref().unwrap();
        let root = dom.root_node();

        let node = root.get_node(node_id);
        if let Some(runtime) = &mut self.js_runtime {
            let context = runtime.cx();

            println!("[Event] Firing {:?} event at ({}, {}) on node {}", event_type, x, y, node_id);

            if let Err(e) = EventDispatcher::dispatch_mouse_event(
                &node,
                event_type,
                x,
                y,
                context,
            ) {
                eprintln!("Error dispatching mouse event: {}", e);
            }
        }
    }

    /// Handle a keyboard event
    pub fn handle_key_event(&mut self, event_type: EventType, key: String, key_code: u32) {
        // For keyboard events, we typically fire them on the focused element
        // For now, we'll fire on the document root
        let dom = self.dom.as_ref().unwrap();

        let root = dom.root_node();

        if let Some(runtime) = &mut self.js_runtime {
            let context = runtime.cx();

            println!("[Event] Firing {:?} event with key: {} (code: {})", event_type, key, key_code);

            if let Err(e) = EventDispatcher::dispatch_keyboard_event(
                root,
                event_type,
                key,
                key_code,
                context,
            ) {
                eprintln!("Error dispatching keyboard event: {}", e);
            }
        }
    }

    /// Handle a scroll event
    pub fn handle_scroll_event(&mut self) {
        let dom = self.dom.as_ref().unwrap();
        let root = dom.root_node();

        if let Some(runtime) = &mut self.js_runtime {
            let context = runtime.cx();

            println!("[Event] Firing scroll event");

            if let Err(e) = EventDispatcher::dispatch_simple_event(
                root,
                EventType::Scroll,
                context,
            ) {
                eprintln!("Error dispatching scroll event: {}", e);
            }
        }
    }

    /// Handle a resize event
    pub fn handle_resize_event(&mut self) {
        let dom = self.dom.as_ref().unwrap();

        let root = dom.root_node();

        if let Some(runtime) = &mut self.js_runtime {
            let context = runtime.cx();

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

    /// Fire a load event (typically called after page is fully loaded)
    pub fn fire_load_event(&mut self) {
        let dom = self.dom.as_ref().unwrap();

        let root = dom.root_node();

        if let Some(runtime) = &mut self.js_runtime {
            let context = runtime.cx();

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

    /// Find the href of a link element by checking the node and its ancestors
    fn find_link_href(&self, node_id: usize) -> Option<String> {
        // Walk up the DOM tree to find an anchor element
        let mut visited = std::collections::HashSet::new();
        let mut current_id = node_id;

        // Limit depth to prevent infinite loops
        let max_depth = 50;
        let mut depth = 0;

        let dom = self.dom.as_ref().unwrap();
        let root = dom.root_node();

        loop {
            if depth >= max_depth || visited.contains(&current_id) {
                break;
            }
            visited.insert(current_id);
            depth += 1;

            let node = root.get_node(current_id);
            // Check if this is an anchor element with an href attribute
            if let NodeData::Element(element_data) = &node.data {
                if element_data.name.local.to_string() == "a" {
                    if let Some(href) = element_data.attr(local_name!("href")) {
                        return Some(href.to_string());
                    }
                }
            }

            // Move to parent
            if let Some(parent) = node.parent_node() {
                let parent_id = parent.id;
                if parent_id == current_id {
                    // Safety check: prevent infinite loop if node is its own parent
                    break;
                }
                current_id = parent_id;
            } else {
                // No parent, we've reached the root
                break;
            }
        }

        None
    }

    // TODO reimplement javascript
    /*/// Process pending JavaScript timers (setTimeout/setInterval)
    /// Returns true if any timers were executed
    #[inline]
    pub fn process_timers(&mut self) -> bool {
        if let Some(runtime) = &mut self.js_runtime {
            runtime.process_timers()
        } else {
            false
        }
    }

    /// Check if there are any active timers
    #[inline]
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
    }*/

    /// Add a URL to the navigation history
    fn add_to_history(&mut self, url: String) {
        // If we're not at the end of history, truncate everything after current position
        if let Some(index) = self.history_index {
            self.history.truncate(index + 1);
        }
        
        // Add the new URL
        self.history.push(url);
        self.history_index = Some(self.history.len() - 1);
    }

    /// Check if we can navigate back
    #[inline]
    pub fn can_go_back(&self) -> bool {
        if let Some(index) = self.history_index {
            index > 0
        } else {
            false
        }
    }

    /// Check if we can navigate forward
    #[inline]
    pub fn can_go_forward(&self) -> bool {
        if let Some(index) = self.history_index {
            index < self.history.len().saturating_sub(1)
        } else {
            false
        }
    }

    /// Navigate back in history
    pub async fn go_back(&mut self) -> Result<(), NetworkError> {
        if !self.can_go_back() {
            return Err(NetworkError::Curl("Cannot go back: no previous page".to_string()));
        }

        if let Some(index) = self.history_index {
            self.history_index = Some(index - 1);
            let url = self.history[index - 1].clone();
            self.navigate(&url, true, false).await
        } else {
            Err(NetworkError::Curl("Invalid history state".to_string()))
        }
    }

    /// Navigate forward in history
    pub async fn go_forward(&mut self) -> Result<(), NetworkError> {
        if !self.can_go_forward() {
            return Err(NetworkError::Curl("Cannot go forward: no next page".to_string()));
        }

        if let Some(index) = self.history_index {
            self.history_index = Some(index + 1);
            let url = self.history[index + 1].clone();
            self.navigate(&url, true, false).await
        } else {
            Err(NetworkError::Curl("Invalid history state".to_string()))
        }
    }
}
