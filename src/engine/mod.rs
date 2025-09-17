// The core browser engine that coordinates between components
mod config;

use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::Arc;

use crate::networking::HttpClient;
use crate::dom::{Dom, DomNode};
//use crate::layout::{LayoutEngine, LayoutBox};
//use crate::renderer::HtmlRenderer;

pub use self::config::EngineConfig;

/// The core browser engine that coordinates all browser activities
pub struct Engine {
    pub config: EngineConfig,
    http_client: HttpClient,
    current_url: String,
    page_title: String,
    is_loading: bool,
    dom: Option<Dom>,
    //layout: Option<LayoutBox>,
    //layout_engine: LayoutEngine,
    node_map: HashMap<usize, Rc<RefCell<DomNode>>>,
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
        //    layout: None,
        //    layout_engine: LayoutEngine::new(800.0, 600.0), // Default viewport size
            node_map: HashMap::new(),
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
    //        self.layout = Some(self.layout_engine.compute_layout(&root_node));
            
            // Get node map for renderer
    //        self.node_map = self.layout_engine.get_node_map();
        }
    }
    
    /// Resize the viewport
    pub fn resize(&mut self, width: f32, height: f32) {
    //    self.layout_engine = LayoutEngine::new(width, height);
        self.recalculate_layout();
    }
    
    /// Render the current page to a canvas
    pub fn render(&self, canvas: &mut skia_safe::Canvas) {
    //    if let Some(layout) = &self.layout {
            // Create a renderer (or use a cached one)
            // In a real implementation, you'd want to cache this
    //        let renderer = HtmlRenderer::new(
    //            Arc::new(wgpu::Device::dummy()),  // You'd want real device/queue here
    //            Arc::new(wgpu::Queue::dummy()),
    //            wgpu::SurfaceConfiguration::default(),
    //        );

            // Render the layout
    //        renderer.render(canvas, layout, &self.node_map);
    //    }
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
}
