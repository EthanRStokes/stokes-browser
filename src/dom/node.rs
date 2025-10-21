use crate::dom::events::EventListenerRegistry;
use std::cell::RefCell;
// DOM node implementation for representing HTML elements
use std::collections::HashMap;
use std::fmt;
use std::rc::{Rc, Weak};

/// Callback type for layout invalidation
pub type LayoutInvalidationCallback = Box<dyn Fn()>;

/// A map of attribute names to values
pub type AttributeMap = HashMap<String, String>;

/// Represents the type of DOM node
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
    Image(RefCell<ImageData>),
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
#[derive(Debug, Clone)]
pub struct ImageData {
    pub src: Rc<String>,
    pub alt: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub loading_state: ImageLoadingState,
    pub cached_image: Option<skia_safe::Image>, // Cached decoded image
}

/// Image loading state
#[derive(Debug, Clone, PartialEq)]
pub enum ImageLoadingState {
    NotLoaded,
    Loading,
    Loaded(Vec<u8>), // Raw image data
    Failed(String),  // Error message
}

impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        // Compare all fields except cached_image (since skia_safe::Image doesn't implement PartialEq)
        self.src == other.src
            && self.alt == other.alt
            && self.width == other.width
            && self.height == other.height
            && self.loading_state == other.loading_state
        // Note: We don't compare cached_image as it's a derived/cached value
    }
}

impl ImageData {
    pub fn new(src: String, alt: String) -> Self {
        Self {
            src: Rc::from(src),
            alt,
            width: None,
            height: None,
            loading_state: ImageLoadingState::NotLoaded,
            cached_image: None,
        }
    }

    /// Get or decode the Skia image, caching the result
    pub fn get_or_decode_image(&mut self) -> Option<&skia_safe::Image> {
        // If we already have a cached image, return it
        if self.cached_image.is_some() {
            return self.cached_image.as_ref();
        }

        // If we have loaded image data but no cached image, decode it
        if let ImageLoadingState::Loaded(image_bytes) = &self.loading_state {
            if let Some(decoded_image) = Self::decode_image_data(image_bytes) {
                self.cached_image = Some(decoded_image);
                return self.cached_image.as_ref();
            }
        }

        None
    }

    /// Decode image data into a Skia image (static method for reuse)
    fn decode_image_data(image_bytes: &[u8]) -> Option<skia_safe::Image> {
        if image_bytes.is_empty() {
            println!("Error: Empty image data");
            return None;
        }

        // Check the first few bytes to identify the image format for debugging
        let format_name = if image_bytes.len() >= 12 {
            let header = &image_bytes[0..12];
            match header {
                [0xFF, 0xD8, 0xFF, ..] => "JPEG",
                [0x89, 0x50, 0x4E, 0x47, ..] => "PNG",
                [0x47, 0x49, 0x46, 0x38, ..] => "GIF",
                [0x42, 0x4D, ..] => "BMP",
                [0x52, 0x49, 0x46, 0x46, _, _, _, _, 0x57, 0x45, 0x42, 0x50] => "WebP",
                _ => {
                    // Check if it's SVG (starts with <svg or <?xml)
                    if let Ok(text) = std::str::from_utf8(&image_bytes[0..image_bytes.len().min(100)]) {
                        if text.trim_start().starts_with("<svg") || text.trim_start().starts_with("<?xml") {
                            "SVG"
                        } else {
                            "Unknown"
                        }
                    } else {
                        "Unknown"
                    }
                }
            }
        } else {
            "Unknown"
        };

        println!("Decoding {} image format", format_name);

        // Handle SVG separately
        if format_name == "SVG" {
            return Self::decode_svg_data(image_bytes);
        }

        // Try Skia first
        let skia_data = skia_safe::Data::new_copy(image_bytes);
        if !skia_data.is_empty() {
            match skia_safe::Image::from_encoded(skia_data) {
                Some(image) => {
                    println!("Successfully decoded image with Skia: {}x{}", image.width(), image.height());
                    return Some(image);
                }
                None => {
                    println!("Skia failed to decode image, trying fallback...");
                }
            }
        } else {
            println!("Error: Failed to create Skia Data object");
        }

        // Fallback to image crate for formats Skia doesn't support (especially WebP)
        match image::load_from_memory(image_bytes) {
            Ok(dynamic_image) => {
                println!("Successfully decoded image with image crate: {}x{}",
                        dynamic_image.width(), dynamic_image.height());

                // Convert to RGBA8 format
                let rgba_image = dynamic_image.to_rgba8();
                let (width, height) = rgba_image.dimensions();
                let rgba_data = rgba_image.into_raw();

                // Create Skia image from RGBA data
                let image_info = skia_safe::ImageInfo::new(
                    skia_safe::ISize::new(width as i32, height as i32),
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Unpremul,
                    None,
                );

                match skia_safe::images::raster_from_data(
                    &image_info,
                    skia_safe::Data::new_copy(&rgba_data),
                    (width * 4) as usize,
                ) {
                    Some(skia_image) => {
                        println!("Successfully converted image crate result to Skia image");
                        Some(skia_image)
                    }
                    None => {
                        println!("Error: Failed to convert image crate result to Skia image");
                        None
                    }
                }
            }
            Err(e) => {
                println!("Error: Both Skia and image crate failed to decode image: {}", e);
                None
            }
        }
    }

    /// Decode SVG data into a Skia image
    fn decode_svg_data(svg_bytes: &[u8]) -> Option<skia_safe::Image> {
        // Convert bytes to string
        let svg_str = match std::str::from_utf8(svg_bytes) {
            Ok(s) => s,
            Err(e) => {
                println!("Error: SVG data is not valid UTF-8: {}", e);
                return None;
            }
        };

        // Parse the SVG using usvg
        let options = usvg::Options::default();
        let tree = match usvg::Tree::from_str(svg_str, &options) {
            Ok(tree) => tree,
            Err(e) => {
                println!("Error: Failed to parse SVG: {}", e);
                return None;
            }
        };

        // Get the SVG size
        let size = tree.size();
        let width = size.width() as i32;
        let height = size.height() as i32;

        if width == 0 || height == 0 {
            println!("Error: SVG has zero dimensions");
            return None;
        }

        println!("Rendering SVG: {}x{}", width, height);

        // Create a pixmap to render into
        let mut pixmap = match tiny_skia::Pixmap::new(width as u32, height as u32) {
            Some(pixmap) => pixmap,
            None => {
                println!("Error: Failed to create pixmap for SVG rendering");
                return None;
            }
        };

        // Render the SVG
        resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());

        // Convert the pixmap to a Skia image
        let rgba_data = pixmap.data();
        let image_info = skia_safe::ImageInfo::new(
            skia_safe::ISize::new(width, height),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );

        match skia_safe::images::raster_from_data(
            &image_info,
            skia_safe::Data::new_copy(rgba_data),
            (width * 4) as usize,
        ) {
            Some(image) => {
                println!("Successfully decoded SVG image");
                Some(image)
            }
            None => {
                println!("Error: Failed to convert SVG pixmap to Skia image");
                None
            }
        }
    }

    /// Public static method for decoding image data (used by engine)
    pub fn decode_image_data_static(image_bytes: &[u8]) -> Option<skia_safe::Image> {
        Self::decode_image_data(image_bytes)
    }

    /// Clear the cached image (useful when image data changes)
    pub fn clear_cache(&mut self) {
        self.cached_image = None;
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
    /// Event listener registry
    pub event_listeners: EventListenerRegistry,
    /// Optional callback to invalidate layout when tree structure changes
    pub layout_invalidation_callback: Option<Rc<LayoutInvalidationCallback>>,
}

impl DomNode {
    /// Create a new DOM node
    pub fn new(node_type: NodeType, parent: Option<Weak<RefCell<DomNode>>>) -> Self {
        Self {
            node_type,
            parent,
            children: Vec::new(),
            event_listeners: EventListenerRegistry::new(),
            layout_invalidation_callback: None,
        }
    }

    /// Set the layout invalidation callback for this node and all its descendants
    pub fn set_layout_invalidation_callback(&mut self, callback: Rc<LayoutInvalidationCallback>) {
        self.layout_invalidation_callback = Some(callback.clone());

        // Recursively set for all children
        for child in &self.children {
            if let Ok(mut child_node) = child.try_borrow_mut() {
                child_node.set_layout_invalidation_callback(callback.clone());
            }
        }
    }

    /// Invalidate layout by calling the callback if set
    fn invalidate_layout(&self) {
        if let Some(callback) = &self.layout_invalidation_callback {
            callback();
        }
    }

    /// Add a child node
    pub fn add_child(&mut self, child: DomNode) -> Rc<RefCell<DomNode>> {
        let child_rc = Rc::new(RefCell::new(child));
        self.children.push(Rc::clone(&child_rc));

        // Set the layout invalidation callback on the new child
        if let Some(callback) = &self.layout_invalidation_callback {
            if let Ok(mut child_node) = child_rc.try_borrow_mut() {
                child_node.set_layout_invalidation_callback(callback.clone());
            }
        }

        // Invalidate layout since tree structure changed
        self.invalidate_layout();

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

    /// Set the text content of this node
    /// For text nodes, replaces the text content
    /// For element nodes, removes all children and creates a single text node child
    pub fn set_text_content(&mut self, text: &str) {
        match &mut self.node_type {
            NodeType::Text(content) => {
                *content = text.to_string();
                // Invalidate layout since content changed
                self.invalidate_layout();
            }
            _ => {
                // Remove all children
                self.children.clear();
                
                // If text is not empty, add a single text node as child
                if !text.is_empty() {
                    let text_node = DomNode::new(NodeType::Text(text.to_string()), None);
                    self.add_child(text_node);
                }
                // Note: invalidate_layout is called in add_child, or here if text is empty
                if text.is_empty() {
                    self.invalidate_layout();
                }
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

        // Set the layout invalidation callback on the new child
        if let Some(callback) = &self.layout_invalidation_callback {
            if let Ok(mut child_node) = child_rc.try_borrow_mut() {
                child_node.set_layout_invalidation_callback(callback.clone());
            }
        }

        // Invalidate layout since tree structure changed
        self.invalidate_layout();

        Ok(child_rc)
    }

    /// Remove a child node
    pub fn remove_child(&mut self, child: &Rc<RefCell<DomNode>>) -> Result<(), &'static str> {
        let position = self.children.iter().position(|c| Rc::ptr_eq(c, child));
        match position {
            Some(index) => {
                self.children.remove(index);
                // Invalidate layout since tree structure changed
                self.invalidate_layout();
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
                let data = data.borrow();
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
