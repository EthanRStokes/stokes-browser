use crate::dom::node::SpecialElementData;
use crate::dom::{Dom, ImageData, NodeData};
use crate::layout::replaced::{replaced_measure_function, ReplacedContext};
use crate::layout::table::{TableContext, TableTreeWrapper};
use markup5ever::local_name;
use std::cell::Ref;
use std::sync::Arc;
use style::values::computed::length_percentage::CalcLengthPercentage;
use style::values::computed::{CSSPixelLength, LineHeight};
use stylo_atoms::Atom;
pub(crate) use taffy::{compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_leaf_layout, AvailableSpace, CacheTree, CollapsibleMarginSet, Display, Layout, LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, PrintTree, ResolveOrZero, RoundTree, RunMode, Size, Style, TraversePartialTree, TraverseTree};

impl TraversePartialTree for Dom {
    type ChildIter<'a> = RefCellChildIter<'a>;

    fn child_ids(&self, parent_node_id: NodeId) -> Self::ChildIter<'_> {
        let layout_children = self.node_from_id(parent_node_id).layout_children.borrow();
        RefCellChildIter::new(Ref::map(layout_children, |children| {
            children.as_ref().map(|c| c.as_slice()).unwrap_or(&[])
        }))
    }

    fn child_count(&self, parent_node_id: NodeId) -> usize {
        self.node_from_id(parent_node_id)
            .layout_children
            .borrow()
            .as_ref()
            .map(|c| c.len())
            .unwrap_or(0)
    }

    fn get_child_id(&self, parent_node_id: NodeId, child_index: usize) -> NodeId {
        NodeId::from(
            self.node_from_id(parent_node_id)
                .layout_children
                .borrow()
                .as_ref()
                .unwrap()[child_index]
        )
    }
}
impl TraverseTree for Dom {}

impl LayoutPartialTree for Dom {
    type CoreContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    type CustomIdent = Atom;

    fn get_core_container_style(&self, node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        &self.node_from_id(node_id).taffy_style
    }

    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        self.node_from_id_mut(node_id).unrounded_layout = *layout;
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        compute_cached_layout(self, node_id, inputs, |tree, node_id, inputs| {
            let node = &mut tree.nodes[node_id.into()];

            let font_styles = node.primary_styles().map(|style| {
                let font_size = style.clone_font_size().used_size().px();
                let line_height = match style.clone_line_height() {
                    LineHeight::Normal => font_size * 1.2,
                    LineHeight::Length(length) => length.0.px(),
                    LineHeight::Number(number) => font_size * number.0,
                };

                (font_size, line_height)
            });
            let font_size = font_styles.map(|s| s.0);
            let line_height = font_styles.map(|s| s.1);

            match &mut node.data {
                NodeData::Document => compute_block_layout(tree, node_id, inputs, None),
                NodeData::Text { .. } => {
                    LayoutOutput::HIDDEN
                }
                NodeData::Element(data) | NodeData::AnonymousBlock(data) => {
                    // TODO: deduplicate with single-line text input
                    if *data.name.local == *"textarea" {
                        let rows = data
                            .attr(local_name!("rows"))
                            .and_then(|val| val.parse::<f32>().ok())
                            .unwrap_or(2.0);

                        let cols = data
                            .attr(local_name!("cols"))
                            .and_then(|val| val.parse::<f32>().ok());

                        return compute_leaf_layout(
                            inputs,
                            &node.taffy_style,
                            resolve_calc_value,
                            |_known_size, _available_space| Size {
                                width: cols
                                    .map(|cols| cols * font_size.unwrap_or(16.0) * 0.6)
                                    .unwrap_or(300.0),
                                height: line_height.unwrap_or(16.0) * rows,
                            },
                        );
                    }

                    if *data.name.local == *"input" {
                        match data.attr(local_name!("type")) {
                            // if the input type is hidden, hide it
                            Some("hidden") => {
                                node.taffy_style.display = Display::None;
                                return LayoutOutput::HIDDEN;
                            }
                            Some("checkbox") => {
                                return compute_leaf_layout(
                                    inputs,
                                    &node.taffy_style,
                                    resolve_calc_value,
                                    |_known_size, _available_space| {
                                        let width = node.taffy_style.size.width.resolve_or_zero(
                                            inputs.parent_size.width,
                                            resolve_calc_value,
                                        );
                                        let height = node.taffy_style.size.height.resolve_or_zero(
                                            inputs.parent_size.height,
                                            resolve_calc_value,
                                        );
                                        let min_size = width.min(height);
                                        taffy::Size {
                                            width: min_size,
                                            height: min_size,
                                        }
                                    },
                                );
                            }
                            None | Some("text" | "password" | "email" | "tel" | "url" | "search") => {
                                return compute_leaf_layout(
                                    inputs,
                                    &node.taffy_style,
                                    resolve_calc_value,
                                    |_known_size, _available_space| Size {
                                        width: 300.0,
                                        height: line_height.unwrap_or(16.0),
                                    },
                                );
                            }
                            _ => {}
                        }
                    }

                    if *data.name.local == *"img"
                        || *data.name.local == *"canvas"
                        || *data.name.local == *"svg"
                    {
                        // Get width and height attributes on image element
                        //
                        // TODO: smarter sizing using these (depending on object-fit, they shouldn't
                        // necessarily just override the native size)
                        let attr_size = taffy::Size {
                            width: data
                                .attr(local_name!("width"))
                                .and_then(|val| val.parse::<f32>().ok()),
                            height: data
                                .attr(local_name!("height"))
                                .and_then(|val| val.parse::<f32>().ok()),
                        };

                        // Get image's native sizespecial_data
                        let inherent_size = match &data.special_data {
                            SpecialElementData::Image(image_data) => match &**image_data {
                                ImageData::Raster(image) => Size {
                                    width: image.width as f32,
                                    height: image.height as f32,
                                },
                                ImageData::Svg(svg) => {
                                    let size = svg.size();
                                    taffy::Size {
                                        width: size.width(),
                                        height: size.height(),
                                    }
                                }
                                ImageData::None => taffy::Size::ZERO,
                            },
                            SpecialElementData::Canvas(_) => taffy::Size::ZERO,
                            SpecialElementData::None => taffy::Size::ZERO,
                            _ => unreachable!(),
                        };

                        let replaced_context = ReplacedContext {
                            inherent_size,
                            attr_size,
                        };

                        let computed = replaced_measure_function(
                            inputs.known_dimensions,
                            inputs.parent_size,
                            inputs.available_space,
                            &replaced_context,
                            &node.taffy_style,
                            false,
                        );

                        return LayoutOutput {
                            size: computed,
                            content_size: computed,
                            first_baselines: taffy::Point::NONE,
                            top_margin: CollapsibleMarginSet::ZERO,
                            bottom_margin: CollapsibleMarginSet::ZERO,
                            margins_can_collapse_through: false,
                        };
                    }

                    if node.flags.is_table_root() {
                        let SpecialElementData::TableRoot(context) = &tree.nodes[node_id.into()]
                            .data
                            .element()
                            .unwrap()
                            .special_data
                        else {
                            panic!("Node marked as table root but doesn't have TableContext");
                        };
                        let context: Arc<TableContext> = Arc::clone(context);

                        let mut table_wrapper = TableTreeWrapper {
                            dom: tree,
                            ctx: context,
                        };
                        let mut output = compute_grid_layout(&mut table_wrapper, node_id, inputs);

                        output.content_size.width = output.content_size.width.min(output.size.width);
                        output.content_size.height = output.content_size.height.min(output.size.height);

                        return output;
                    }

                    if node.flags.is_inline_root() {
                        return tree.compute_inline_layout(usize::from(node_id), inputs, None)
                    }

                    // The default CSS file will set
                    match node.taffy_style.display {
                        Display::Block => compute_block_layout(tree, node_id, inputs, None),
                        Display::Flex => compute_flexbox_layout(tree, node_id, inputs),
                        Display::Grid => compute_grid_layout(tree, node_id, inputs),
                        Display::None => taffy::LayoutOutput::HIDDEN,
                    }
                }

                _ => LayoutOutput::HIDDEN,
            }
        })
    }

    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        resolve_calc_value(val, basis)
    }
}

impl CacheTree for Dom {
    fn cache_get(&self, node_id: NodeId, known_dimensions: Size<Option<f32>>, available_space: Size<AvailableSpace>, run_mode: RunMode) -> Option<LayoutOutput> {
        self.node_from_id(node_id).cache.get(known_dimensions, available_space, run_mode)
    }

    fn cache_store(&mut self, node_id: NodeId, known_dimensions: Size<Option<f32>>, available_space: Size<AvailableSpace>, run_mode: RunMode, layout_output: LayoutOutput) {
        self.node_from_id_mut(node_id).cache.store(known_dimensions, available_space, run_mode, layout_output)
    }

    fn cache_clear(&mut self, node_id: NodeId) {
        self.node_from_id_mut(node_id).cache.clear();
    }
}

impl LayoutBlockContainer for Dom {
    type BlockContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        self.get_core_container_style(node_id)
    }

    fn get_block_child_style(&self, child_node_id: NodeId) -> Self::BlockItemStyle<'_> {
        self.get_core_container_style(child_node_id)
    }
}

impl LayoutFlexboxContainer for Dom {
    type FlexboxContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.get_core_container_style(node_id)
    }

    fn get_flexbox_child_style(&self, child_node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        self.get_core_container_style(child_node_id)
    }
}

impl LayoutGridContainer for Dom {
    type GridContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        self.get_core_container_style(node_id)
    }

    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        self.get_core_container_style(child_node_id)
    }
}

impl RoundTree for Dom {
    fn get_unrounded_layout(&self, node_id: NodeId) -> Layout {
        self.node_from_id(node_id).unrounded_layout
    }

    fn set_final_layout(&mut self, node_id: NodeId, layout: &Layout) {
        self.node_from_id_mut(node_id).final_layout = *layout;
    }
}

impl PrintTree for Dom {
    fn get_debug_label(&self, node_id: NodeId) -> &'static str {
        let node = self.node_from_id(node_id);
        match node.data {
            NodeData::Document => "DOCUMENT",
            NodeData::Text { .. } => "TEXT",
            NodeData::Comment { .. } => "COMMENT",
            NodeData::Element(_) => "ELEMENT",
            NodeData::AnonymousBlock(_) => "ANONYMOUS BLOCK"
        }
    }

    fn get_final_layout(&self, node_id: NodeId) -> Layout {
        self.node_from_id(node_id).final_layout
    }
}

pub struct RefCellChildIter<'a> {
    items: Ref<'a, [usize]>,
    idx: usize,
}
impl<'a> RefCellChildIter<'a> {
    fn new(items: Ref<'a, [usize]>) -> RefCellChildIter<'a> {
        RefCellChildIter { items, idx: 0 }
    }
}

impl Iterator for RefCellChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.items.get(self.idx).map(|id| {
            self.idx += 1;
            NodeId::from(*id)
        })
    }
}

pub(crate) fn resolve_calc_value(calc_ptr: *const (), parent_size: f32) -> f32 {
    let calc = unsafe { &*(calc_ptr as *const CalcLengthPercentage) };
    let result = calc.resolve(CSSPixelLength::new(parent_size));
    result.px()
}