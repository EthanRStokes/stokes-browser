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
        // Determine node type from rcdom
        match &handle.data {
            EverNodeData::Document => {
                // Our Dom already has a root Document node at id 0
                let root_id = 0usize;
                // Recurse into children of the document, setting parent to root
                let parent_weak = Some(Rc::downgrade(&dom.nodes[root_id]));
                let children = handle.children.borrow();
                for child in children.iter() {
                    self.build_dom_from_handle(child, parent_weak.clone(), dom);
                }
            }
            EverNodeData::Doctype { name, public_id, system_id } => {
                // Create a DocType node
                let data = NodeData::DocType {
                    name: name.clone(),
                    public_id: public_id.clone(),
                    system_id: system_id.clone(),
                };
                let id = dom.create_node(data);
                // Attach to parent if provided
                if let Some(p) = parent {
                    if let Some(parent_rc) = p.upgrade() {
                        // set parent on new node and add as child
                        let node_rc = dom.nodes[id].clone();
                        node_rc.borrow_mut().parent = Some(Rc::downgrade(&parent_rc));
                        parent_rc.borrow_mut().children.push(id);
                    }
                }
            }
            EverNodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                let processed = self.process_html_whitespace(&text);
                if processed.is_empty() {
                    return;
                }
                let data = NodeData::Text { contents: RefCell::new(StrTendril::from(processed)) };
                let id = dom.create_node(data);
                if let Some(p) = parent {
                    if let Some(parent_rc) = p.upgrade() {
                        let node_rc = dom.nodes[id].clone();
                        node_rc.borrow_mut().parent = Some(Rc::downgrade(&parent_rc));
                        parent_rc.borrow_mut().children.push(id);
                    }
                }
            }
            EverNodeData::Comment { contents } => {
                let data = NodeData::Comment { contents: contents.clone() };
                let id = dom.create_node(data);
                if let Some(p) = parent {
                    if let Some(parent_rc) = p.upgrade() {
                        let node_rc = dom.nodes[id].clone();
                        node_rc.borrow_mut().parent = Some(Rc::downgrade(&parent_rc));
                        parent_rc.borrow_mut().children.push(id);
                    }
                }
            }
            EverNodeData::Element { name, attrs, template_contents, .. } => {
                // Convert attributes to AttributeMap
                let mut attributes: AttributeMap = AttributeMap::new();
                for attr in attrs.borrow().iter() {
                    let local = attr.name.local.to_string();
                    let val = attr.value.to_string();
                    attributes.insert(local, val);
                }

                let elem_data = ElementData::with_attributes(name.clone(), attributes.clone());

                // Special handling for <img> to create Image node variant, otherwise Element
                let node_kind = if name.local.as_ref().eq_ignore_ascii_case("img") {
                    // create ImageData from attributes
                    let src = attributes.get("src").cloned().unwrap_or_default();
                    let alt = attributes.get("alt").cloned().unwrap_or_default();
                    NodeData::Image(RefCell::new(ImageData::new(src, alt)))
                } else {
                    NodeData::Element(elem_data)
                };

                let id = dom.create_node(node_kind);

                // Attach to parent
                if let Some(p) = &parent {
                    if let Some(parent_rc) = p.upgrade() {
                        let node_rc = dom.nodes[id].clone();
                        node_rc.borrow_mut().parent = Some(Rc::downgrade(&parent_rc));
                        parent_rc.borrow_mut().children.push(id);
                    }
                }

                // If element has template contents (e.g., <template>), recurse into them and attach to the created element's template_contents
                if let Some(template_handle) = template_contents.borrow().as_ref() {
                    // Recurse but with parent being the new element
                    let new_parent = Some(Rc::downgrade(&dom.nodes[id]));
                    // Build a DocumentFragment to hold template children
                    let frag_id = dom.create_node(NodeData::DocumentFragment);
                    // attach fragment as child of the element
                    {
                        let elem_rc = dom.nodes[id].clone();
                        let frag_rc = dom.nodes[frag_id].clone();
                        frag_rc.borrow_mut().parent = Some(Rc::downgrade(&elem_rc));
                        elem_rc.borrow_mut().children.push(frag_id);
                        // set the element's template_contents to the fragment handle
                        if let NodeData::Element(ed) = &elem_rc.borrow().data {
                            *ed.template_contents.borrow_mut() = Some(frag_rc.clone());
                        }
                    }
                    // Recurse children of template into the fragment
                    let children = template_handle.children.borrow();
                    for child in children.iter() {
                        self.build_dom_from_handle(child, Some(Rc::downgrade(&dom.nodes[frag_id])), dom);
                    }
                }

                // Recurse into children (normal child nodes)
                let new_parent = Some(Rc::downgrade(&dom.nodes[id]));
                let children = handle.children.borrow();
                for child in children.iter() {
                    self.build_dom_from_handle(child, new_parent.clone(), dom);
                }
            }
            EverNodeData::ProcessingInstruction { target, contents } => {
                let data = NodeData::ProcessingInstruction { target: target.to_string(), data: contents.to_string() };
                let id = dom.create_node(data);
                if let Some(p) = parent {
                    if let Some(parent_rc) = p.upgrade() {
                        let node_rc = dom.nodes[id].clone();
                        node_rc.borrow_mut().parent = Some(Rc::downgrade(&parent_rc));
                        parent_rc.borrow_mut().children.push(id);
                    }
                }
            }
            _ => {
                // Unknown or unhandled node types - skip
            }
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
