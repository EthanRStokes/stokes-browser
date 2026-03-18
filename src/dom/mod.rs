// DOM module for parsing and representing HTML content
mod parser;
pub(crate) mod node;
pub mod events;
mod config;
pub(crate) mod damage;
mod url;
mod layout;
mod traverse;
pub mod stylo_to_parley;
mod attr;
mod snapshot;
mod stylo_to_cursor;
mod resource;
mod resolve;
mod state;
mod selection;
pub(crate) mod form;
mod sub_dom;

use html5ever::ns;
pub use events::{EventDispatcher, EventType};
use std::any::Any;
use std::cell::RefCell;
pub use self::node::{
    AttributeMap,
    DomNode,
    ElementData,
    ImageData,
    NodeData,
    ShadowRootData,
    ShadowRootMode,
};
pub use self::parser::HtmlParser;
use crate::css::stylo::RecalcStyle;
use crate::dom::config::DomConfig;
use crate::dom::damage::{ALL_DAMAGE, CONSTRUCT_BOX, CONSTRUCT_DESCENDENT, CONSTRUCT_FC};
use crate::dom::layout::collect_layout_children;
use crate::dom::node::{Attribute, DomNodeFlags, SpecialElementData, TextData};
use crate::dom::url::DocUrl;
use crate::events::UiEvent;
use crate::networking::{ImageType, ResourceLoadResponse, StylesheetLoader};
use crate::ui::TextBrush;
use blitz_traits::events::HitResult;
use blitz_traits::net::{DummyNetProvider, NetProvider};
use blitz_traits::shell::{DummyShellProvider, ShellProvider, Viewport};
use euclid::Size2D;
use markup5ever::{local_name, QualName};
use parley::fontique::{Attributes, Blob, Query, QueryFont, QueryStatus};
use parley::{FontContext, FontVariation, LayoutContext};
use selectors::matching::QuirksMode;
use selectors::Element;
use skrifa::charmap::Charmap;
use skrifa::instance::{LocationRef, Size};
use skrifa::metrics::{GlyphMetrics, Metrics};
use skrifa::{MetadataProvider, Tag};
use slab::Slab;
use std::collections::{BTreeMap, Bound, HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::mem;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, RwLockReadGuard, RwLockWriteGuard};
use std::task::Context;
use std::time::Instant;
use cursor_icon::CursorIcon;
use skia_safe::wrapper::NativeTransmutableWrapper;
use style::animation::{AnimationState, DocumentAnimationSet};
use style::context::{RegisteredSpeculativePainter, RegisteredSpeculativePainters, SharedStyleContext};
use style::data::ElementStyles;
use style::dom::{TDocument, TNode};
use style::font_metrics::FontMetrics;
use style::global_style_data::{GLOBAL_STYLE_DATA, STYLE_THREAD_POOL};
use style::invalidation::element::restyle_hints::RestyleHint;
use style::media_queries::{Device, MediaList, MediaType};
use style::properties::style_structs::Font;
use style::properties::ComputedValues;
use style::queries::values::PrefersColorScheme;
use style::selector_parser::SnapshotMap;
use style::servo::media_queries::FontMetricsProvider;
use style::shared_lock::{SharedRwLock, StylesheetGuards};
use style::stylesheets::{AllowImportRules, DocumentStyleSheet, Origin, Stylesheet};
use style::stylist::Stylist;
use style::thread_state::ThreadState;
use style::traversal::DomTraversal;
use style::traversal_flags::TraversalFlags;
use style::values::computed::font::{GenericFontFamily, QueryFontMetricsFlags};
use style::values::computed::{Au, CSSPixelLength, Length, Overflow};
use stylo_atoms::Atom;
use taffy::Point;
use crate::dom::events::pointer::{DragMode, ScrollAnimationState};
use crate::dom::selection::TextSelection;
use crate::dom::stylo_to_cursor::stylo_to_cursor_icon;
use crate::dom::traverse::{AncestorTraverser, TreeTraverser};
use crate::engine::nav_provider::StokesNavigationProvider;
use crate::engine::net_provider::StokesNetProvider;
use crate::events::{BlitzScrollEvent, DomEventData};
use crate::qual_name;
use crate::shell_provider::{ShellProviderMessage, StokesShellProvider};
use crate::dom::events::{EventDriver, NoopEventHandler};
use crate::js::bindings::event_listeners::JsEventHandler;
use crate::dom::parser::HtmlProvider;
use crate::engine::js_provider::StokesJsProvider;

const ZERO: Point<f64> = Point { x: 0.0, y: 0.0 };

pub enum DomGuard<'a> {
    Ref(&'a Dom),
    RefCell(std::cell::Ref<'a, Dom>),
    RwLock(RwLockReadGuard<'a, Dom>),
    Mutex(MutexGuard<'a, Dom>),
}

impl Deref for DomGuard<'_> {
    type Target = Dom;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ref(base_document) => base_document,
            Self::RefCell(refcell_guard) => refcell_guard,
            Self::RwLock(rw_lock_read_guard) => rw_lock_read_guard,
            Self::Mutex(mutex_guard) => mutex_guard,
        }
    }
}

pub enum DomGuardMut<'a> {
    Ref(&'a mut Dom),
    RefCell(std::cell::RefMut<'a, Dom>),
    RwLock(RwLockWriteGuard<'a, Dom>),
    Mutex(MutexGuard<'a, Dom>),
}

impl Deref for DomGuardMut<'_> {
    type Target = Dom;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ref(base_document) => base_document,
            Self::RefCell(refcell_guard) => refcell_guard,
            Self::RwLock(rw_lock_read_guard) => rw_lock_read_guard,
            Self::Mutex(mutex_guard) => mutex_guard,
        }
    }
}

impl DerefMut for DomGuardMut<'_> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Ref(base_document) => base_document,
            Self::RefCell(refcell_guard) => &mut *refcell_guard,
            Self::RwLock(rw_lock_read_guard) => &mut *rw_lock_read_guard,
            Self::Mutex(mutex_guard) => &mut *mutex_guard,
        }
    }
}

pub trait AbstractDom: Any + 'static {
    fn inner(&self) -> DomGuard<'_>;
    fn inner_mut(&mut self) -> DomGuardMut<'_>;

    /// Update the [`Document`] in response to a [`UiEvent`] (click, keypress, etc)
    fn handle_ui_event(&mut self, event: UiEvent) {
        let mut doc = self.inner_mut();
        let mut driver = EventDriver::new(&mut *doc, JsEventHandler);
        //let mut driver = EventDriver::new(&mut *doc, NoopEventHandler);
        driver.handle_ui_event(event);
    }

    /// Poll any pending async operations, and flush changes to the underlying [`BaseDocument`]
    fn poll(&mut self, task_context: Option<Context>) -> bool {
        // Default implementation does nothing
        let _ = task_context;
        false
    }

    /// Get the [`Document`]'s id
    fn id(&self) -> usize {
        self.inner().id
    }
}

pub struct PlainDom(pub Dom);
impl AbstractDom for PlainDom {
    fn inner(&self) -> DomGuard<'_> {
        DomGuard::Ref(&self.0)
    }
    fn inner_mut(&mut self) -> DomGuardMut<'_> {
        DomGuardMut::Ref(&mut self.0)
    }
}

impl AbstractDom for Dom {
    fn inner(&self) -> DomGuard<'_> {
        DomGuard::Ref(self)
    }
    fn inner_mut(&mut self) -> DomGuardMut<'_> {
        DomGuardMut::Ref(self)
    }
}

impl AbstractDom for Rc<RefCell<Dom>> {
    fn inner(&self) -> DomGuard<'_> {
        DomGuard::RefCell(self.borrow())
    }

    fn inner_mut(&mut self) -> DomGuardMut<'_> {
        DomGuardMut::RefCell(self.borrow_mut())
    }
}

/// Represents a DOM tree
pub struct Dom {
    /// ID of the DOM
    pub(crate) id: usize,

    pub(crate) url: DocUrl,
    // Viewport information (dimensions, HiDPI scale, zoom)
    pub(crate) viewport: Viewport,
    // Scroll position in the viewport
    pub(crate) viewport_scroll: Point<f64>,

    pub(crate) tx: Sender<DomEvent>,
    pub(crate) rx: Option<Receiver<DomEvent>>,

    pub(crate) nodes: Box<Slab<DomNode>>,

    // Stylo
    pub(crate) stylist: Stylist,
    pub(crate) animations: DocumentAnimationSet,
    pub(crate) lock: SharedRwLock,
    // Stylo invalidation map
    pub(crate) snapshots: SnapshotMap,

    pub(crate) font_ctx: Arc<Mutex<FontContext>>,
    pub(crate) layout_ctx: LayoutContext<TextBrush>,

    // mouse hover
    pub(crate) hover_node_id: Option<usize>,
    pub(crate) hover_node_is_text: bool,
    // currently focused node
    pub(crate) focus_node_id: Option<usize>,
    // currently active node
    pub(crate) active_node_id: Option<usize>,
    pub(crate) mousedown_node_id: Option<usize>,
    pub(crate) last_mousedown_time: Option<Instant>,
    pub(crate) mousedown_pos: taffy::Point<f32>,
    pub(crate) quick_clicks: u16,
    pub(crate) drag_mode: DragMode,
    pub(crate) scroll_animation: ScrollAnimationState,

    pub(crate) text_selection: TextSelection,

    pub(crate) has_active_animations: bool,
    pub(crate) has_canvas: bool,
    pub(crate) subdom_is_animating: bool,

    pub(crate) nodes_to_id: HashMap<String, usize>,
    pub(crate) nodes_to_stylesheet: BTreeMap<usize, DocumentStyleSheet>,
    pub(crate) stylesheets: HashMap<String, DocumentStyleSheet>,
    pub(crate) controls_to_form: HashMap<usize, usize>,
    pub(crate) sub_dom_nodes: HashSet<usize>,

    pub(crate) image_cache: HashMap<String, ImageData>,
    pub(crate) pending_images: HashMap<String, Vec<(usize, ImageType)>>,

    pub net_provider: Arc<StokesNetProvider>,
    pub shell_provider: Arc<StokesShellProvider>,
    pub nav_provider: Arc<StokesNavigationProvider>,
    pub html_provider: Arc<HtmlProvider>,
    pub js_provider: Arc<StokesJsProvider>,
}

pub enum DomEvent {
    ResourceLoad(ResourceLoadResponse)
}

pub(crate) fn device(viewport: &Viewport, font_ctx: Arc<Mutex<FontContext>>) -> Device {
    let width = viewport.window_size.0 as f32 / viewport.scale();
    let height = viewport.window_size.1 as f32 / viewport.scale();
    let size = Size2D::new(width, height);
    let pixel_ratio = euclid::Scale::new(viewport.scale());

    Device::new(
        MediaType::screen(),
        QuirksMode::NoQuirks,
        size,
        pixel_ratio,
        Box::new(StokesFontMetricsProvider { font_ctx }),
        ComputedValues::initial_values_with_font_override(Font::initial_values()),
        PrefersColorScheme::Dark // TODO detect color scheme preference
    )
}

struct StokesFontMetricsProvider {
    font_ctx: Arc<Mutex<FontContext>>,
}

impl Debug for StokesFontMetricsProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "StokesFontMetricsProvider")
    }
}

impl FontMetricsProvider for StokesFontMetricsProvider {
    fn query_font_metrics(&self, vertical: bool, font: &Font, font_size: CSSPixelLength, flags: QueryFontMetricsFlags) -> FontMetrics {
        let mut font_ctx = self.font_ctx.lock().unwrap();
        let font_ctx = &mut *font_ctx;

        let mut query = font_ctx.collection.query(&mut font_ctx.source_cache);
        let families = font.font_family.families.iter().map(stylo_to_parley::query_font_family);
        query.set_families(families);
        query.set_attributes(Attributes {
            width: stylo_to_parley::font_width(font.font_stretch),
            weight: stylo_to_parley::font_weight(font.font_weight),
            style: stylo_to_parley::font_style(font.font_style),
        });

        let variations = stylo_to_parley::font_variations(&font.font_variation_settings);

        fn find_font_for(query: &mut Query, ch: char) -> Option<QueryFont> {
            let mut font = None;
            query.matches_with(|q_font: &QueryFont| {
                use skrifa::MetadataProvider;

                let Ok(font_ref) = skrifa::FontRef::from_index(q_font.blob.as_ref(), q_font.index)
                else {
                    return QueryStatus::Continue;
                };

                let charmap = font_ref.charmap();
                if charmap.map(ch).is_some() {
                    font = Some(q_font.clone());
                    QueryStatus::Stop
                } else {
                    QueryStatus::Continue
                }
            });
            font
        }

        fn advance_of(
            query: &mut Query,
            ch: char,
            font_size: Size,
            variations: &[FontVariation],
        ) -> Option<f32> {
            let font = find_font_for(query, ch)?;
            let font_ref = skrifa::FontRef::from_index(font.blob.as_ref(), font.index).ok()?;
            let location = font_ref.axes().location(
                variations
                    .iter()
                    .map(|v| (Tag::from_be_bytes(v.tag.to_bytes()), v.value)),
            );
            let location_ref = LocationRef::from(&location);
            let glyph_metrics = GlyphMetrics::new(&font_ref, font_size, location_ref);
            let char_map = Charmap::new(&font_ref);
            let glyph_id = char_map.map(ch)?;
            glyph_metrics.advance_width(glyph_id)
        }

        fn metrics_of(
            query: &mut Query,
            ch: char,
            font_size: Size,
            variations: &[FontVariation],
        ) -> Option<(f32, Option<f32>, Option<f32>)> {
            let font = find_font_for(query, ch)?;
            let font_ref = skrifa::FontRef::from_index(font.blob.as_ref(), font.index).ok()?;
            let location = font_ref.axes().location(
                variations
                    .iter()
                    .map(|v| (Tag::from_be_bytes(v.tag.to_bytes()), v.value)),
            );
            let location_ref = LocationRef::from(&location);
            let metrics = Metrics::new(&font_ref, font_size, location_ref);
            Some((metrics.ascent, metrics.x_height, metrics.cap_height))
        }

        let font_size = Size::new(font_size.px());
        let zero_advance = advance_of(&mut query, '0', font_size, &variations);
        let ic_advance = advance_of(&mut query, '\u{6C34}', font_size, &variations);
        let (ascent, x_height, cap_height) =
            metrics_of(&mut query, ' ', font_size, &variations).unwrap_or((0.0, None, None));

        FontMetrics {
            ascent: CSSPixelLength::new(ascent),
            x_height: x_height.filter(|xh| *xh != 0.0).map(CSSPixelLength::new),
            cap_height: cap_height.map(CSSPixelLength::new),
            zero_advance_measure: zero_advance.map(CSSPixelLength::new),
            ic_width: ic_advance.map(CSSPixelLength::new),
            script_percent_scale_down: None,
            script_script_percent_scale_down: None,
        }
    }

    fn base_size_for_generic(&self, generic: GenericFontFamily) -> Length {
        let size = match generic {
            GenericFontFamily::Monospace => 13.0,
            _ => 16.0,
        };
        Length::from(Au::from_f32_px(size))
    }
}

pub(crate) const DEFAULT_CSS: &str = include_str!("../../assets/default.css");
pub(crate) const BULLET_FONT: &[u8] = include_bytes!("../../assets/moz-bullet-font.otf");

impl Dom {
    /// Create a new empty DOM
    pub fn new(config: DomConfig) -> Self {
        static ID_GENERATOR: AtomicUsize = AtomicUsize::new(1);

        let id = ID_GENERATOR.fetch_add(1, Ordering::SeqCst);

        stylo_config::set_bool("layout.flexbox.enabled", true);
        stylo_config::set_bool("layout.grid.enabled", true);
        stylo_config::set_bool("layout.legacy_layout", true);
        stylo_config::set_bool("layout.unimplemented", true);
        stylo_config::set_bool("layout.columns.enabled", true);
        stylo_config::set_bool("layout.css.attr.enabled", true);
        stylo_config::set_bool("layout.writing-mode.enabled", true);
        stylo_config::set_bool("layout.variable_fonts.enabled", true);
        stylo_config::set_bool("layout.container-queries.enabled", true);

        let viewport = config.viewport.unwrap_or_default();
        let font_ctx = config.font_ctx.unwrap_or_else(|| {
            let mut font_ctx = FontContext::default();
            font_ctx.collection.register_fonts(Blob::new(Arc::new(BULLET_FONT) as _), None);
            font_ctx
        });
        let font_ctx = Arc::new(Mutex::new(font_ctx));
        let device = device(&viewport, font_ctx.clone());

        let base_url = config.base_url.and_then(|url| DocUrl::from_str(&url).ok()).unwrap_or_default();
        let net_provider = config.net_provider.unwrap();
        let shell_provider = config.shell_provider.unwrap();
        let nav_provider = config.nav_provider.unwrap();
        let js_provider = config.js_provider.unwrap();

        let (tx, rx) = channel();

        let mut dom = Self {
            id,
            url: base_url,
            viewport,
            viewport_scroll: ZERO,
            tx,
            rx: Some(rx),
            nodes: Box::new(Slab::new()),
            //root: Rc::new(RefCell::from(DomNode::new(NodeData::Document, None))),
            stylist: Stylist::new(device, QuirksMode::NoQuirks),
            animations: Default::default(),
            lock: SharedRwLock::new(),
            snapshots: SnapshotMap::new(),
            font_ctx,
            layout_ctx: LayoutContext::new(),
            hover_node_id: None,
            hover_node_is_text: false,
            focus_node_id: None,
            active_node_id: None,
            mousedown_node_id: None,
            last_mousedown_time: None,
            mousedown_pos: Point::ZERO,
            quick_clicks: 0,
            drag_mode: DragMode::None,
            scroll_animation: ScrollAnimationState::None,
            text_selection: TextSelection::default(),
            has_active_animations: false,
            has_canvas: false,
            subdom_is_animating: false,
            nodes_to_id: Default::default(),
            nodes_to_stylesheet: Default::default(),
            stylesheets: Default::default(),
            controls_to_form: HashMap::new(),
            sub_dom_nodes: HashSet::new(),
            image_cache: HashMap::new(),
            pending_images: HashMap::new(),
            net_provider,
            shell_provider,
            nav_provider,
            html_provider: Arc::new(HtmlProvider),
            js_provider,
        };

        // Create the root document node
        dom.create_node(NodeData::Document);
        dom.root_node_mut().flags.insert(DomNodeFlags::IS_IN_DOCUMENT);

        match config.stylesheets {
            Some(stylesheets) => {
                for sheet in &stylesheets {
                    dom.add_stylesheet(sheet);
                }
            }
            None => {
                dom.add_stylesheet(DEFAULT_CSS);
            }
        }

        let stylo_element_data = style::data::ElementData {
            styles: ElementStyles {
                primary: Some(ComputedValues::initial_values_with_font_override(Font::initial_values()).to_arc()),
                ..Default::default()
            },
            ..Default::default()
        };
        *dom.root_node().stylo_data.borrow_mut() = Some(stylo_element_data);

        dom
    }

    pub(crate) fn create_node(&mut self, data: NodeData) -> usize {
        let slab_ptr = self.nodes.as_mut() as *mut Slab<DomNode>;

        let entry = self.nodes.vacant_entry();
        let id = entry.key();
        entry.insert(DomNode::new(slab_ptr, id, self.lock.clone(), data));

        id
    }

    pub(crate) fn create_element(
        &mut self,
        tag_name: QualName,
        attributes: AttributeMap,
    ) -> usize {
        let mut data = ElementData::new(tag_name, attributes);
        data.flush_style_attribute(&self.lock, &self.url.url_extra_data());

        let id = self.create_node(NodeData::Element(data));
        let node = self.get_node(id).unwrap();

        *node.stylo_data.borrow_mut() = Some(style::data::ElementData {
            damage: ALL_DAMAGE,
            ..Default::default()
        });

        id
    }

    pub(crate) fn create_comment_node(&mut self) -> usize {
        self.create_node(NodeData::Comment)
    }

    pub(crate) fn create_text_node(&mut self, text: &str) -> usize {
        let content = text.to_string();
        let data = NodeData::Text(TextData::new(content));
        self.create_node(data)
    }

    pub(crate) fn create_shadow_root_node(&mut self, mode: ShadowRootMode) -> usize {
        self.create_node(NodeData::ShadowRoot(ShadowRootData {
            mode,
            style_data: None,
        }))
    }

    pub fn attach_shadow(&mut self, host_id: usize, mode: ShadowRootMode) -> Result<usize, &'static str> {
        let is_host_element = self
            .nodes
            .get(host_id)
            .is_some_and(|node| node.is_element());
        if !is_host_element {
            return Err("Shadow host must be an element");
        }

        if let Some(existing) = self.nodes[host_id].shadow_root {
            return Ok(existing);
        }

        let shadow_root_id = self.create_shadow_root_node(mode);
        {
            let host = &mut self.nodes[host_id];
            host.shadow_root = Some(shadow_root_id);
            host.insert_damage(ALL_DAMAGE);
            host.mark_ancestors_dirty();
            if let Some(data) = &mut *host.stylo_data.borrow_mut() {
                data.hint |= RestyleHint::restyle_subtree();
            }
        }

        let host_in_doc = self.nodes[host_id].flags.is_in_document();
        {
            let shadow_root = &mut self.nodes[shadow_root_id];
            shadow_root.parent = None;
            shadow_root.shadow_host = Some(host_id);
            shadow_root.flags.set(DomNodeFlags::IS_IN_DOCUMENT, host_in_doc);
            shadow_root.insert_damage(ALL_DAMAGE);
        }

        if host_in_doc {
            self.process_added_subtree(shadow_root_id);
        }

        Ok(shadow_root_id)
    }

    pub fn shadow_root_id(&self, host_id: usize) -> Option<usize> {
        self.nodes.get(host_id).and_then(|node| node.shadow_root)
    }

    pub fn open_shadow_root_id(&self, host_id: usize) -> Option<usize> {
        let shadow_root_id = self.shadow_root_id(host_id)?;
        match self.nodes.get(shadow_root_id)?.data.shadow_root()?.mode {
            ShadowRootMode::Open => Some(shadow_root_id),
            ShadowRootMode::Closed => None,
        }
    }

    pub fn shadow_host_id(&self, shadow_root_id: usize) -> Option<usize> {
        self.nodes.get(shadow_root_id).and_then(|node| node.shadow_host)
    }

    pub fn shadow_root_style_data(
        &self,
        shadow_root_id: usize,
    ) -> Option<&style::stylist::CascadeData> {
        self
            .nodes
            .get(shadow_root_id)
            .and_then(|node| node.data.shadow_root())
            .and_then(|data| data.style_data.as_deref())
    }

    pub fn set_shadow_root_style_data(
        &mut self,
        shadow_root_id: usize,
        style_data: Option<style::servo_arc::Arc<style::stylist::CascadeData>>,
    ) -> Result<(), &'static str> {
        let Some(node) = self.nodes.get_mut(shadow_root_id) else {
            return Err("Shadow root node not found");
        };
        let Some(shadow_root) = node.data.shadow_root_mut() else {
            return Err("Node is not a shadow root");
        };
        shadow_root.style_data = style_data;
        Ok(())
    }

    /// Parse HTML into a DOM
    pub fn parse_html(
        url: &str,
        html: &str,
        user_agent: String,
        debug_net: bool,
        viewport: Viewport,
        shell_provider: Arc<StokesShellProvider>,
        nav_provider: Arc<StokesNavigationProvider>,
        js_provider: Arc<StokesJsProvider>,
    ) -> Self {
        let parser = HtmlParser::new();
        parser.parse(html, DomConfig {
            viewport: Some(viewport),
            base_url: Some(url.to_string()),
            net_provider: Some(Arc::new(StokesNetProvider::new(user_agent, debug_net,))),
            shell_provider: Some(shell_provider),
            nav_provider: Some(nav_provider),
            js_provider: Some(js_provider),
            ..Default::default()
        })
    }

    pub fn add_stylesheet(&mut self, css: &str) {
        self.add_stylesheet_with_origin(css, Origin::UserAgent);
    }

    pub fn add_author_stylesheet(&mut self, css: &str) {
        self.add_stylesheet_with_origin(css, Origin::Author);
    }

    fn add_stylesheet_with_origin(&mut self, css: &str, origin: Origin) {
        let sheet = self.make_stylesheet(css, origin);
        self.stylesheets.insert(css.to_string(), sheet.clone());
        self.stylist.append_stylesheet(sheet, &self.lock.read());
    }

    pub fn remove_stylesheet(&mut self, css: &str) {
        if let Some(sheet) = self.stylesheets.remove(css) {
            self.stylist.remove_stylesheet(sheet, &self.lock.read());
        }
    }

    pub fn add_stylesheet_for_node(&mut self, stylesheet: DocumentStyleSheet, node_id: usize) {
        let old = self.nodes_to_stylesheet.insert(node_id, stylesheet.clone());

        if let Some(old) = old {
            self.stylist.remove_stylesheet(old, &self.lock.read())
        }

        // Fetch @font-face fonts
        crate::networking::fetch_font_face(
            self.tx.clone(),
            self.id,
            Some(node_id),
            &stylesheet.0,
            &self.net_provider,
            &self.shell_provider,
            &self.lock.read(),
        );

        // Store data on element
        let element = &mut self.nodes[node_id].element_data_mut().unwrap();
        element.special_data = SpecialElementData::Stylesheet(stylesheet.clone());

        // TODO: Nodes could potentially get reused so ordering by node_id might be wrong.
        let insertion_point = self
            .nodes_to_stylesheet
            .range((Bound::Excluded(node_id), Bound::Unbounded))
            .next()
            .map(|(_, sheet)| sheet);

        if let Some(insertion_point) = insertion_point {
            self.stylist.insert_stylesheet_before(
                stylesheet,
                insertion_point.clone(),
                &self.lock.read(),
            )
        } else {
            self.stylist
                .append_stylesheet(stylesheet, &self.lock.read())
        }
    }

    pub fn process_style_element(&mut self, target_id: usize) {
        let css = self.nodes[target_id].text_content();
        let css = html_escape::decode_html_entities(&css);
        let sheet = self.make_stylesheet(&css, Origin::Author);
        self.add_stylesheet_for_node(sheet, target_id);
    }

    pub fn make_stylesheet(&self, css: impl AsRef<str>, origin: Origin) -> DocumentStyleSheet {
        let data = Stylesheet::from_str(
            css.as_ref(),
            self.url.url_extra_data(),
            origin,
            style::servo_arc::Arc::new(self.lock.wrap(MediaList::empty())),
            self.lock.clone(),
            Some(&StylesheetLoader {
                tx: self.tx.clone(),
                dom_id: self.id,
                net_provider: self.net_provider.clone(),
                shell_provider: self.shell_provider.clone(),
            }),
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::Yes
        );

        DocumentStyleSheet(style::servo_arc::Arc::new(data))
    }

    pub fn flush_styles(&mut self, now: f64) {
        style::thread_state::enter(ThreadState::LAYOUT);
        let lock = &self.lock;
        let author = lock.read();
        let ua_or_user = lock.read();
        let guards = StylesheetGuards {
            author: &author,
            ua_or_user: &ua_or_user,
        };

        // Flush the stylist with all loaded stylesheets
        {
            let root = TDocument::as_node(&&self.nodes[0])
                .first_element_child()
                .unwrap()
                .as_element()
                .unwrap();

            self.stylist.flush(&guards).process_style(root, Some(&self.snapshots));
        }

        // mark animating as dirty
        let mut sets = self.animations.sets.write();
        for (key, value) in sets.iter_mut() {
            let node_id = key.node.id();
            self.nodes[node_id].set_restyle_hint(RestyleHint::RESTYLE_SELF);

            for animation in value.animations.iter_mut() {
                if animation.state == AnimationState::Pending && animation.started_at <= now {
                    animation.state = AnimationState::Running;
                }
                animation.iterate_if_necessary(now);

                if animation.state == AnimationState::Running && animation.has_ended(now) {
                    animation.state = AnimationState::Finished;
                }
            }

            for transition in value.transitions.iter_mut() {
                if transition.state == AnimationState::Pending && transition.start_time <= now {
                    transition.state = AnimationState::Running;
                }
                if transition.state == AnimationState::Running && transition.has_ended(now) {
                    transition.state = AnimationState::Finished;
                }
            }
        }
        drop(sets);

        struct Painters;
        impl RegisteredSpeculativePainters for Painters {
            fn get(&self, name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
                None
            }
        }

        // Perform style traversal to compute styles for all elements
        {
            let context = SharedStyleContext {
                stylist: &self.stylist,
                visited_styles_enabled: false,
                options: GLOBAL_STYLE_DATA.options.clone(),
                guards: guards,
                animations: self.animations.clone(),
                current_time_for_animations: now,
                traversal_flags: TraversalFlags::empty(),
                snapshot_map: &self.snapshots,
                registered_speculative_painters: &Painters,
            };

            let root = self.root_element();
            let token = RecalcStyle::pre_traverse(root, &context);

            if token.should_traverse() {
                let traverser = RecalcStyle::new(context);
                let rayon_pool = STYLE_THREAD_POOL.pool();
                style::driver::traverse_dom(&traverser, token, rayon_pool.as_ref());
            }

            for opaque in self.snapshots.keys() {
                let id = opaque.id();
                if let Some(node) = self.nodes.get_mut(id) {
                    node.has_snapshot = false;
                }
            }
            self.snapshots.clear();

            let mut sets = self.animations.sets.write();
            for set in sets.values_mut() {
                set.clear_canceled_animations();
                for animation in set.animations.iter_mut() {
                    animation.is_new = false;
                }
                for transition in set.transitions.iter_mut() {
                    transition.is_new = false;
                }
            }
            sets.retain(|_, state| !state.is_empty());
            self.has_active_animations = sets.values().any(|state| state.needs_animation_ticks());

            self.stylist.rule_tree().maybe_gc();
        }
        drop(author);
        drop(ua_or_user);
        style::thread_state::exit(ThreadState::LAYOUT);
    }

    pub fn get_layout_children(&mut self) {
        get_layout_children_recursive(self, self.root_node().id);

        fn get_layout_children_recursive(dom: &mut Dom, node_id: usize) {
            let mut damage = dom.nodes[node_id].damage().unwrap_or(ALL_DAMAGE);
            let _flags = &dom.nodes[node_id].flags;

            if damage.intersects(CONSTRUCT_FC | CONSTRUCT_BOX) {
                let mut layout_children = Vec::new();
                let mut anonymous_block: Option<usize> = None;
                collect_layout_children(dom, node_id, &mut layout_children, &mut anonymous_block);

                // Recurse into newly collected layout children
                for child_id in layout_children.iter().copied() {
                    get_layout_children_recursive(dom, child_id);
                    dom.nodes[child_id].layout_parent.set(Some(node_id));
                    if let Some(data) = dom.nodes[child_id].stylo_data.get_mut() {
                        data.damage
                            .remove(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                    }
                }

                *dom.nodes[node_id].layout_children.borrow_mut() = Some(layout_children.clone());

                damage.remove(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                // damage.insert(RestyleDamage::RELAYOUT | RestyleDamage::REPAINT);
            } else {
                let layout_children = dom.nodes[node_id].layout_children.borrow_mut().take();
                if let Some(layout_children) = layout_children {
                    for child_id in layout_children.iter().copied() {
                        get_layout_children_recursive(dom, child_id);
                        dom.nodes[child_id].layout_parent.set(Some(node_id));
                    }

                    *dom.nodes[node_id].layout_children.borrow_mut() = Some(layout_children);
                }
            }

            dom.nodes[node_id].set_damage(damage);
        }
    }

    pub fn flush_layout_style(&mut self, node_id: usize) {
        {
            // set layout style
            let node = self.nodes.get_mut(node_id).unwrap();
            let stylo_data = node.stylo_data.borrow();
            let primary_styles = stylo_data.as_ref().and_then(|data| data.styles.get_primary());

            let Some(style) = primary_styles else {
                return;
            };

            node.taffy_style = stylo_taffy::to_taffy_style(style);
        }

        // set layout styles for children
        for child_id in self.nodes[node_id].children.clone() {
            self.flush_layout_style(child_id);
        }
    }

    pub fn node_from_id(&self, node_id: taffy::prelude::NodeId) -> &DomNode {
        &self.nodes[node_id.into()]
    }

    pub fn node_from_id_mut(&mut self, node_id: taffy::prelude::NodeId) -> &mut DomNode {
        &mut self.nodes[node_id.into()]
    }

    pub(crate) fn remove_and_drop_pe(&mut self, node_id: usize) -> Option<DomNode> {
        fn remove_pe_ignoring_parent(dom: &mut Dom, node_id: usize) -> Option<DomNode> {
            let mut node = dom.nodes.try_remove(node_id);
            if let Some(node) = &mut node {
                for &child in &node.children {
                    remove_pe_ignoring_parent(dom, child);
                }
            }
            node
        }

        let node = remove_pe_ignoring_parent(self, node_id);

        // Update child_idx values
        if let Some(parent_id) = node.as_ref().and_then(|node| node.parent) {
            let parent = &mut self.nodes[parent_id];
            parent.children.retain(|id| *id != node_id);
        }

        node
    }

    pub(crate) fn compute_has_canvas(&self) -> bool {
        TreeTraverser::new(self).any(|node_id| {
            let node = &self.nodes[node_id];
            let Some(element) = node.element_data() else {
                return false;
            };
            if element.name.local == local_name!("canvas") && element.has_attr(local_name!("src")) {
                return true;
            }

            false
        })
    }

    pub fn animating(&self) -> bool {
        self.has_canvas
            | self.has_active_animations
            | self.subdom_is_animating
            | (self.scroll_animation != ScrollAnimationState::None)
    }

    /// Find nodes by tag name
    pub fn query_selector(&self, selector: &str) -> Vec<&DomNode> {
        let ids = self.root_node().query_selector(selector);
        ids.into_iter()
            .filter_map(|id| self.nodes.get(id))
            .collect()
    }

    /// Find nodes that match a predicate
    pub fn find_nodes<F>(&self, predicate: F) -> Vec<&DomNode>
    where
        F: Fn(&DomNode) -> bool + Clone,
    {
        let ids = self.root_node().find_nodes(predicate);
        ids.into_iter()
            .filter_map(|id| self.nodes.get(id))
            .collect()
    }

    /// Find nodes that match a predicate
    pub fn find_node_ids<F>(&self, predicate: F) -> Vec<usize>
    where
        F: Fn(&DomNode) -> bool + Clone,
    {
        self.root_node().find_nodes(predicate)
    }

    /// Extract the page title
    pub fn get_title(&self) -> String {
        // Find the title element in the head
        let title_nodes = self.query_selector("title");
        if let Some(title_node) = title_nodes.first() {
            // Get text content of the title
            title_node.text_content()
        } else {
            // Default title if not found
            "Untitled".to_string()
        }
    }

    /// Clear the layout cache for a node and all its ancestors.
    /// This is necessary when a node's intrinsic size changes (e.g., when an image loads)
    /// so that layout will be recomputed correctly.
    pub fn clear_layout_cache_with_ancestors(&mut self, node_id: usize) {
        let mut current_id = Some(node_id);
        while let Some(id) = current_id {
            if let Some(node) = self.nodes.get_mut(id) {
                node.cache.clear();
                current_id = node.layout_parent.get();
            } else {
                break;
            }
        }
    }

    /// Find the label's bound input elements:
    /// the element id referenced by the "for" attribute of a given label element
    /// or the first input element which is nested in the label
    /// Note that although there should only be one bound element,
    /// we return all possibilities instead of just the first
    /// in order to allow the caller to decide which one is correct
    pub fn label_bound_input_element(&self, label_node_id: usize) -> Option<&DomNode> {
        let label_element = self.nodes[label_node_id].element_data()?;
        if let Some(target_element_dom_id) = label_element.attr(local_name!("for")) {
            TreeTraverser::new(self)
                .filter_map(|id| {
                    let node = self.get_node(id)?;
                    let element_data = node.element_data()?;
                    if element_data.name.local != local_name!("input") {
                        return None;
                    }
                    let id = element_data.id.as_ref()?;
                    if *id == *target_element_dom_id {
                        Some(node)
                    } else {
                        None
                    }
                })
                .next()
        } else {
            TreeTraverser::new_with_root(self, label_node_id)
                .filter_map(|child_id| {
                    let node = self.get_node(child_id)?;
                    let element_data = node.element_data()?;
                    if element_data.name.local == local_name!("input") {
                        Some(node)
                    } else {
                        None
                    }
                })
                .next()
        }
    }

    pub fn toggle_checkbox(el: &mut ElementData) -> bool {
        let Some(is_checked) = el.checkbox_input_checked_mut() else {
            return false;
        };
        *is_checked = !*is_checked;

        *is_checked
    }

    pub fn toggle_radio(&mut self, radio_set_name: String, target_radio_id: usize) {
        for i in 0..self.nodes.len() {
            let node = &mut self.nodes[i];
            if let Some(node_data) = node.data.element_mut() {
                if node_data.attr(local_name!("name")) == Some(&radio_set_name) {
                    let was_clicked = i == target_radio_id;
                    let Some(is_checked) = node_data.checkbox_input_checked_mut() else {
                        continue;
                    };
                    *is_checked = was_clicked;
                }
            }
        }
    }

    pub fn set_style_property(&mut self, node_id: usize, name: &str, value: &str) {
        let node = &mut self.nodes[node_id];
        let did_change = node.element_data_mut().unwrap().set_style_property(
            name,
            value,
            &self.lock,
            self.url.url_extra_data(),
        );
        if did_change {
            node.mark_style_attr_updated();
        }
    }

    pub fn remove_style_property(&mut self, node_id: usize, name: &str) {
        let node = &mut self.nodes[node_id];
        let did_change = node.element_data_mut().unwrap().remove_style_property(
            name,
            &self.lock,
            self.url.url_extra_data(),
        );
        if did_change {
            node.mark_style_attr_updated();
        }
    }

    /// Set the text content of a node
    /// For text nodes, replaces the text content
    /// For element nodes, removes all children and creates a single text node child
    pub fn set_text_content(&mut self, node_id: usize, value: String) {
        println!("Setting text content of node {} to '{}'", node_id, value);

        match self.nodes[node_id].data {
            NodeData::Text(ref mut text) => {
                if text.content == value {
                    return;
                }

                text.content.clear();
                text.content.push_str(&value);
                self.nodes[node_id].insert_damage(ALL_DAMAGE);
                self.nodes[node_id].mark_ancestors_dirty();

                if let Some(parent_id) = self.nodes[node_id].parent {
                    self.nodes[parent_id].insert_damage(ALL_DAMAGE);
                    self.maybe_record_node(parent_id);
                }
            }
            NodeData::Element(_) => {
                let old_text = self.nodes[node_id].text_content();
                if old_text == value {
                    return;
                }

                // `Element.textContent = value` replaces all children with one text node.
                let child_ids = self.nodes[node_id].children.clone();
                for child_id in child_ids {
                    self.remove_node(child_id);
                }

                if !value.is_empty() {
                    let text_node = self.create_text_node(&value);
                    self.append_children(node_id, &[text_node]);
                }

                self.nodes[node_id].insert_damage(ALL_DAMAGE);
                self.nodes[node_id].mark_ancestors_dirty();
                self.maybe_record_node(node_id);
            }
            _ => {}
        }
    }

    pub fn set_hover(&mut self, x: f32, y: f32) -> bool {
        let hit = self.hit(x, y);
        let hover_node_id = hit.map(|hit| hit.node_id);
        let new_is_text = hit.map(|hit| hit.is_text).unwrap_or(false);

        // Return early if the new node is the same as the already-hovered node
        if hover_node_id == self.hover_node_id {
            return false;
        }

        let old_node_path = self.maybe_node_layout_ancestors(self.hover_node_id);
        let new_node_path = self.maybe_node_layout_ancestors(hover_node_id);
        let same_count = old_node_path
            .iter()
            .zip(&new_node_path)
            .take_while(|(o, n)| o == n)
            .count();
        for &id in old_node_path.iter().skip(same_count) {
            self.snapshot_and(id, |node| node.unhover());
        }
        for &id in new_node_path.iter().skip(same_count) {
            self.snapshot_and(id, |node| node.hover());
        }

        self.hover_node_id = hover_node_id;
        self.hover_node_is_text = new_is_text;

        let cursor = self.get_cursor().unwrap_or_default();
        self.shell_provider.set_cursor(cursor);

        // Request redraw
        self.shell_provider.request_redraw();

        true
    }

    pub fn clear_hover(&mut self) -> bool {
        let Some(hover_node_id) = self.hover_node_id else {
            return false;
        };

        let old_node_path = self.maybe_node_layout_ancestors(Some(hover_node_id));
        for &id in old_node_path.iter() {
            self.snapshot_and(id, |node| node.unhover());
        }

        self.hover_node_id = None;
        self.hover_node_is_text = false;

        // Update the cursor
        let cursor = self.get_cursor().unwrap_or_default();
        self.shell_provider.set_cursor(cursor);

        // Request redraw
        self.shell_provider.request_redraw();

        true
    }

    pub fn get_hover_node_id(&self) -> Option<usize> {
        self.hover_node_id
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        let scale_changed = viewport.scale_f64() != self.viewport.scale_f64();
        self.viewport = viewport;
        self.set_stylist_device(device(&self.viewport, self.font_ctx.clone()));
        // todo clamp scroll

        if scale_changed {
            self.invalidate_inline_contexts();
            self.shell_provider.request_redraw();
        }
    }

    pub fn set_stylist_device(&mut self, device: Device) {
        let origins = {
            let lock = &self.lock;
            let guards = StylesheetGuards {
                author: &lock.read(),
                ua_or_user: &lock.read(),
            };
            self.stylist.set_device(device, &guards)
        };
        self.stylist.force_stylesheet_origins_dirty(origins);
    }

    pub fn stylist_device(&self) -> &Device {
        self.stylist.device()
    }

    pub fn get_cursor(&self) -> Option<CursorIcon> {
        let node = &self.nodes[self.get_hover_node_id()?];

        // TODO subdoc

        let style = node.primary_styles()?;
        let keyword = stylo_to_cursor_icon(style.clone_cursor().keyword);

        // Return cursor from style if it is non-auto
        if keyword != CursorIcon::Default {
            return Some(keyword);
        }

        if node.element_data().is_some_and(|el| el.text_input_data().is_some()) {
            return Some(CursorIcon::Text);
        }

        // Use "pointer" cursor if any ancestor is a link
        let mut maybe_node = Some(node);
        while let Some(node) = maybe_node {
            if node.is_link() {
                return Some(CursorIcon::Pointer);
            }

            maybe_node = node.layout_parent.get().map(|node_id| node.get_node(node_id));
        }

        // Return text cursor for text nodes
        if self.hover_node_is_text {
            return Some(CursorIcon::Text);
        }

        // Else fallback to default cursor
        Some(CursorIcon::Default)
    }

    pub fn scroll_node_by<F: FnMut(crate::events::DomEvent)>(
        &mut self,
        node_id: usize,
        x: f64,
        y: f64,
        dispatch_event: F,
    ) {
        self.scroll_node_by_has_changed(node_id, x, y, dispatch_event);
    }

    pub fn scroll_node_by_has_changed<F: FnMut(crate::events::DomEvent)>(
        &mut self,
        node_id: usize,
        x: f64,
        y: f64,
        mut dispatch_event: F,
    ) -> bool {
        let Some(node) = self.nodes.get_mut(node_id) else {
            return false;
        };

        let is_html_or_body = node.data.element().is_some_and(|e| {
            let tag = &e.name.local;
            tag == "html" || tag == "body"
        });

        let (can_x_scroll, can_y_scroll) = node
            .primary_styles()
            .map(|styles| {
                (
                    matches!(styles.clone_overflow_x(), Overflow::Scroll | Overflow::Auto),
                    matches!(styles.clone_overflow_y(), Overflow::Scroll | Overflow::Auto)
                        || (styles.clone_overflow_y() == Overflow::Visible && is_html_or_body),
                )
            })
            .unwrap_or((false, false));

        let initial = node.scroll_offset;
        let new_x = node.scroll_offset.x - x;
        let new_y = node.scroll_offset.y - y;

        let mut bubble_x = 0.0;
        let mut bubble_y = 0.0;

        let scroll_width = node.final_layout.scroll_width() as f64;
        let scroll_height = node.final_layout.scroll_height() as f64;

        // Todo subdoc

        if !can_x_scroll {
            bubble_x = x
        } else if new_x < 0.0 {
            bubble_x = -new_x;
            node.scroll_offset.x = 0.0;
        } else if new_x > scroll_width {
            bubble_x = scroll_width - new_x;
            node.scroll_offset.x = scroll_width;
        } else {
            node.scroll_offset.x = new_x;
        }

        if !can_y_scroll {
            bubble_y = y
        } else if new_y < 0.0 {
            bubble_y = -new_y;
            node.scroll_offset.y = 0.0;
        } else if new_y > scroll_height {
            bubble_y = scroll_height - new_y;
            node.scroll_offset.y = scroll_height;
        } else {
            node.scroll_offset.y = new_y;
        }

        let has_changed = node.scroll_offset != initial;

        if has_changed {
            let layout = node.final_layout;
            let event = BlitzScrollEvent {
                scroll_top: node.scroll_offset.y,
                scroll_left: node.scroll_offset.x,
                scroll_width: layout.scroll_width() as i32,
                scroll_height: layout.scroll_height() as i32,
                client_width: layout.size.width as i32,
                client_height: layout.size.height as i32,
            };

            dispatch_event(crate::events::DomEvent::new(node_id, DomEventData::Scroll(event)));
        }

        if bubble_x != 0.0 || bubble_y != 0.0 {
            if let Some(parent) = node.parent {
                return self.scroll_node_by_has_changed(parent, bubble_x, bubble_y, dispatch_event)
                    | has_changed;
            } else {
                return self.scroll_viewport_by_has_changed(bubble_x, bubble_y) | has_changed;
            }
        }

        has_changed
    }

    pub fn scroll_viewport_by(&mut self, x: f64, y: f64) {
        self.scroll_viewport_by_has_changed(x, y);
    }

    /// Scroll the viewport by the given values
    pub fn scroll_viewport_by_has_changed(&mut self, x: f64, y: f64) -> bool {
        let content_size = self.root_element().final_layout.size;
        let new_scroll = (self.viewport_scroll.x - x, self.viewport_scroll.y - y);
        let window_width = self.viewport.window_size.0 as f64 / self.viewport.scale() as f64;
        let window_height = self.viewport.window_size.1 as f64 / self.viewport.scale() as f64;

        let (initial_x, inital_y) = (self.viewport_scroll.x, self.viewport_scroll.y);
        self.viewport_scroll.x = f64::max(
            0.0,
            f64::min(new_scroll.0, content_size.width as f64 - window_width),
        );
        self.viewport_scroll.y = f64::max(
            0.0,
            f64::min(new_scroll.1, content_size.height as f64 - window_height),
        );

        let result = self.viewport_scroll.x != initial_x || self.viewport_scroll.y != inital_y;
        if result {
            let _ = self.shell_provider.sender.send(ShellProviderMessage::ViewportScroll((self.viewport_scroll.x, self.viewport_scroll.y)));
        }
        result
    }

    pub fn scroll_by(
        &mut self,
        anchor_node_id: Option<usize>,
        scroll_x: f64,
        scroll_y: f64,
        dispatch_event: &mut dyn FnMut(crate::events::DomEvent),
    ) -> bool {
        if let Some(anchor_node_id) = anchor_node_id {
            self.scroll_node_by_has_changed(anchor_node_id, scroll_x, scroll_y, dispatch_event)
        } else {
            self.scroll_viewport_by_has_changed(scroll_x, scroll_y)
        }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<HitResult> {
        if TDocument::as_node(&&self.nodes[0])
            .first_element_child()
            .is_none()
        {
            println!("Hit - NO DOM");
            return None;
        }

        self.root_element().hit(x, y)
    }

    pub fn try_root_element(&self) -> Option<&DomNode> {
        TDocument::as_node(&self.root_node()).first_element_child()
    }

    pub fn document_element_id(&self) -> Option<usize> {
        self.try_root_element().map(|node| node.id)
    }

    pub fn head_id(&self) -> Option<usize> {
        let html_id = self.document_element_id()?;
        self.nodes[html_id].children.iter().copied().find(|child_id| {
            self.nodes
                .get(*child_id)
                .and_then(|node| node.element_data())
                .is_some_and(|el| el.name.local == local_name!("head"))
        })
    }

    pub fn body_id(&self) -> Option<usize> {
        let html_id = self.document_element_id()?;
        self.nodes[html_id].children.iter().copied().find(|child_id| {
            self.nodes
                .get(*child_id)
                .and_then(|node| node.element_data())
                .is_some_and(|el| {
                    el.name.local == local_name!("body") || el.name.local == local_name!("frameset")
                })
        })
    }

    pub fn set_document_body(&mut self, new_body_id: usize) -> Result<(), &'static str> {
        let Some(new_body_node) = self.nodes.get(new_body_id) else {
            return Err("Body node not found");
        };
        let Some(new_body_el) = new_body_node.element_data() else {
            return Err("document.body must be an element");
        };
        let is_body_like = new_body_el.name.local == local_name!("body")
            || new_body_el.name.local == local_name!("frameset");
        if !is_body_like {
            return Err("document.body must be a <body> or <frameset> element");
        }

        let Some(html_id) = self.document_element_id() else {
            return Err("Document element not found");
        };

        if self.body_id() == Some(new_body_id) {
            return Ok(());
        }

        if let Some(existing_body_id) = self.body_id() {
            self.replace_node_with(existing_body_id, &[new_body_id]);
        } else {
            self.append_children(html_id, &[new_body_id]);
        }

        Ok(())
    }

    pub fn get_focused_node_id(&self) -> Option<usize> {
        self.focus_node_id
            .or(self.try_root_element().map(|el| el.id))
    }

    pub fn focus_next_node(&mut self) -> Option<usize> {
        let focussed_node_id = self.get_focused_node_id()?;
        let id = self.next_node(&self.nodes[focussed_node_id], |node| node.is_focusable())?;
        self.set_focus_to(id);
        Some(id)
    }

    /// Clear the focussed node
    pub fn clear_focus(&mut self) {
        if let Some(id) = self.focus_node_id {
            let shell_provider = self.shell_provider.clone();
            self.snapshot_and(id, |node| node.blur(shell_provider));
            self.focus_node_id = None;
        }
    }

    pub fn set_mousedown_node_id(&mut self, node_id: Option<usize>) {
        self.mousedown_node_id = node_id;
    }
    pub fn set_focus_to(&mut self, focus_node_id: usize) -> bool {
        if Some(focus_node_id) == self.focus_node_id {
            return false;
        }

        let shell_provider = self.shell_provider.clone();

        // Remove focus from the old node
        if let Some(id) = self.focus_node_id {
            self.snapshot_and(id, |node| node.blur(shell_provider.clone()));
        }

        // Focus the new node
        self.snapshot_and(focus_node_id, |node| node.focus(shell_provider));

        self.focus_node_id = Some(focus_node_id);

        true
    }

    pub fn active_node(&mut self) -> bool {
        let Some(hover_node_id) = self.get_hover_node_id() else {
            return false;
        };

        if let Some(active_node_id) = self.active_node_id {
            if active_node_id == hover_node_id {
                return true;
            }
            self.unactive_node();
        }

        let active_node_id = Some(hover_node_id);

        let node_path = self.maybe_node_layout_ancestors(active_node_id);
        for &id in node_path.iter() {
            self.snapshot_and(id, |node| node.active());
        }

        self.active_node_id = active_node_id;

        true
    }

    pub fn unactive_node(&mut self) -> bool {
        let Some(active_node_id) = self.active_node_id.take() else {
            return false;
        };

        let node_path = self.maybe_node_layout_ancestors(Some(active_node_id));
        for &id in node_path.iter() {
            self.snapshot_and(id, |node| node.unactive());
        }

        true
    }

    pub fn find_text_position(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let hit = self.hit(x, y)?;
        let hit_node = self.get_node(hit.node_id)?;
        let inline_root = hit_node.inline_root_ancestor()?;
        let byte_offset = inline_root.text_offset_at_point(hit.x, hit.y)?;
        Some((inline_root.id, byte_offset))
    }

    pub fn set_text_selection(
        &mut self,
        anchor_node: usize,
        anchor_offset: usize,
        focus_node: usize,
        focus_offset: usize,
    ) {
        self.text_selection =
            TextSelection::new(anchor_node, anchor_offset, focus_node, focus_offset);

        // For anonymous blocks, switch to storing parent+sibling_index (stable reference)
        if let (Some(parent), Some(idx)) = self.anonymous_block_location(anchor_node) {
            self.text_selection
                .anchor
                .set_anonymous(parent, idx, anchor_offset);
        }
        if let (Some(parent), Some(idx)) = self.anonymous_block_location(focus_node) {
            self.text_selection
                .focus
                .set_anonymous(parent, idx, focus_offset);
        }
    }

    fn anonymous_block_location(&self, node_id: usize) -> (Option<usize>, Option<usize>) {
        let Some(node) = self.get_node(node_id) else {
            return (None, None);
        };

        if !node.is_anonymous() {
            return (None, None);
        }

        let Some(parent_id) = node.parent else {
            return (None, None);
        };

        let Some(parent) = self.get_node(parent_id) else {
            return (Some(parent_id), None);
        };

        let layout_children = parent.layout_children.borrow();
        let Some(children) = layout_children.as_ref() else {
            return (Some(parent_id), None);
        };

        // Find the index of this anonymous block among siblings
        let mut anon_index = 0;
        for &child_id in children.iter() {
            if child_id == node_id {
                return (Some(parent_id), Some(anon_index));
            }
            if self.get_node(child_id).is_some_and(|n| n.is_anonymous()) {
                anon_index += 1;
            }
        }

        (Some(parent_id), None)
    }

    pub fn clear_text_selection(&mut self) {
        self.text_selection.clear();
    }

    /// Update the selection focus point (used during mouse drag to extend selection).
    pub fn update_selection_focus(&mut self, focus_node: usize, focus_offset: usize) {
        // For anonymous blocks, store parent+sibling_index; otherwise store node directly
        if let (Some(parent), Some(idx)) = self.anonymous_block_location(focus_node) {
            self.text_selection
                .focus
                .set_anonymous(parent, idx, focus_offset);
        } else {
            self.text_selection.set_focus(focus_node, focus_offset);
        }
    }

    /// Extend text selection to the given point. Returns true if selection was updated.
    /// This is a convenience method that combines find_text_position and update_selection_focus.
    pub fn extend_text_selection_to_point(&mut self, x: f32, y: f32) -> bool {
        if !self.text_selection.anchor.is_some() {
            return false;
        }

        if let Some((node, offset)) = self.find_text_position(x, y) {
            self.update_selection_focus(node, offset);
            self.shell_provider.request_redraw();
            true
        } else {
            false
        }
    }

    /// Find the Nth anonymous block under a parent.
    fn find_anonymous_block_by_index(
        &self,
        parent_id: usize,
        target_index: usize,
    ) -> Option<usize> {
        let parent = self.get_node(parent_id)?;
        let layout_children = parent.layout_children.borrow();
        let children = layout_children.as_ref()?;

        children
            .iter()
            .filter(|&&child_id| self.get_node(child_id).is_some_and(|n| n.is_anonymous()))
            .nth(target_index)
            .copied()
    }

    /// Check if there is an active (non-empty) text selection
    pub fn has_text_selection(&self) -> bool {
        self.text_selection.is_active()
    }

    /// Get the selected text content, supporting selection across multiple inline roots.
    pub fn get_selected_text(&self) -> Option<String> {
        let ranges = self.get_text_selection_ranges();
        if ranges.is_empty() {
            return None;
        }

        let mut result = String::new();
        for (node_id, start, end) in &ranges {
            let node = self.get_node(*node_id)?;
            let element_data = node.element_data()?;
            let inline_layout = element_data.inline_layout_data.as_ref()?;

            if *end > inline_layout.text.len() {
                continue;
            }

            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&inline_layout.text[*start..*end]);
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Get all selection ranges as Vec<(node_id, start_offset, end_offset)>.
    /// Returns empty vec if no selection.
    pub fn get_text_selection_ranges(&self) -> Vec<(usize, usize, usize)> {
        let lookup = |parent_id, idx| self.find_anonymous_block_by_index(parent_id, idx);

        let anchor_node = match self.text_selection.anchor.resolve_node_id(lookup) {
            Some(id) => id,
            None => return Vec::new(),
        };
        let focus_node = match self.text_selection.focus.resolve_node_id(lookup) {
            Some(id) => id,
            None => return Vec::new(),
        };

        // Single node selection
        if anchor_node == focus_node {
            let start = self
                .text_selection
                .anchor
                .offset
                .min(self.text_selection.focus.offset);
            let end = self
                .text_selection
                .anchor
                .offset
                .max(self.text_selection.focus.offset);

            if start == end {
                return Vec::new();
            }
            return vec![(anchor_node, start, end)];
        }

        // Multi-node selection: collect all inline roots between anchor and focus
        let inline_roots = self.collect_inline_roots_in_range(anchor_node, focus_node);
        if inline_roots.is_empty() {
            return Vec::new();
        }

        // Determine document order using the collected inline_roots order
        // (inline_roots is already in document order from first to last)
        let first_in_roots = inline_roots[0];

        let (first_node, first_offset, last_node, last_offset) =
            if first_in_roots == anchor_node || (first_in_roots != focus_node) {
                // anchor is first (or neither endpoint is in roots, which shouldn't happen)
                (
                    anchor_node,
                    self.text_selection.anchor.offset,
                    focus_node,
                    self.text_selection.focus.offset,
                )
            } else {
                // focus is first
                (
                    focus_node,
                    self.text_selection.focus.offset,
                    anchor_node,
                    self.text_selection.anchor.offset,
                )
            };

        let mut ranges = Vec::with_capacity(inline_roots.len());

        for &node_id in &inline_roots {
            let Some(node) = self.get_node(node_id) else {
                continue;
            };
            let Some(element_data) = node.element_data() else {
                continue;
            };
            let Some(inline_layout) = element_data.inline_layout_data.as_ref() else {
                continue;
            };

            let text_len = inline_layout.text.len();

            if node_id == first_node && node_id == last_node {
                let start = first_offset.min(last_offset);
                let end = first_offset.max(last_offset);
                if start < end && end <= text_len {
                    ranges.push((node_id, start, end));
                }
            } else if node_id == first_node {
                if first_offset < text_len {
                    ranges.push((node_id, first_offset, text_len));
                }
            } else if node_id == last_node {
                if last_offset > 0 && last_offset <= text_len {
                    ranges.push((node_id, 0, last_offset));
                }
            } else if text_len > 0 {
                ranges.push((node_id, 0, text_len));
            }
        }

        ranges
    }

    pub fn node_has_parent(&self, node_id: usize) -> bool {
        self.nodes[node_id].parent.is_some()
    }

    pub fn previous_sibling_id(&self, node_id: usize) -> Option<usize> {
        self.nodes[node_id].backward(1).map(|node| node.id)
    }

    pub fn next_sibling_id(&self, node_id: usize) -> Option<usize> {
        self.nodes[node_id].forward(1).map(|node| node.id)
    }

    pub fn parent_id(&self, node_id: usize) -> Option<usize> {
        self.nodes[node_id].parent
    }

    pub fn last_child_id(&self, node_id: usize) -> Option<usize> {
        self.nodes[node_id].children.last().copied()
    }

    pub fn child_ids(&self, node_id: usize) -> Vec<usize> {
        self.nodes[node_id].children.clone()
    }

    pub(crate) fn append_children(&mut self, parent_id: usize, child_ids: &[usize]) {
        self.add_children_to_parent(parent_id, child_ids, &|parent, child_ids| {
            parent.children.extend_from_slice(child_ids);
        })
    }

    pub fn insert_nodes_before(&mut self, anchor_node_id: usize, new_node_ids: &[usize]) {
        let parent_id = self.nodes[anchor_node_id].parent.unwrap();
        self.add_children_to_parent(parent_id, new_node_ids, &|parent, child_ids| {
            let node_child_idx = parent.index_of_child(anchor_node_id).unwrap();
            parent
                .children
                .splice(node_child_idx..node_child_idx, child_ids.iter().copied());
        });
    }

    fn add_children_to_parent(
        &mut self,
        parent_id: usize,
        child_ids: &[usize],
        insert_children_fn: &dyn Fn(&mut DomNode, &[usize]),
    ) {
        let new_parent = &mut self.nodes[parent_id];
        new_parent.insert_damage(ALL_DAMAGE);
        let new_parent_is_in_doc = new_parent.flags.is_in_document();

        // TODO: make this fine grained / conditional based on ElementSelectorFlags
        if new_parent_is_in_doc {
            if let Some(data) = &mut *new_parent.stylo_data.borrow_mut() {
                data.hint |= RestyleHint::restyle_subtree();
            }
            // Mark ancestors dirty so the style traversal visits this subtree.
            new_parent.mark_ancestors_dirty();
        }

        insert_children_fn(new_parent, child_ids);

        for child_id in child_ids.iter().copied() {
            let child = &mut self.nodes[child_id];
            let old_parent_id = child.parent.replace(parent_id);

            let child_was_in_doc = child.flags.is_in_document();
            if new_parent_is_in_doc != child_was_in_doc {
                self.process_added_subtree(child_id);
            }

            if let Some(old_parent_id) = old_parent_id {
                let old_parent = &mut self.nodes[old_parent_id];
                old_parent.insert_damage(ALL_DAMAGE);

                // TODO: make this fine grained / conditional based on ElementSelectorFlags
                if child_was_in_doc {
                    if let Some(data) = &mut *old_parent.stylo_data.borrow_mut() {
                        data.hint |= RestyleHint::restyle_subtree();
                    }
                    // Mark ancestors dirty so the style traversal visits this subtree.
                    old_parent.mark_ancestors_dirty();
                }

                old_parent.children.retain(|id| *id != child_id);
                self.maybe_record_node(old_parent_id);
            }
        }

        self.maybe_record_node(parent_id);
    }

    // Tree mutation methods (that defer to other methods)
    pub fn insert_nodes_after(&mut self, anchor_node_id: usize, new_node_ids: &[usize]) {
        match self.next_sibling_id(anchor_node_id) {
            Some(id) => self.insert_nodes_before(id, new_node_ids),
            None => {
                let parent_id = self.parent_id(anchor_node_id).unwrap();
                self.append_children(parent_id, new_node_ids)
            }
        }
    }

    pub fn reparent_children(&mut self, old_parent_id: usize, new_parent_id: usize) {
        let child_ids = std::mem::take(&mut self.nodes[old_parent_id].children);
        self.maybe_record_node(old_parent_id);
        self.append_children(new_parent_id, &child_ids);
    }

    pub fn replace_node_with(&mut self, anchor_node_id: usize, new_node_ids: &[usize]) {
        self.insert_nodes_before(anchor_node_id, new_node_ids);
        self.remove_node(anchor_node_id);
    }

    pub fn remove_node(&mut self, node_id: usize) {
        let node = &mut self.nodes[node_id];

        // Update child_idx values
        if let Some(parent_id) = node.parent.take() {
            let parent = &mut self.nodes[parent_id];
            parent.insert_damage(ALL_DAMAGE);
            // Mark ancestors dirty so the style traversal visits this subtree.
            parent.mark_ancestors_dirty();
            parent.children.retain(|id| *id != node_id);
            self.maybe_record_node(parent_id);
        }

        self.process_removed_subtree(node_id);
    }

    fn maybe_record_node(&mut self, node_id: impl Into<Option<usize>>) {
        let Some(node_id) = node_id.into() else {
            return;
        };

        let Some(tag_name) = self.nodes[node_id]
            .data
            .element()
            .map(|elem| &elem.name.local)
        else {
            return;
        };

        match tag_name.as_ref() {
            "title" => {
                let title = self.nodes[node_id].text_content();
                self.shell_provider.set_window_title(title);
            },
            "style" => {
                self.process_style_element(node_id);
            }
            _ => {}
        }
    }

    fn process_added_subtree(&mut self, node_id: usize) {
        self.iter_subtree_mut(node_id, |node_id, dom| {
            let node = &mut dom.nodes[node_id];
            node.flags.set(DomNodeFlags::IS_IN_DOCUMENT, true);
            node.insert_damage(ALL_DAMAGE);

            // If the node has an "id" attribute, store it in the ID map.
            if let Some(id_attr) = node.attr(local_name!("id")) {
                dom.nodes_to_id.insert(id_attr.to_string(), node_id);
            }

            let NodeData::Element(ref mut element) = node.data else {
                return;
            };

            // TODO Custom post-processing by element tag name
            let tag = element.name.local.as_ref();
            match tag {
                "title" => dom.shell_provider.set_window_title(dom.nodes[node_id].text_content()),
                "link" => dom.load_linked_stylesheet(node_id),
                "img" => dom.load_image(node_id),
                "canvas" => dom.load_custom_paint_src(node_id),
                "style" => dom.process_style_element(node_id),
                "button" | "fieldset" | "input" | "select" | "textarea" | "object" | "output" => {
                    dom.process_button_input(node_id);
                    dom.reset_form_owner(node_id);
                }
                _ => {}
            };
            let node = &dom.nodes[node_id];

            if node.is_focusable() {
                if let NodeData::Element(ref element) = node.data {
                    if let Some(value) = element.attr(local_name!("autofocus")) {
                        if value == "true" {
                            dom.autofocus(node_id);
                        }
                    }
                }
            }
        });
    }

    fn autofocus(&mut self, node_id: usize) {
        if self.get_node(node_id).is_some() {
            self.set_focus_to(node_id);
        }
    }

    pub(crate) fn drop_node_ignoring_parent(&mut self, node_id: usize) -> Option<DomNode> {
        let mut node = self.nodes.try_remove(node_id);
        if let Some(node) = &mut node {
            if let Some(before) = node.before {
                self.drop_node_ignoring_parent(before);
            }
            if let Some(after) = node.after {
                self.drop_node_ignoring_parent(after);
            }

            for &child in &node.children {
                self.drop_node_ignoring_parent(child);
            }
        }
        node
    }

    pub fn remove_and_drop_all_children(&mut self, node_id: usize) {
        let parent = &mut self.nodes[node_id];
        let parent_is_in_doc = parent.flags.is_in_document();

        // TODO: make this fine grained / conditional based on ElementSelectorFlags
        if parent_is_in_doc {
            if let Some(data) = &mut *parent.stylo_data.borrow_mut() {
                data.hint |= RestyleHint::restyle_subtree();
            }
            // Mark ancestors dirty so the style traversal visits this subtree.
            parent.mark_ancestors_dirty();
        }

        let children = mem::take(&mut parent.children);
        for child_id in children {
            self.process_removed_subtree(child_id);
            let _ = self.drop_node_ignoring_parent(child_id);
        }
        self.maybe_record_node(node_id);
    }

    pub fn set_inner_html(&mut self, node_id: usize, html: &str) {
        self.remove_and_drop_all_children(node_id);
        self.html_provider.clone().parse_inner_html(self, node_id, html);
    }

    fn process_removed_subtree(&mut self, node_id: usize) {
        let mut compute_canvas: bool = false;
        let mut stylesheets_to_unload = Vec::new();
        let mut removed_form = false;
        self.iter_subtree_mut(node_id, |node_id, doc| {
            let node = &mut doc.nodes[node_id];
            node.flags.set(DomNodeFlags::IS_IN_DOCUMENT, false);

            // Remove any form-owner mapping for removed nodes.
            doc.controls_to_form.remove(&node_id);

            // Clear hover state if this node was being hovered.
            // This prevents stale hover_node_id references.
            if doc.hover_node_id == Some(node_id) {
                doc.hover_node_id = None;
                doc.hover_node_is_text = false;
            }

            // Clear active state if this node was active
            // This prevents stale active_node_id references.
            if doc.active_node_id == Some(node_id) {
                doc.active_node_id = None;
            }

            // Remove any snapshot for this node to prevent stale snapshot references
            // during style invalidation.
            if node.has_snapshot {
                let opaque_id = style::dom::TNode::opaque(&&*node);
                doc.snapshots.remove(&opaque_id);
                node.has_snapshot = false;
            }

            // If the node has an "id" attribute remove it from the ID map.
            if let Some(id_attr) = node.attr(local_name!("id")) {
                doc.nodes_to_id.remove(id_attr);
            }

            let NodeData::Element(ref mut element) = node.data else {
                return;
            };

            if element.name.local == local_name!("form") {
                removed_form = true;
            }

            match &element.special_data {
                SpecialElementData::SubDom(_) => {}
                SpecialElementData::Stylesheet(_) => {
                    stylesheets_to_unload.push(node_id);
                }
                SpecialElementData::Image(_) => {}
                SpecialElementData::Canvas(_) => {
                    compute_canvas = true;
                }
                SpecialElementData::TableRoot(_) => {}
                SpecialElementData::TextInput(_) => {}
                SpecialElementData::CheckboxInput(_) => {}
                SpecialElementData::FileInput(_) => {}
                SpecialElementData::None => {}
            }
        });

        if removed_form {
            self.reset_all_form_owners();
        }

        if compute_canvas {
            self.has_canvas = self.compute_has_canvas();
        }
        for node_id in stylesheets_to_unload {
            self.unload_stylesheet(node_id);
        }
    }

    fn process_button_input(&mut self, target_id: usize) {
        let node = &self.nodes[target_id];
        let Some(data) = node.element_data() else {
            return;
        };

        let tagname = data.name.local.as_ref();
        let type_attr = data.attr(local_name!("type"));
        let value = data.attr(local_name!("value"));

        // Add content of "value" attribute as a text node child if:
        //   - Tag name is
        if let ("input", Some("button" | "submit" | "reset"), Some(value)) =
            (tagname, type_attr, value)
        {
            let value = value.to_string();
            let id = self.create_text_node(&value);
            self.append_children(target_id, &[id]);
            return;
        }
        if let ("input", Some("file")) = (tagname, type_attr) {
            let button_id = self.create_element(
                qual_name!("button", html),
                AttributeMap::new(vec![
                    Attribute {
                        name: qual_name!("type", html),
                        value: "button".to_string(),
                    },
                    Attribute {
                        name: qual_name!("tabindex", html),
                        value: "-1".to_string(),
                    },
                ]),
            );
            let label_id = self.create_element(qual_name!("label", html), AttributeMap::empty());
            let text_id = self.create_text_node("No File Selected");
            let button_text_id = self.create_text_node("Browse");
            self.append_children(target_id, &[button_id, label_id]);
            self.append_children(label_id, &[text_id]);
            self.append_children(button_id, &[button_text_id]);
        }
    }

    pub fn set_sub_dom(&mut self, node_id: usize, sub_dom: Box<dyn AbstractDom>) {
        self.nodes[node_id].element_data_mut().unwrap().set_sub_dom(sub_dom);
        self.sub_dom_nodes.insert(node_id);
    }

    pub fn remove_sub_dom(&mut self, node_id: usize) {
        self.nodes[node_id].element_data_mut().unwrap().remove_sub_dom();
        self.sub_dom_nodes.remove(&node_id);
    }

    pub fn append_text_to_node(&mut self, node_id: usize, text: &str) -> Result<(), AppendTextErr> {
        let node = &mut self.nodes[node_id];
        node.insert_damage(ALL_DAMAGE);
        node.mark_ancestors_dirty();
        match node.text_data_mut() {
            Some(data) => {
                data.content += text;
                Ok(())
            }
            None => Err(AppendTextErr::NotTextNode),
        }
    }

    pub fn tree(&self) -> &Slab<DomNode> {
        &self.nodes
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn get_node(&self, node_id: usize) -> Option<&DomNode> {
        self.nodes.get(node_id)
    }

    pub fn get_node_mut(&mut self, node_id: usize) -> Option<&mut DomNode> {
        self.nodes.get_mut(node_id)
    }

    pub(crate) fn root_node(&self) -> &DomNode {
        &self.nodes[0]
    }

    pub(crate) fn root_node_mut(&mut self) -> &mut DomNode {
        &mut self.nodes[0]
    }

    pub(crate) fn root_element(&self) -> &DomNode {
        TDocument::as_node(&self.root_node())
            .first_element_child()
            .unwrap()
            .as_element()
            .unwrap()
    }
}

#[derive(Debug, Clone)]
pub enum AppendTextErr {
    /// The node is not a text node
    NotTextNode,
}

