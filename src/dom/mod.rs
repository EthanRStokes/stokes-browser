// DOM module for parsing and representing HTML content
mod parser;
pub(crate) mod node;
mod events;
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

pub use self::events::{EventDispatcher, EventType};
pub use self::node::{AttributeMap, DomNode, ElementData, ImageData, ImageLoadingState, NodeData};
pub use self::parser::HtmlParser;
use crate::css::stylo::RecalcStyle;
use crate::dom::config::DomConfig;
use crate::dom::damage::{ALL_DAMAGE, CONSTRUCT_BOX, CONSTRUCT_DESCENDENT, CONSTRUCT_FC};
use crate::dom::layout::collect_layout_children;
use crate::dom::node::{DomNodeFlags, SpecialElementData, TextData};
use crate::dom::url::DocUrl;
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
use std::collections::{BTreeMap, Bound, HashMap};
use std::fmt::{Debug, Formatter};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use cursor_icon::CursorIcon;
use skia_safe::wrapper::NativeTransmutableWrapper;
use style::animation::DocumentAnimationSet;
use style::context::{RegisteredSpeculativePainter, RegisteredSpeculativePainters, SharedStyleContext};
use style::data::ElementStyles;
use style::dom::{TDocument, TNode};
use style::font_metrics::FontMetrics;
use style::global_style_data::GLOBAL_STYLE_DATA;
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
use style::values::computed::{Au, CSSPixelLength, Length};
use stylo_atoms::Atom;
use taffy::Point;
use crate::dom::stylo_to_cursor::stylo_to_cursor_icon;
use crate::engine::net_provider::StokesNetProvider;

const ZERO: Point<f64> = Point { x: 0.0, y: 0.0 };

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

    // todo text selection

    pub(crate) has_active_animations: bool,
    pub(crate) has_canvas: bool,

    pub(crate) nodes_to_id: HashMap<String, usize>,
    pub(crate) nodes_to_stylesheet: BTreeMap<usize, DocumentStyleSheet>,
    pub(crate) stylesheets: HashMap<String, DocumentStyleSheet>,

    pub(crate) image_cache: HashMap<String, ImageData>,
    pub(crate) pending_images: HashMap<String, Vec<(usize, ImageType)>>,

    pub net_provider: Arc<dyn NetProvider>,
    pub shell_provider: Arc<dyn ShellProvider>,
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
        PrefersColorScheme::Light // TODO detect color scheme preference
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

        let viewport = config.viewport.unwrap_or_default();
        let font_ctx = config.font_ctx.unwrap_or_else(|| {
            let mut font_ctx = FontContext::default();
            font_ctx.collection.register_fonts(Blob::new(Arc::new(BULLET_FONT) as _), None);
            font_ctx
        });
        let font_ctx = Arc::new(Mutex::new(font_ctx));
        let device = device(&viewport, font_ctx.clone());

        stylo_config::set_bool("layout.flexbox.enabled", true);
        stylo_config::set_bool("layout.grid.enabled", true);
        stylo_config::set_bool("layout.legacy_layout", true);
        stylo_config::set_bool("layout.unimplemented", true);
        stylo_config::set_bool("layout.columns.enabled", true);

        let base_url = config.base_url.and_then(|url| DocUrl::from_str(&url).ok()).unwrap_or_default();
        let net_provider = config.net_provider.unwrap_or_else(|| Arc::new(DummyNetProvider));
        let shell_provider = config.shell_provider.unwrap_or_else(|| Arc::new(DummyShellProvider));

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
            has_active_animations: false,
            has_canvas: false,
            nodes_to_id: Default::default(),
            nodes_to_stylesheet: Default::default(),
            stylesheets: Default::default(),
            image_cache: HashMap::new(),
            pending_images: HashMap::new(),
            net_provider,
            shell_provider,
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
        let data = NodeData::Element(ElementData::new(tag_name, attributes));
        self.create_node(data)
    }

    pub(crate) fn create_comment_node(&mut self) -> usize {
        self.create_node(NodeData::Comment)
    }

    pub(crate) fn create_text_node(&mut self, text: &str) -> usize {
        let content = text.to_string();
        let data = NodeData::Text(TextData::new(content));
        self.create_node(data)
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

    /// Parse HTML into a DOM
    pub fn parse_html(url: &str, html: &str, viewport: Viewport, shell_provider: Arc<dyn ShellProvider>) -> Self {
        let parser = HtmlParser::new();
        parser.parse(html, DomConfig {
            viewport: Some(viewport),
            base_url: Some(url.to_string()),
            net_provider: Some(Arc::new(StokesNetProvider::new())),
            shell_provider: Some(shell_provider),
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

    pub fn flush_styles(&mut self) {
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
                current_time_for_animations: 0.0, // TODO animations
                traversal_flags: TraversalFlags::empty(),
                snapshot_map: &self.snapshots,
                animations: Default::default(),
                registered_speculative_painters: &Painters,
            };

            let root = self.root_element();
            let token = RecalcStyle::pre_traverse(root, &context);

            if token.should_traverse() {
                let traverser = RecalcStyle::new(context);
                style::driver::traverse_dom(&traverser, token, None);
            }

            for opaque in self.snapshots.keys() {
                let id = opaque.id();
                if let Some(node) = self.nodes.get_mut(id) {
                    node.has_snapshot = false;
                }
            }
            self.snapshots.clear();

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

    /// Set the text content of a node
    /// For text nodes, replaces the text content
    /// For element nodes, removes all children and creates a single text node child
    pub fn set_text_content(&mut self, node_id: usize, value: String) {
        println!("Setting text content of node {} to '{}'", self.id, value);

        let text = match self.nodes[node_id].data {
            NodeData::Text(ref mut text) => text,
            NodeData::Element(ref element) => {
                // find child text node
                if let Some(child_id) = self.nodes[node_id].children.clone().iter().find(|child_id| {
                    matches!(self.tree()[**child_id].data, NodeData::Text { .. })
                }) {
                    self.nodes[*child_id].data.element_mut().unwrap().inline_layout_data = None;
                    if let NodeData::Text(ref mut text) = self.nodes[*child_id].data {
                        text
                    } else {
                        unreachable!()
                    }
                } else {
                    // no existing text node, create one
                    let text_data = TextData::new(value.to_string());
                    let text_node = self.create_node(NodeData::Text(text_data));
                    self.nodes[node_id].add_child(text_node);
                    if let NodeData::Text(ref mut text) = self.nodes[text_node].data {
                        text
                    } else {
                        unreachable!()
                    }
                }
            }
            _ => return,
        };

        let changed = text.content != value;
        if changed {
            println!("changing text content to {value}");
            text.content.clear();
            text.content.push_str(&value);
            self.nodes[node_id].insert_damage(ALL_DAMAGE);
            // todo mark ancestors dirty

            let parent_id = self.nodes[node_id].parent;
            if let Some(parent_id) = parent_id {
                self.nodes[parent_id].insert_damage(ALL_DAMAGE);
            }

            // todo record ig
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

    pub fn get_hover_node_id(&self) -> Option<usize> {
        self.hover_node_id
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

        // todo text cursor for text inputs

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
        // todo impl
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
                // todo buttons
                _ => {}
            };
        });

        // todo idk
    }

    fn process_removed_subtree(&mut self, node_id: usize) {
        self.iter_subtree_mut(node_id, |node_id, doc| {
            let node = &mut doc.nodes[node_id];
            node.flags.set(DomNodeFlags::IS_IN_DOCUMENT, false);

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

            match &element.special_data {
                //todo sub document
                SpecialElementData::Stylesheet(_) => {} // todo
                SpecialElementData::Image(_) => {}
                SpecialElementData::Canvas(_) => {} // todo animation
                SpecialElementData::TableRoot(_) => {}
                SpecialElementData::TextInput => {}
                SpecialElementData::CheckboxInput(_) => {}
                SpecialElementData::FileInput(_) => {}
                SpecialElementData::None => {}
            }
        });

        // todo idk
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