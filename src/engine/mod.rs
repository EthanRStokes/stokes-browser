// The core browser engine that coordinates between components
mod config;
pub mod net_provider;
pub mod nav_provider;
pub mod resolve;
pub mod js_provider;

pub use self::config::EngineConfig;
use crate::dom::{Dom, NodeData};
use crate::engine::js_provider::{JsProviderMessage, StokesJsProvider};
use crate::engine::nav_provider::StokesNavigationProvider;
use crate::js::JsRuntime;
use crate::networking;
use crate::networking::{HttpClient, NetworkError};
use crate::renderer::painter::ScenePainter;
use crate::renderer::HtmlRenderer;
use crate::shell_provider::StokesShellProvider;
use blitz_traits::net::Request;
use blitz_traits::shell::Viewport;
use markup5ever::local_name;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use style::dom::TNode;
use style::thread_state::ThreadState;

thread_local! {
    pub(crate) static ENGINE_REF: RefCell<Option<*mut Engine>> = RefCell::new(None);
    pub(crate) static USER_AGENT_REF: RefCell<Option<String>> = RefCell::new(None);
}

/// The core browser engine that coordinates all browser activities
pub struct Engine {
    pub config: EngineConfig,
    new_http_client: Option<HttpClient>,
    current_url: String,
    page_title: String,
    is_loading: bool,
    pub(crate) dom: Option<Dom>,
    content_height: f32,
    content_width: f32,
    pub(crate) viewport: Viewport,
    // JavaScript runtime
    js_runtime: Option<JsRuntime>,
    // Navigation history
    history: Vec<String>,
    history_index: Option<usize>,
    shell_provider: Arc<StokesShellProvider>,
    pub(crate) navigation_provider: Arc<StokesNavigationProvider>,
    pub(crate) js_rx: Option<Receiver<JsProviderMessage>>,
    pub js_provider: Arc<StokesJsProvider>,
}

impl Engine {
    pub fn new(config: EngineConfig, viewport: Viewport, shell_provider: Arc<StokesShellProvider>, navigation_provider: Arc<StokesNavigationProvider>) -> Self {
        let (js_tx, js_rx) = channel();
        let js_provider = Arc::new(StokesJsProvider::new(js_tx));

        Self {
            config,
            new_http_client: None,
            current_url: String::new(),
            page_title: "New Tab".to_string(),
            is_loading: false,
            dom: None,
            content_height: 0.0,
            content_width: 0.0,
            viewport,
            js_runtime: None,
            history: Vec::new(),
            history_index: None,
            shell_provider,
            navigation_provider,
            js_rx: Some(js_rx),
            js_provider,
        }
    }

    pub(crate) fn dom(&self) -> &Dom {
        self.dom.as_ref().unwrap()
    }

    pub(crate) fn dom_mut(&mut self) -> &mut Dom {
        self.dom.as_mut().unwrap()
    }

    /// Navigate to a new URL
    pub async fn navigate(&mut self, url: &str, contents: String, invalidate_js: bool, history: bool) -> Result<(), NetworkError> {
        println!("Navigating to: {}", url);
        self.is_loading = true;
        self.current_url = url.to_string();

        // Fetch the page content
        let result = async {


            // Parse the HTML into our DOM
            let dom = Dom::parse_html(
                url,
                &contents,
                self.config.user_agent.clone(),
                self.viewport.clone(),
                self.shell_provider.clone(),
                self.navigation_provider.clone(),
                self.js_provider.clone(),
            );

            // Extract page title
            self.page_title = dom.get_title();

            // set http client
            self.new_http_client = Some(HttpClient {
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

            // Parse and apply CSS styles from the document
            self.parse_document_styles().await;

            // TODO Execute JavaScript in the page after everything is loaded
            if self.config.enable_javascript {
                style::thread_state::enter(ThreadState::SCRIPT);
                self.execute_document_scripts().await;
                style::thread_state::exit(ThreadState::SCRIPT);
            }

            self.resolve(0.0);

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

    /// Update the viewport size
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport.clone();
        if let Some(dom) = &mut self.dom {
            dom.set_viewport(viewport);
        }

        if let Some(runtime) = &mut self.js_runtime {
            let width = self.viewport.window_size.0;
            let height = self.viewport.window_size.1;
            let script = format!(
                "if (typeof globalThis !== 'undefined') {{\
                    globalThis.innerWidth = {width};\
                    globalThis.outerWidth = {width};\
                    globalThis.innerHeight = {height};\
                    globalThis.outerHeight = {height};\
                    if (typeof globalThis.__notifyMatchMediaListeners === 'function') {{\
                        globalThis.__notifyMatchMediaListeners();\
                    }}\
                }}"
            );
            let _ = runtime.execute_script(&script);
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
    pub fn render(&mut self, painter: &mut ScenePainter, now: f64) {
        self.resolve(now);

        let dom = self.dom.as_ref().unwrap();
        let node = dom.root_node();

        let selection: HashMap<usize, (usize, usize)> = dom
            .get_text_selection_ranges()
            .into_iter()
            .map(|(node_id, start, end)| (node_id, (start, end)))
            .collect();

        let mut renderer = HtmlRenderer {
            dom: &dom,
            scale_factor: self.viewport.scale_f64(),
            width: self.viewport_width() as u32,
            height: self.viewport_height() as u32,
            selection_ranges: selection,
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

    /// Get current scroll position
    pub fn scroll_position(&self) -> taffy::Point<f64> {
        self.dom().viewport_scroll
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

        let dom = self.dom();
        let script_elements = dom.query_selector("script");

        for script_element in script_elements {
            if let NodeData::Element(element_data) = &script_element.data {
                // Check for external scripts
                if let Some(src) = element_data.attr(local_name!("src")) {
                    println!("Found external script: {}", src);
                    let net_provider = dom.net_provider.clone();
                    let js_provider = self.js_provider.clone();
                    let url = dom.resolve_url(src);
                    net_provider.fetch_with_callback(
                        Request::get(url),
                        Box::new(move |result| {
                            js_provider.execute_script(String::from_utf8(Vec::from(result.unwrap().1)).unwrap());
                        })
                    );
                } else {
                    // Get inline script content
                    let script_content = script_element.text_content();
                    if !script_content.trim().is_empty() {
                        self.js_provider.execute_script(script_content);
                    }
                }
            }
        }
    }

    /// Process pending JavaScript timers (setTimeout/setInterval)
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
    }

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
            let contents = networking::fetch(&url, &self.config.user_agent).unwrap_or_else(|err| {
                include_str!("../../assets/404.html").to_string()
            });
            self.navigate(&url, contents, true, false).await
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
            let contents = networking::fetch(&url, &self.config.user_agent).unwrap_or_else(|err| {
                include_str!("../../assets/404.html").to_string()
            });
            self.navigate(&url, contents, true, false).await
        } else {
            Err(NetworkError::Curl("Invalid history state".to_string()))
        }
    }
}
