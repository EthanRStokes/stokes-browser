use crate::dom::events::EventListenerRegistry;
use std::cell::RefCell;
// DOM node implementation for representing HTML elements
use std::collections::HashMap;
use std::{fmt, ptr};
use std::io::Cursor;
use std::ops::{Deref, DerefMut};
use std::rc::{Rc, Weak};
use std::sync::atomic::AtomicBool;
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use bitflags::bitflags;
use html5ever::{LocalName, QualName};
use html5ever::tendril::StrTendril;
use markup5ever::local_name;
use peniko::Blob;
use selectors::matching::{ElementSelectorFlags, QuirksMode};
use skia_safe::FontMgr;
use skia_safe::wrapper::PointerWrapper;
use slab::Slab;
use style::data::ElementData as StyleElementData;
use style::properties::{parse_style_attribute, PropertyDeclarationBlock};
use style::servo_arc::{Arc as ServoArc, Arc};
use style::shared_lock::{Locked, SharedRwLock};
use stylo_atoms::Atom;
use stylo_dom::ElementState;
use style::data::ElementData as StyloElementData;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::properties::style_structs::Font;
use style::selector_parser::RestyleDamage;
use style::stylesheets::{CssRuleType, UrlExtraData};
use taffy::Style;

/// Callback type for layout invalidation
pub type LayoutInvalidationCallback = Box<dyn Fn()>;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct Attribute {
    pub name: QualName,
    pub value: String,
}

/// A map of attribute names to values
#[derive(Clone, Debug)]
pub struct AttributeMap {
    attrs: Vec<Attribute>
}

impl AttributeMap {
    pub fn new(attrs: Vec<Attribute>) -> Self {
        Self { attrs }
    }

    pub fn empty() -> Self {
        Self { attrs: Vec::new() }
    }

    pub fn set(&mut self, name: QualName, value: &str) {
        // existing attribute
        let attr = self.attrs.iter_mut().find(|attr| attr.name == name);
        if let Some(attr) = attr {
            attr.value.clear();
            attr.value.push_str(value);
        } else {
            self.attrs.push(Attribute { name, value: value.to_string() });
        }
    }

    pub fn remove(&mut self, name: &QualName) -> Option<Attribute> {
        let index = self.attrs.iter().position(|attr| attr.name == *name);
        index.map(|index| self.attrs.remove(index))
    }
}

impl Deref for AttributeMap {
    type Target = Vec<Attribute>;
    fn deref(&self) -> &Self::Target {
        &self.attrs
    }
}
impl DerefMut for AttributeMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.attrs
    }
}

/// Represents the type of DOM node
#[derive(Debug, Clone)]
pub enum NodeData {
    /// The `Document` itself - the root node.
    Document,
    DocType {
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    },
    DocumentFragment,
    Text { contents: RefCell<StrTendril> },
    Comment { contents: StrTendril },
    Element(ElementData),
    // TODO better pseudo element support
    AnonymousBlock(ElementData),
    ProcessingInstruction {
        target: String,
        data: String,
    },
    Image(RefCell<ImageData>),
}

impl NodeData {
    pub fn element(&self) -> Option<&ElementData> {
        match self {
            NodeData::Element(data) | NodeData::AnonymousBlock(data) => Some(data),
            _ => None,
        }
    }

    pub fn element_mut(&mut self) -> Option<&mut ElementData> {
        match self {
            NodeData::Element(data) | NodeData::AnonymousBlock(data) => Some(data),
            _ => None,
        }
    }

    pub fn attrs(&self) -> Option<&AttributeMap> {
        Some(&self.element()?.attributes)
    }

    pub fn attr(&self, name: impl PartialEq<LocalName>) -> Option<&str> {
        self.element()?.attr(name)
    }

    pub fn has_attr(&self, name: &QualName) -> bool {
        self.element().is_some_and(|element| element.has_attr(name.local.clone()))
    }

    pub fn is_element_with_tag_name(&self, tag_name: &impl PartialEq<LocalName>) -> bool {
        let Some(element) = self.element() else {
            return false;
        };
        *tag_name == element.name.local
    }
}

/// Data specific to element nodes
#[derive(Debug, Clone)]
pub struct ElementData {
    /// Tag name of the element (e.g., "div", "span", "a")
    pub name: QualName,

    /// Element's id as an atom
    pub id: Option<Atom>,

    /// Element attributes (e.g., id, class, href)
    pub attributes: AttributeMap,

    pub style_attribute: Option<ServoArc<Locked<PropertyDeclarationBlock>>>,

    /// For HTML <template> elements, holds the template contents
    pub template_contents: Option<usize>,
}

impl ElementData {
    pub fn new(name: QualName, attrs: AttributeMap) -> Self {
        let id_attr_atom = attrs
            .iter()
            .find(|attr| &attr.name.local == "id")
            .map(|attr| attr.value.as_ref())
            .map(|value: &str| Atom::from(value));

        Self {
            name,
            id: id_attr_atom,
            attributes: attrs,
            style_attribute: Default::default(),
            template_contents: None,
        }
    }

    pub fn flush_style_attribute(&mut self, guard: &SharedRwLock, url_extra_data: &UrlExtraData) {
        self.style_attribute = self.attr(local_name!("style")).map(|style| {
            ServoArc::new(guard.wrap(parse_style_attribute(
                style,
                url_extra_data,
                None,
                QuirksMode::NoQuirks,
                CssRuleType::Style
            )))
        })
    }

    /// Get the ID attribute
    pub fn id(&self) -> Option<&str> {
        self.attributes.iter().find(|attr| &attr.name.local == "id").map(|attr| attr.value.as_str())
    }

    /// Get the class attribute as a list of class names
    pub fn classes(&self) -> Vec<&str> {
        match self.attributes.iter().find(|attr| &attr.name.local == "class") {
            Some(classlist) => classlist.value.split_whitespace().collect(),
            None => Vec::new(),
        }
    }

    pub fn attr(&self, attr_name: impl PartialEq<LocalName>) -> Option<&str> {
        let attr = self.attributes.iter().find(|attr| attr_name == attr.name.local)?;
        Some(&attr.value)
    }

    pub fn has_attr(&self, attr_name: impl PartialEq<LocalName>) -> bool {
        self.attributes.iter().any(|attr| attr_name == attr.name.local)
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
    pub cached_image: CachedImage, // Cached decoded image
}

#[derive(Debug, Clone)]
pub enum CachedImage {
    Raster(Arc<RasterImageData>),
    Svg(Box<usvg::Tree>),
    None
}

impl CachedImage {
    fn is_some(&self) -> bool {
        !matches!(self, Self::None)
    }

    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RasterImageData {
    pub width: u32,
    pub height: u32,
    pub data: Blob<u8>,
}
impl RasterImageData {
    pub fn new(width: u32, height: u32, data: std::sync::Arc<Vec<u8>>) -> Self {
        Self { width, height, data: Blob::new(data) }
    }
}

/// Image loading state
#[derive(Debug, Clone, PartialEq)]
pub enum ImageLoadingState {
    NotLoaded,
    Loading,
    Loaded(Arc<RasterImageData>), // Raw image data
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
            cached_image: CachedImage::None,
        }
    }

    /// Get or decode the Skia image, caching the result
    pub fn get_or_decode_image(&mut self) -> &CachedImage {
        // If we already have a cached image, return it
        if self.cached_image.is_some() {
            return &self.cached_image;
        }

        // If we have loaded image data but no cached image, decode it
        if let ImageLoadingState::Loaded(image_bytes) = &self.loading_state {
            self.cached_image = CachedImage::Raster(image_bytes.clone());
            return &self.cached_image;
        }

        &CachedImage::None
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
    // TODO use Skia's SVG support when available
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
        self.cached_image = CachedImage::None;
    }
}

bitflags! {
    pub struct DomNodeFlags: u32 {
        const IS_INLINE_ROOT = 0b00000001;
        const IS_TABLE_ROOT = 0b00000010;
        const IS_IN_DOCUMENT = 0b00000100;
    }
}

impl DomNodeFlags {
    #[inline]
    pub fn is_inline_root(self) -> bool {
        self.contains(DomNodeFlags::IS_INLINE_ROOT)
    }

    #[inline]
    pub fn is_table_root(self) -> bool {
        self.contains(DomNodeFlags::IS_TABLE_ROOT)
    }

    #[inline]
    pub fn is_in_document(self) -> bool {
        self.contains(DomNodeFlags::IS_IN_DOCUMENT)
    }

    #[inline]
    pub fn reset_reconstruction_flags(&mut self) {
        self.remove(DomNodeFlags::IS_INLINE_ROOT | DomNodeFlags::IS_TABLE_ROOT);
    }
}

/// A node in the DOM tree
#[allow(dead_code)] // Allow unused warnings for this struct
pub struct DomNode {
    // the tree this belongs to
    tree: *mut Slab<DomNode>,

    pub id: usize,
    /// Parent node
    pub parent: Option<usize>,
    /// Child nodes
    pub children: Vec<usize>,

    pub flags: DomNodeFlags,

    /// The type of node
    pub data: NodeData,

    pub stylo_data: AtomicRefCell<Option<StyleElementData>>,
    pub selector_flags: AtomicRefCell<ElementSelectorFlags>,
    pub lock: SharedRwLock,
    pub element_state: ElementState,

    // layout data:
    pub taffy_style: Style<Atom>,

    pub has_snapshot: bool,
    pub snapshot_handled: AtomicBool,

    /// Event listener registry
    pub event_listeners: EventListenerRegistry,
    /// Optional callback to invalidate layout when tree structure changes
    pub layout_invalidation_callback: Option<Rc<LayoutInvalidationCallback>>,
}

unsafe impl Send for DomNode {}

unsafe impl Sync for DomNode {}

impl PartialEq for DomNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for DomNode {}

impl DomNode {
    /// Create a new DOM node
    pub fn new(
        tree: *mut Slab<DomNode>,
        id: usize,
        lock: SharedRwLock,
        data: NodeData,
    ) -> Self {
        Self {
            tree,

            id,
            parent: None,
            children: Vec::new(),
            flags: DomNodeFlags::empty(),
            data,
            stylo_data: Default::default(),
            selector_flags: AtomicRefCell::new(ElementSelectorFlags::empty()),
            lock,
            element_state: ElementState::empty(),
            taffy_style: Default::default(),
            has_snapshot: false,
            snapshot_handled: AtomicBool::new(false),
            event_listeners: EventListenerRegistry::new(),
            layout_invalidation_callback: None,
        }
    }

    pub fn tree(&self) -> &Slab<DomNode> {
        unsafe { &*self.tree }
    }

    pub fn tree_mut(&mut self) -> &mut Slab<DomNode> {
        unsafe { &mut *self.tree }
    }

    pub fn element_data(&self) -> Option<&ElementData> {
        match &self.data {
            NodeData::Element(data) | NodeData::AnonymousBlock(data) => Some(data),
            _ => None,
        }
    }

    pub fn index(&self) -> Option<usize> {
        self.tree()[self.parent?]
            .children
            .iter()
            .position(|i| *i == self.id)
    }

    pub fn backward(&self, num: usize) -> Option<&DomNode> {
        let index = self.index().unwrap_or(0);
        if index < num {
            return None;
        }

        self.tree()[self.parent?]
            .children
            .get(index - num)
            .map(|id| self.get_node(*id))
    }

    pub fn forward(&self, num: usize) -> Option<&DomNode> {
        let index = self.index().unwrap_or(0);
        self.tree()[self.parent?]
            .children
            .get(index + num)
            .map(|id| self.get_node(*id))
    }

    pub fn get_node(&self, id: usize) -> &DomNode {
        self.tree().get(id).unwrap()
    }

    pub fn get_node_mut(&mut self, id: usize) -> &mut DomNode {
        self.tree_mut().get_mut(id).unwrap()
    }

    /// Invalidate layout by calling the callback if set
    fn invalidate_layout(&self) {
        if let Some(callback) = &self.layout_invalidation_callback {
            callback();
        }
    }

    /// Add a child node
    pub fn add_child(&mut self, child: usize) -> &DomNode {
        self.children.push(child);

        if let Some(data) = &mut *self.stylo_data.borrow_mut() {
            data.hint |= RestyleHint::restyle_subtree()
        }

        // Invalidate layout since tree structure changed
        self.invalidate_layout();

        let child = self.get_node_mut(child);

        child
    }

    pub fn set_restyle_hint(&self, hint: RestyleHint) {
        if let Some(element) = self.stylo_data.borrow_mut().as_mut() {
            element.hint.insert(hint);
        }
    }

    pub fn damage_mut(&self) -> Option<AtomicRefMut<'_, RestyleDamage>> {
        let element = self.stylo_data.borrow_mut();
        match *element {
            Some(_) => Some(AtomicRefMut::map(
                element,
                |data: &mut Option<StyloElementData>| &mut data.as_mut().unwrap().damage,
            )),
            None => None,
        }
    }

    pub fn damage(&mut self) -> Option<RestyleDamage> {
        self.stylo_data.get_mut().as_ref().map(|data| data.damage)
    }

    pub fn set_damage(&self, damage: RestyleDamage) {
        if let Some(element) = self.stylo_data.borrow_mut().as_mut() {
            element.damage = damage;
        }
    }

    pub fn insert_damage(&mut self, damage: RestyleDamage) {
        if let Some(element) = self.stylo_data.borrow_mut().as_mut() {
            element.damage |= damage;
        }
    }

    pub fn remove_damage(&self, damage: RestyleDamage) {
        if let Some(element) = self.stylo_data.borrow_mut().as_mut() {
            element.damage -= damage;
        }
    }

    pub fn clear_damage_mut(&mut self) {
        if let Some(element) = self.stylo_data.borrow_mut().as_mut() {
            element.damage = RestyleDamage::empty();
        }
    }

    pub fn hover(&mut self) {
        self.element_state.insert(ElementState::HOVER);
        self.set_restyle_hint(RestyleHint::restyle_subtree())
    }

    pub fn unhover(&mut self) {
        self.element_state.remove(ElementState::HOVER);
        self.set_restyle_hint(RestyleHint::restyle_subtree())
    }

    pub fn is_hovered(&self) -> bool {
        self.element_state.contains(ElementState::HOVER)
    }

    pub fn attrs(&self) -> Option<&[Attribute]> {
        Some(&self.element_data()?.attributes)
    }

    pub fn attr(&self, name: LocalName) -> Option<&str> {
        self.element_data()?.attr(name)
    }

    pub fn primary_styles(&self) -> Option<AtomicRef<'_, StyloComputedValues>> {
        let stylo_data = self.stylo_data.borrow();
        if stylo_data.as_ref().and_then(|data| data.styles.get_primary()).is_some() {
            Some(AtomicRef::map(
                stylo_data,
                |data: &Option<StyloElementData>| -> &StyloComputedValues {
                    data.as_ref().unwrap().styles.get_primary().unwrap()
                },
            ))
        } else {
            None
        }
    }

    pub fn style_arc(&self) -> Arc<StyloComputedValues> {
        self.stylo_data
            .borrow()
            .as_ref()
            .map(|element_data| element_data.styles.primary().clone())
            .unwrap_or(StyloComputedValues::initial_values_with_font_override(Font::initial_values())).to_arc()
    }

    /// Get text content of this node and its descendants
    pub fn text_content(&self) -> String {
        match &self.data {
            NodeData::Text { contents } => contents.borrow().to_string(),
            _ => {
                // Concatenate text from all children
                let mut result = String::new();
                for child in &self.children {
                    let child = self.get_node(*child);
                    result.push_str(&child.text_content());
                }
                result
            }
        }
    }

    /// Set the text content of this node
    /// For text nodes, replaces the text content
    /// For element nodes, removes all children and creates a single text node child
    pub fn set_text_content(&mut self, text: &str) {
        match &mut self.data {
            NodeData::Text { contents } => {
                let contents = contents.get_mut();
                *contents = StrTendril::from(text);
                // Invalidate layout since content changed
                self.invalidate_layout();
            }
            _ => {
                // Remove all children
                self.children.clear();
                
                // If text is not empty, add a single text node as child
                /*if !text.is_empty() {
                    let text_node = DomNode::new(NodeData::Text { contents: RefCell::new(StrTendril::from(text.to_string())) }, None);
                    self.add_child(text_node);
                }*/
                // Note: invalidate_layout is called in add_child, or here if text is empty
                if text.is_empty() {
                    self.invalidate_layout();
                }
            }
        }
    }

    /// Enhanced CSS selector matching (still simplified but more comprehensive)
    pub fn query_selector(&self, selector: &str) -> Vec<usize> {
        self.find_nodes(|node| self.matches_selector(node, selector))
    }

    /// Check if a node matches a CSS selector
    fn matches_selector(&self, node: &DomNode, selector: &str) -> bool {
        if let NodeData::Element(data) = &node.data {
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
                            return data.attributes.iter().find(|attr| &attr.name.local == attr_name).map(|attr| &attr.value) == Some(&attr_value.to_string());
                        } else {
                            // Just check if attribute exists
                            return data.attributes.iter().any(|attr| &attr.name.local == attr_part);
                        }
                    }
                }
            } else {
                // Tag selector
                return data.name.local.to_string() == selector;
            }
        }
        false
    }

    /// Get element by ID (returns first match)
    pub fn get_element_by_id(&self, id: &str) -> Option<&DomNode> {
        let selector = format!("#{}", id);
        let id = self.query_selector(&selector).into_iter().next();
        id.map(|id| self.get_node(id))
    }

    pub fn get_element_by_id_mut(&mut self, id: &str) -> Option<&mut DomNode> {
        let selector = format!("#{}", id);
        let id = self.query_selector(&selector).into_iter().next();
        id.map(|id| self.get_node_mut(id))
    }

    /// Get elements by class name
    pub fn get_elements_by_class_name(&self, class_name: &str) -> Vec<&DomNode> {
        let selector = format!(".{}", class_name);
        let ids = self.query_selector(&selector);
        ids.into_iter().map(|id| self.get_node(id)).collect()
    }

    /// Get elements by tag name
    pub fn get_elements_by_tag_name(&self, tag_name: &str) -> Vec<&DomNode> {
        let ids = self.query_selector(tag_name);
        ids.into_iter().map(|id| self.get_node(id)).collect()
    }

    /// Insert a child node at a specific position
    pub fn insert_child(&mut self, index: usize, child: DomNode) -> Result<Rc<RefCell<DomNode>>, &'static str> {
        if index > self.children.len() {
            return Err("Index out of bounds");
        }

        let child_rc = Rc::new(RefCell::new(child));
        self.children.insert(index, child_rc.borrow().id);

        Ok(child_rc)
    }

    /// Remove a child node
    pub fn remove_child(&mut self, child: &DomNode) -> Result<(), &'static str> {
        let position = self.children.iter().position(|c| ptr::eq(self.get_node(*c), child));
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
    pub fn contains(&self, other: &DomNode) -> bool {
        for child in &self.children {
            let child = self.get_node(*child);
            if ptr::eq(child, other) {
                return true;
            }
            if child.contains(other) {
                return true;
            }
        }
        false
    }

    /// Find nodes that match a predicate, returning owned references
    pub fn find_nodes<F>(&self, predicate: F) -> Vec<usize>
    where
        F: Fn(&DomNode) -> bool + Clone,
    {
        let mut result: Vec<usize> = Vec::new();

        // We can't include self in the result since we don't have an Rc to self
        // This method is meant to be called on nodes that are already in Rc<RefCell<>>

        // Recursively check children
        for child in &self.children {
            let child = self.get_node(*child);
            if predicate(&*child) {
                result.push(child.id);
            }

            // Recursively search in child's children
            let mut child_matches = child.find_nodes(predicate.clone());
            result.append(&mut child_matches);
        }

        result
    }
}

impl fmt::Debug for DomNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.data {
            NodeData::Document => write!(f, "Document"),
            NodeData::Element(data) => {
                write!(f, "<{}", data.name.local)?;
                
                // Write attributes
                for attr in data.attributes.iter() {
                    write!(f, " {}=\"{}\"", attr.name.local, attr.value)?;
                }
                
                if self.children.is_empty() {
                    write!(f, "/>")
                } else {
                    write!(f, ">")?;
                    
                    // Write children
                    for child in &self.children {
                        let child = self.get_node(*child);
                        write!(f, "{:?}", child)?;
                    }
                    
                    write!(f, "</{}>", data.name.local)
                }
            },
            NodeData::AnonymousBlock(data) => {
                write!(f, "<#anonymous-block {}>", data.name.local)
            },
            NodeData::Text { contents } => {
                let contents = contents.borrow();
                let content = if contents.len() > 50 {
                    format!("{}...", &contents[..50])
                } else {
                    contents.to_string()
                };
                write!(f, "{}", content)
            },
            NodeData::Comment { contents } => {
                write!(f, "<!-- {} -->", contents)
            },
            NodeData::DocType { name, public_id, system_id } => {
                write!(f, "<!DOCTYPE {} PUBLIC \"{}\" \"{}\">", name, public_id, system_id)
            },
            NodeData::DocumentFragment => {
                write!(f, "<#document-fragment>")
            },
            NodeData::ProcessingInstruction { target, data } => {
                write!(f, "<?{} {}?>", target, data)
            },
            NodeData::Image(data) => {
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
