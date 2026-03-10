//! Fragment Tree - a serializable snapshot of DOM rendering data.
//!
//! Following the Servo architecture: the tab (content) process builds the fragment
//! tree from the DOM after style resolution and layout. This tree is sent via IPC
//! to the main (compositor) process, which hands it to the `HtmlRenderer` (our
//! equivalent of WebRender) for rasterization.
//!
//! The fragment tree captures exactly the data that `HtmlRenderer` needs from the
//! DOM, without referencing live DOM nodes.

use crate::dom::node::{
    ImageData, ListItemLayoutPosition, Marker, SpecialElementData,
};
use crate::dom::{Dom, ElementData, NodeData};
use crate::renderer::painter::ToColorColor;
use crate::ui::TextBrush;
use markup5ever::local_name;
use parley::PositionedLayoutItem;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use style::properties::generated::longhands::border_collapse::computed_value::T as BorderCollapse;
use style::properties::generated::longhands::visibility::computed_value::T as Visibility;
use style::properties::{longhands, ComputedValues};
use style::values::computed::{BorderStyle, CSSPixelLength, OutlineStyle, Overflow};
use style::values::generics::color::{GenericColor, GenericColorOrAuto};
use taffy::{Point, Rect, Size};

/// Serializable version of `taffy::Layout` with only the fields the renderer needs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FragmentLayout {
    pub location: Point<f32>,
    pub size: Size<f32>,
    pub content_size: Size<f32>,
    pub scrollbar_size: Size<f32>,
    pub border: Rect<f32>,
    pub padding: Rect<f32>,
    pub margin: Rect<f32>,
}

impl FragmentLayout {
    pub fn from_taffy(layout: &taffy::Layout) -> Self {
        Self {
            location: layout.location,
            size: layout.size,
            content_size: layout.content_size,
            scrollbar_size: layout.scrollbar_size,
            border: layout.border,
            padding: layout.padding,
            margin: layout.margin,
        }
    }

    /// Convert back to a taffy::Layout for use by the renderer.
    pub fn to_taffy(&self) -> taffy::Layout {
        taffy::Layout {
            order: 0,
            location: self.location,
            size: self.size,
            content_size: self.content_size,
            scrollbar_size: self.scrollbar_size,
            border: self.border,
            padding: self.padding,
            margin: self.margin,
        }
    }
}
use usvg::{Indent, WriteOptions};

/// A serializable snapshot of all DOM data the renderer needs.
/// This is built by the tab process and sent via IPC to the main process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentTree {
    /// All fragment nodes, indexed by the same IDs as the original DOM.
    /// Uses a HashMap because DOM node IDs may be sparse (from a Slab).
    pub nodes: HashMap<usize, FragmentNode>,

    /// The root element ID.
    pub root_element_id: usize,

    /// Viewport scroll position.
    pub viewport_scroll: taffy::Point<f64>,

    /// Pre-resolved text styles for glyph run rendering.
    /// Maps node ID → resolved text style data.
    pub text_styles: HashMap<usize, ResolvedTextStyle>,

    /// Table row background colors, indexed by node ID.
    pub table_row_styles: HashMap<usize, ResolvedTableRowStyle>,
}

/// Pre-resolved text style data for a node, used by `stroke_text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedTextStyle {
    /// The text color as RGBA components.
    pub text_color: [f32; 4],
    /// The text decoration color as RGBA components.
    pub text_decoration_color: [f32; 4],
    /// Whether the text has an underline decoration.
    pub has_underline: bool,
    /// Whether the text has a strikethrough decoration.
    pub has_strikethrough: bool,
}

/// Pre-resolved table row style for background rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedTableRowStyle {
    pub background_color: [f32; 4],
}

/// A serializable snapshot of a single DOM node's rendering data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentNode {
    /// Node ID (same as in the original DOM).
    pub id: usize,

    /// Parent node ID.
    pub parent: Option<usize>,

    /// Child node IDs.
    pub children: Vec<usize>,

    /// What kind of node this is.
    pub node_kind: FragmentNodeKind,

    /// The final layout (position, size, padding, border, content_size).
    pub final_layout: FragmentLayout,

    /// The taffy display style.
    pub display: taffy::Display,

    /// Node flags (IS_INLINE_ROOT, IS_TABLE_ROOT, etc.).
    pub flags: FragmentFlags,

    /// Layout children IDs (in layout order).
    pub layout_children: Option<Vec<usize>>,

    /// Paint children IDs (in paint order).
    pub paint_children: Option<Vec<usize>>,

    /// Stacking context (hoisted children with z-index).
    pub stacking_context: Option<FragmentStackingContext>,

    /// Element scroll offset.
    pub scroll_offset: taffy::Point<f64>,

    /// Whether this node is currently focused.
    pub is_focused: bool,

    /// Resolved element-specific rendering data (only for Element/AnonymousBlock nodes).
    pub element_data: Option<FragmentElementData>,

    /// Resolved computed style data needed for rendering.
    pub resolved_style: Option<ResolvedStyle>,
}

/// Serializable node kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FragmentNodeKind {
    Document,
    Element {
        tag_name: String,
    },
    AnonymousBlock,
    Text,
    Comment,
    ShadowRoot,
}

/// Serializable fragment flags mirroring DomNodeFlags.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FragmentFlags {
    pub is_inline_root: bool,
    pub is_table_root: bool,
}

/// Serializable stacking context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentStackingContext {
    pub children: Vec<FragmentHoistedChild>,
    pub negative_z_count: u32,
}

impl FragmentStackingContext {
    pub fn neg_z_range(&self) -> std::ops::Range<usize> {
        0..(self.negative_z_count as usize)
    }

    pub fn pos_z_range(&self) -> std::ops::Range<usize> {
        (self.negative_z_count as usize)..self.children.len()
    }

    pub fn neg_z_hoisted_children(
        &self,
    ) -> impl ExactSizeIterator<Item = &FragmentHoistedChild> + DoubleEndedIterator {
        self.children[self.neg_z_range()].iter()
    }

    pub fn pos_z_hoisted_children(
        &self,
    ) -> impl ExactSizeIterator<Item = &FragmentHoistedChild> + DoubleEndedIterator {
        self.children[self.pos_z_range()].iter()
    }
}

/// Serializable hoisted paint child.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentHoistedChild {
    pub node_id: usize,
    pub z_index: i32,
    pub position: taffy::Point<f32>,
}

/// Resolved style data needed for rendering, extracted from ComputedValues.
/// Only the fields that HtmlRenderer actually reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedStyle {
    // Visibility
    pub visibility_visible: bool,
    pub opacity: f32,

    // Overflow
    pub overflow_x: SerializedOverflow,
    pub overflow_y: SerializedOverflow,

    // Background
    pub background: ResolvedBackground,

    // Border
    pub border_top_color: [f32; 4],
    pub border_right_color: [f32; 4],
    pub border_bottom_color: [f32; 4],
    pub border_left_color: [f32; 4],

    // Border radii (resolved to px)
    pub border_top_left_radius: (f64, f64),
    pub border_top_right_radius: (f64, f64),
    pub border_bottom_right_radius: (f64, f64),
    pub border_bottom_left_radius: (f64, f64),

    // Outline
    pub outline_width: f64,
    pub outline_color: [f32; 4],
    pub outline_style: SerializedOutlineStyle,

    // Box shadow
    pub box_shadows: Vec<ResolvedBoxShadow>,

    // Effects
    pub current_color: [f32; 4],

    // Text input caret
    pub caret_color: Option<[f32; 4]>,

    // Object positioning
    pub object_fit: SerializedObjectFit,
    pub object_position_h: f32,
    pub object_position_v: f32,

    // Image rendering
    pub image_quality: SerializedImageQuality,

    // Table
    pub border_collapse: Option<SerializedBorderCollapse>,
    pub border_top_style: SerializedBorderStyle,
    pub border_bottom_style: SerializedBorderStyle,
    pub border_left_style: SerializedBorderStyle,
    pub border_right_style: SerializedBorderStyle,

    // Table border width
    pub table_border_width: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedBackground {
    pub background_color: [f32; 4],
    // Full background rendering data would be complex; for initial implementation
    // we serialize the display commands for backgrounds
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SerializedOverflow {
    Visible,
    Hidden,
    Scroll,
    Auto,
    Clip,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SerializedOutlineStyle {
    Auto,
    None,
    Solid,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SerializedBorderStyle {
    None,
    Hidden,
    Solid,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SerializedObjectFit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SerializedImageQuality {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SerializedBorderCollapse {
    Separate,
    Collapse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedBoxShadow {
    pub horizontal: f64,
    pub vertical: f64,
    pub blur: f64,
    pub spread: f64,
    pub color: [f32; 4],
    pub inset: bool,
}

/// Element-specific rendering data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentElementData {
    pub tag_name: String,

    /// Whether this has an inline layout.
    pub has_inline_layout: bool,

    /// Raster image data (for <img> elements).
    pub raster_image: Option<FragmentRasterImage>,

    /// SVG data (serialized as SVG string for re-parsing).
    pub svg_source: Option<Vec<u8>>,

    /// Canvas custom paint source ID.
    pub canvas_paint_source_id: Option<u64>,

    /// Whether this is a text input.
    pub has_text_input: bool,

    /// Checkbox checked state.
    pub checkbox_checked: Option<bool>,

    /// Background image data.
    pub background_images: Vec<Option<FragmentBackgroundImage>>,

    /// List item marker data.
    pub list_item: Option<FragmentListItem>,

    /// Table context data.
    pub table: Option<FragmentTableData>,

    /// Hidden input type.
    pub is_hidden_input: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentRasterImage {
    pub width: u32,
    pub height: u32,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentBackgroundImage {
    pub image: FragmentBackgroundImageKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FragmentBackgroundImageKind {
    Raster(FragmentRasterImage),
    Svg(Vec<u8>),
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FragmentListItem {
    Inside {
        marker: FragmentMarker,
    },
    Outside {
        marker: FragmentMarker,
        // For outside markers, we'll pre-render the text into display commands
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FragmentMarker {
    Char(char),
    String(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentTableData {
    pub border_collapse: SerializedBorderCollapse,
    pub rows: Vec<FragmentTableRow>,
    pub grid_column_sizes: Vec<f32>,
    pub grid_column_gutters: Vec<f32>,
    pub grid_row_sizes: Vec<f32>,
    pub grid_row_gutters: Vec<f32>,
    pub border_width: f64,
    pub border_color: [f32; 4],
    pub border_top_style: SerializedBorderStyle,
    pub border_bottom_style: SerializedBorderStyle,
    pub border_left_style: SerializedBorderStyle,
    pub border_right_style: SerializedBorderStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentTableRow {
    pub node_id: usize,
    pub height: f32,
}

// ── Fragment Tree Builder ──────────────────────────────────────────────────

impl FragmentTree {
    /// Build a FragmentTree from a Dom, capturing all data the renderer needs.
    pub fn build(dom: &Dom, selection_ranges: &HashMap<usize, (usize, usize)>) -> Self {
        let root_element = dom.root_element();
        let root_id = root_element.id;

        let mut tree = FragmentTree {
            nodes: HashMap::new(),
            root_element_id: root_id,
            viewport_scroll: taffy::Point {
                x: dom.viewport_scroll.x,
                y: dom.viewport_scroll.y,
            },
            text_styles: HashMap::new(),
            table_row_styles: HashMap::new(),
        };

        // Walk the DOM tree and extract rendering data for each node
        tree.extract_node(dom, root_id);

        tree
    }

    fn extract_node(&mut self, dom: &Dom, node_id: usize) {
        let node = &dom.tree()[node_id];

        let node_kind = match &node.data {
            NodeData::Document => FragmentNodeKind::Document,
            NodeData::Element(elem) => FragmentNodeKind::Element {
                tag_name: elem.name.local.to_string(),
            },
            NodeData::AnonymousBlock(_) => FragmentNodeKind::AnonymousBlock,
            NodeData::Text { .. } => FragmentNodeKind::Text,
            NodeData::Comment => FragmentNodeKind::Comment,
            NodeData::ShadowRoot(_) => FragmentNodeKind::ShadowRoot,
        };

        let flags = FragmentFlags {
            is_inline_root: node.flags.is_inline_root(),
            is_table_root: node.flags.is_table_root(),
        };

        let stacking_context = node.stacking_context.as_ref().map(|sc| {
            FragmentStackingContext {
                children: sc
                    .children
                    .iter()
                    .map(|child| FragmentHoistedChild {
                        node_id: child.node_id,
                        z_index: child.z_index,
                        position: child.position,
                    })
                    .collect(),
                negative_z_count: sc.negative_z_count,
            }
        });

        let layout_children = node.layout_children.borrow().clone();
        let paint_children = node.paint_children.borrow().clone();

        // Extract element-specific data and resolved style
        let (element_data, resolved_style) = if let Some(elem) = node.element_data() {
            let styles = node.primary_styles();
            let resolved = styles.map(|s| self.resolve_style(&s, elem, dom, node_id));
            let elem_data = self.extract_element_data(elem, dom, node_id);

            // Extract text styles for inline layout nodes
            if node.flags.is_inline_root() {
                if let Some(ild) = &elem.inline_layout_data {
                    self.extract_text_styles_from_layout(dom, &ild.layout);
                }
            }

            // Extract text styles for list item marker
            if let Some(list_item) = &elem.list_item_data {
                if let ListItemLayoutPosition::Outside(layout) = &list_item.position {
                    self.extract_text_styles_from_layout(dom, layout);
                }
            }

            // Extract text styles for text input
            if let Some(text_input) = elem.text_input_data() {
                if let Some(layout) = text_input.editor.try_layout() {
                    self.extract_text_styles_from_layout(dom, layout);
                }
            }

            // Extract table row styles
            if let SpecialElementData::TableRoot(table) = &elem.special_data {
                for row in &table.rows {
                    if let Some(row_node) = dom.get_node(row.node_id) {
                        if let Some(style) = row_node.primary_styles() {
                            let current_color = style.clone_color();
                            let bg_color = style
                                .get_background()
                                .background_color
                                .resolve_to_absolute(&current_color)
                                .as_color_color();
                            self.table_row_styles.insert(
                                row.node_id,
                                ResolvedTableRowStyle {
                                    background_color: bg_color.components,
                                },
                            );
                        }
                    }
                }
            }

            (Some(elem_data), resolved)
        } else {
            (None, None)
        };

        let fragment = FragmentNode {
            id: node_id,
            parent: node.parent,
            children: node.children.clone(),
            node_kind,
            final_layout: FragmentLayout::from_taffy(&node.final_layout),
            display: node.taffy_style.display,
            flags,
            layout_children,
            paint_children,
            stacking_context,
            scroll_offset: node.scroll_offset,
            is_focused: node.is_focused(),
            element_data,
            resolved_style,
        };

        self.nodes.insert(node_id, fragment);

        // Recursively extract layout children
        let lc = dom.tree()[node_id].layout_children.borrow().clone();
        if let Some(children) = &lc {
            for &child_id in children {
                if !self.nodes.contains_key(&child_id) {
                    self.extract_node(dom, child_id);
                }
            }
        }

        // Also extract stacking context children
        if let Some(sc) = &dom.tree()[node_id].stacking_context {
            for child in &sc.children {
                if !self.nodes.contains_key(&child.node_id) {
                    self.extract_node(dom, child.node_id);
                }
            }
        }

        // Extract regular children too
        let children = dom.tree()[node_id].children.clone();
        for child_id in &children {
            if !self.nodes.contains_key(child_id) {
                self.extract_node(dom, *child_id);
            }
        }
    }

    fn resolve_style(
        &self,
        style: &ComputedValues,
        elem: &ElementData,
        dom: &Dom,
        node_id: usize,
    ) -> ResolvedStyle {
        let current_color = style.clone_color().as_color_color();
        let border = style.get_border();
        let outline = style.get_outline();
        let effects = style.get_effects();

        let resolve_border_color = |color: &longhands::border_top_color::computed_value::T| -> [f32; 4] {
            color.resolve_to_absolute(&style.clone_color()).as_color_color().components
        };

        let outline_color = outline
            .outline_color
            .resolve_to_absolute(&style.clone_color())
            .as_color_color()
            .components;

        let outline_style = match outline.outline_style {
            OutlineStyle::Auto => SerializedOutlineStyle::Auto,
            OutlineStyle::BorderStyle(BorderStyle::None | BorderStyle::Hidden) => {
                SerializedOutlineStyle::None
            }
            OutlineStyle::BorderStyle(BorderStyle::Solid) => SerializedOutlineStyle::Solid,
            OutlineStyle::BorderStyle(_) => SerializedOutlineStyle::Other,
        };

        let bg_color = style
            .get_background()
            .background_color
            .resolve_to_absolute(&style.clone_color())
            .as_color_color();

        let box_shadows = effects
            .box_shadow
            .0
            .iter()
            .map(|s| ResolvedBoxShadow {
                horizontal: s.base.horizontal.px() as f64,
                vertical: s.base.vertical.px() as f64,
                blur: s.base.blur.px() as f64,
                spread: s.spread.px() as f64,
                color: s
                    .base
                    .color
                    .resolve_to_absolute(&style.clone_color())
                    .as_color_color()
                    .components,
                inset: s.inset,
            })
            .collect();

        let caret_color = if let Some(text_input) = elem.text_input_data() {
            let itext_color = style.get_inherited_text().color;
            let caret = match &style.get_inherited_ui().caret_color.0 {
                GenericColorOrAuto::Color(c) => c.resolve_to_absolute(&itext_color),
                GenericColorOrAuto::Auto => itext_color,
            };
            Some(caret.as_color_color().components)
        } else {
            None
        };

        let serialize_overflow = |o: Overflow| -> SerializedOverflow {
            match o {
                Overflow::Visible => SerializedOverflow::Visible,
                Overflow::Hidden => SerializedOverflow::Hidden,
                Overflow::Scroll => SerializedOverflow::Scroll,
                Overflow::Auto => SerializedOverflow::Auto,
                Overflow::Clip => SerializedOverflow::Clip,
            }
        };

        let serialize_border_style = |bs: BorderStyle| -> SerializedBorderStyle {
            match bs {
                BorderStyle::None => SerializedBorderStyle::None,
                BorderStyle::Hidden => SerializedBorderStyle::Hidden,
                BorderStyle::Solid => SerializedBorderStyle::Solid,
                _ => SerializedBorderStyle::Other,
            }
        };

        let width = node_id; // placeholder
        let resolve_w = CSSPixelLength::new(style.get_border().border_top_left_radius.0.width.0.resolve(CSSPixelLength::new(0.0)).px());

        // Table-specific
        let (border_collapse, table_border_width) =
            if let SpecialElementData::TableRoot(table) = &elem.special_data {
                let bc = match table.border_collapse {
                    BorderCollapse::Separate => Some(SerializedBorderCollapse::Separate),
                    BorderCollapse::Collapse => Some(SerializedBorderCollapse::Collapse),
                };
                let bw = table
                    .border_style
                    .as_deref()
                    .map(|bs| bs.border_top_width.0.to_f64_px());
                (bc, bw)
            } else {
                (None, None)
            };

        ResolvedStyle {
            visibility_visible: style.get_inherited_box().visibility == Visibility::Visible,
            opacity: effects.opacity,
            overflow_x: serialize_overflow(style.get_box().overflow_x),
            overflow_y: serialize_overflow(style.get_box().overflow_y),
            background: ResolvedBackground {
                background_color: bg_color.components,
            },
            border_top_color: resolve_border_color(&border.border_top_color),
            border_right_color: resolve_border_color(&border.border_right_color),
            border_bottom_color: resolve_border_color(&border.border_bottom_color),
            border_left_color: resolve_border_color(&border.border_left_color),
            border_top_left_radius: (0.0, 0.0), // Will be computed in renderer from layout
            border_top_right_radius: (0.0, 0.0),
            border_bottom_right_radius: (0.0, 0.0),
            border_bottom_left_radius: (0.0, 0.0),
            outline_width: outline.outline_width.0.to_f64_px(),
            outline_color,
            outline_style,
            box_shadows,
            current_color: current_color.components,
            caret_color,
            object_fit: SerializedObjectFit::Fill, // Will be set properly if needed
            object_position_h: 0.0,
            object_position_v: 0.0,
            image_quality: SerializedImageQuality::Medium,
            border_collapse,
            border_top_style: serialize_border_style(style.get_border().border_top_style),
            border_bottom_style: serialize_border_style(style.get_border().border_bottom_style),
            border_left_style: serialize_border_style(style.get_border().border_left_style),
            border_right_style: serialize_border_style(style.get_border().border_right_style),
            table_border_width,
        }
    }

    fn extract_element_data(
        &self,
        elem: &ElementData,
        dom: &Dom,
        node_id: usize,
    ) -> FragmentElementData {
        let raster_image = elem.raster_image_data().map(|img| FragmentRasterImage {
            width: img.width,
            height: img.height,
            data: img.data.data().to_vec(),
        });

        let svg_source = elem.svg_data().map(|svg| {
            // Serialize usvg tree back to SVG bytes
            svg.to_string(&WriteOptions {
                indent: Indent::None,
                ..Default::default()
            }).into_bytes()
        });

        let canvas_paint_source_id = elem.canvas_data().map(|c| c.custom_paint_source_id);

        let has_text_input = elem.text_input_data().is_some();

        let checkbox_checked = match &elem.special_data {
            SpecialElementData::CheckboxInput(checked) => Some(*checked),
            _ => None,
        };

        let background_images = elem
            .background_images
            .iter()
            .map(|bg_opt| {
                bg_opt.as_ref().map(|bg| FragmentBackgroundImage {
                    image: match &bg.image {
                        ImageData::Raster(img) => {
                            FragmentBackgroundImageKind::Raster(FragmentRasterImage {
                                width: img.width,
                                height: img.height,
                                data: img.data.data().to_vec(),
                            })
                        }
                        ImageData::Svg(svg) => {
                            FragmentBackgroundImageKind::Svg(svg.to_string(&WriteOptions {
                                indent: Indent::None,
                                ..Default::default()
                            }).into_bytes())
                        }
                        ImageData::None => FragmentBackgroundImageKind::None,
                    },
                })
            })
            .collect();

        let list_item = elem.list_item_data.as_deref().map(|li| {
            let marker = match &li.marker {
                Marker::Char(c) => FragmentMarker::Char(*c),
                Marker::String(s) => FragmentMarker::String(s.clone()),
            };
            match &li.position {
                ListItemLayoutPosition::Inside => FragmentListItem::Inside { marker },
                ListItemLayoutPosition::Outside(_layout) => FragmentListItem::Outside { marker },
            }
        });

        let table = if let SpecialElementData::TableRoot(table) = &elem.special_data {
            let grid_info = table.computed_grid_info.borrow();
            if let Some(grid) = grid_info.as_ref() {
                let current_color = dom.tree()[node_id]
                    .primary_styles()
                    .map(|s| s.clone_color())
                    .unwrap();

                let border_color = table
                    .border_style
                    .as_deref()
                    .map(|bs| {
                        bs.border_top_color
                            .resolve_to_absolute(&current_color)
                            .as_color_color()
                            .components
                    })
                    .unwrap_or([0.0, 0.0, 0.0, 0.0]);

                let border_width = table
                    .border_style
                    .as_deref()
                    .map(|bs| bs.border_top_width.0.to_f64_px())
                    .unwrap_or(0.0);

                let outer_border = dom.tree()[node_id]
                    .primary_styles()
                    .map(|s| s.get_border().clone())
                    .unwrap();

                let serialize_border_style = |bs: BorderStyle| -> SerializedBorderStyle {
                    match bs {
                        BorderStyle::None => SerializedBorderStyle::None,
                        BorderStyle::Hidden => SerializedBorderStyle::Hidden,
                        BorderStyle::Solid => SerializedBorderStyle::Solid,
                        _ => SerializedBorderStyle::Other,
                    }
                };

                Some(FragmentTableData {
                    border_collapse: match table.border_collapse {
                        BorderCollapse::Separate => SerializedBorderCollapse::Separate,
                        BorderCollapse::Collapse => SerializedBorderCollapse::Collapse,
                    },
                    rows: table
                        .rows
                        .iter()
                        .map(|r| FragmentTableRow {
                            node_id: r.node_id,
                            height: r.height,
                        })
                        .collect(),
                    grid_column_sizes: grid.columns.sizes.clone(),
                    grid_column_gutters: grid.columns.gutters.clone(),
                    grid_row_sizes: grid.rows.sizes.clone(),
                    grid_row_gutters: grid.rows.gutters.clone(),
                    border_width,
                    border_color,
                    border_top_style: serialize_border_style(outer_border.border_top_style),
                    border_bottom_style: serialize_border_style(outer_border.border_bottom_style),
                    border_left_style: serialize_border_style(outer_border.border_left_style),
                    border_right_style: serialize_border_style(outer_border.border_right_style),
                })
            } else {
                None
            }
        } else {
            None
        };

        let is_hidden_input = elem.name.local == local_name!("input")
            && elem.attr(local_name!("type")) == Some("hidden");

        FragmentElementData {
            tag_name: elem.name.local.to_string(),
            has_inline_layout: elem.inline_layout_data.is_some(),
            raster_image,
            svg_source,
            canvas_paint_source_id,
            has_text_input,
            checkbox_checked,
            background_images,
            list_item,
            table,
            is_hidden_input,
        }
    }

    /// Extract text styles for all nodes referenced by glyph runs in a parley Layout.
    fn extract_text_styles_from_layout(
        &mut self,
        dom: &Dom,
        layout: &parley::Layout<TextBrush>,
    ) {
        use style::values::specified::TextDecorationLine;

        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let brush_id = glyph_run.style().brush.id;
                    if self.text_styles.contains_key(&brush_id) {
                        continue;
                    }
                    if let Some(node) = dom.get_node(brush_id) {
                        if let Some(styles) = node.primary_styles() {
                            let itext = styles.get_inherited_text();
                            let text = styles.get_text();
                            let text_color = itext.color.as_color_color();
                            let decoration_color = text
                                .text_decoration_color
                                .as_absolute()
                                .map(|c| c.as_color_color())
                                .unwrap_or(text_color);
                            let decoration_line = text.text_decoration_line;

                            self.text_styles.insert(
                                brush_id,
                                ResolvedTextStyle {
                                    text_color: text_color.components,
                                    text_decoration_color: decoration_color.components,
                                    has_underline: decoration_line
                                        .contains(TextDecorationLine::UNDERLINE),
                                    has_strikethrough: decoration_line
                                        .contains(TextDecorationLine::LINE_THROUGH),
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}

