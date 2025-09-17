// DOM module for parsing and representing HTML content
mod parser;
mod node;

pub use self::parser::HtmlParser;
pub use self::node::{DomNode, NodeType, ElementData, AttributeMap};

/// Represents a DOM tree
pub struct Dom {
    pub root: DomNode,
}

impl Dom {
    /// Create a new empty DOM
    pub fn new() -> Self {
        Self {
            root: DomNode::new(NodeType::Document, None),
        }
    }

    /// Parse HTML into a DOM
    pub fn parse_html(html: &str) -> Self {
        let parser = HtmlParser::new();
        parser.parse(html)
    }

    /// Find nodes by tag name
    pub fn query_selector(&self, selector: &str) -> Vec<&DomNode> {
        self.root.query_selector(selector)
    }

    /// Find nodes that match a predicate
    pub fn find_nodes<F>(&self, predicate: F) -> Vec<&DomNode>
    where
        F: Fn(&DomNode) -> bool,
    {
        self.root.find_nodes(predicate)
    }

    /// Extract the page title
    pub fn get_title(&self) -> String {
        // Find the title element in the head
        let title_nodes = self.query_selector("title");
        if let Some(title_node) = title_nodes.first() {
            // Get text content of the title
            title_node.text_content()
        } else {
            // Default title if not found
            "Untitled Page".to_string()
        }
    }
}
