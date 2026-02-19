use selectors::Element;
use style::dom::TDocument;
use style::selector_parser::RestyleDamage;
use crate::dom::{Dom, DomEvent};

impl Dom {
    pub(crate) fn resolve(&mut self) {
        if TDocument::as_node(&&self.nodes[0])
            .first_element_child()
            .is_none()
        {
            eprintln!("ERROR: No DOM - not resolving");
            return;
        }

        self.handle_messages();

        let root_node_id = self.root_element().id;

        self.flush_styles();

        self.propagate_damage_flags(root_node_id, RestyleDamage::empty());

        self.get_layout_children();

        self.flush_styles_to_layout(root_node_id);

        self.compute_layout();

        {
            for (_, node) in self.nodes.iter_mut() {
                node.clear_damage_mut();
                node.unset_dirty_descendants();
            }
        }
    }

    fn handle_messages(&mut self) {
        let rx = self.rx.take().unwrap();

        while let Ok(message) = rx.try_recv() {
            self.handle_message(message);
        }

        self.rx = Some(rx);
    }

    fn handle_message(&mut self, message: DomEvent) {
        match message {
            DomEvent::ResourceLoad(resource) => {
                self.load_resource(resource);
            }
        }
    }
}