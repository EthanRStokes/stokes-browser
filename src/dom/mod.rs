// DOM module for parsing and representing HTML content
mod parser;
pub(crate) mod node;
mod events;

use std::cell::RefCell;
use std::rc::Rc;
use slab::Slab;
pub use self::events::{EventDispatcher, EventType};
pub use self::node::{AttributeMap, DomNode, ElementData, ImageData, ImageLoadingState, NodeData};
pub use self::parser::HtmlParser;

/// Represents a DOM tree
#[derive(Debug)]
pub struct Dom {
    pub(crate) nodes: Box<Slab<Rc<RefCell<DomNode>>>>,
}

impl Dom {
    /// Create a new empty DOM
    pub fn new() -> Self {
        let mut dom = Self {
            nodes: Box::new(Slab::new()),
            //root: Rc::new(RefCell::from(DomNode::new(NodeData::Document, None))),
        };

        // Create the root document node
        dom.create_node(NodeData::Document);

        dom
    }

    pub(crate) fn create_node(&mut self, data: NodeData) -> usize {
        let slab_ptr = self.nodes.as_mut() as *mut Slab<Rc<RefCell<DomNode>>>;

        let entry = self.nodes.vacant_entry();
        let id = entry.key();
        entry.insert(Rc::new(RefCell::new(DomNode::new(slab_ptr, id, data))));

        id
    }

    /// Parse HTML into a DOM
    pub fn parse_html(html: &str) -> Self {
        let parser = HtmlParser::new();
        parser.parse(html)
    }

    /// Find nodes by tag name
    pub fn query_selector(&mut self, selector: &str) -> Vec<Rc<RefCell<DomNode>>> {
        let ids = self.root_node().borrow().query_selector(selector);
        ids.into_iter()
            .filter_map(|id| self.nodes.get(id).cloned())
            .collect()
    }

    /// Find nodes that match a predicate
    pub fn find_nodes<F>(&mut self, predicate: F) -> Vec<Rc<RefCell<DomNode>>>
    where
        F: Fn(&DomNode) -> bool + Clone,
    {
        let ids = self.root_node().borrow().find_nodes(predicate);
        ids.into_iter()
            .filter_map(|id| self.nodes.get(id).cloned())
            .collect()
    }

    /// Extract the page title
    pub fn get_title(&mut self) -> String {
        // Find the title element in the head
        let title_nodes = self.query_selector("title");
        if let Some(title_node) = title_nodes.first() {
            // Get text content of the title
            title_node.borrow().text_content()
        } else {
            // Default title if not found
            "Untitled".to_string()
        }
    }

    pub(crate) fn root_node(&self) -> &Rc<RefCell<DomNode>> {
        &self.nodes[0]
    }

    pub(crate) fn root_node_mut(&mut self) -> &mut Rc<RefCell<DomNode>> {
        &mut self.nodes[0]
    }
}
