use markup5ever::{ns, QualName};
use html5ever::local_name;
use parley::{FontWeight, GenericFamily, InlineBox, LineHeight, StyleProperty, TextStyle};
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
use crate::dom::{AttributeMap, Dom, DomNode, ElementData, NodeData};
use crate::dom::node::{DomNodeFlags, NodeKind, SpecialElementData, TextLayout};
use crate::networking::parse_svg;
use crate::qual_name;
use crate::ui::TextBrush;

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

                // Build the inline layout with text content
                build_inline_layout(dom, node_id, &mut layout);

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
            push_children_and_pseudos(layout_children, &dom.nodes[node_id]);
        }
        _ => {
            push_children_and_pseudos(layout_children, &dom.nodes[node_id]);
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

/// Build the inline layout for a node that is an inline root.
/// This traverses the inline children and builds the parley layout with proper text and styles.
pub(crate) fn build_inline_layout(dom: &mut Dom, node_id: usize, layout: &mut TextLayout) {
    let scale = dom.viewport.scale();

    // Get the root style for the inline container
    let root_style = dom.nodes[node_id].primary_styles();
    let root_style = match root_style {
        Some(s) => s,
        None => return,
    };

    // Extract root font properties
    let font = root_style.get_font();
    let font_size = font.font_size.computed_size().px();
    let font_weight_val = font.font_weight.value();
    let line_height_val = &font.line_height;

    let line_height = match line_height_val {
        style::values::computed::font::LineHeight::Normal => LineHeight::FontSizeRelative(1.2),
        style::values::computed::font::LineHeight::Number(num) => LineHeight::FontSizeRelative(num.0),
        style::values::computed::font::LineHeight::Length(len) => LineHeight::Absolute(len.px()),
    };

    // Get text brush for root node
    let root_brush = TextBrush::from_id(node_id);

    // Create root text style
    let root_text_style = TextStyle {
        font_stack: parley::FontStack::Source(std::borrow::Cow::Borrowed("sans-serif")),
        font_size,
        font_weight: FontWeight::new(font_weight_val),
        line_height,
        brush: root_brush,
        ..Default::default()
    };

    // Get font and layout contexts
    let mut font_ctx = dom.font_ctx.lock().unwrap();
    let mut layout_ctx = dom.layout_ctx.lock().unwrap();

    // Create tree builder
    let mut builder = layout_ctx.tree_builder(
        &mut font_ctx,
        scale,
        true, // quantize
        &root_text_style,
    );

    // Collect inline boxes from layout_children
    let layout_children = dom.nodes[node_id].layout_children.borrow();
    let inline_box_ids: Vec<usize> = layout_children.as_ref().map(|c| c.clone()).unwrap_or_default();
    drop(layout_children);
    drop(root_style);

    // Track inline boxes that need to be added
    let mut pending_inline_boxes: Vec<(usize, usize)> = Vec::new(); // (text_index, node_id)

    // Recursively traverse the inline children and build the layout
    fn traverse_inline_children(
        dom: &Dom,
        node_id: usize,
        builder: &mut parley::TreeBuilder<'_, TextBrush>,
        inline_box_ids: &[usize],
        pending_inline_boxes: &mut Vec<(usize, usize)>,
        text_len: &mut usize,
        scale: f32,
    ) {
        let node = &dom.nodes[node_id];

        // Process pseudo-elements and children
        if let Some(before_id) = node.before {
            traverse_inline_node(dom, before_id, builder, inline_box_ids, pending_inline_boxes, text_len, scale);
        }

        for &child_id in &node.children {
            traverse_inline_node(dom, child_id, builder, inline_box_ids, pending_inline_boxes, text_len, scale);
        }

        if let Some(after_id) = node.after {
            traverse_inline_node(dom, after_id, builder, inline_box_ids, pending_inline_boxes, text_len, scale);
        }
    }

    fn traverse_inline_node(
        dom: &Dom,
        node_id: usize,
        builder: &mut parley::TreeBuilder<'_, TextBrush>,
        inline_box_ids: &[usize],
        pending_inline_boxes: &mut Vec<(usize, usize)>,
        text_len: &mut usize,
        scale: f32,
    ) {
        let node = &dom.nodes[node_id];

        match &node.data {
            NodeData::Text { contents } => {
                let text = contents.borrow();

                // Apply text transformation if needed
                let style = node.primary_styles();
                let transformed_text = if let Some(style) = &style {
                    let inherited_text = style.get_inherited_text();
                    let text_transform = inherited_text.text_transform.case();
                    match text_transform {
                        TextTransformCase::None => text.to_string(),
                        TextTransformCase::Uppercase => text.chars().flat_map(|c| c.to_uppercase()).collect(),
                        TextTransformCase::Lowercase => text.chars().flat_map(|c| c.to_lowercase()).collect(),
                        TextTransformCase::Capitalize => {
                            let mut capitalize_next = true;
                            text.chars()
                                .map(|c| {
                                    if c.is_whitespace() {
                                        capitalize_next = true;
                                        c
                                    } else if capitalize_next {
                                        capitalize_next = false;
                                        c.to_uppercase().next().unwrap_or(c)
                                    } else {
                                        c
                                    }
                                })
                                .collect()
                        }
                    }
                } else {
                    text.to_string()
                };

                if !transformed_text.is_empty() {
                    // Get the parent's style for text properties
                    if let Some(parent_id) = node.parent {
                        if let Some(parent_style) = dom.nodes[parent_id].primary_styles() {
                            let font = parent_style.get_font();
                            let text_styles = parent_style.get_text();

                            let font_size = font.font_size.computed_size().px();
                            let font_weight_val = font.font_weight.value();

                            let line_height = match &font.line_height {
                                style::values::computed::font::LineHeight::Normal => LineHeight::FontSizeRelative(1.2),
                                style::values::computed::font::LineHeight::Number(num) => LineHeight::FontSizeRelative(num.0),
                                style::values::computed::font::LineHeight::Length(len) => LineHeight::Absolute(len.px()),
                            };

                            // Check for text decorations
                            let text_decoration_line = text_styles.text_decoration_line;
                            let has_underline = text_decoration_line.contains(TextDecorationLine::UNDERLINE);
                            let has_strikethrough = text_decoration_line.contains(TextDecorationLine::LINE_THROUGH);

                            let text_style = TextStyle {
                                font_stack: parley::FontStack::Source(std::borrow::Cow::Borrowed("sans-serif")),
                                font_size,
                                font_weight: FontWeight::new(font_weight_val),
                                line_height,
                                brush: TextBrush::from_id(parent_id),
                                has_underline,
                                has_strikethrough,
                                ..Default::default()
                            };

                            builder.push_style_span(text_style);
                            builder.push_text(&transformed_text);
                            *text_len += transformed_text.len();
                            builder.pop_style_span();
                        } else {
                            builder.push_text(&transformed_text);
                            *text_len += transformed_text.len();
                        }
                    } else {
                        builder.push_text(&transformed_text);
                        *text_len += transformed_text.len();
                    }
                }
            }
            NodeData::Element(element_data) | NodeData::AnonymousBlock(element_data) => {
                let tag_name = &element_data.name.local;

                // Handle line break
                if *tag_name == local_name!("br") {
                    builder.push_text("\n");
                    *text_len += 1;
                    return;
                }

                // Check if this is an inline box (img, svg, input, textarea, button, or block-like elements)
                let is_inline_box = inline_box_ids.contains(&node_id);

                if is_inline_box {
                    // This is an inline box - add it to pending list
                    pending_inline_boxes.push((*text_len, node_id));

                    // Push inline box into the builder
                    builder.push_inline_box(InlineBox {
                        id: node_id as u64,
                        index: *text_len,
                        width: 0.0, // Will be computed during layout
                        height: 0.0, // Will be computed during layout
                    });
                } else {
                    // Regular inline element - push style span and recurse
                    if let Some(style) = node.primary_styles() {
                        let font = style.get_font();
                        let text_styles = style.get_text();

                        let font_size = font.font_size.computed_size().px();
                        let font_weight_val = font.font_weight.value();

                        let line_height = match &font.line_height {
                            style::values::computed::font::LineHeight::Normal => LineHeight::FontSizeRelative(1.2),
                            style::values::computed::font::LineHeight::Number(num) => LineHeight::FontSizeRelative(num.0),
                            style::values::computed::font::LineHeight::Length(len) => LineHeight::Absolute(len.px()),
                        };

                        let text_decoration_line = text_styles.text_decoration_line;
                        let has_underline = text_decoration_line.contains(TextDecorationLine::UNDERLINE);
                        let has_strikethrough = text_decoration_line.contains(TextDecorationLine::LINE_THROUGH);

                        let text_style = TextStyle {
                            font_stack: parley::FontStack::Source(std::borrow::Cow::Borrowed("sans-serif")),
                            font_size,
                            font_weight: FontWeight::new(font_weight_val),
                            line_height,
                            brush: TextBrush::from_id(node_id),
                            has_underline,
                            has_strikethrough,
                            ..Default::default()
                        };

                        builder.push_style_span(text_style);
                        traverse_inline_children(dom, node_id, builder, inline_box_ids, pending_inline_boxes, text_len, scale);
                        builder.pop_style_span();
                    } else {
                        traverse_inline_children(dom, node_id, builder, inline_box_ids, pending_inline_boxes, text_len, scale);
                    }
                }
            }
            NodeData::Comment { .. } => {
                // Skip comments
            }
            NodeData::Document => {
                // Shouldn't happen in inline context
            }
        }
    }

    // Traverse and build
    let mut text_len = 0usize;
    traverse_inline_children(&*dom, node_id, &mut builder, &inline_box_ids, &mut pending_inline_boxes, &mut text_len, scale);

    // Build the layout
    let (parley_layout, text) = builder.build();

    // Store in TextLayout
    layout.text = text;
    layout.layout = parley_layout;
    layout.content_widths = None; // Will be computed on demand
}

