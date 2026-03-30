use crate::dom::ns;
use std::collections::HashSet;
use crate::dom::damage::ALL_DAMAGE;
use crate::dom::{Dom, ElementData, NodeData};
use html5ever::local_name;
use markup5ever::QualName;
use style::invalidation::element::restyle_hints::RestyleHint;
use stylo_atoms::Atom;
use crate::dom::node::{Attribute, SpecialElementData};
use crate::qual_name;

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
        if let Some(mut data) = node.stylo_data.get_mut() {
            data.hint |= RestyleHint::restyle_subtree();
            data.damage.insert(ALL_DAMAGE);
        }

        // TODO: make this fine grained / conditional based on ElementSelectorFlags
        let parent = node.parent;
        if let Some(parent_id) = parent {
            let parent = &mut self.nodes[parent_id];
            if let Some(mut data) = parent.stylo_data.get_mut() {
                data.hint |= RestyleHint::restyle_subtree();
            }
        }

        self.nodes[node_id].mark_ancestors_dirty();

        let mut old_id = None;
        let mut old_class = None;
        let mut tag_local = None;
        let node_in_doc = self.nodes[node_id].flags.is_in_document();

        let node = &mut self.nodes[node_id];

        let NodeData::Element(ref mut element) = node.data else {
            return;
        };

        if name.local == local_name!("id") {
            old_id = element.attr(local_name!("id")).map(ToOwned::to_owned);
        }

        if name.local == local_name!("class") {
            old_class = element.attr(local_name!("class")).map(ToOwned::to_owned);
        }

        element.attributes.set(name.clone(), value);

        let tag = &element.name.local;
        let attr = &name.local;
        tag_local = Some(tag.clone());

        if *attr == local_name!("id") {
            element.id = Some(Atom::from(value));
            if let Some(old_id) = old_id.as_deref() {
                if old_id != value {
                    self.nodes_to_id.remove(old_id);
                }
            }
            self.nodes_to_id.insert(value.to_string(), node_id);
        }

        if *attr == local_name!("class") && node_in_doc {
            if let Some(old_class) = old_class.as_deref() {
                let mut empty_keys = Vec::new();
                for class_name in old_class.split_whitespace() {
                    if class_name.is_empty() {
                        continue;
                    }
                    if let Some(nodes) = self.nodes_by_class.get_mut(class_name) {
                        nodes.retain(|id| *id != node_id);
                        if nodes.is_empty() {
                            empty_keys.push(class_name.to_string());
                        }
                    }
                }
                for key in empty_keys {
                    self.nodes_by_class.remove(&key);
                }
            }
            for class_name in value.split_whitespace() {
                if class_name.is_empty() {
                    continue;
                }
                let nodes = self.nodes_by_class.entry(class_name.to_string()).or_default();
                if !nodes.contains(&node_id) {
                    nodes.push(node_id);
                }
            }
        }

        if *attr == local_name!("value") {
            if let Some(input_data) = &mut element.text_input_data_mut() {
                input_data.set_text(
                    &mut self.font_ctx.lock().unwrap(),
                    &mut self.layout_ctx,
                    value,
                );
            }
            return;
        }

        if *attr == local_name!("style") {
            element.flush_style_attribute(&self.lock, &self.url.url_extra_data());
            node.mark_style_attr_updated();
            return;
        }

        if *attr == local_name!("disabled") && element.can_be_disabled() {
            node.disable();
            return;
        }

        // If node if not in the document, then don't apply any special behaviours
        // and simply set the attribute value
        if !node.flags.is_in_document() {
            return;
        }

        if (tag, attr) == tag_attr!("input", "checked") {
            set_input_checked_state(element, value.to_string());
        } else if (tag, attr) == tag_attr!("img", "src") {
            self.load_image(node_id);
        } else if (tag, attr) == tag_attr!("canvas", "src") {
            self.load_custom_paint_src(node_id);
        } else if (tag, attr) == tag_attr!("link", "href") {
            self.load_linked_stylesheet(node_id);
        }

        let is_form_associated = matches!(
            tag_local.as_ref().map(|tag| tag.as_ref()),
            Some("button" | "fieldset" | "input" | "select" | "textarea" | "object" | "output")
        );

        if name.local == local_name!("form") && is_form_associated {
            self.reset_form_owner(node_id);
        }

        if name.local == local_name!("id") && tag_local.as_ref().is_some_and(|tag| *tag == local_name!("form")) {
            self.reset_all_form_owners();
        }
    }

    pub fn clear_attribute(&mut self, node_id: usize, name: QualName) {
        self.snapshot(node_id);

        let mut should_recompute_canvas = false;
        let mut should_unload_stylesheet = false;
        let mut should_reset_form_owner = false;
        let mut should_reset_all_form_owners = false;
        let mut removed_class_value: Option<String> = None;
        let node_in_doc = self.nodes[node_id].flags.is_in_document();

        {
            let node = &mut self.nodes[node_id];

            if let Some(mut data) = node.stylo_data.get_mut() {
                data.hint |= RestyleHint::restyle_subtree();
                data.damage.insert(ALL_DAMAGE);
            }

            // Mark ancestors dirty so the style traversal visits this subtree.
            // Without this, the traversal may skip nodes with pending RestyleHint/damage.
            node.mark_ancestors_dirty();

            let Some(element) = node.element_data_mut() else {
                return;
            };

            let removed_attr = element.attributes.remove(&name);
            let had_attr = removed_attr.is_some();
            if !had_attr {
                return;
            }

            if name.local == local_name!("id") {
                element.id = None;
                if let Some(id_attr) = removed_attr.as_ref() {
                    self.nodes_to_id.remove(id_attr.value.as_str());
                }
            }

            if name.local == local_name!("class") {
                removed_class_value = removed_attr.as_ref().map(|attr| attr.value.clone());
            }

            // Update text input value
            if name.local == local_name!("value") {
                if let Some(input_data) = element.text_input_data_mut() {
                    input_data.set_text(
                        &mut self.font_ctx.lock().unwrap(),
                        &mut self.layout_ctx,
                        "",
                    );
                }
            }

            let tag = element.name.local.clone();
            let attr = name.local.clone();

            if attr == local_name!("disabled") && element.can_be_disabled() {
                node.enable();
                return;
            }

            if attr == local_name!("style") {
                element.flush_style_attribute(&self.lock, &self.url.url_extra_data());
                node.mark_style_attr_updated();
            }

            should_recompute_canvas = tag == local_name!("canvas") && attr == local_name!("src");
            should_unload_stylesheet = tag == local_name!("link") && attr == local_name!("href");

            let is_form_associated = matches!(
                tag.as_ref(),
                "button" | "fieldset" | "input" | "select" | "textarea" | "object" | "output"
            );
            should_reset_form_owner = name.local == local_name!("form") && is_form_associated;
            should_reset_all_form_owners = name.local == local_name!("id") && tag == local_name!("form");
        }

        if should_recompute_canvas {
            self.has_canvas = self.compute_has_canvas();
        }
        if should_unload_stylesheet {
            self.unload_stylesheet(node_id);
        }
        if should_reset_form_owner {
            self.reset_form_owner(node_id);
        }
        if should_reset_all_form_owners {
            self.reset_all_form_owners();
        }

        if node_in_doc {
            if let Some(removed_classes) = removed_class_value.as_deref() {
                let mut empty_keys = Vec::new();
                for class_name in removed_classes.split_whitespace() {
                    if class_name.is_empty() {
                        continue;
                    }
                    if let Some(nodes) = self.nodes_by_class.get_mut(class_name) {
                        nodes.retain(|id| *id != node_id);
                        if nodes.is_empty() {
                            empty_keys.push(class_name.to_string());
                        }
                    }
                }
                for key in empty_keys {
                    self.nodes_by_class.remove(&key);
                }
            }
        }
    }

    pub fn element_name(&self, node_id: usize) -> Option<&QualName> {
        self.nodes[node_id].element_data().map(|el| &el.name)
    }
}

fn set_input_checked_state(element: &mut ElementData, value: String) {
    let Ok(checked) = value.parse() else {
        return;
    };
    match element.special_data {
        SpecialElementData::CheckboxInput(ref mut checked_mut) => *checked_mut = checked,
        // If we have just constructed the element, set the node attribute,
        // and NodeSpecificData will be created from that later
        // this simulates the checked attribute being set in html,
        // and the element's checked property being set from that
        SpecialElementData::None => element.attributes.push(Attribute {
            name: qual_name!("checked", html),
            value: checked.to_string(),
        }),
        _ => {}
    }
}