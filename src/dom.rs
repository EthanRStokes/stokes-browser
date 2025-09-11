use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use html5ever::parse_document;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use html5ever::tendril::TendrilSink;

// We'll use html5ever's NodeData directly
pub type HtmlNode = markup5ever_rcdom::Node;

// Helper enum for easier handling of node types
#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    Element,
    Text,
    Comment,
    Document,
    Other
}

// DOM structure that wraps the html5ever DOM
pub struct Dom {
    pub document: Handle,
    dom: RcDom,
}

impl Dom {
    pub fn new() -> Self {
        // Initialize with an empty document
        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut "".as_bytes())
            .unwrap();

        Self {
            document: dom.document.clone(),
            dom,
        }
    }

    pub fn parse_html(&mut self, html: &str) {
        // Parse HTML string into html5ever DOM
        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())
            .unwrap();

        self.dom = dom;
        self.document = self.dom.document.clone();
    }

    // Get node type from html5ever NodeData
    pub fn get_node_type(node: &Handle) -> NodeType {
        match &node.data {
            NodeData::Document => NodeType::Document,
            NodeData::Element { .. } => NodeType::Element,
            NodeData::Text { .. } => NodeType::Text,
            NodeData::Comment { .. } => NodeType::Comment,
            _ => NodeType::Other,
        }
    }

    // Get tag name for element nodes
    pub fn get_tag_name(node: &Handle) -> Option<String> {
        match &node.data {
            NodeData::Element { name, .. } => Some(name.local.to_string()),
            _ => None,
        }
    }

    // Get node attributes
    pub fn get_attributes(node: &Handle) -> HashMap<String, String> {
        let mut attributes = HashMap::new();

        if let NodeData::Element { attrs, .. } = &node.data {
            for attr in attrs.borrow().iter() {
                let name = attr.name.local.to_string();
                let value = attr.value.to_string();
                attributes.insert(name, value);
            }
        }

        attributes
    }

    // Get text content of a node
    pub fn get_text_content(node: &Handle) -> String {
        match &node.data {
            NodeData::Text { contents } => contents.borrow().to_string(),
            _ => {
                let mut text = String::new();
                for child in node.children.borrow().iter() {
                    text.push_str(&Self::get_text_content(child));
                    if !text.is_empty() {
                        text.push(' '); // Add space between text segments
                    }
                }
                text.trim().to_string()
            }
        }
    }

    // Get the page title from the DOM
    pub fn get_title(&self) -> String {
        self.find_title_element(&self.document)
    }

    // Find the title element in the DOM
    fn find_title_element(&self, node: &Handle) -> String {
        if let NodeData::Element { name, .. } = &node.data {
            if name.local.to_string() == "title" {
                return Self::get_text_content(node);
            }
        }

        // Recursively search in children
        for child in node.children.borrow().iter() {
            let title = self.find_title_element(child);
            if !title.is_empty() {
                return title;
            }
        }

        String::new()
    }

    // Find all links in the document
    pub fn get_links(&self) -> Vec<(String, String)> {
        let mut links = Vec::new();
        self.collect_links(&self.document, &mut links);
        links
    }

    // Collect links from the DOM
    fn collect_links(&self, node: &Handle, links: &mut Vec<(String, String)>) {
        if let NodeData::Element { name, .. } = &node.data {
            if name.local.to_string() == "a" {
                let attrs = Self::get_attributes(node);
                if let Some(href) = attrs.get("href") {
                    let text = Self::get_text_content(node);
                    links.push((text, href.clone()));
                }
            }
        }

        // Process children
        for child in node.children.borrow().iter() {
            self.collect_links(child, links);
        }
    }

    // Find all images in the document
    pub fn get_images(&self) -> Vec<String> {
        let mut images = Vec::new();
        self.collect_images(&self.document, &mut images);
        images
    }

    // Collect images from the DOM
    fn collect_images(&self, node: &Handle, images: &mut Vec<String>) {
        if let NodeData::Element { name, .. } = &node.data {
            if name.local.to_string() == "img" {
                let attrs = Self::get_attributes(node);
                if let Some(src) = attrs.get("src") {
                    images.push(src.clone());
                }
            }
        }

        // Process children
        for child in node.children.borrow().iter() {
            self.collect_images(child, images);
        }
    }

    // Helper method to print the DOM structure for debugging
    pub fn print_dom(&self) {
        self.print_node(&self.document, 0);
    }

    // Helper method to print a node and its children
    fn print_node(&self, node: &Handle, depth: usize) {
        let indent = " ".repeat(depth * 2);

        match &node.data {
            NodeData::Document => {
                println!("{}Document", indent);
            }
            NodeData::Element { name, attrs, .. } => {
                print!("{}Element: <{}", indent, name.local);

                // Print attributes
                for attr in attrs.borrow().iter() {
                    print!(" {}=\"{}\"", attr.name.local, attr.value);
                }

                println!(">");
            }
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                let text = text.trim();
                if !text.is_empty() {
                    println!("{}Text: \"{}\"", indent, text);
                }
            }
            NodeData::Comment { contents } => {
                println!("{}Comment: \"{}\"", indent, contents);
            }
            _ => {
                println!("{}Other", indent);
            }
        }

        // Print children
        for child in node.children.borrow().iter() {
            self.print_node(child, depth + 1);
        }
    }
}
