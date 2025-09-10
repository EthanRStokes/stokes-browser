use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use html5ever::parse_document;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use html5ever::tendril::TendrilSink;

#[derive(Debug, Clone)]
pub enum NodeType {
    Document,
    Element(String), // Tag name (e.g., "div", "p", "h1")
    Text(String),    // Text content
    Comment(String), // Comment content
}

#[derive(Debug, Clone)]
pub struct DomNode {
    pub node_type: NodeType,
    pub attributes: HashMap<String, String>,
    pub children: Vec<Rc<RefCell<DomNode>>>,
    pub parent: Option<Rc<RefCell<DomNode>>>,
    // Style properties - these would expand in a real browser
    pub styles: HashMap<String, String>,
}

impl DomNode {
    pub fn new(node_type: NodeType) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            node_type,
            attributes: HashMap::new(),
            children: Vec::new(),
            parent: None,
            styles: HashMap::new(),
        }))
    }

    pub fn append_child(&mut self, child: Rc<RefCell<DomNode>>) {
        child.borrow_mut().parent = Some(Rc::new(RefCell::new(self.clone())));
        self.children.push(child);
    }

    pub fn get_attribute(&self, name: &str) -> Option<&String> {
        self.attributes.get(name)
    }

    // Get text content of this node and all descendants
    pub fn get_text_content(&self) -> String {
        match &self.node_type {
            NodeType::Text(content) => content.clone(),
            _ => {
                let mut result = String::new();
                for child in &self.children {
                    result.push_str(&child.borrow().get_text_content());
                    result.push(' '); // Add space between child text nodes
                }
                result.trim().to_string()
            }
        }
    }
}

pub struct Dom {
    pub document: Rc<RefCell<DomNode>>,
}

impl Dom {
    pub fn new() -> Self {
        Self {
            document: DomNode::new(NodeType::Document),
        }
    }

    pub fn parse_html(&mut self, html: &str) {
        let dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .read_from(&mut html.as_bytes())
            .unwrap();

        // Clear existing document
        self.document = DomNode::new(NodeType::Document);

        // Convert from html5ever DOM to our DOM
        self.convert_node(dom.document, self.document.clone());
    }

    fn convert_node(&self, handle: Handle, parent: Rc<RefCell<DomNode>>) {
        let node = handle;

        match &node.data {
            NodeData::Document => {
                // Process document's children
                for child in node.children.borrow().iter() {
                    self.convert_node(child.clone(), parent.clone());
                }
            }

            NodeData::Element { name, attrs, .. } => {
                let tag_name = name.local.to_string();
                let element = DomNode::new(NodeType::Element(tag_name));

                // Add attributes
                let mut element_ref = element.borrow_mut();
                for attr in attrs.borrow().iter() {
                    let name = attr.name.local.to_string();
                    let value = attr.value.to_string();
                    element_ref.attributes.insert(name, value);
                }

                // Link to parent
                parent.borrow_mut().append_child(element.clone());

                // Process element's children
                drop(element_ref); // Release borrow
                for child in node.children.borrow().iter() {
                    self.convert_node(child.clone(), element.clone());
                }
            }

            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                if !text.trim().is_empty() {
                    let text_node = DomNode::new(NodeType::Text(text));
                    parent.borrow_mut().append_child(text_node);
                }
            }

            NodeData::Comment { contents } => {
                let comment = contents.to_string();
                let comment_node = DomNode::new(NodeType::Comment(comment));
                parent.borrow_mut().append_child(comment_node);
            }

            // Ignore other node types for simplicity
            _ => {}
        }
    }

    // Helper to traverse DOM and print for debugging
    pub fn print_dom(&self) {
        self.print_node(self.document.clone(), 0);
    }

    fn print_node(&self, node: Rc<RefCell<DomNode>>, depth: usize) {
        let node_ref = node.borrow();
        let indent = "  ".repeat(depth);

        match &node_ref.node_type {
            NodeType::Document => println!("{}Document", indent),
            NodeType::Element(tag) => {
                print!("{}<{}", indent, tag);

                // Print attributes
                for (name, value) in &node_ref.attributes {
                    print!(" {}=\"{}\"", name, value);
                }
                println!(">");
            }
            NodeType::Text(content) => {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    println!("{}\"{}\"", indent, trimmed);
                }
            }
            NodeType::Comment(content) => println!("{}<!-- {} -->", indent, content),
        }

        // Print children
        for child in &node_ref.children {
            self.print_node(child.clone(), depth + 1);
        }

        // Print closing tag for elements
        if let NodeType::Element(tag) = &node_ref.node_type {
            println!("{}</{}>", indent, tag);
        }
    }
}
