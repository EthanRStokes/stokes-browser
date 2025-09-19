// The core browser engine that coordinates between components
mod config;

use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use skia_safe::Canvas;
use crate::networking::HttpClient;
use crate::dom::{Dom, DomNode};
use crate::layout::{LayoutEngine, LayoutBox};
use crate::renderer::HtmlRenderer;

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
        
        // Calculate layout
        self.recalculate_layout();
        
        self.is_loading = false;
        Ok(())
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

            // Render the layout with scroll offset
            renderer.render_with_scroll(canvas, layout, &self.node_map, self.scroll_x, self.scroll_y);
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
