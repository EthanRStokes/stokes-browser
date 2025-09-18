// DOM node implementation for representing HTML elements
use std::collections::HashMap;
use std::fmt;
use std::rc::{Rc, Weak};
use std::cell::RefCell;

/// A map of attribute names to values
pub type AttributeMap = HashMap<String, String>;

/// Represents the type of a DOM node
#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    Document,
    Element(ElementData),
    Text(String),
    Comment(String),
}

/// Data specific to element nodes
#[derive(Debug, Clone, PartialEq)]
pub struct ElementData {
    /// Tag name of the element (e.g., "div", "span", "a")
    pub tag_name: String,
    /// Element attributes (e.g., id, class, href)
    pub attributes: AttributeMap,
}

impl ElementData {
    pub fn new(tag_name: &str) -> Self {
        Self {
            tag_name: tag_name.to_lowercase(),
            attributes: HashMap::new(),
        }
    }

    pub fn with_attributes(tag_name: &str, attributes: AttributeMap) -> Self {
        Self {
            tag_name: tag_name.to_lowercase(),
            attributes,
        }
    }

    /// Get the ID attribute
    pub fn id(&self) -> Option<&str> {
        self.attributes.get("id").map(|s| s.as_str())
    }

    /// Get the class attribute as a list of class names
    pub fn classes(&self) -> Vec<&str> {
        match self.attributes.get("class") {
            Some(classlist) => classlist.split_whitespace().collect(),
            None => Vec::new(),
        }
    }
}

/// A node in the DOM tree
#[derive(Clone)]
pub struct DomNode {
    /// The type of node
    pub node_type: NodeType,
    /// Parent node
    pub parent: Option<Weak<RefCell<DomNode>>>,
    /// Child nodes
    pub children: Vec<Rc<RefCell<DomNode>>>,
}

impl DomNode {
    /// Create a new DOM node
    pub fn new(node_type: NodeType, parent: Option<Weak<RefCell<DomNode>>>) -> Self {
        Self {
            node_type,
            parent,
            children: Vec::new(),
        }
    }

    /// Add a child node
    pub fn add_child(&mut self, child: DomNode) -> Rc<RefCell<DomNode>> {
        let child_rc = Rc::new(RefCell::new(child));
        self.children.push(Rc::clone(&child_rc));
        child_rc
    }

    /// Get text content of this node and its descendants
    pub fn text_content(&self) -> String {
        match &self.node_type {
            NodeType::Text(content) => content.clone(),
            _ => {
                // Concatenate text from all children
                let mut result = String::new();
                for child in &self.children {
                    result.push_str(&child.borrow().text_content());
                }
                result
            }
        }
    }

    /// Find nodes that match a CSS selector (simplified)
    pub fn query_selector(&self, selector: &str) -> Vec<Rc<RefCell<DomNode>>> {
        // Very simplified selector matching for now - just match by tag name
        self.find_nodes(|node| {
            if let NodeType::Element(data) = &node.node_type {
                data.tag_name == selector
            } else {
                false
            }
        })
    }

    /// Find nodes that match a predicate, returning owned references
    pub fn find_nodes<F>(&self, predicate: F) -> Vec<Rc<RefCell<DomNode>>>
    where
        F: Fn(&DomNode) -> bool + Clone,
    {
        let mut result = Vec::new();
        
        // We can't include self in the result since we don't have an Rc to self
        // This method is meant to be called on nodes that are already in Rc<RefCell<>>

        // Recursively check children
        for child in &self.children {
            let child_borrowed = child.borrow();
            if predicate(&*child_borrowed) {
                result.push(Rc::clone(child));
            }
            drop(child_borrowed); // Explicitly drop the borrow

            // Recursively search in child's children
            let child_borrowed = child.borrow();
            let mut child_matches = child_borrowed.find_nodes(predicate.clone());
            result.append(&mut child_matches);
        }
        
        result
    }
}

impl fmt::Debug for DomNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.node_type {
            NodeType::Document => write!(f, "Document"),
            NodeType::Element(data) => {
                write!(f, "<{}", data.tag_name)?;
                
                // Write attributes
                for (name, value) in &data.attributes {
                    write!(f, " {}=\"{}\"", name, value)?;
                }
                
                if self.children.is_empty() {
                    write!(f, "/>")
                } else {
                    write!(f, ">")?;
                    
                    // Write children
                    for child in &self.children {
                        write!(f, "{:?}", child.borrow())?;
                    }
                    
                    write!(f, "</{}>", data.tag_name)
                }
            },
            NodeType::Text(content) => {
                let content = if content.len() > 50 {
                    format!("{}...", &content[..50])
                } else {
                    content.clone()
                };
                write!(f, "{}", content)
            },
            NodeType::Comment(content) => {
                write!(f, "<!-- {} -->", content)
            }
        }
    }
}
