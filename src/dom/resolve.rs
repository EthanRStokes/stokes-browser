use crate::dom::events::pointer::ScrollAnimationState;
use crate::dom::{Dom, DomEvent};
use selectors::Element;
use std::time::{SystemTime, UNIX_EPOCH};
use style::dom::TDocument;
use style::selector_parser::RestyleDamage;

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

        let mut subdom_is_animating = false;
        for &node_id in &self.sub_dom_nodes {
            let node = &mut self.nodes[node_id];
            let size = node.final_layout.size;
            if let Some(mut sub_doc) = node.subdom_mut().map(|doc| doc.inner_mut()) {
                // Set viewport
                // viewport_mut handles change detection. So we just unconditionally set the values;
                let mut sub_viewport = sub_doc.viewport_mut();
                sub_viewport.hidpi_scale = self.viewport.hidpi_scale;
                sub_viewport.zoom = self.viewport.zoom;
                sub_viewport.color_scheme = self.viewport.color_scheme;

                let viewport_scale = self.viewport.scale();
                sub_viewport.window_size = (
                    (size.width * viewport_scale) as u32,
                    (size.height * viewport_scale) as u32,
                );
                drop(sub_viewport);

                sub_doc.resolve(now);

                subdom_is_animating |= sub_doc.animating();
            }
        }
        self.subdom_is_animating = subdom_is_animating;
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