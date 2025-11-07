// DOM module for parsing and representing HTML content
mod parser;
pub(crate) mod node;
mod events;
mod config;
pub(crate) mod damage;
mod url;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Debug, Formatter};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use blitz_traits::net::{DummyNetProvider, NetProvider};
use blitz_traits::shell::Viewport;
use euclid::Size2D;
use parley::{FontContext, LayoutContext};
use parley::fontique::Blob;
use selectors::Element;
use selectors::matching::QuirksMode;
use slab::Slab;
use style::animation::DocumentAnimationSet;
use style::data::ElementStyles;
use style::dom::{TDocument, TNode};
use style::font_metrics::FontMetrics;
use style::media_queries::{Device, MediaList, MediaType};
use style::properties::ComputedValues;
use style::properties::style_structs::Font;
use style::queries::values::PrefersColorScheme;
use style::selector_parser::SnapshotMap;
use style::servo::media_queries::FontMetricsProvider;
use style::shared_lock::SharedRwLock;
use style::stylesheets::{AllowImportRules, DocumentStyleSheet, Origin, Stylesheet};
use style::stylist::Stylist;
use style::values::computed::{Au, CSSPixelLength, Length};
use style::values::computed::font::{GenericFontFamily, QueryFontMetricsFlags};
use taffy::Point;
use crate::dom::config::DomConfig;
use crate::dom::node::DomNodeFlags;
use crate::dom::url::DocUrl;
use crate::networking::{Resource, StylesheetLoader};
use crate::ui::TextBrush;
pub use self::events::{EventDispatcher, EventType};
pub use self::node::{AttributeMap, DomNode, ElementData, ImageData, ImageLoadingState, NodeData};
pub use self::parser::HtmlParser;

const ZERO: Point<f64> = Point { x: 0.0, y: 0.0 };

/// Represents a DOM tree
pub struct Dom {
    /// ID of the DOM
    id: usize,

    pub(crate) url: DocUrl,
    // Viewport information (dimensions, HiDPI scale, zoom)
    pub(crate) viewport: Viewport,
    // Scroll position in the viewport
    pub(crate) viewport_scroll: Point<f64>,

    pub(crate) nodes: Box<Slab<DomNode>>,

    // Stylo
    pub(crate) stylist: Stylist,
    pub(crate) animations: DocumentAnimationSet,
    pub(crate) lock: SharedRwLock,
    // Stylo invalidation map
    pub(crate) snapshots: SnapshotMap,

    pub(crate) font_ctx: Arc<Mutex<FontContext>>,
    pub(crate) layout_ctx: Arc<Mutex<LayoutContext<TextBrush>>>,

    pub(crate) has_active_animations: bool,
    pub(crate) has_canvas: bool,

    pub(crate) nodes_to_id: HashMap<String, usize>,
    pub(crate) nodes_to_stylesheet: BTreeMap<usize, DocumentStyleSheet>,
    pub(crate) stylesheets: HashMap<String, DocumentStyleSheet>,

    pub net_provider: Arc<dyn NetProvider<Resource>>,
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
    fn query_font_metrics(&self, vertical: bool, font: &Font, base_size: CSSPixelLength, flags: QueryFontMetricsFlags) -> FontMetrics {
        todo!()
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

        let mut dom = Self {
            id,
            url: base_url,
            viewport,
            viewport_scroll: ZERO,
            nodes: Box::new(Slab::new()),
            //root: Rc::new(RefCell::from(DomNode::new(NodeData::Document, None))),
            stylist: Stylist::new(device, QuirksMode::NoQuirks),
            animations: Default::default(),
            lock: SharedRwLock::new(),
            snapshots: SnapshotMap::new(),
            font_ctx,
            layout_ctx: Arc::new(Mutex::new(LayoutContext::new())),
            has_active_animations: false,
            has_canvas: false,
            nodes_to_id: Default::default(),
            nodes_to_stylesheet: Default::default(),
            stylesheets: Default::default(),
            net_provider,
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
    pub fn parse_html(html: &str, viewport: Viewport) -> Self {
        let parser = HtmlParser::new();
        parser.parse(html, DomConfig {
            viewport: Some(viewport),
            net_provider: Some(Arc::new(DummyNetProvider)),
            ..Default::default()
        })
    }

    pub fn add_stylesheet(&mut self, css: &str) {
        let sheet = self.make_stylesheet(css, Origin::UserAgent);
        self.stylesheets.insert(css.to_string(), sheet.clone());
        self.stylist.append_stylesheet(sheet, &self.lock.read());
    }

    pub fn make_stylesheet(&self, css: impl AsRef<str>, origin: Origin) -> DocumentStyleSheet {
        let data = Stylesheet::from_str(
            css.as_ref(),
            self.url.url_extra_data(),
            origin,
            style::servo_arc::Arc::new(self.lock.wrap(MediaList::empty())),
            self.lock.clone(),
            Some(&StylesheetLoader(self.id, self.net_provider.clone())), // todo
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::Yes
        );

        DocumentStyleSheet(style::servo_arc::Arc::new(data))
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
