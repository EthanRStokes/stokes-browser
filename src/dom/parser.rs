// HTML parser using html5ever
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom as rcdom;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use markup5ever_rcdom::{Handle, NodeData};
use super::{Dom, DomNode, NodeType, ElementData, AttributeMap, ImageData};

/// HTML Parser for converting HTML strings into DOM structures
pub struct HtmlParser;

impl HtmlParser {
    pub fn new() -> Self {
        Self {}
    }

    /// Parse HTML string into a DOM structure
    pub fn parse(&self, html: &str) -> Dom {
        // Parse with html5ever
        let parser = parse_document(rcdom::RcDom::default(), Default::default());
        let rcdom = parser.one(html);

        // Convert RcDom to our DOM structure
        let mut dom = Dom::new();
        self.build_dom_from_handle(&rcdom.document, None, &mut dom.root);

        dom
    }

    /// Convert html5ever's DOM structure to our DOM structure
    fn build_dom_from_handle(
        &self, 
        handle: &Handle, 
        parent: Option<Weak<RefCell<DomNode>>>, // Remove underscore since we'll use it
        target_node: &mut DomNode
    ) {
        let node = handle;

        // Set the parent reference in the target node
        target_node.parent = parent;

        // Set the node type based on the html5ever node data
        match node.data {
            NodeData::Document => {
                // Document node, just process children
                target_node.node_type = NodeType::Document;
            },
            NodeData::Element { ref name, ref attrs, .. } => {
                // Element node
                let tag_name = name.local.to_string();
                
                // Process attributes
                let mut attributes = AttributeMap::new();
                for attr in attrs.take() {
                    let name = attr.name.local.to_string();
                    let value = attr.value.to_string();
                    attributes.insert(name, value);
                }
                
                // Special handling for img tags
                if tag_name == "img" {
                    let src = attributes.get("src").cloned().unwrap_or_default();
                    let alt = attributes.get("alt").cloned().unwrap_or_default();
                    let mut image_data = ImageData::new(src, alt);

                    // Parse width and height attributes if present
                    if let Some(width_str) = attributes.get("width") {
                        if let Ok(width) = width_str.parse::<u32>() {
                            image_data.width = Some(width);
                        }
                    }
                    if let Some(height_str) = attributes.get("height") {
                        if let Ok(height) = height_str.parse::<u32>() {
                            image_data.height = Some(height);
                        }
                    }

                    target_node.node_type = NodeType::Image(image_data);
                } else {
                    target_node.node_type = NodeType::Element(ElementData::with_attributes(&tag_name, attributes));
                }
            },
            NodeData::Text { ref contents } => {
                // Text node - process whitespace according to HTML rules
                let raw_text = contents.borrow().to_string();
                let processed_text = self.process_html_whitespace(&raw_text);
                target_node.node_type = NodeType::Text(processed_text);
            },
            NodeData::Comment { ref contents } => {
                // Comment node
                let comment = contents.to_string();
                target_node.node_type = NodeType::Comment(comment);
            },
            // Ignore other node types
            _ => {}
        }

        // Process children
        for child_handle in node.children.take().iter() {
            // Skip processing if this is a doctype node
            if let NodeData::Doctype { .. } = child_handle.data {
                continue;
            }

            // Create a new child node
            let child_node = DomNode::new(NodeType::Document, None);  // Temporary type

            // Add the child to the parent first to get the Rc reference
            let child_rc = target_node.add_child(child_node);
            
            // Create a weak reference to pass as parent to the recursive call
            let parent_weak = Some(Rc::downgrade(&child_rc));
            
            // Get a mutable reference to the child for the recursive call
            let mut child_ref = child_rc.borrow_mut();
            
            // Recursively build the DOM for this child, passing the current node as parent
            self.build_dom_from_handle(child_handle, parent_weak, &mut child_ref);
        }
    }

    /// Process raw HTML whitespace in text nodes according to HTML standards
    fn process_html_whitespace(&self, raw_text: &str) -> String {
        // HTML whitespace processing rules:
        // 1. Convert sequences of whitespace characters to single spaces
        // 2. Preserve explicit line breaks (\n) as they may be intentional
        // 3. Trim leading and trailing whitespace from text nodes

        if raw_text.trim().is_empty() {
            return String::new();
        }

        // Replace sequences of spaces and tabs with single spaces
        // but preserve newlines as they represent intentional line breaks
        let mut result = String::new();
        let mut prev_was_space = false;

        for ch in raw_text.chars() {
            match ch {
                ' ' | '\t' | '\r' => {
                    if !prev_was_space {
                        result.push(' ');
                        prev_was_space = true;
                    }
                }
                '\n' => {
                    // Preserve newlines for proper line break handling
                    result.push('\n');
                    prev_was_space = false;
                }
                _ => {
                    result.push(ch);
                    prev_was_space = false;
                }
            }
        }

        // Trim whitespace from start and end, but preserve internal structure
        result.trim().to_string()
    }
}
