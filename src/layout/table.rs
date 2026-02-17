use crate::dom::damage::{CONSTRUCT_BOX, CONSTRUCT_DESCENDENT, CONSTRUCT_FC};
use crate::dom::Dom;
use crate::layout::taffy::resolve_calc_value;
use atomic_refcell::AtomicRefCell;
use markup5ever::local_name;
use std::{ops::Range, sync::Arc};
use style::computed_values::border_collapse::T as BorderCollapse;
use style::dom::NodeInfo;
use style::properties::style_structs::Border;
use style::servo_arc::Arc as ServoArc;
use style::values::specified::box_::{DisplayInside, DisplayOutside};
use style::{computed_values::table_layout::T as TableLayout, Atom};
use taffy::style_helpers::{auto, length, percent};
use taffy::{compute_leaf_layout, style_helpers, DetailedGridInfo, Dimension, LayoutPartialTree as _, NodeId, Rect, ResolveOrZero, Size, TrackSizingFunction};

pub struct TableTreeWrapper<'doc> {
    pub(crate) dom: &'doc mut Dom,
    pub(crate) ctx: Arc<TableContext>,
}

#[derive(Debug, Clone)]
pub struct TableContext {
    pub style: taffy::Style<Atom>,
    pub items: Vec<TableItem>,
    pub computed_grid_info: AtomicRefCell<Option<DetailedGridInfo>>,
    pub border_style: Option<ServoArc<Border>>,
    pub border_collapse: BorderCollapse
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TableItemKind {
    Row,
    Cell,
}

#[derive(Debug, Clone)]
pub struct TableItem {
    kind: TableItemKind,
    node_id: usize,
    style: taffy::Style<Atom>,
}

pub(crate) fn build_table_context(
    dom: &mut Dom,
    table_root_node_id: usize,
) -> (TableContext, Vec<usize>) {
    let mut items: Vec<TableItem> = Vec::new();
    let mut row = 0u16;
    let mut col = 0u16;
    println!("table is actually used");

    let root_node = &mut dom.nodes[table_root_node_id];

    let children = std::mem::take(&mut root_node.children);

    let Some(stylo_styles) = root_node.primary_styles() else {
        panic!("Ignoring table because it has no styles");
    };

    let mut style = stylo_taffy::to_taffy_style(&stylo_styles);
    style.item_is_table = true;
    style.grid_auto_columns = Vec::new();
    style.grid_auto_rows = Vec::new();

    let is_fixed = match stylo_styles.clone_table_layout() {
        TableLayout::Fixed => true,
        TableLayout::Auto => false,
    };

    let border_collapse = stylo_styles.clone_border_collapse();
    let border_spacing = stylo_styles.clone_border_spacing();

    drop(stylo_styles);

    let mut column_sizes: Vec<taffy::Dimension> = Vec::new();
    let mut first_cell_border: Option<ServoArc<Border>> = None;
    for child_id in children.iter().copied() {
        collect_table_cells(
            dom,
            child_id,
            is_fixed,
            border_collapse,
            &mut row,
            &mut col,
            &mut items,
            &mut column_sizes,
            &mut first_cell_border,
        );
    }
    column_sizes.resize(col as usize, style_helpers::auto());

    style.grid_template_columns = column_sizes
        .into_iter()
        .map(|dim| TrackSizingFunction::from(dim).into())
        .collect();
    style.grid_template_rows = vec![style_helpers::auto(); row as usize];

    style.gap = match border_collapse {
        BorderCollapse::Separate => Size {
            width: length(border_spacing.0.width.px()),
            height: length(border_spacing.0.height.px()),
        },
        BorderCollapse::Collapse => first_cell_border
            .as_ref()
            .map(|border| {
                let x = border.border_left_width.0.max(border.border_right_width.0).to_f32_px();
                let y = border.border_top_width.0.max(border.border_bottom_width.0).to_f32_px();
                Size {
                    width: length(x),
                    height: length(y),
                }
            }).unwrap_or(Size::ZERO.map(length))
    };

    if border_collapse == BorderCollapse::Collapse {
        style.border = Rect {
            left: style.gap.width,
            right: style.gap.width,
            top: style.gap.height,
            bottom: style.gap.height,
        }
    }

    let layout_children = items
        .iter()
        .filter(|item| item.kind == TableItemKind::Cell)
        .map(|cell| cell.node_id)
        .collect();
    let root_node = &mut dom.nodes[table_root_node_id];
    root_node.children = children;

    (TableContext { style, items, computed_grid_info: AtomicRefCell::new(None), border_collapse, border_style: first_cell_border }, layout_children)
}

pub(crate) fn collect_table_cells(
    dom: &mut Dom,
    node_id: usize,
    is_fixed: bool,
    border_collapse: BorderCollapse,
    row: &mut u16,
    col: &mut u16,
    cells: &mut Vec<TableItem>,
    columns: &mut Vec<Dimension>,
    first_cell_border: &mut Option<ServoArc<Border>>,
) {
    let node = &dom.nodes[node_id];

    if !node.is_element() {
        return;
    }

    let Some(display) = node.primary_styles().map(|s| s.clone_display()) else {
        println!("Ignoring table descendent because it has no styles");
        return;
    };

    if display.outside() == DisplayOutside::None {
        node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
        return;
    }

    match display.inside() {
        DisplayInside::TableRowGroup
        | DisplayInside::TableHeaderGroup
        | DisplayInside::TableFooterGroup
        | DisplayInside::Contents => {
            let children = std::mem::take(&mut dom.nodes[node_id].children);
            for child_id in children.iter().copied() {
                dom.nodes[child_id]
                    .remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                collect_table_cells(dom, child_id, is_fixed, border_collapse, row, col, cells, columns, first_cell_border);
            }
            dom.nodes[node_id].children = children;
        }
        DisplayInside::TableRow => {
            node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            *row += 1;
            *col = 0;

            {
                let stylo_style = &node.primary_styles().unwrap();
                let mut style = stylo_taffy::to_taffy_style(stylo_style);
                style.grid_column = taffy::Line {
                    start: style_helpers::line(0),
                    end: style_helpers::line(-1),
                };
                style.grid_row = taffy::Line {
                    start: style_helpers::line(*row as i16),
                    end: style_helpers::span(1),
                };
                cells.push(TableItem {
                    kind: TableItemKind::Row,
                    node_id,
                    style,
                });
            }

            let children = std::mem::take(&mut dom.nodes[node_id].children);
            for child_id in children.iter().copied() {
                collect_table_cells(dom, child_id, is_fixed, border_collapse, row, col, cells, columns, first_cell_border);
            }
            dom.nodes[node_id].children = children;
        }
        DisplayInside::TableCell => {
            // node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            let stylo_style = &node.primary_styles().unwrap();
            let colspan: u16 = node
                .attr(local_name!("colspan"))
                .and_then(|val| val.parse().ok())
                .unwrap_or(1);
            let mut style = stylo_taffy::to_taffy_style(stylo_style);

            if first_cell_border.is_none() {
                *first_cell_border = Some(stylo_style.clone_border());
            }

            // TODO: account for padding/border/margin
            if *row == 1 {
                let column = match style.size.width.tag() {
                    taffy::CompactLength::LENGTH_TAG => {
                        let len = style.size.width.value();
                        let padding = style.padding.resolve_or_zero(None, resolve_calc_value);
                        length(len + padding.left + padding.right)
                    }
                    taffy::CompactLength::PERCENT_TAG => {
                        if is_fixed {
                            percent(style.size.width.value())
                        } else {
                            auto()
                        }
                    }
                    taffy::CompactLength::AUTO_TAG => auto(),
                    _ => unreachable!(),
                };
                columns.push(column);
            } else if !is_fixed
                && (*col as usize) < columns.len()
                && taffy::CompactLength::LENGTH_TAG == style.size.width.tag()
            {
                let new_len = style.size.width.value();
                let tag = columns[*col as usize].tag();
                let value = columns[*col as usize].value();
                columns[*col as usize] = match tag {
                    taffy::CompactLength::LENGTH_TAG => length(value.max(new_len)),
                    taffy::CompactLength::AUTO_TAG => length(new_len),
                    taffy::CompactLength::PERCENT_TAG => percent(value),
                    _ => unreachable!(),
                }
            }

            if border_collapse == BorderCollapse::Collapse {
                style.border = Rect::ZERO.map(length);
            }

            style.grid_column = taffy::Line {
                start: style_helpers::line((*col + 1) as i16),
                end: style_helpers::span(colspan),
            };
            style.grid_row = taffy::Line {
                start: style_helpers::line(*row as i16),
                end: style_helpers::span(1),
            };
            style.size.width = style_helpers::auto();
            cells.push(TableItem {
                kind: TableItemKind::Cell,
                node_id,
                style,
            });

            *col += colspan;
        }
        DisplayInside::Flow
        | DisplayInside::FlowRoot
        | DisplayInside::Flex
        | DisplayInside::Grid => {
            node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            // Probably a table caption: ignore
            // println!(
            //     "Warning: ignoring non-table typed descendent of table ({:?})",
            //     display.inside()
            // );
        }
        DisplayInside::TableColumnGroup | DisplayInside::TableColumn | DisplayInside::Table => {
            node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            //Ignore
        }
        DisplayInside::None => {
            node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            // Ignore
        }
    }
}

pub struct RangeIter(Range<usize>);

impl Iterator for RangeIter {
    type Item = taffy::NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(taffy::NodeId::from)
    }
}

impl taffy::TraversePartialTree for TableTreeWrapper<'_> {
    type ChildIter<'a>
        = RangeIter
    where
        Self: 'a;

    #[inline(always)]
    fn child_ids(&self, _node_id: taffy::NodeId) -> Self::ChildIter<'_> {
        RangeIter(0..self.ctx.items.len())
    }

    #[inline(always)]
    fn child_count(&self, node_id: taffy::NodeId) -> usize {
        self.ctx.items.len()
    }

    #[inline(always)]
    fn get_child_id(&self, _node_id: taffy::NodeId, index: usize) -> taffy::NodeId {
        index.into()
    }
}
impl taffy::TraverseTree for TableTreeWrapper<'_> {}

impl taffy::LayoutPartialTree for TableTreeWrapper<'_> {
    type CoreContainerStyle<'a>
        = &'a taffy::Style<Atom>
    where
        Self: 'a;

    type CustomIdent = Atom;

    fn get_core_container_style(&self, _node_id: taffy::NodeId) -> &taffy::Style<Atom> {
        &self.ctx.style
    }

    fn resolve_calc_value(&self, calc_ptr: *const (), parent_size: f32) -> f32 {
        resolve_calc_value(calc_ptr, parent_size)
    }

    fn set_unrounded_layout(&mut self, node_id: taffy::NodeId, layout: &taffy::Layout) {
        let node_id = taffy::NodeId::from(self.ctx.items[usize::from(node_id)].node_id);
        self.dom.set_unrounded_layout(node_id, layout)
    }

    fn compute_child_layout(
        &mut self,
        node_id: taffy::NodeId,
        inputs: taffy::tree::LayoutInput,
    ) -> taffy::LayoutOutput {
        let cell = &self.ctx.items[usize::from(node_id)];
        match cell.kind {
            TableItemKind::Row => {
                compute_leaf_layout(inputs, &cell.style, resolve_calc_value, |_, _| {
                    taffy::Size::ZERO
                })
            }
            TableItemKind::Cell => {
                let node_id = taffy::NodeId::from(cell.node_id);
                self.dom.compute_child_layout(node_id, inputs)
            }
        }
    }
}

impl taffy::LayoutGridContainer for TableTreeWrapper<'_> {
    type GridContainerStyle<'a>
        = &'a taffy::Style<Atom>
    where
        Self: 'a;

    type GridItemStyle<'a>
        = &'a taffy::Style<Atom>
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: taffy::NodeId) -> Self::GridContainerStyle<'_> {
        self.get_core_container_style(node_id)
    }

    fn get_grid_child_style(&self, child_node_id: taffy::NodeId) -> Self::GridItemStyle<'_> {
        &self.ctx.items[usize::from(child_node_id)].style
    }

    fn set_detailed_grid_info(&mut self, _node_id: NodeId, _detailed_grid_info: DetailedGridInfo) {
        *self.ctx.computed_grid_info.borrow_mut() = Some(_detailed_grid_info);
    }
}
