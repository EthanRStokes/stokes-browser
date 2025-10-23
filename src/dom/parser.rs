use super::{AttributeMap, Dom, DomNode, ElementData, ImageData, NodeData};
// HTML parser using html5ever
use html5ever::parse_document;
use html5ever::tendril::{StrTendril, TendrilSink};
use markup5ever_rcdom as rcdom;
use markup5ever_rcdom::{Handle, NodeData as EverNodeData};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

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
        self.build_dom_from_handle(&rcdom.document, None, &mut dom);

        dom
    }

    /// Convert html5ever's DOM structure to our DOM structure
    fn build_dom_from_handle(
        &self, 
        handle: &Handle, 
        parent: Option<Weak<RefCell<DomNode>>>, // Remove underscore since we'll use it
        dom: &mut Dom,
    ) {
        // Get the target node (Rc<RefCell<DomNode>>) for this position in our DOM
        let target_rc = dom.root_node().clone();

        // Set the parent reference and node data while holding a short-lived borrow
        {
            let mut target_node = target_rc.borrow_mut();
            // Set the parent reference in the target node
            target_node.parent = parent;

            // Set the node type based on the html5ever node data
            match handle.data {
                EverNodeData::Document => {
                    // Document node, just process children
                    target_node.data = NodeData::Document;
                },
                EverNodeData::Element { ref name, ref attrs, .. } => {
                    // Element node
                    let tag_name = name.local.to_string();

                    // Process attributes
                    let mut attributes = AttributeMap::new();
                    for attr in attrs.borrow().iter() {
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

                        println!("Found image: src='{}', alt='{}', width={:?}, height={:?}",
                            image_data.src, image_data.alt, image_data.width, image_data.height);
                        target_node.data = NodeData::Image(RefCell::new(image_data));
                    } else {
                        target_node.data = NodeData::Element(ElementData::with_attributes(name.clone(), attributes));
                    }
                },
                EverNodeData::Text { ref contents } => {
                    // Text node - process whitespace according to HTML rules
                    let raw_text = contents.borrow().to_string();
                    let processed_text = self.process_html_whitespace(&raw_text);
                    target_node.data = NodeData::Text { contents: RefCell::new(StrTendril::from(processed_text)) };
                },
                EverNodeData::Comment { ref contents } => {
                    // Comment node
                    let comment = contents.to_string();
                    target_node.data = NodeData::Comment { contents: StrTendril::from(comment) };
                },
                // Ignore other node types
                _ => {}
            }
        } // target_node borrow dropped here

        // Clone children list while borrowed immutably, so we can iterate without holding any borrow
        let children: Vec<Handle> = {
            let children_borrow = handle.children.borrow();
            children_borrow.clone()
        };

        // Process children
        for child_handle in children.iter() {
            // Skip processing if this is a doctype node
            if let EverNodeData::Doctype { .. } = child_handle.data {
                continue;
            }

            // Create a new child node (temporary type) and get its id
            let child_id = dom.create_node(NodeData::Document);

            // Add the child id to the parent's children inside a short-lived borrow
            {
                let mut target_node = target_rc.borrow_mut();
                target_node.children.push(child_id);

                // If the parent has a layout invalidation callback, propagate it to the child
                if let Some(callback) = &target_node.layout_invalidation_callback {
                    let child_rc = dom.nodes[child_id].clone();
                    if let Ok(mut child_node) = child_rc.try_borrow_mut() {
                        child_node.set_layout_invalidation_callback(callback.clone());
                    }
                }
                // Note: we don't call invalidate_layout() directly here because it's private to DomNode; add_child would call it.
            }

            // Get an owned Rc for the child so we can set its parent and create a Weak for recursion
            let child_rc = dom.nodes[child_id].clone();

            // Update the child's parent reference
            child_rc.borrow_mut().parent = Some(Rc::downgrade(&target_rc));

            // Create a weak reference to pass as parent to the recursive call (child becomes the parent for its own children)
            let parent_weak = Some(Rc::downgrade(&child_rc));

             // Recursively build the DOM for this child, passing the current node as parent
             self.build_dom_from_handle(child_handle, parent_weak, dom);
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
