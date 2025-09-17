// HTML parser using html5ever
use html5ever::parse_document;
use html5ever::rcdom::{Handle, NodeData, RcDom};
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom as rcdom;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use super::{Dom, DomNode, NodeType, ElementData, AttributeMap};

/// HTML Parser for converting HTML strings into DOM structures
pub struct HtmlParser;

impl HtmlParser {
    pub fn new() -> Self {
        Self {}
    }

    /// Parse HTML string into a DOM structure
    pub fn parse(&self, html: &str) -> Dom {
        // Parse with html5ever
        let parser = parse_document(RcDom::default(), Default::default());
        let rcdom = parser.one(html.into());

        // Convert RcDom to our DOM structure
        let mut dom = Dom::new();
        self.build_dom_from_handle(&rcdom.document, None, &mut dom.root);

        dom
    }

    /// Convert html5ever's DOM structure to our DOM structure
    fn build_dom_from_handle(
        &self, 
        handle: &Handle, 
        parent: Option<Weak<RefCell<DomNode>>>,
        target_node: &mut DomNode
    ) {
        let node = handle;
        let borrow = node.borrow();

        // Set the node type based on the html5ever node data
        match borrow.data {
            NodeData::Document => {
                // Document node, just process children
                target_node.node_type = NodeType::Document;
            },
            NodeData::Element { ref name, ref attrs, .. } => {
                // Element node
                let tag_name = name.local.to_string();
                
                // Process attributes
                let mut attributes = AttributeMap::new();
                for attr in attrs.iter() {
                    let name = attr.name.local.to_string();
                    let value = attr.value.to_string();
                    attributes.insert(name, value);
                }
                
                target_node.node_type = NodeType::Element(ElementData::with_attributes(&tag_name, attributes));
            },
            NodeData::Text { ref contents } => {
                // Text node
                let text_content = contents.borrow().to_string();
                target_node.node_type = NodeType::Text(text_content);
            },
            NodeData::Comment { ref contents } => {
                // Comment node
                let comment = contents.borrow().to_string();
                target_node.node_type = NodeType::Comment(comment);
            },
            // Ignore other node types
            _ => {}
        }

        // Process children
        for child_handle in borrow.children.iter() {
            // Skip processing if this is a doctype node
            if let NodeData::Doctype { .. } = child_handle.borrow().data {
                continue;
            }

            // Create a new child node
            let mut child_node = DomNode::new(NodeType::Document, None);  // Temporary type
            
            // Get a weak reference to parent
            let parent_weak = Rc::downgrade(&Rc::new(RefCell::new(target_node.clone())));
            
            // Recursively build the DOM for this child
            self.build_dom_from_handle(child_handle, Some(parent_weak), &mut child_node);
            
            // Add the child to the parent
            target_node.add_child(child_node);
        }
    }
}
