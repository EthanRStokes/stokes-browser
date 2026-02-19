use std::ascii::AsciiExt;
use super::{AttributeMap, Dom, ElementData, ImageData, NodeData};
use crate::dom::config::DomConfig;
use crate::dom::node::{SpecialElementData, TextData};
// HTML parser using html5ever
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever::local_name;
use markup5ever_rcdom as rcdom;
use markup5ever_rcdom::{Handle, NodeData as EverNodeData};

/// HTML Parser for converting HTML strings into DOM structures
pub struct HtmlParser;

impl HtmlParser {
    pub fn new() -> Self {
        Self {}
    }

    /// Parse HTML string into a DOM structure
    pub fn parse(&self, html: &str, config: DomConfig) -> Dom {
        // Parse with html5ever
        let parser = parse_document(rcdom::RcDom::default(), Default::default());
        let rcdom = parser.one(html);

        // Convert RcDom to our DOM structure
        let mut dom = Dom::new(config);
        self.build_dom_from_handle(&rcdom.document, None, &mut dom);

        dom
    }

    /// Convert html5ever's DOM structure to our DOM structure
    fn build_dom_from_handle(
        &self, 
        handle: &Handle, 
        parent_id: Option<usize>,
        dom: &mut Dom,
    ) {
        // Determine node type from rcdom
        match &handle.data {
            EverNodeData::Document => {
                // Our Dom already has a root Document node at id 0
                let root_id = 0usize;
                // Recurse into children of the document, setting parent to root
                let children = handle.children.borrow();
                for child in children.iter() {
                    self.build_dom_from_handle(child, Some(root_id), dom);
                }
            }
            EverNodeData::Doctype { name, public_id, system_id } => {
                // IGNORE DOCTYPE for now
            }
            EverNodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                let processed = self.process_html_whitespace(&text);
                if processed.is_empty() {
                    return;
                }
                let data = NodeData::Text(TextData::new(processed));
                let id = dom.create_node(data);
                if let Some(pid) = parent_id {
                    dom.nodes[id].parent = Some(pid);
                    dom.nodes[pid].children.push(id);
                }
            }
            EverNodeData::Comment { contents } => {
                let data = NodeData::Comment { contents: contents.clone() };
                let id = dom.create_node(data);
                if let Some(pid) = parent_id {
                    dom.nodes[id].parent = Some(pid);
                    dom.nodes[pid].children.push(id);
                }
            }
            EverNodeData::Element { name, attrs, template_contents, .. } => {
                // Convert attributes to AttributeMap
                let mut attributes: AttributeMap = AttributeMap::empty();
                for attr in attrs.borrow().iter() {
                    let local = attr.name.clone();
                    if local.local.as_ref() == "id" {
                        let val = &attr.value;
                        attributes.set(local, &*val);
                    }
                }

                let elem_data = ElementData::new(name.clone(), attributes);

                let node_kind = NodeData::Element(elem_data);

                let id = dom.create_node(node_kind);

                for attr in attrs.borrow().iter() {
                    let local = attr.name.clone();
                    let val = &attr.value;
                    dom.set_attribute(id, local, val);
                }
                let elem_data = dom.nodes[id].element_data_mut().unwrap();
                elem_data.flush_style_attribute(&dom.lock, &dom.url.url_extra_data());

                {
                    let tag = name.local.as_ref();
                    if tag.eq_ignore_ascii_case("img") {
                        dom.load_image(id);
                    } else if tag.eq_ignore_ascii_case("canvas") {
                        dom.load_custom_paint_src(id);
                    } else if tag.eq_ignore_ascii_case("link") {
                        dom.load_linked_stylesheet(id);
                    }
                }

                // Register element id attribute in nodes_to_id map for getElementById lookups
                if let Some(id_attr) = attrs.borrow().iter().find(|a| a.name.local.as_ref() == "id") {
                    let id_value = id_attr.value.to_string();
                    if !id_value.is_empty() {
                        dom.nodes_to_id.insert(id_value, id);
                    }
                }

                // Attach to parent
                if let Some(pid) = parent_id {
                    dom.nodes[id].parent = Some(pid);
                    dom.nodes[pid].children.push(id);
                }

                // Recurse into children (normal child nodes)
                let children = handle.children.borrow();
                for child in children.iter() {
                    self.build_dom_from_handle(child, Some(id), dom);
                }
            }
            EverNodeData::ProcessingInstruction { target, contents } => {
                // Ignore ProcessingInstruction for now
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
