use std::time::{SystemTime, UNIX_EPOCH};
use selectors::Element;
use style::dom::TDocument;
use style::selector_parser::RestyleDamage;
use crate::dom::{Dom, DomEvent};
use crate::dom::events::pointer::ScrollAnimationState;

impl Dom {
    pub(crate) fn resolve(&mut self, now: f64) {
        if TDocument::as_node(&&self.nodes[0])
            .first_element_child()
            .is_none()
        {
            eprintln!("ERROR: No DOM - not resolving");
            return;
        }

        self.handle_messages();

        self.resolve_scroll_animation();

        let root_node_id = self.root_element().id;

        self.flush_styles(now);

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

    pub fn resolve_scroll_animation(&mut self) {
        match &mut self.scroll_animation {
            ScrollAnimationState::Fling(fling_state) => {
                let time_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64 as f64;

                let time_diff_ms = time_ms - fling_state.last_seen_time;

                // 0.95 @ 60fps normalized to actual frame times
                let deceleration = 1.0 - ((0.05 / 16.66666) * time_diff_ms);

                fling_state.x_velocity *= deceleration;
                fling_state.y_velocity *= deceleration;
                fling_state.last_seen_time = time_ms;
                let fling_state = fling_state.clone();

                let dx = fling_state.x_velocity * time_diff_ms;
                let dy = fling_state.y_velocity * time_diff_ms;

                self.scroll_by(Some(fling_state.target), dx, dy, &mut |_| {});
                if fling_state.x_velocity.abs() < 0.1 && fling_state.y_velocity.abs() < 0.1 {
                    self.scroll_animation = ScrollAnimationState::None;
                }
            }
            ScrollAnimationState::None => {
                // Do nothing
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