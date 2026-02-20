use crate::dom::damage::{HoistedPaintChildren, ALL_DAMAGE};
use crate::dom::events::EventListenerRegistry;
use crate::dom::ZERO;
use crate::layout::table::TableContext;
use crate::ui::TextBrush;
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use bitflags::bitflags;
use blitz_traits::events::HitResult;
use html5ever::tendril::StrTendril;
use html5ever::{LocalName, QualName};
use html_escape::encode_quoted_attribute_to_string;
use markup5ever::local_name;
use parley::{Cluster, ContentWidths};
use peniko::Blob;
use selectors::matching::{ElementSelectorFlags, QuirksMode};
use skia_safe::wrapper::PointerWrapper;
use slab::Slab;
use std::cell::{Cell, RefCell};
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt, ptr};
use style::data::ElementData as StyleElementData;
use style::data::ElementData as StyloElementData;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::properties::style_structs::Font;
use style::properties::{parse_style_attribute, PropertyDeclarationBlock};
use style::selector_parser::{PseudoElement, RestyleDamage};
use style::servo_arc::{Arc as ServoArc, Arc};
use style::shared_lock::{Locked, SharedRwLock};
use style::stylesheets::{CssRuleType, DocumentStyleSheet, UrlExtraData};
use style::values::computed::{Display, PositionProperty};
use style::values::specified::box_::{DisplayInside, DisplayOutside};
use style::properties::generated::longhands::position::computed_value::T as Position;
use style_traits::ToCss;
use stylo_atoms::Atom;
use stylo_dom::ElementState;
use taffy::{Cache, Layout, Point, Style};
use url::Url;

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeKind {
    Document,
    Element,
    AnonymousBlock,
    Text,
    Comment,
}

/// Represents the type of DOM node
#[derive(Debug, Clone)]
pub enum NodeData {
    /// The `Document` itself - the root node.
    Document,
    Text(TextData),
    Comment { contents: StrTendril },
    Element(ElementData),
    // TODO better pseudo element support
    AnonymousBlock(ElementData),
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

    pub fn kind(&self) -> NodeKind {
        match self {
            NodeData::Document => NodeKind::Document,
            NodeData::Element(_) => NodeKind::Element,
            NodeData::AnonymousBlock(_) => NodeKind::AnonymousBlock,
            NodeData::Text { .. } => NodeKind::Text,
            NodeData::Comment { .. } => NodeKind::Comment,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextData {
    pub content: String,
}

impl TextData {
    pub fn new(content: String) -> Self {
        Self { content }
    }
}

#[derive(Clone, Default)]
pub struct TextLayout {
    pub text: String,
    pub content_widths: Option<ContentWidths>,
    pub layout: parley::layout::Layout<TextBrush>,
}

impl TextLayout {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn content_widths(&mut self) -> ContentWidths {
        *self
            .content_widths
            .get_or_insert_with(|| self.layout.calculate_content_widths())
    }
}

impl std::fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextLayout")
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

    pub special_data: SpecialElementData,

    pub background_images: Vec<Option<BackgroundImageData>>,

    pub inline_layout_data: Option<Box<TextLayout>>,

    pub list_item_data: Option<Box<ListItemLayout>>,

    /// For HTML <template> elements, holds the template contents
    pub template_contents: Option<usize>,
}

/// Heterogeneous data that depends on the element's type.
#[derive(Clone, Default)]
#[derive(Debug)]
pub enum SpecialElementData {
    Stylesheet(DocumentStyleSheet),
    /// An \<img\> element's image data
    Image(Box<ImageData>),
    /// A \<canvas\> element's custom paint source
    Canvas(CanvasData),
    /// Pre-computed table layout data
    TableRoot(std::sync::Arc<TableContext>),
    TextInput,
    /// Checkbox checked state
    CheckboxInput(bool),
    FileInput(FileData),
    /// No data (for nodes that don't need any node-specific data)
    #[default]
    None,
}

impl SpecialElementData {
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }
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
            special_data: SpecialElementData::None,
            inline_layout_data: None,
            list_item_data: None,
            template_contents: None,
            background_images: Vec::new(),
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

    pub fn attr_parsed<T: FromStr>(&self, attr_name: impl PartialEq<LocalName>) -> Option<T> {
        let attr = self.attributes.iter().find(|attr| attr_name == attr.name.local)?;
        attr.value.parse::<T>().ok()
    }

    pub fn attrs(&self) -> &AttributeMap {
        &self.attributes
    }

    pub fn has_attr(&self, attr_name: impl PartialEq<LocalName>) -> bool {
        self.attributes.iter().any(|attr| attr_name == attr.name.local)
    }

    pub fn image_data(&self) -> Option<&ImageData> {
        match &self.special_data {
            SpecialElementData::Image(data) => Some(&**data),
            _ => None,
        }
    }

    pub fn image_data_mut(&mut self) -> Option<&mut ImageData> {
        match self.special_data {
            SpecialElementData::Image(ref mut data) => Some(&mut **data),
            _ => None,
        }
    }

    pub fn raster_image_data(&self) -> Option<&RasterImageData> {
        match self.image_data()? {
            ImageData::Raster(data) => Some(data),
            _ => None,
        }
    }

    pub fn raster_image_data_mut(&mut self) -> Option<&mut RasterImageData> {
        match self.image_data_mut()? {
            ImageData::Raster(data) => Some(data),
            _ => None,
        }
    }

    pub fn canvas_data(&self) -> Option<&CanvasData> {
        match &self.special_data {
            SpecialElementData::Canvas(data) => Some(data),
            _ => None,
        }
    }

    pub fn svg_data(&self) -> Option<&usvg::Tree> {
        match self.image_data()? {
            ImageData::Svg(data) => Some(data),
            _ => None,
        }
    }

    pub fn svg_data_mut(&mut self) -> Option<&mut usvg::Tree> {
        match self.image_data_mut()? {
            ImageData::Svg(data) => Some(data),
            _ => None,
        }
    }

    pub fn take_inline_layout(&mut self) -> Option<Box<TextLayout>> {
        std::mem::take(&mut self.inline_layout_data)
    }
}

#[derive(Clone)]
pub struct ListItemLayout {
    pub marker: Marker,
    pub position: ListItemLayoutPosition,
}

//We seperate chars from strings in order to optimise rendering - ie not needing to
//construct a whole parley layout for simple char markers
#[derive(Debug, PartialEq, Clone)]
pub enum Marker {
    Char(char),
    String(String),
}

//Value depends on list-style-position, determining whether a seperate layout is created for it
#[derive(Clone)]
pub enum ListItemLayoutPosition {
    Inside,
    Outside(Box<parley::Layout<TextBrush>>),
}

impl std::fmt::Debug for ListItemLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ListItemLayout - marker {:?}", self.marker)
    }
}

#[derive(Debug, Clone)]
pub struct CanvasData {
    pub custom_paint_source_id: u64,
}

#[derive(Clone, Debug)]
pub struct FileData(pub Vec<PathBuf>);
impl Deref for FileData {
    type Target = Vec<PathBuf>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for FileData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl From<Vec<PathBuf>> for FileData {
    fn from(files: Vec<PathBuf>) -> Self {
        Self(files)
    }
}

#[derive(Debug, Clone)]
pub enum ImageData {
    Raster(RasterImageData),
    Svg(Box<usvg::Tree>),
    None
}

impl From<usvg::Tree> for ImageData {
    fn from(value: usvg::Tree) -> Self {
        ImageData::Svg(Box::new(value))
    }
}

impl ImageData {
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

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Ok,
    Error,
    Loading,
}

#[derive(Debug, Clone)]
pub struct BackgroundImageData {
    /// The url of the background image
    pub url: ServoArc<Url>,
    /// The loading status of the background image
    pub status: Status,
    /// The image data
    pub image: ImageData,
}

impl BackgroundImageData {
    pub fn new(url: ServoArc<Url>) -> Self {
        Self {
            url,
            status: Status::Loading,
            image: ImageData::None,
        }
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
    pub fn is_inline_root(&self) -> bool {
        self.contains(DomNodeFlags::IS_INLINE_ROOT)
    }

    #[inline]
    pub fn is_table_root(&self) -> bool {
        self.contains(DomNodeFlags::IS_TABLE_ROOT)
    }

    #[inline]
    pub fn is_in_document(&self) -> bool {
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
    pub layout_parent: Cell<Option<usize>>,
    pub layout_children: RefCell<Option<Vec<usize>>>,
    pub paint_children: RefCell<Option<Vec<usize>>>,
    pub stacking_context: Option<Box<HoistedPaintChildren>>,

    pub flags: DomNodeFlags,

    /// The type of node
    pub data: NodeData,

    pub stylo_data: AtomicRefCell<Option<StyleElementData>>,
    pub selector_flags: AtomicRefCell<ElementSelectorFlags>,
    pub lock: SharedRwLock,
    pub element_state: ElementState,

    // Pseudo element nodes
    pub before: Option<usize>,
    pub after: Option<usize>,

    // layout data:
    pub taffy_style: Style<Atom>,
    pub cache: Cache,
    pub unrounded_layout: Layout,
    pub final_layout: Layout,
    // todo proper node scroll impl
    pub scroll_offset: Point<f64>,

    pub has_snapshot: bool,
    pub snapshot_handled: AtomicBool,
    pub dirty_descendants: AtomicBool,
    pub display_constructed_as: Display,

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
            layout_parent: Cell::new(None),
            layout_children: RefCell::new(None),
            paint_children: RefCell::new(None),
            stacking_context: None,
            flags: DomNodeFlags::empty(),
            data,
            stylo_data: Default::default(),
            selector_flags: AtomicRefCell::new(ElementSelectorFlags::empty()),
            lock,
            element_state: ElementState::empty(),
            before: None,
            after: None,
            taffy_style: Default::default(),
            cache: Cache::new(),
            unrounded_layout: Layout::new(),
            final_layout: Layout::new(),
            scroll_offset: ZERO,
            has_snapshot: false,
            snapshot_handled: AtomicBool::new(false),
            dirty_descendants: AtomicBool::new(true),
            display_constructed_as: Display::Block,
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

    pub fn is_element(&self) -> bool {
        matches!(self.data, NodeData::Element(_))
    }

    pub fn is_anonymous(&self) -> bool {
        matches!(self.data, NodeData::AnonymousBlock(_))
    }

    pub fn is_text_node(&self) -> bool {
        matches!(self.data, NodeData::Text { .. })
    }

    pub fn element_data(&self) -> Option<&ElementData> {
        match &self.data {
            NodeData::Element(data) | NodeData::AnonymousBlock(data) => Some(data),
            _ => None,
        }
    }

    pub fn element_data_mut(&mut self) -> Option<&mut ElementData> {
        match &mut self.data {
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
        println!("Adding child {child} to {}", self.id);
        self.children.push(child);

        self.insert_damage(ALL_DAMAGE);
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
        self.mark_ancestors_dirty();
    }

    pub fn has_dirty_descendants(&self) -> bool {
        self.dirty_descendants.load(Ordering::Relaxed)
    }

    pub fn set_dirty_descendants(&self) {
        self.dirty_descendants.store(true, Ordering::Relaxed);
    }

    pub fn unset_dirty_descendants(&self) {
        self.dirty_descendants.store(false, Ordering::Relaxed);
    }

    pub fn mark_ancestors_dirty(&self) {
        let mut current = self.parent;
        while let Some(parent_id) = current {
            let parent = &self.tree()[parent_id];
            if parent.dirty_descendants.swap(true, Ordering::Relaxed) {
                break;
            }
            current = parent.parent;
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

    pub fn attrs(&self) -> Option<&AttributeMap> {
        Some(&self.element_data()?.attributes)
    }

    pub fn attr(&self, name: LocalName) -> Option<&str> {
        self.element_data()?.attr(name)
    }

    pub fn pe_by_index(&self, index: usize) -> Option<usize> {
        match index {
            0 => self.after,
            1 => self.before,
            _ => panic!("Invalid pseudo element index"),
        }
    }

    pub fn set_pe_by_index(&mut self, index: usize, value: Option<usize>) {
        match index {
            0 => self.after = value,
            1 => self.before = value,
            _ => panic!("Invalid pseudo element index"),
        }
    }

    pub(crate) fn display_style(&self) -> Option<Display> {
        Some(self.primary_styles().as_ref()?.clone_display())
    }

    pub fn is_or_contains_block(&self) -> bool {
        let style = self.primary_styles();
        let style = style.as_ref();

        // Ignore out-of-flow items
        let position = style
            .map(|s| s.clone_position())
            .unwrap_or(PositionProperty::Relative);
        let is_in_flow = matches!(
            position,
            PositionProperty::Static | PositionProperty::Relative | PositionProperty::Sticky
        );
        if !is_in_flow {
            return false;
        }
        let display = style
            .map(|s| s.clone_display())
            .unwrap_or(Display::inline());
        match display.outside() {
            DisplayOutside::None => false,
            DisplayOutside::Block => true,
            _ => {
                if display.inside() == DisplayInside::Flow {
                    self.children
                        .iter()
                        .copied()
                        .any(|child_id| self.tree()[child_id].is_or_contains_block())
                } else {
                    false
                }
            }
        }
    }

    pub fn is_whitespace_node(&self) -> bool {
        match &self.data {
            NodeData::Text(text) => text.content.chars().all(|c| c.is_ascii_whitespace()),
            _ => false,
        }
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
        match self.data {
            NodeData::Text(ref text) => text.content.clone(),
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

    pub fn order(&self) -> i32 {
        self.primary_styles()
            .map(|s| match s.pseudo() {
                Some(PseudoElement::Before) => i32::MIN,
                Some(PseudoElement::After) => i32::MAX,
                _ => s.clone_order(),
            })
            .unwrap_or(0)
    }

    pub fn z_index(&self) -> i32 {
        self.primary_styles()
            .map(|s| s.clone_z_index().integer_or(0))
            .unwrap_or(0)
    }

    // https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_positioned_layout/Stacking_context#features_creating_stacking_contexts
    pub fn is_stacking_context_root(&self, is_flex_or_grid_item: bool) -> bool {
        let Some(style) = self.primary_styles() else {
            return false;
        };

        let position = style.clone_position();
        let has_z_index = !style.clone_z_index().is_auto();

        if style.clone_opacity() != 1.0 {
            return true;
        }

        let position_based = match position {
            Position::Fixed | Position::Sticky => true,
            Position::Relative | Position::Absolute => has_z_index,
            Position::Static => has_z_index && is_flex_or_grid_item,
        };
        if position_based {
            return true;
        }

        // TODO: mix-blend-mode
        // TODO: transforms
        // TODO: filter
        // TODO: clip-path
        // TODO: mask
        // TODO: isolation
        // TODO: contain

        false
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

    /// Find nodes that match a predicate, returning owned references
    pub fn find_nodes_mut<F>(&mut self, predicate: F) -> Vec<usize>
    where
        F: Fn(&mut DomNode) -> bool + Clone,
    {
        let mut result: Vec<usize> = Vec::new();

        // We can't include self in the result since we don't have an Rc to self
        // This method is meant to be called on nodes that are already in Rc<RefCell<>>

        // Recursively check children
        for child in self.children.clone() {
            let child = self.get_node_mut(child);
            if predicate(&mut *child) {
                result.push(child.id);
            }

            // Recursively search in child's children
            let mut child_matches = child.find_nodes_mut(predicate.clone());
            result.append(&mut child_matches);
        }

        result
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<HitResult> {
        use style::computed_values::visibility::T as Visibility;

        // Don't hit on visbility:hidden elements
        if let Some(style) = self.primary_styles() {
            if matches!(
                style.clone_visibility(),
                Visibility::Hidden | Visibility::Collapse
            ) {
                return None;
            }
        }

        let mut x = x - self.final_layout.location.x + self.scroll_offset.x as f32;
        let mut y = y - self.final_layout.location.y + self.scroll_offset.y as f32;

        let size = self.final_layout.size;
        let matches_self = !(x < 0.0
            || x > size.width + self.scroll_offset.x as f32
            || y < 0.0
            || y > size.height + self.scroll_offset.y as f32);

        let content_size = self.final_layout.content_size;
        let matches_content = !(x < 0.0
            || x > content_size.width + self.scroll_offset.x as f32
            || y < 0.0
            || y > content_size.height + self.scroll_offset.y as f32);

        let matches_hoisted_content = match &self.stacking_context {
            Some(sc) => {
                let content_area = sc.content_area;
                x >= content_area.left + self.scroll_offset.x as f32
                && x <= content_area.right + self.scroll_offset.x as f32
                && y >= content_area.top + self.scroll_offset.y as f32
                && y <= content_area.bottom + self.scroll_offset.y as f32
            },
            None => false,
        };

        if !matches_self && !matches_content && !matches_hoisted_content {
            return None;
        }

        if self.flags.is_inline_root() {
            let content_box_offset = taffy::Point {
                x: self.final_layout.padding.left + self.final_layout.border.left,
                y: self.final_layout.padding.top + self.final_layout.border.top,
            };
            x -= content_box_offset.x;
            y -= content_box_offset.y;
        }

        // Positive z_index hoisted children
        if matches_hoisted_content {
            if let Some(hoisted) = &self.stacking_context {
                for child in hoisted.pos_z_hoisted_children().rev() {
                    let x = x - child.position.x;
                    let y = y - child.position.y;
                    if let Some(hit) = self.get_node(child.node_id).hit(x, y) {
                        return Some(hit);
                    }
                }
            }
        }

        // Call `.hit()` on each child in turn. If any return `Some` then return that value. Else return `Some(self.id).
        for child_id in self.paint_children.borrow().iter().flatten().rev() {
            if let Some(hit) = self.get_node(*child_id).hit(x, y) {
                return Some(hit);
            }
        }

        // Negative z_index hoisted children
        if matches_hoisted_content {
            if let Some(hoisted) = &self.stacking_context {
                for child in hoisted.neg_z_hoisted_children().rev() {
                    let x = x - child.position.x;
                    let y = y - child.position.y;
                    if let Some(hit) = self.get_node(child.node_id).hit(x, y) {
                        return Some(hit);
                    }
                }
            }
        }

        // Inline children
        if self.flags.is_inline_root() {
            let element_data = &self.element_data().unwrap();
            if let Some(ild) = element_data.inline_layout_data.as_ref() {
                let layout = &ild.layout;
                let scale = layout.scale();

                if let Some((cluster, _side)) =
                    Cluster::from_point_exact(layout, x * scale, y * scale)
                {
                    let style_index = cluster.glyphs().next()?.style_index();
                    let node_id = layout.styles()[style_index].brush.id;
                    return Some(HitResult {
                        node_id,
                        x,
                        y,
                        is_text: true,
                    });
                }
            }
        }

        // Self (this node)
        if matches_self {
            return Some(HitResult {
                node_id: self.id,
                x,
                y,
                is_text: false,
            });
        }

        None
    }

    pub fn outer_html(&self) -> String {
        let mut output = String::new();
        self.write_outer_html(&mut output);
        output
    }

    pub fn write_outer_html(&self, writer: &mut String) {
        let has_children = !self.children.is_empty();
        let current_color = self
            .primary_styles()
            .map(|style| style.clone_color())
            .map(|color| color.to_css_string());

        match &self.data {
            NodeData::Document => {}
            NodeData::Comment { contents: _ } => {}
            NodeData::AnonymousBlock(_) => {}
            // NodeData::Doctype { name, .. } => write!(s, "DOCTYPE {name}"),
            NodeData::Text(text) => {
                writer.push_str(&text.content);
            }
            NodeData::Element(data) => {
                writer.push('<');
                writer.push_str(&data.name.local);

                for attr in data.attrs().iter() {
                    writer.push(' ');
                    writer.push_str(&attr.name.local);
                    writer.push_str("=\"");
                    #[allow(clippy::unnecessary_unwrap)] // Convert to if-let chain once stabilised
                    if current_color.is_some() && attr.value.contains("currentColor") {
                        let value = attr
                            .value
                            .replace("currentColor", current_color.as_ref().unwrap());
                        encode_quoted_attribute_to_string(&value, writer);
                    } else {
                        encode_quoted_attribute_to_string(&attr.value, writer);
                    }
                    writer.push('"');
                }
                if !has_children {
                    writer.push_str(" /");
                }
                writer.push('>');

                if has_children {
                    for &child_id in &self.children {
                        self.tree()[child_id].write_outer_html(writer);
                    }

                    writer.push_str("</");
                    writer.push_str(&data.name.local);
                    writer.push('>');
                }
            }
        }
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
            NodeData::Text(text) => {
                let content = if text.content.len() > 50 {
                    format!("{}...", &text.content[..50])
                } else {
                    text.content.to_string()
                };
                write!(f, "{}", content)
            },
            NodeData::Comment { contents } => {
                write!(f, "<!-- {} -->", contents)
            },
        }
    }
}
