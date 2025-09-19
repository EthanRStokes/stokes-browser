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
    DocumentType {
        name: String,
        public_id: String,
        system_id: String,
    },
    DocumentFragment,
    Element(ElementData),
    Text(String),
    Comment(String),
    ProcessingInstruction {
        target: String,
        data: String,
    },
    Image(ImageData),
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

/// Data specific to image nodes
#[derive(Debug, Clone, PartialEq)]
pub struct ImageData {
    pub src: String,
    pub alt: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub loading_state: ImageLoadingState,
}

/// Image loading state
#[derive(Debug, Clone, PartialEq)]
pub enum ImageLoadingState {
    NotLoaded,
    Loading,
    Loaded(Vec<u8>), // Raw image data
    Failed(String),  // Error message
}

impl ImageData {
    pub fn new(src: String, alt: String) -> Self {
        Self {
            src,
            alt,
            width: None,
            height: None,
            loading_state: ImageLoadingState::NotLoaded,
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

    /// Enhanced CSS selector matching (still simplified but more comprehensive)
    pub fn query_selector(&self, selector: &str) -> Vec<Rc<RefCell<DomNode>>> {
        self.find_nodes(|node| self.matches_selector(node, selector))
    }

    /// Check if a node matches a CSS selector
    fn matches_selector(&self, node: &DomNode, selector: &str) -> bool {
        if let NodeType::Element(data) = &node.node_type {
            // Handle different selector types
            if selector.starts_with('#') {
                // ID selector
                let id = &selector[1..];
                return data.id() == Some(id);
            } else if selector.starts_with('.') {
                // Class selector
                let class_name = &selector[1..];
                return data.classes().contains(&class_name);
            } else if selector.contains('[') && selector.contains(']') {
                // Attribute selector [attr=value]
                if let Some(start) = selector.find('[') {
                    if let Some(end) = selector.find(']') {
                        let attr_part = &selector[start+1..end];
                        if let Some(eq_pos) = attr_part.find('=') {
                            let attr_name = &attr_part[..eq_pos];
                            let attr_value = &attr_part[eq_pos+1..].trim_matches('"');
                            return data.attributes.get(attr_name) == Some(&attr_value.to_string());
                        } else {
                            // Just check if attribute exists
                            return data.attributes.contains_key(attr_part);
                        }
                    }
                }
            } else {
                // Tag selector
                return data.tag_name == selector;
            }
        }
        false
    }

    /// Get element by ID (returns first match)
    pub fn get_element_by_id(&self, id: &str) -> Option<Rc<RefCell<DomNode>>> {
        let selector = format!("#{}", id);
        self.query_selector(&selector).into_iter().next()
    }

    /// Get elements by class name
    pub fn get_elements_by_class_name(&self, class_name: &str) -> Vec<Rc<RefCell<DomNode>>> {
        let selector = format!(".{}", class_name);
        self.query_selector(&selector)
    }

    /// Get elements by tag name
    pub fn get_elements_by_tag_name(&self, tag_name: &str) -> Vec<Rc<RefCell<DomNode>>> {
        self.query_selector(tag_name)
    }

    /// Insert a child node at a specific position
    pub fn insert_child(&mut self, index: usize, child: DomNode) -> Result<Rc<RefCell<DomNode>>, &'static str> {
        if index > self.children.len() {
            return Err("Index out of bounds");
        }

        let child_rc = Rc::new(RefCell::new(child));
        self.children.insert(index, Rc::clone(&child_rc));
        Ok(child_rc)
    }

    /// Remove a child node
    pub fn remove_child(&mut self, child: &Rc<RefCell<DomNode>>) -> Result<(), &'static str> {
        let position = self.children.iter().position(|c| Rc::ptr_eq(c, child));
        match position {
            Some(index) => {
                self.children.remove(index);
                Ok(())
            },
            None => Err("Child not found")
        }
    }

    /// Check if this node contains another node as a descendant
    pub fn contains(&self, other: &Rc<RefCell<DomNode>>) -> bool {
        for child in &self.children {
            if Rc::ptr_eq(child, other) {
                return true;
            }
            if child.borrow().contains(other) {
                return true;
            }
        }
        false
    }

    /// Get the next sibling element
    pub fn next_element_sibling(&self) -> Option<Rc<RefCell<DomNode>>> {
        if let Some(parent_weak) = &self.parent {
            if let Some(parent_rc) = parent_weak.upgrade() {
                let parent = parent_rc.borrow();
                let mut found_self = false;

                for child in &parent.children {
                    if found_self {
                        if let NodeType::Element(_) = child.borrow().node_type {
                            return Some(Rc::clone(child));
                        }
                    } else if std::ptr::eq(self, &*child.borrow()) {
                        found_self = true;
                    }
                }
            }
        }
        None
    }

    /// Get the previous sibling element
    pub fn previous_element_sibling(&self) -> Option<Rc<RefCell<DomNode>>> {
        if let Some(parent_weak) = &self.parent {
            if let Some(parent_rc) = parent_weak.upgrade() {
                let parent = parent_rc.borrow();
                let mut previous_element: Option<Rc<RefCell<DomNode>>> = None;

                for child in &parent.children {
                    if std::ptr::eq(self, &*child.borrow()) {
                        return previous_element;
                    }
                    if let NodeType::Element(_) = child.borrow().node_type {
                        previous_element = Some(Rc::clone(child));
                    }
                }
            }
        }
        None
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
            },
            NodeType::DocumentType { name, public_id, system_id } => {
                write!(f, "<!DOCTYPE {} PUBLIC \"{}\" \"{}\">", name, public_id, system_id)
            },
            NodeType::DocumentFragment => {
                write!(f, "<#document-fragment>")
            },
            NodeType::ProcessingInstruction { target, data } => {
                write!(f, "<?{} {}?>", target, data)
            },
            NodeType::Image(data) => {
                write!(f, "<img src=\"{}\" alt=\"{}\"", data.src, data.alt)?;
                if let Some(width) = data.width {
                    write!(f, " width=\"{}\"", width)?;
                }
                if let Some(height) = data.height {
                    write!(f, " height=\"{}\"", height)?;
                }
                write!(f, "/>")
            },
        }
    }
}
