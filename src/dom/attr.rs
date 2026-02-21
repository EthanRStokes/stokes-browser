use std::collections::HashSet;
use crate::dom::damage::ALL_DAMAGE;
use crate::dom::{Dom, NodeData};
use html5ever::local_name;
use markup5ever::QualName;
use style::invalidation::element::restyle_hints::RestyleHint;
use stylo_atoms::Atom;
use crate::dom::node::Attribute;

macro_rules! tag_attr {
    ($tag:tt, $attr:tt) => {
        (&local_name!($tag), &local_name!($attr))
    };
}

impl Dom {
    pub fn add_attrs_if_missing(&mut self, node_id: usize, attrs: Vec<Attribute>) {
        let node = &mut self.nodes[node_id];
        node.insert_damage(ALL_DAMAGE);
        let element_data = node.element_data_mut().expect("Not an element");

        let existing_names = element_data
            .attributes
            .iter()
            .map(|e| e.name.clone())
            .collect::<HashSet<_>>();

        for attr in attrs
            .into_iter()
            .filter(|attr| !existing_names.contains(&attr.name))
        {
            self.set_attribute(node_id, attr.name, &attr.value);
        }
    }

    pub fn set_attribute(&mut self, node_id: usize, name: QualName, value: &str) {
        self.snapshot(node_id);

        let node = &mut self.nodes[node_id];
        if let Some(data) = &mut *node.stylo_data.borrow_mut() {
            data.hint |= RestyleHint::restyle_subtree();
            data.damage.insert(ALL_DAMAGE);
        }

        // TODO: make this fine grained / conditional based on ElementSelectorFlags
        let parent = node.parent;
        if let Some(parent_id) = parent {
            let parent = &mut self.nodes[parent_id];
            if let Some(data) = &mut *parent.stylo_data.borrow_mut() {
                data.hint |= RestyleHint::restyle_subtree();
            }
        }

        let node = &mut self.nodes[node_id];

        let NodeData::Element(ref mut element) = node.data else {
            return;
        };

        element.attributes.set(name.clone(), value);

        let tag = &element.name.local;
        let attr = &name.local;

        if *attr == local_name!("id") {
            element.id = Some(Atom::from(value))
        }

        // todo text input

        if *attr == local_name!("style") {
            element.flush_style_attribute(&self.lock, &self.url.url_extra_data());
            return;
        }

        // If node if not in the document, then don't apply any special behaviours
        // and simply set the attribute value
        if !node.flags.is_in_document() {
            return;
        }

        if (tag, attr) == tag_attr!("img", "src") {
            self.load_image(node_id);
        } else if (tag, attr) == tag_attr!("canvas", "src") {
            self.load_custom_paint_src(node_id);
        } else if (tag, attr) == tag_attr!("link", "href") {
            self.load_linked_stylesheet(node_id);
        }
    }

    pub fn element_name(&self, node_id: usize) -> Option<&QualName> {
        self.nodes[node_id].element_data().map(|el| &el.name)
    }
}