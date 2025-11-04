use std::ptr::NonNull;
use boa_engine::ast::expression::Identifier;
use markup5ever::{LocalName, Namespace};
use selectors::{OpaqueElement};
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::MatchingContext;
use selectors::matching::ElementSelectorFlags;
use style::context::QuirksMode;
use style::dom::{NodeInfo, OpaqueNode, TDocument, TElement, TNode, TShadowRoot};
use style::selector_parser::SelectorImpl;
use style::shared_lock::SharedRwLock;
use style::stylist::CascadeData;
use crate::dom::{DomNode, NodeData};

impl DomNode {

}

type Node<'a> = &'a DomNode;

impl<'a> TDocument for Node<'a> {
    type ConcreteNode = Node<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn is_html_document(&self) -> bool {
        true
    }

    fn quirks_mode(&self) -> QuirksMode {
        QuirksMode::NoQuirks
    }

    fn shared_lock(&self) -> &SharedRwLock {
        &self.lock
    }
}

impl NodeInfo for Node<'_> {
    fn is_element(&self) -> bool {
        matches!(self.data, NodeData::Element { .. })
    }

    fn is_text_node(&self) -> bool {
        matches!(self.data, NodeData::Text { .. })
    }
}

impl<'a> TShadowRoot for Node<'a> {
    type ConcreteNode = Node<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn host(&self) -> <Self::ConcreteNode as TNode>::ConcreteElement {
        todo!("Shadow DOM isn't implemented yet")
    }

    fn style_data<'b>(&self) -> Option<&'b CascadeData>
    where
        Self: 'b
    {
        todo!("Shadow DOM isn't implemented yet")
    }
}

impl<'a> TNode for Node<'a> {
    type ConcreteElement = Node<'a>;
    type ConcreteDocument = Node<'a>;
    type ConcreteShadowRoot = Node<'a>;

    fn parent_node(&self) -> Option<Self> {
        self.parent.map(|id| self.get_node(id))
    }

    fn first_child(&self) -> Option<Self> {
        self.children.first().map(|id| self.get_node(*id))
    }

    fn last_child(&self) -> Option<Self> {
        self.children.last().map(|id| self.get_node(*id))
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.backward(1)
    }

    fn next_sibling(&self) -> Option<Self> {
        self.forward(1)
    }

    fn owner_doc(&self) -> Self::ConcreteDocument {
        self.get_node(1)
    }

    fn is_in_document(&self) -> bool {
        true
    }

    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        self.parent_node().and_then(|node| node.as_element())
    }

    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.id)
    }

    fn debug_id(self) -> usize {
        self.id
    }

    fn as_element(&self) -> Option<Self::ConcreteElement> {
        match self.data {
            NodeData::Element { .. } => Some(self),
            _ => None,
        }
    }

    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        match self.data {
            NodeData::Document { .. } => Some(self),
            _ => None,
        }
    }

    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        None
    }
}

impl selectors::Element for Node<'_> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        let ptr = NonNull::new((self.id + 1) as *mut ()).unwrap();
        OpaqueElement::from_non_null_ptr(ptr)
    }

    fn parent_element(&self) -> Option<Self> {
        TElement::traversal_parent(self)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        matches!(self.data, NodeData::AnonymousBlock(_))
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(sibling) = self.backward(n) {
            if let NodeData::Element { .. } = sibling.data {
                return Some(sibling);
            }
            n += 1;
        }
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(sibling) = self.forward(n) {
            if let NodeData::Element { .. } = sibling.data {
                return Some(sibling);
            }
            n += 1;
        }
    }

    fn first_element_child(&self) -> Option<Self> {
        let mut children = self.dom_children();
        children.find(|child| child.is_element())
    }

    fn is_html_element_in_html_document(&self) -> bool {
        todo!()
    }

    fn has_local_name(&self, local_name: &LocalName) -> bool {
        self.data.is_element_with_tag_name(local_name)
    }

    fn has_namespace(&self, ns: &Namespace) -> bool {
        todo!()
    }

    fn is_same_type(&self, other: &Self) -> bool {
        todo!()
    }

    fn attr_matches(&self, ns: &NamespaceConstraint<&<Self::Impl as SelectorImpl>::NamespaceUrl>, local_name: &<Self::Impl as SelectorImpl>::LocalName, operation: &AttrSelectorOperation<&<Self::Impl as SelectorImpl>::AttrValue>) -> bool {
        todo!()
    }

    fn match_non_ts_pseudo_class(&self, pc: &<Self::Impl as SelectorImpl>::NonTSPseudoClass, context: &mut MatchingContext<Self::Impl>) -> bool {
        todo!()
    }

    fn match_pseudo_element(&self, pe: &<Self::Impl as SelectorImpl>::PseudoElement, context: &mut MatchingContext<Self::Impl>) -> bool {
        todo!()
    }

    fn apply_selector_flags(&self, flags: ElementSelectorFlags) {
        todo!()
    }

    fn is_link(&self) -> bool {
        todo!()
    }

    fn is_html_slot_element(&self) -> bool {
        todo!()
    }

    fn has_id(&self, id: &<Self::Impl as SelectorImpl>::Identifier, case_sensitivity: CaseSensitivity) -> bool {
        todo!()
    }

    fn has_class(&self, name: &<Self::Impl as SelectorImpl>::Identifier, case_sensitivity: CaseSensitivity) -> bool {
        todo!()
    }

    fn has_custom_state(&self, name: &<Self::Impl as SelectorImpl>::Identifier) -> bool {
        todo!()
    }

    fn imported_part(&self, name: &<Self::Impl as SelectorImpl>::Identifier) -> Option<<Self::Impl as SelectorImpl>::Identifier> {
        todo!()
    }

    fn is_part(&self, name: &<Self::Impl as SelectorImpl>::Identifier) -> bool {
        todo!()
    }

    fn is_empty(&self) -> bool {
        todo!()
    }

    fn is_root(&self) -> bool {
        todo!()
    }

    fn add_element_unique_hashes(&self, filter: &mut BloomFilter) -> bool {
        todo!()
    }
}

impl<'a> TElement for Node<'a> {
    type ConcreteNode = Node<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn tag_name(&self) -> &str {
        if let NodeData::Element { name, .. } = &self.data {
            &name.local
        } else {
            ""
        }
    }

    fn attributes(&self) -> &std::collections::HashMap<String, String> {
        if let NodeData::Element { attributes, .. } = &self.data {
            attributes
        } else {
            panic!("Not an element node");
        }
    }
}