use std::hash::{Hash, Hasher};
use std::ptr::NonNull;
use atomic_refcell::{AtomicRef, AtomicRefMut};
use boa_engine::ast::expression::Identifier;
use markup5ever::{local_name, LocalName, LocalNameStaticSet, Namespace, NamespaceStaticSet};
use selectors::{Element, OpaqueElement};
use selectors::attr::{AttrSelectorOperation, AttrSelectorOperator, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::{MatchingContext, VisitedHandlingMode};
use selectors::matching::ElementSelectorFlags;
use selectors::sink::Push;
use skia_safe::wrapper::NativeTransmutableWrapper;
use style::animation::AnimationSetKey;
use style::applicable_declarations::ApplicableDeclarationBlock;
use style::context::{QuirksMode, SharedStyleContext, StyleContext};
use style::data::ElementData;
use style::dom::{LayoutIterator, NodeInfo, OpaqueNode, TDocument, TElement, TNode, TShadowRoot};
use style::properties::PropertyDeclarationBlock;
use style::selector_parser::{AttrValue, Lang, NonTSPseudoClass, PseudoElement, SelectorImpl};
use style::servo_arc::{Arc, ArcBorrow};
use style::shared_lock::{Locked, SharedRwLock};
use style::stylist::CascadeData;
use style::traversal::{recalc_style_at, DomTraversal, PerLevelTraversalData};
use style::traversal_flags::TraversalFlags;
use style::values::{AtomIdent, AtomString, GenericAtomIdent};
use style::values::computed::{Au, Display};
use stylo_atoms::Atom;
use stylo_dom::ElementState;
use crate::dom::{damage, DomNode, NodeData};

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
        None
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let mut n = 1;
        while let Some(sibling) = self.forward(n) {
            if let NodeData::Element { .. } = sibling.data {
                return Some(sibling);
            }
            n += 1;
        }
        None
    }

    fn first_element_child(&self) -> Option<Self> {
        let mut children = self.dom_children();
        children.find(|child| child.is_element())
    }

    fn is_html_element_in_html_document(&self) -> bool {
        true
    }

    fn has_local_name(&self, local_name: &LocalName) -> bool {
        self.data.is_element_with_tag_name(local_name)
    }

    fn has_namespace(&self, ns: &Namespace) -> bool {
        self.element_data().expect("Not an element").name.ns == *ns
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.local_name() == other.local_name() && self.namespace() == other.namespace()
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&GenericAtomIdent<NamespaceStaticSet>>,
        local_name: &GenericAtomIdent<LocalNameStaticSet>,
        operation: &AttrSelectorOperation<&AtomString>
    ) -> bool {
        let Some(attr) = self.data.attr(&local_name.0.to_string()) else {
            return false;
        };

        match operation {
            AttrSelectorOperation::Exists => true,
            AttrSelectorOperation::WithValue {
                operator,
                case_sensitivity: _,
                value,
            } => {
                let value = value.as_ref();

                match operator {
                    AttrSelectorOperator::Equal => attr == value,
                    AttrSelectorOperator::Includes => attr
                        .split_ascii_whitespace()
                        .any(|word| word == value),
                    AttrSelectorOperator::DashMatch => {
                        attr.starts_with(value) && (attr.len() == value.len() || attr.chars().nth(value.len()) == Some('-'))
                    }
                    AttrSelectorOperator::Prefix => attr.starts_with(value),
                    AttrSelectorOperator::Substring => attr.contains(value),
                    AttrSelectorOperator::Suffix => attr.ends_with(value),
                }
            }
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &<Self::Impl as selectors::SelectorImpl>::NonTSPseudoClass,
        context: &mut MatchingContext<Self::Impl>
    ) -> bool {
        match *pc {
            NonTSPseudoClass::Active => self.element_state.contains(ElementState::ACTIVE),
            NonTSPseudoClass::AnyLink => self.data.element().map(|element| {
                (element.name.local == local_name!("a") || element.name.local == local_name!("area")) && element.attributes.get("href").is_some()
            }).unwrap_or(false),
            NonTSPseudoClass::Autofill => false,
            NonTSPseudoClass::Checked => false, // TODO support checkboxes
            NonTSPseudoClass::CustomState(_) => false,
            NonTSPseudoClass::Default => false,
            NonTSPseudoClass::Defined => false,
            NonTSPseudoClass::Disabled => false,
            NonTSPseudoClass::Enabled => false,
            NonTSPseudoClass::Focus => self.element_state.contains(ElementState::FOCUS),
            NonTSPseudoClass::FocusWithin => false,
            NonTSPseudoClass::FocusVisible => false,
            NonTSPseudoClass::Fullscreen => false,
            NonTSPseudoClass::Hover => self.element_state.contains(ElementState::HOVER),
            NonTSPseudoClass::InRange => false,
            NonTSPseudoClass::Indeterminate => false,
            NonTSPseudoClass::Invalid => false,
            NonTSPseudoClass::Lang(_) => false,
            NonTSPseudoClass::Link => self.data.element().map(|element| {
                (element.name.local == local_name!("a") || element.name.local == local_name!("area")) && element.attributes.get("href").is_some()
            }).unwrap_or(false),
            NonTSPseudoClass::Modal => false,
            NonTSPseudoClass::MozMeterOptimum => false,
            NonTSPseudoClass::MozMeterSubOptimum => false,
            NonTSPseudoClass::MozMeterSubSubOptimum => false,
            NonTSPseudoClass::Optional => false,
            NonTSPseudoClass::OutOfRange => false,
            NonTSPseudoClass::PlaceholderShown => false,
            NonTSPseudoClass::PopoverOpen => false,
            NonTSPseudoClass::ReadOnly => false,
            NonTSPseudoClass::ReadWrite => false,
            NonTSPseudoClass::Required => false,
            NonTSPseudoClass::ServoNonZeroBorder => false,
            NonTSPseudoClass::Target => false,
            NonTSPseudoClass::UserInvalid => false,
            NonTSPseudoClass::UserValid => false,
            NonTSPseudoClass::Valid => false,
            NonTSPseudoClass::Visited => false
        }
    }

    fn match_pseudo_element(
        &self,
        pe: &PseudoElement,
        context: &mut MatchingContext<Self::Impl>
    ) -> bool {
        match self.data {
            NodeData::AnonymousBlock(_) => *pe == PseudoElement::ServoAnonymousBox,
            _ => false,
        }
    }

    fn apply_selector_flags(&self, flags: ElementSelectorFlags) {
        todo!()
    }

    fn is_link(&self) -> bool {
        self.data.is_element_with_tag_name(&local_name!("a"))
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(
        &self,
        id: &<Self::Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity
    ) -> bool {
        self.element_data()
            .and_then(|data| data.id())
            .map(|id_attribute| case_sensitivity.eq(id_attribute.as_ref(), id.as_ref().as_ref()))
            .unwrap_or(false)
    }

    fn has_class(
        &self,
        name: &<Self::Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity
    ) -> bool {
        todo!()
    }

    fn has_custom_state(&self, name: &<Self::Impl as selectors::SelectorImpl>::Identifier) -> bool {
        false
    }

    fn imported_part(&self, name: &<Self::Impl as selectors::SelectorImpl>::Identifier) -> Option<<Self::Impl as selectors::SelectorImpl>::Identifier> {
        None
    }

    fn is_part(&self, name: &<Self::Impl as selectors::SelectorImpl>::Identifier) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.dom_children().next().is_none()
    }

    fn is_root(&self) -> bool {
        self.parent_node().and_then(|parent| parent.parent_node()).is_none()
    }

    fn add_element_unique_hashes(&self, filter: &mut BloomFilter) -> bool {
        false
    }
}

impl<'a> TElement for Node<'a> {
    type ConcreteNode = Node<'a>;

    type TraversalChildrenIterator = NodeTraverser<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        self
    }

    fn traversal_children(&self) -> LayoutIterator<Self::TraversalChildrenIterator> {
        LayoutIterator(NodeTraverser {
            parent: self,
            child_index: 0,
        })
    }

    fn is_html_element(&self) -> bool {
        self.is_element()
    }

    fn is_mathml_element(&self) -> bool {
        false // Not implemented
    }

    fn is_svg_element(&self) -> bool {
        false // idk
    }

    fn style_attribute(&self) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        self.element_data().expect("Not an element").style_attribute.as_ref().map(|block| block.borrow_arc())
    }

    fn animation_rule(&self, _: &SharedStyleContext) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        todo!()
    }

    fn transition_rule(&self, context: &SharedStyleContext) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        todo!()
    }

    fn state(&self) -> ElementState {
        self.element_state
    }

    fn has_part_attr(&self) -> bool {
        false
    }

    fn exports_any_part(&self) -> bool {
        false
    }

    fn id(&self) -> Option<&Atom> {
        self.element_data().and_then(|data| data.id.as_ref())
    }

    fn each_class<F>(&self, mut callback: F)
    where
        F: FnMut(&AtomIdent)
    {
        let class = self.data.attr(&"class".to_string());
        if let Some(class) = class {
            for class_name in class.split_ascii_whitespace() {
                let atom = Atom::from(class_name);
                callback(AtomIdent::cast(&atom));
            }
        }
    }

    fn each_custom_state<F>(&self, callback: F)
    where
        F: FnMut(&AtomIdent)
    {
        todo!()
    }

    fn each_attr_name<F>(&self, mut callback: F)
    where
        F: FnMut(&style::LocalName)
    {
        if let Some(attrs) = self.data.attrs() {
            for attr in attrs.keys() {
                callback(&GenericAtomIdent(LocalName::from(attr.clone())));
            }
        }

    }

    fn has_dirty_descendants(&self) -> bool {
        true
    }

    fn has_snapshot(&self) -> bool {
        self.has_snapshot
    }

    fn handled_snapshot(&self) -> bool {
        todo!()
    }

    unsafe fn set_handled_snapshot(&self) {
        todo!()
    }

    unsafe fn set_dirty_descendants(&self) {}

    unsafe fn unset_dirty_descendants(&self) {
    }

    fn store_children_to_process(&self, n: isize) {
        unimplemented!()
    }

    fn did_process_child(&self) -> isize {
        unimplemented!()
    }

    unsafe fn ensure_data(&self) -> AtomicRefMut<'_, ElementData> {
        let mut stylo_data = self.stylo_data.borrow_mut();
        if stylo_data.is_none() {
            *stylo_data = Some(ElementData {
                damage: damage::ALL_DAMAGE,
                ..Default::default()
            });
        }
        AtomicRefMut::map(stylo_data, |sd| sd.as_mut().unwrap())
    }

    unsafe fn clear_data(&self) {
        *self.stylo_data.borrow_mut() = None;
    }

    fn has_data(&self) -> bool {
        self.stylo_data.borrow().is_some()
    }

    fn borrow_data(&self) -> Option<AtomicRef<'_, ElementData>> {
        let stylo_data = self.stylo_data.borrow();
        if stylo_data.is_some() {
            Some(AtomicRef::map(stylo_data, |sd| sd.as_ref().unwrap()))
        } else {
            None
        }
    }

    fn mutate_data(&self) -> Option<AtomicRefMut<'_, ElementData>> {
        let stylo_data = self.stylo_data.borrow_mut();
        if stylo_data.is_some() {
            Some(AtomicRefMut::map(stylo_data, |sd| sd.as_mut().unwrap()))
        } else {
            None
        }
    }

    fn skip_item_display_fixup(&self) -> bool {
        false
    }

    fn may_have_animations(&self) -> bool {
        true
    }

    fn has_animations(&self, context: &SharedStyleContext) -> bool {
        self.has_css_animations(context, None) || self.has_css_transitions(context, None)
    }

    fn has_css_animations(&self, context: &SharedStyleContext, pseudo_element: Option<PseudoElement>) -> bool {
        let key = AnimationSetKey::new(TNode::opaque(&TElement::as_node(self)), pseudo_element);
        context.animations.has_active_animations(&key)
    }

    fn has_css_transitions(&self, context: &SharedStyleContext, pseudo_element: Option<PseudoElement>) -> bool {
        let key = AnimationSetKey::new(TNode::opaque(&TElement::as_node(self)), pseudo_element);
        context.animations.has_active_transitions(&key)
    }

    fn shadow_root(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

    fn containing_shadow(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

    fn lang_attr(&self) -> Option<AttrValue> {
        None
    }

    fn match_element_lang(&self, override_lang: Option<Option<AttrValue>>, value: &Lang) -> bool {
        false
    }

    fn is_html_document_body_element(&self) -> bool {
        let is_body_element = self.data.is_element_with_tag_name(&local_name!("body"));

        if !is_body_element {
            return false;
        }

        let root_node = &self.tree()[0];
        let root_element = TDocument::as_node(&root_node).first_element_child().unwrap();
        root_element.children.contains(&self.id)
    }

    fn synthesize_presentational_hints_for_legacy_attributes<V>(&self, visited_handling: VisitedHandlingMode, hints: &mut V)
    where
        V: Push<ApplicableDeclarationBlock>
    {
        todo!()
    }

    fn local_name(&self) -> &<SelectorImpl as selectors::SelectorImpl>::BorrowedLocalName {
        &self.element_data().expect("Not an element").name.local
    }

    fn namespace(&self) -> &<SelectorImpl as selectors::SelectorImpl>::BorrowedNamespaceUrl {
        &self.element_data().expect("Not an element").name.ns
    }

    fn query_container_size(&self, display: &Display) -> euclid::default::Size2D<Option<Au>> {
        Default::default() // TODO impl
    }

    fn has_selector_flags(&self, flags: ElementSelectorFlags) -> bool {
        todo!()
    }

    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        todo!()
    }
}

pub struct NodeTraverser<'a> {
    parent: Node<'a>,
    child_index: usize,
}

impl<'a> Iterator for NodeTraverser<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let node_id = self.parent.children.get(self.child_index);
        let node = self.parent.get_node(*node_id.unwrap());
        self.child_index += 1;
        Some(node)
    }
}

impl Hash for Node<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.id)
    }
}

pub struct RecalcStyle<'a> {
    context: SharedStyleContext<'a>,
}

impl<'a> RecalcStyle<'a> {
    pub fn new(context: SharedStyleContext<'a>) -> Self {
        Self { context }
    }
}

impl<E> DomTraversal<E> for RecalcStyle<'_>
where
    E: TElement,
{
    fn process_preorder<F>(&self, traversal_data: &PerLevelTraversalData, context: &mut StyleContext<E>, node: E::ConcreteNode, node_child: F)
    where
        F: FnMut(E::ConcreteNode)
    {
        if node.is_text_node() {
            return;
        }

        let el = node.as_element().unwrap();
        let mut data = unsafe { el.ensure_data() };
        recalc_style_at(self, traversal_data, context, el, &mut data, node_child);

        unsafe { el.unset_dirty_descendants() };
    }

    #[inline]
    fn needs_postorder_traversal() -> bool {
        false
    }

    fn process_postorder(&self, contect: &mut StyleContext<E>, node: E::ConcreteNode) {
        unimplemented!()
    }

    #[inline]
    fn shared_context(&self) -> &SharedStyleContext<'_> {
        &self.context
    }
}