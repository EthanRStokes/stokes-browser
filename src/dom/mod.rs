// DOM module for parsing and representing HTML content
mod parser;
mod node;
mod events;

use std::cell::{RefCell, RefMut};
use std::rc::Rc;

pub use self::events::{EventDispatcher, EventType};
pub use self::node::{AttributeMap, DomNode, ElementData, ImageData, ImageLoadingState, NodeType};
pub use self::parser::HtmlParser;

/// Represents a DOM tree
pub struct Dom {
    pub root: Rc<RefCell<DomNode>>,
}

impl Dom {
    /// Create a new empty DOM
    pub fn new() -> Self {
        Self {
            root: Rc::new(RefCell::from(DomNode::new(NodeType::Document, None))),
        }
    }

    /// Parse HTML into a DOM
    pub fn parse_html(html: &str) -> Self {
        let parser = HtmlParser::new();
        parser.parse(html)
    }

    /// Find nodes by tag name
    pub fn query_selector(&mut self, selector: &str) -> Vec<Rc<RefCell<DomNode>>> {
        self.get_mut_root().query_selector(selector)
    }

    /// Find nodes that match a predicate
    pub fn find_nodes<F>(&mut self, predicate: F) -> Vec<Rc<RefCell<DomNode>>>
    where
        F: Fn(&DomNode) -> bool + Clone,
    {
        self.get_mut_root().find_nodes(predicate)
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

    /// Get the root node as Rc<RefCell<DomNode>>
    pub fn get_root(&self) -> Rc<RefCell<DomNode>> {
        Rc::clone(&self.root)
    }

    pub fn get_mut_root(&mut self) -> RefMut<'_, DomNode> {
        self.root.borrow_mut()
    }
}
