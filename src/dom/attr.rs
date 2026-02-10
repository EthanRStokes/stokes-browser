use html5ever::local_name;
use markup5ever::QualName;
use style::invalidation::element::restyle_hints::RestyleHint;
use stylo_atoms::Atom;
use crate::dom::damage::ALL_DAMAGE;
use crate::dom::{Dom, NodeData};

macro_rules! tag_attr {
    ($tag:tt, $attr:tt) => {
        (&local_name!($tag), &local_name!($attr))
    };
}

impl Dom {
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

        // TODO load stuff
    }
}