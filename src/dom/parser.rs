use super::Dom;
use crate::dom::config::DomConfig;
use crate::dom::node::Attribute;
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tokenizer::TokenizerOpts;
use html5ever::tree_builder::TreeBuilderOpts;
// HTML parser using html5ever
use html5ever::{parse_document, ParseOpts};
use markup5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use markup5ever::QualName;
use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell, RefMut};

/// HTML Parser for converting HTML strings into DOM structures
pub struct HtmlParser;

fn html5ever_to_stokes(attribute: html5ever::Attribute) -> Attribute {
    Attribute {
        name: attribute.name.clone(),
        value: attribute.value.to_string(),
    }
}

impl HtmlParser {
    pub fn new() -> Self {
        Self {}
    }

    /// Parse HTML string into a DOM structure
    pub fn parse(&self, html: &str, config: DomConfig) -> Dom {
        let mut dom = Dom::new(config);

        DomHtmlParser::parse_dom(&mut dom, html);

        dom
    }
}

pub struct DomHtmlParser<'m> {
    dom: RefCell<&'m mut Dom>,

    pub errors: RefCell<Vec<Cow<'static, str>>>,

    pub quirks_mode: Cell<QuirksMode>,
    pub is_xml: bool,
}

impl<'m> DomHtmlParser<'m> {
    pub fn new(dom: &'m mut Dom) -> Self {
        Self {
            dom: RefCell::new(dom),
            errors: RefCell::new(Vec::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
            is_xml: false,
        }
    }

    pub fn parse_dom<'a>(dom: &'a mut Dom, html: &str) {
        let mut sink = DomHtmlParser::new(dom);

        let is_xhtml_doc = html.starts_with("<?xml")
            || html.starts_with("<!DOCTYPE") && {
                let first_line = html.lines().next().unwrap();
                first_line.contains("XHTML") || first_line.contains("xhtml")
            };

        if is_xhtml_doc {
            sink.is_xml = true;
            xml5ever::driver::parse_document(sink, Default::default())
                .from_utf8()
                .read_from(&mut html.as_bytes())
                .unwrap();
        } else {
            sink.is_xml = false;
            let opts = ParseOpts {
                tokenizer: TokenizerOpts::default(),
                tree_builder: TreeBuilderOpts {
                    exact_errors: false,
                    scripting_enabled: true,
                    iframe_srcdoc: false,
                    drop_doctype: true,
                    quirks_mode: QuirksMode::NoQuirks,
                },
            };
            parse_document(sink, opts)
                .from_utf8()
                .read_from(&mut html.as_bytes())
                .unwrap();
        }
    }

    #[track_caller]
    fn dom(&self) -> RefMut<'_, &'m mut Dom> {
        self.dom.borrow_mut()
    }
}

impl<'m> TreeSink for DomHtmlParser<'m> {
    type Output = ();

    type Handle = usize;

    type ElemName<'a>
        = Ref<'a, QualName>
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        for error in self.errors.borrow().iter() {
            println!("ERROR: {error}");
        }
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        self.errors.borrow_mut().push(msg);
    }

    fn get_document(&self) -> Self::Handle {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        Ref::map(self.dom.borrow(), |dom| {
            dom.element_name(*target).expect("TreeSink::elem_name called on non-element node")
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<markup5ever::Attribute>, flags: ElementFlags) -> Self::Handle {
        let attrs = attrs.into_iter().map(html5ever_to_stokes).collect();
        self.dom().create_element(name, attrs)
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.dom().create_comment_node()
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.dom().create_comment_node()
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(id) => self.dom().append_children(*parent, &[id]),
            NodeOrText::AppendText(text) => {
                let last_child_id = self.dom().last_child_id(*parent);
                let has_appended = if let Some(id) = last_child_id {
                    self.dom().append_text_to_node(id, &text).is_ok()
                } else {
                    false
                };
                if !has_appended {
                    let new_child_id = self.dom().create_text_node(&text);
                    self.dom().append_children(*parent, &[new_child_id]);
                }
            }
        }
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        match new_node {
            NodeOrText::AppendNode(id) => self.dom().insert_nodes_before(*sibling, &[id]),
            NodeOrText::AppendText(text) => {
                let previous_sibling_id = self.dom().previous_sibling_id(*sibling);
                let has_appended = if let Some(id) = previous_sibling_id {
                    self.dom().append_text_to_node(id, &text).is_ok()
                } else {
                    false
                };
                if !has_appended {
                    let new_child_id = self.dom().create_text_node(&text);
                    self.dom().insert_nodes_before(*sibling, &[new_child_id]);
                }
            }
        }
    }

    fn append_based_on_parent_node(&self, element: &Self::Handle, prev_element: &Self::Handle, child: NodeOrText<Self::Handle>) {
        if self.dom().node_has_parent(*element) {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, name: StrTendril, public_id: StrTendril, system_id: StrTendril) {
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        // todo
        *target
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<markup5ever::Attribute>) {
        let attrs = attrs.into_iter().map(html5ever_to_stokes).collect();
        self.dom().add_attrs_if_missing(*target, attrs);
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.dom().remove_node(*target);
    }

    fn reparent_children(&self, old_parent: &Self::Handle, new_parent: &Self::Handle) {
        self.dom().reparent_children(*old_parent, *new_parent);
    }
}