use std::cell::RefCell;
use std::sync::Arc;
use markup5ever::{ns, QualName};
use html5ever::local_name;
use parley::{FontContext, FontWeight, GenericFamily, InlineBox, InlineBoxKind, LayoutContext, LineHeight, StyleProperty, TextStyle, TreeBuilder, WhiteSpaceCollapse};
use style::selector_parser::RestyleDamage;
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{Content, ContentItem, Display, Float, PositionProperty};
use style::values::computed::font::SingleFontFamily;
use style::values::specified::text::TextTransformCase;
use style::values::specified::TextDecorationLine;
use log::log;
use slab::Slab;
use style::values::specified::box_::{DisplayInside, DisplayOutside};
use style::data::ElementData as StyloElementData;
use style::shared_lock::StylesheetGuards;
use crate::dom::damage::{ALL_DAMAGE, CONSTRUCT_BOX, CONSTRUCT_DESCENDENT, CONSTRUCT_FC};
use crate::dom::{stylo_to_parley, AttributeMap, Dom, DomNode, ElementData, NodeData};
use crate::dom::node::{DomNodeFlags, NodeKind, SpecialElementData, TextLayout};
use crate::layout::table::build_table_context;
use crate::networking::parse_svg;
use crate::qual_name;
use crate::ui::TextBrush;

thread_local! {
    pub static LAYOUT_CTX: RefCell<Option<Box<LayoutContext<TextBrush>>>> = const { RefCell::new(None) };
    pub static FONT_CTX: RefCell<Option<Box<FontContext>>> = const { RefCell::new(None) };
}

#[macro_export]
macro_rules! qual_name {
    ($local:tt $(, $ns:ident)?) => {
        markup5ever::interface::QualName {
            prefix: None,
            ns: ns!($($ns)?),
            local: local_name!($local),
        }
    };
}

const DUMMY_NAME: QualName = qual_name!("div", html);

fn push_children_and_pseudos(layout_children: &mut Vec<usize>, node: &DomNode) {
    if let Some(before) = node.before {
        layout_children.push(before);
    }
    layout_children.extend_from_slice(&node.children);
    if let Some(after) = node.after {
        layout_children.push(after);
    }
}

fn push_non_whitespace_children_and_pseudos(layout_children: &mut Vec<usize>, node: &DomNode) {
    if let Some(before) = node.before {
        layout_children.push(before);
    }
    layout_children.extend(
        node.children
            .iter()
            .copied()
            .filter(|child_id| !node.get_node(*child_id).is_whitespace_node()),
    );
    if let Some(after) = node.after {
        layout_children.push(after);
    }
}

pub(crate) fn collect_layout_children(
    dom: &mut Dom,
    node_id: usize,
    layout_children: &mut Vec<usize>,
    anonymous_block_id: &mut Option<usize>
) {
    // Rset construction flags
    dom.nodes[node_id]
        .flags
        .reset_reconstruction_flags();
    if let Some(element_data) = dom.nodes[node_id].element_data_mut() {
        element_data.take_inline_layout();
    }

    flush_pseudo_elements(dom, node_id);

    if let Some(element_data) = dom.nodes[node_id].data.element() {
        // TODO handle text input
        let tag_name = element_data.name.local.as_ref();

        if matches!(tag_name, "svg") {
            let mut outer_html = dom.get_node(node_id).unwrap().outer_html();

            if !outer_html.contains("xmlns") {
                outer_html = outer_html.replace("<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"");
            }

            // TODO Remove construction damage from subtree

            match parse_svg(outer_html.as_bytes()) {
                Ok(svg) => {
                    let special_data = &mut dom.get_node_mut(node_id)
                        .unwrap()
                        .element_data_mut()
                        .unwrap()
                        .special_data;

                    match special_data {
                        SpecialElementData::Image(data) => *data = Box::new(svg.into()),
                        _ => {
                            log::error!("SVG element does not have image special data");
                        }
                    }
                }
                Err(e) => {
                    println!("{node_id} SVG parse failed");
                    print!("{outer_html}");

                    log::error!("Failed to parse inline SVG: {}", e);
                }
            }
        }

        // TODO collect list item children
    }

    // Skip further construction if the node has no children or pseudo-children
    {
        let node = &dom.nodes[node_id];
        if node.children.is_empty() && node.before.is_none() && node.after.is_none() {
            return;
        }
    }

    let display = dom.nodes[node_id].display_style().unwrap_or(
        match dom.nodes[node_id].data.kind() {
            NodeKind::AnonymousBlock => Display::Block,
            _ => Display::Inline,
        },
    );

    match display.inside() {
        DisplayInside::None => {},
        DisplayInside::Contents => {
            dom.nodes[node_id].remove_damage(CONSTRUCT_BOX | CONSTRUCT_DESCENDENT | CONSTRUCT_FC);
            let children = std::mem::take(&mut dom.nodes[node_id].children);

            for child_id in children.iter().copied() {
                collect_layout_children(dom, child_id, layout_children, anonymous_block_id)
            }

            dom.nodes[node_id].children = children;
        }
        DisplayInside::Flow | DisplayInside::FlowRoot | DisplayInside::TableCell => {
            // TODO: make "all_inline" detection work in the presence of display:contents nodes
            let mut all_block = true;
            let mut all_inline = true;
            let mut all_out_of_flow = true;
            let mut has_contents = false;
            for child in dom.nodes[node_id]
                .children
                .iter()
                .copied()
                .map(|child_id| &dom.nodes[child_id])
            {
                // Unwraps on Text and SVG nodes
                let style = child.primary_styles();
                let style = style.as_ref();
                let display = style
                    .map(|s| s.clone_display())
                    .unwrap_or(Display::inline());
                if matches!(display.inside(), DisplayInside::Contents) {
                    has_contents = true;
                    all_out_of_flow = false;
                } else {
                    let position = style
                        .map(|s| s.clone_position())
                        .unwrap_or(PositionProperty::Static);
                    let float = style.map(|s| s.clone_float()).unwrap_or(Float::None);

                    // Ignore nodes that are entirely whitespace
                    if child.is_whitespace_node() {
                        continue;
                    }

                    let is_in_flow = matches!(
                        position,
                        PositionProperty::Static
                            | PositionProperty::Relative
                            | PositionProperty::Sticky
                    ) && matches!(float, Float::None);

                    if !is_in_flow {
                        continue;
                    }

                    all_out_of_flow = false;
                    match display.outside() {
                        DisplayOutside::None => {}
                        DisplayOutside::Block
                        | DisplayOutside::TableCaption
                        | DisplayOutside::InternalTable => all_inline = false,
                        DisplayOutside::Inline => {
                            all_block = false;

                            // We need the "complex" tree fixing when an inline contains a block
                            if child.is_or_contains_block() {
                                all_inline = false;
                            }
                        }
                    }
                }
            }

            if all_out_of_flow {
                return push_non_whitespace_children_and_pseudos(
                    layout_children,
                    &dom.nodes[node_id],
                );
            }

            // TODO: fix display:contents
            if all_inline {
                let existing_layout = dom.nodes[node_id]
                    .element_data_mut()
                    .and_then(|el| el.inline_layout_data.take());
                let mut layout = existing_layout.unwrap_or_else(|| Box::new(TextLayout::new()));

                dom.nodes[node_id]
                    .flags
                    .insert(DomNodeFlags::IS_INLINE_ROOT);
                find_inline_layout_embedded_boxes(dom, node_id, layout_children);

                let mut layout_ctx = LAYOUT_CTX.take().unwrap_or_else(|| Box::new(LayoutContext::new()));

                let mut font_ctx = FONT_CTX.take().unwrap_or_else(|| Box::new(FontContext::new()));

                // Build the inline layout with text content
                build_inline_layout(&*dom.nodes, &mut *layout_ctx, &mut *font_ctx, &mut layout, dom.viewport.scale(), node_id);

                LAYOUT_CTX.set(Some(layout_ctx));
                FONT_CTX.set(Some(font_ctx));

                dom.nodes[node_id].element_data_mut().unwrap().inline_layout_data = Some(layout);
                return;
            }

            // If the children are either all inline or all block then simply return the regular children
            // as the layout children
            if all_block & !has_contents {
                return push_non_whitespace_children_and_pseudos(
                    layout_children,
                    &dom.nodes[node_id],
                );
            } else if all_inline & !has_contents {
                return push_children_and_pseudos(layout_children, &dom.nodes[node_id]);
            }

            fn block_item_needs_wrap(
                child_node_kind: NodeKind,
                display_outside: DisplayOutside,
            ) -> bool {
                child_node_kind == NodeKind::Text || display_outside == DisplayOutside::Inline
            }

            collect_complex_layout_children(
                dom,
                node_id,
                layout_children,
                anonymous_block_id,
                false,
                block_item_needs_wrap,
            );
        }
        DisplayInside::Flex | DisplayInside::Grid => {
            let has_text_node_or_contents = dom.nodes[node_id]
                .children
                .iter()
                .copied()
                .map(|child_id| &dom.nodes[child_id])
                .any(|child| {
                    let display = child.display_style().unwrap_or(Display::inline());
                    let node_kind = child.data.kind();
                    display.inside() == DisplayInside::Contents || node_kind == NodeKind::Text
                });

            if !has_text_node_or_contents {
                return push_non_whitespace_children_and_pseudos(
                    layout_children,
                    &dom.nodes[node_id],
                );
            }

            fn flex_or_grid_item_needs_wrap(
                child_node_kind: NodeKind,
                _display_outside: DisplayOutside,
            ) -> bool {
                child_node_kind == NodeKind::Text
            }
            collect_complex_layout_children(
                dom,
                node_id,
                layout_children,
                anonymous_block_id,
                true,
                flex_or_grid_item_needs_wrap,
            );
        }
        DisplayInside::Table => {
            let (table_context, tlayout_children) = build_table_context(dom, node_id);
            let data = SpecialElementData::TableRoot(Arc::new(table_context));
            dom.nodes[node_id].flags.insert(DomNodeFlags::IS_TABLE_ROOT);
            dom.nodes[node_id].data.element_mut().unwrap().special_data = data;
            if let Some(before) = dom.nodes[node_id].before {
                layout_children.push(before)
            };
            layout_children.extend_from_slice(&tlayout_children);
            if let Some(after) = dom.nodes[node_id].after {
                layout_children.push(after);
            }
        }
        _ => {
            push_non_whitespace_children_and_pseudos(
                layout_children,
                &dom.nodes[node_id]
            );
        }
    }
}

fn flush_pseudo_elements(dom: &mut Dom, node_id: usize) {
    let (before_style, after_style, before_node_id, after_node_id) = {
        let node = &dom.nodes[node_id];

        let before_node_id = node.before;
        let after_node_id = node.after;

        // Note: yes these are kinda backwards
        let style_data = node.stylo_data.borrow();
        let before_style = style_data
            .as_ref()
            .and_then(|d| d.styles.pseudos.as_array()[1].clone());
        let after_style = style_data
            .as_ref()
            .and_then(|d| d.styles.pseudos.as_array()[0].clone());

        (before_style, after_style, before_node_id, after_node_id)
    };

    // Sync pseudo element
    for (idx, pe_style, pe_node_id) in [
        (1, before_style, before_node_id),
        (0, after_style, after_node_id),
    ] {
        // Delete psuedo element if it exists but shouldn't
        if let (Some(pe_node_id), None) = (pe_node_id, &pe_style) {
            dom.remove_and_drop_pe(pe_node_id);
            let node = &mut dom.nodes[node_id];
            node.set_pe_by_index(idx, None);
            node.insert_damage(ALL_DAMAGE);
        }

        // Create pseudo element if it should exist but doesn't
        if let (None, Some(pe_style)) = (pe_node_id, &pe_style) {
            let new_node_id = dom.create_node(NodeData::AnonymousBlock(ElementData::new(
                DUMMY_NAME,
                AttributeMap::empty(),
            )));
            dom.nodes[new_node_id].parent = Some(node_id);
            dom.nodes[new_node_id].layout_parent.set(Some(node_id));

            let content = &pe_style.as_ref().get_counters().content;
            if let Content::Items(item_data) = content {
                let items = &item_data.items[0..item_data.alt_start];
                match &items[0] {
                    ContentItem::String(owned_str) => {
                        // create text node
                    }
                    _ => {
                        // TODO: other types of content
                    }
                }
            }

            let mut element_data = style::data::ElementData::default();
            element_data.styles.primary = Some(pe_style.clone());
            element_data.set_restyled();
            element_data.damage = RestyleDamage::all();
            *dom.nodes[new_node_id].stylo_data.borrow_mut() = Some(element_data);

            let node = &mut dom.nodes[node_id];
            node.set_pe_by_index(idx, Some(new_node_id));
            node.insert_damage(ALL_DAMAGE);
        }

        // Else: Update psuedo element
        if let (Some(pe_node_id), Some(pe_style)) = (pe_node_id, pe_style) {
            // TODO: Update content

            let mut node_styles = dom.nodes[pe_node_id].stylo_data.borrow_mut();
            let node_styles = &mut node_styles.as_mut().unwrap();
            node_styles.damage.insert(RestyleDamage::all());
            let primary_styles = &mut node_styles.styles.primary;

            if !std::ptr::eq(&**primary_styles.as_ref().unwrap(), &*pe_style) {
                *primary_styles = Some(pe_style);
                node_styles.set_restyled();
            }
        }
    }
}

fn collect_complex_layout_children(
    doc: &mut Dom,
    container_node_id: usize,
    layout_children: &mut Vec<usize>,
    anonymous_block_id: &mut Option<usize>,
    hide_whitespace: bool,
    needs_wrap: impl Fn(NodeKind, DisplayOutside) -> bool,
) {
    fn block_is_only_whitespace(doc: &Dom, node_id: usize) -> bool {
        for child_id in doc.nodes[node_id].children.iter().copied() {
            let child = &doc.nodes[child_id];
            if !child.is_whitespace_node() {
                return false;
            }
        }

        true
    }

    doc.iter_children_and_pseudos_mut(container_node_id, |child_id, doc| {
        // Get node kind (text, element, comment, etc)
        let child_node_kind = doc.nodes[child_id].data.kind();

        // Get Display style. Default to inline because nodes without styles are probably text nodes
        let contains_block = doc.nodes[child_id].is_or_contains_block();
        let child_display = &doc.nodes[child_id]
            .display_style()
            .unwrap_or(Display::inline());
        let display_inside = child_display.inside();
        let display_outside = if contains_block {
            DisplayOutside::Block
        } else {
            child_display.outside()
        };

        let is_whitespace_node = doc.nodes[child_id].is_whitespace_node();

        // Skip comment nodes. Note that we do *not* skip `Display::None` nodes as they may need to be hidden.
        // Taffy knows how to deal with `Display::None` children.
        //
        // Also hide all-whitespace flexbox children as these should be ignored
        if child_node_kind == NodeKind::Comment || (hide_whitespace && is_whitespace_node) {
            // return;
        }
        // Recurse into `Display::Contents` nodes
        else if display_inside == DisplayInside::Contents {
            collect_layout_children(doc, child_id, layout_children, anonymous_block_id)
        }
        // Push nodes that need wrapping into the current "anonymous block container".
        // If there is not an open one then we create one.
        else if needs_wrap(child_node_kind, display_outside) {
            use style::selector_parser::PseudoElement;

            if anonymous_block_id.is_none() {
                const NAME: QualName = QualName {
                    prefix: None,
                    ns: ns!(html),
                    local: local_name!("div"),
                };
                let node_id =
                    doc.create_node(NodeData::AnonymousBlock(ElementData::new(NAME, AttributeMap::new(Vec::new()))));

                // Set style data
                let parent_style = doc.nodes[container_node_id].primary_styles().unwrap();
                let read_guard = doc.lock.read();
                let guards = StylesheetGuards::same(&read_guard);
                let style = doc.stylist.style_for_anonymous::<&DomNode>(
                    &guards,
                    &PseudoElement::ServoAnonymousBox,
                    &parent_style,
                );
                let mut stylo_element_data = StyloElementData {
                    damage: ALL_DAMAGE,
                    ..Default::default()
                };
                drop(parent_style);

                stylo_element_data.styles.primary = Some(style);
                stylo_element_data.set_restyled();
                *doc.nodes[node_id].stylo_data.borrow_mut() = Some(stylo_element_data);
                doc.nodes[node_id].parent = Some(container_node_id);
                doc.nodes[node_id]
                    .layout_parent
                    .set(Some(container_node_id));

                layout_children.push(node_id);
                *anonymous_block_id = Some(node_id);
            }

            doc.nodes[anonymous_block_id.unwrap()]
                .children
                .push(child_id);
        }
        // Else push the child directly (and close any open "anonymous block container")
        else {
            // If anonymous block node only contains whitespace then delete it
            if let Some(anon_id) = *anonymous_block_id {
                if block_is_only_whitespace(doc, anon_id) {
                    layout_children.pop();
                    doc.nodes.remove(anon_id);
                    *anonymous_block_id = None;
                }
            }

            *anonymous_block_id = None;
            layout_children.push(child_id);
        }
    });

    // If anonymous block node only contains whitespace then delete it
    if let Some(anon_id) = *anonymous_block_id {
        if block_is_only_whitespace(doc, anon_id) {
            layout_children.pop();
            doc.nodes.remove(anon_id);
            *anonymous_block_id = None;
        }
    }
}

pub(crate) fn find_inline_layout_embedded_boxes(
    doc: &mut Dom,
    inline_context_root_node_id: usize,
    layout_children: &mut Vec<usize>,
) {
    flush_inline_pseudos_recursive(doc, inline_context_root_node_id);

    let root_node = &doc.nodes[inline_context_root_node_id];
    if let Some(before_id) = root_node.before {
        find_inline_layout_embedded_boxes_recursive(
            &doc.nodes,
            inline_context_root_node_id,
            before_id,
            layout_children,
        );
    }
    for child_id in root_node.children.iter().copied() {
        find_inline_layout_embedded_boxes_recursive(
            &doc.nodes,
            inline_context_root_node_id,
            child_id,
            layout_children,
        );
    }
    if let Some(after_id) = root_node.after {
        find_inline_layout_embedded_boxes_recursive(
            &doc.nodes,
            inline_context_root_node_id,
            after_id,
            layout_children,
        );
    }

    fn flush_inline_pseudos_recursive(doc: &mut Dom, node_id: usize) {
        doc.iter_children_mut(node_id, |child_id, doc| {
            flush_pseudo_elements(doc, child_id);
            let display = doc.nodes[node_id]
                .display_style()
                .unwrap_or(Display::inline());
            let do_recurse = match (display.outside(), display.inside()) {
                (DisplayOutside::None, DisplayInside::Contents) => true,
                (DisplayOutside::Inline, DisplayInside::Flow) => true,
                (_, _) => false,
            };
            if do_recurse {
                flush_inline_pseudos_recursive(doc, child_id);
            }
        });
    }

    fn find_inline_layout_embedded_boxes_recursive(
        nodes: &Slab<DomNode>,
        parent_id: usize,
        node_id: usize,
        layout_children: &mut Vec<usize>,
    ) {
        let node = &nodes[node_id];

        // Set layout_parent for node.
        node.layout_parent.set(Some(parent_id));

        match &node.data {
            NodeData::Element(element_data) | NodeData::AnonymousBlock(element_data) => {
                // if the input type is hidden, hide it
                if *element_data.name.local == *"input" {
                    if let Some("hidden") = element_data.attr(local_name!("type")) {
                        return;
                    }
                }

                let display = node.display_style().unwrap_or(Display::inline());

                match (display.outside(), display.inside()) {
                    (DisplayOutside::None, DisplayInside::None) => {
                        node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                    }
                    (DisplayOutside::None, DisplayInside::Contents) => {
                        for child_id in node.children.iter().copied() {
                            node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                            find_inline_layout_embedded_boxes_recursive(
                                nodes,
                                parent_id,
                                child_id,
                                layout_children,
                            );
                        }
                    }
                    (DisplayOutside::Inline, DisplayInside::Flow) => {
                        let tag_name = &element_data.name.local;

                        if *tag_name == local_name!("img")
                            || *tag_name == local_name!("svg")
                            || *tag_name == local_name!("input")
                            || *tag_name == local_name!("textarea")
                            || *tag_name == local_name!("button")
                        {
                            layout_children.push(node_id);
                        } else if *tag_name == local_name!("br") {
                            node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                        } else {
                            node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);

                            if let Some(before_id) = node.before {
                                find_inline_layout_embedded_boxes_recursive(
                                    nodes,
                                    node_id,
                                    before_id,
                                    layout_children,
                                );
                            }
                            for child_id in node.children.iter().copied() {
                                find_inline_layout_embedded_boxes_recursive(
                                    nodes,
                                    node_id,
                                    child_id,
                                    layout_children,
                                );
                            }
                            if let Some(after_id) = node.after {
                                find_inline_layout_embedded_boxes_recursive(
                                    nodes,
                                    node_id,
                                    after_id,
                                    layout_children,
                                );
                            }
                        }
                    }
                    // Inline box
                    (_, _) => {
                        layout_children.push(node_id);
                    }
                };
            }
            NodeData::Comment { .. } | NodeData::Text { .. } => {
                node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            }
            NodeData::Document => unreachable!(),
        }
    }
}

// Copyright DioxusLabs
// Licensed under the Apache License, Version 2.0 or the MIT license.

fn resolve_line_height(line_height: parley::LineHeight, font_size: f32) -> f32 {
    match line_height {
        parley::LineHeight::FontSizeRelative(relative) => relative * font_size,
        parley::LineHeight::Absolute(absolute) => absolute,
        parley::LineHeight::MetricsRelative(relative) => relative * font_size, //unreachable!(),
    }
}

/// Build the inline layout for a node that is an inline root.
/// This traverses the inline children and builds the parley layout with proper text and styles.
pub(crate) fn build_inline_layout(
    nodes: &Slab<DomNode>,
    layout_ctx: &mut LayoutContext<TextBrush>,
    font_ctx: &mut FontContext,
    text_layout: &mut TextLayout,
    scale: f32,
    inline_context_root_node_id: usize,
) {
    // Get the inline context's root node's text styles
    let root_node = &nodes[inline_context_root_node_id];
    let root_node_style = root_node.primary_styles().or_else(|| {
        root_node
            .parent
            .and_then(|parent_id| nodes[parent_id].primary_styles())
    });

    let parley_style = root_node_style
        .as_ref()
        .map(|s| stylo_to_parley::style(inline_context_root_node_id, s))
        .unwrap_or_default();

    let root_line_height = resolve_line_height(parley_style.line_height, parley_style.font_size);

    // Create a parley tree builder
    let mut builder = layout_ctx.tree_builder(font_ctx, scale, true, &parley_style);

    // Set whitespace collapsing mode
    let collapse_mode = root_node_style
        .map(|s| s.get_inherited_text().white_space_collapse)
        .map(stylo_to_parley::white_space_collapse)
        .unwrap_or(WhiteSpaceCollapse::Collapse);
    builder.set_white_space_mode(collapse_mode);

    // FIXME Render position-inside list items, markers

    if let Some(before_id) = root_node.before {
        build_inline_layout_recursive(
            &mut builder,
            nodes,
            inline_context_root_node_id,
            before_id,
            collapse_mode,
            root_line_height,
        );
    }
    for child_id in root_node.children.iter().copied() {
        build_inline_layout_recursive(
            &mut builder,
            nodes,
            inline_context_root_node_id,
            child_id,
            collapse_mode,
            root_line_height,
        );
    }
    if let Some(after_id) = root_node.after {
        build_inline_layout_recursive(
            &mut builder,
            nodes,
            inline_context_root_node_id,
            after_id,
            collapse_mode,
            root_line_height,
        );
    }

    text_layout.text = builder.build_into(&mut text_layout.layout);
    return;

    fn build_inline_layout_recursive(
        builder: &mut TreeBuilder<TextBrush>,
        nodes: &Slab<DomNode>,
        parent_id: usize,
        node_id: usize,
        collapse_mode: WhiteSpaceCollapse,
        root_line_height: f32,
    ) {
        let node = &nodes[node_id];

        // Set layout_parent for node.
        node.layout_parent.set(Some(parent_id));

        let style = node.primary_styles();
        let style = style.as_ref();

        // Set whitespace collapsing mode
        let collapse_mode = style
            .map(|s| s.clone_white_space_collapse())
            .map(stylo_to_parley::white_space_collapse)
            .unwrap_or(collapse_mode);
        builder.set_white_space_mode(collapse_mode);

        match &node.data {
            NodeData::Element(element_data) | NodeData::AnonymousBlock(element_data) => {
                // if the input type is hidden, hide it
                if *element_data.name.local == *"input" {
                    if let Some("hidden") = element_data.attr(local_name!("type")) {
                        return;
                    }
                }

                let display = node.display_style().unwrap_or(Display::inline());
                let position = style
                    .map(|s| s.clone_position())
                    .unwrap_or(PositionProperty::Static);
                let float = style.map(|s| s.clone_float()).unwrap_or(Float::None);
                let box_kind = if position.is_absolutely_positioned() {
                    InlineBoxKind::OutOfFlow
                } else if float.is_floating() {
                    InlineBoxKind::CustomOutOfFlow
                } else {
                    InlineBoxKind::InFlow
                };

                match (display.outside(), display.inside()) {
                    (DisplayOutside::None, DisplayInside::None) => {
                        // node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                    }
                    (DisplayOutside::None, DisplayInside::Contents) => {
                        for child_id in node.children.iter().copied() {
                            // node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                            build_inline_layout_recursive(
                                builder,
                                nodes,
                                parent_id,
                                child_id,
                                collapse_mode,
                                root_line_height,
                            );
                        }
                    }
                    (DisplayOutside::Inline, DisplayInside::Flow) => {
                        let tag_name = &element_data.name.local;

                        if *tag_name == local_name!("img")
                            || *tag_name == local_name!("svg")
                            || *tag_name == local_name!("input")
                            || *tag_name == local_name!("textarea")
                            || *tag_name == local_name!("button")
                        {
                            builder.push_inline_box(InlineBox {
                                id: node_id as u64,
                                kind: box_kind,
                                // Overridden by push_inline_box method
                                index: 0,
                                // Width and height are set during layout
                                width: 0.0,
                                height: 0.0,
                            });
                        } else if *tag_name == local_name!("br") {
                            // node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                            // TODO: update span id for br spans
                            builder.push_style_modification_span(&[]);
                            builder.set_white_space_mode(WhiteSpaceCollapse::Preserve);
                            builder.push_text("\n");
                            builder.pop_style_span();
                            builder.set_white_space_mode(collapse_mode);
                        } else {
                            // node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                            let mut style = node
                                .primary_styles()
                                .map(|s| stylo_to_parley::style(node.id, &s))
                                .unwrap_or_default();

                            // dbg!(&style);

                            let font_size = style.font_size;

                            // Floor the line-height of the span by the line-height of the inline context
                            // See https://www.w3.org/TR/CSS21/visudet.html#line-height
                            style.line_height = parley::LineHeight::Absolute(
                                resolve_line_height(style.line_height, font_size)
                                    .max(root_line_height),
                            );

                            // dbg!(node_id);
                            // dbg!(&style);

                            builder.push_style_span(style);

                            if let Some(before_id) = node.before {
                                build_inline_layout_recursive(
                                    builder,
                                    nodes,
                                    node_id,
                                    before_id,
                                    collapse_mode,
                                    root_line_height,
                                );
                            }

                            for child_id in node.children.iter().copied() {
                                build_inline_layout_recursive(
                                    builder,
                                    nodes,
                                    node_id,
                                    child_id,
                                    collapse_mode,
                                    root_line_height,
                                );
                            }
                            if let Some(after_id) = node.after {
                                build_inline_layout_recursive(
                                    builder,
                                    nodes,
                                    node_id,
                                    after_id,
                                    collapse_mode,
                                    root_line_height,
                                );
                            }

                            builder.pop_style_span();
                        }
                    }
                    // Inline box
                    (_, _) => {
                        builder.push_inline_box(InlineBox {
                            id: node_id as u64,
                            kind: box_kind,
                            // Overridden by push_inline_box method
                            index: 0,
                            // Width and height are set during layout
                            width: 0.0,
                            height: 0.0,
                        });
                    }
                };
            }
            NodeData::Text { contents } => {
                // node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
                // dbg!(&data.content);
                builder.push_text(&contents.borrow().to_string());
            }
            NodeData::Comment { contents: _ } => {
                // node.remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            }
            NodeData::Document => unreachable!(),
        }
    }
}

