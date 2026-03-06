use crate::dom::config::DomConfig;
use crate::dom::damage::ALL_DAMAGE;
use crate::dom::node::{CanvasData, RasterImageData, SpecialElementData, Status};
use crate::dom::{Dom, HtmlParser, ImageData};
use crate::networking::{
    HtmlDocumentHandler, ImageHandler, ImageType, Resource, ResourceHandler, ResourceLoadResponse,
    StylesheetHandler,
};
use blitz_traits::net::{NetProvider, Request};
use blitz_traits::shell::{ShellProvider, Viewport};
use markup5ever::local_name;
use peniko::Blob;
use std::sync::Arc;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::selector_parser::RestyleDamage;
use style::stylesheets::OriginSet;

impl Dom {
    pub(crate) fn resolve_url(&self, raw: &str) -> url::Url {
        self.url.resolve_relative(raw).unwrap_or_else(|| {
            panic!(
                "to be able to resolve {raw} with the base_url: {:?}",
                *self.url
            )
        })
    }

    pub(crate) fn load_image(&mut self, node_id: usize) {
        let node = &self.nodes[node_id];
        if let Some(raw_src) = node.attr(local_name!("src")) {
            if !raw_src.is_empty() {
                let src = self.resolve_url(raw_src);
                let src_string = src.as_str();

                // Check cache first
                if let Some(cached_image) = self.image_cache.get(src_string) {
                    let node = &mut self.nodes[node_id];
                    node.element_data_mut().unwrap().special_data =
                        SpecialElementData::Image(Box::new(cached_image.clone()));
                    node.cache.clear();
                    node.insert_damage(ALL_DAMAGE);
                    return;
                }

                // Check if there's already a pending request for this URL
                if let Some(waiting_list) = self.pending_images.get_mut(src_string) {
                    waiting_list.push((node_id, ImageType::Image));
                    return;
                }

                self.pending_images
                    .insert(src_string.to_string(), vec![(node_id, ImageType::Image)]);

                self.net_provider.fetch(
                    self.id(),
                    Request::get(src),
                    ResourceHandler::boxed(
                        self.tx.clone(),
                        self.id(),
                        None, // Don't pass node_id, we'll handle it via pending_images
                        self.shell_provider.clone(),
                        ImageHandler::new(ImageType::Image),
                    ),
                );
            }
        }
    }

    pub(crate) fn load_custom_paint_src(&mut self, node_id: usize) {
        let node = &mut self.nodes[node_id];
        let mut compute_has_canvas = false;
        if let Some(raw_src) = node.attr(local_name!("src")) {
            if let Ok(custom_paint_source_id) = raw_src.parse::<u64>() {
                compute_has_canvas = true;
                let canvas_data = SpecialElementData::Canvas(CanvasData {
                    custom_paint_source_id,
                });
                node.element_data_mut().unwrap().special_data = canvas_data;
            }
        }

        if compute_has_canvas {
            self.has_canvas = self.compute_has_canvas();
        }
    }

    pub(crate) fn load_linked_stylesheet(&mut self, target_id: usize) {
        let node = &self.nodes[target_id];

        let rel_attr = node.attr(local_name!("rel"));
        let href_attr = node.attr(local_name!("href"));

        let (Some(rels), Some(href)) = (rel_attr, href_attr) else {
            return;
        };
        println!("Loading linked stylesheet for element <{}> link <{}>", target_id, href);
        if !rels.split_ascii_whitespace().any(|rel| rel == "stylesheet") {
            return;
        }

        let url = self.resolve_url(href);
        self.net_provider.fetch(
            self.id(),
            Request::get(url.clone()),
            ResourceHandler::boxed(
                self.tx.clone(),
                self.id(),
                Some(node.id),
                self.shell_provider.clone(),
                StylesheetHandler {
                    source_url: url,
                    guard: self.lock.clone(),
                    net_provider: self.net_provider.clone(),
                },
            ),
        );
    }

    pub(crate) fn unload_stylesheet(&mut self, node_id: usize) {
        let node = &mut self.nodes[node_id];
        let Some(element) = node.element_data_mut() else {
            unreachable!();
        };
        let SpecialElementData::Stylesheet(stylesheet) = element.special_data.take() else {
            unreachable!();
        };

        let guard = self.lock.read();
        self.stylist.remove_stylesheet(stylesheet, &guard);
        self
            .stylist
            .force_stylesheet_origins_dirty(OriginSet::all());

        self.nodes_to_stylesheet.remove(&node_id);
    }

    fn iframe_viewport_from_host(&self, node_id: usize) -> Viewport {
        let mut viewport = self.viewport.clone();
        if let Some(node) = self.nodes.get(node_id) {
            let width = node.final_layout.content_box_width().max(1.0) as u32;
            let height = node.final_layout.content_box_height().max(1.0) as u32;
            viewport.window_size = (width, height);
        }
        viewport
    }

    fn build_iframe_dom(&self, html: &str, base_url: &str, viewport: Viewport) -> Dom {
        HtmlParser::new().parse(
            html,
            DomConfig {
                viewport: Some(viewport),
                base_url: Some(base_url.to_string()),
                net_provider: Some(self.net_provider.clone()),
                shell_provider: Some(self.shell_provider.clone()),
                nav_provider: Some(self.nav_provider.clone()),
                js_provider: Some(self.js_provider.clone()),
                font_ctx: Some(self.font_ctx.lock().unwrap().clone()),
                ..Default::default()
            },
        )
    }

    pub(crate) fn load_iframe(&mut self, node_id: usize) {
        let Some(node) = self.nodes.get(node_id) else {
            return;
        };
        let Some(element) = node.element_data() else {
            return;
        };
        if element.name.local != local_name!("iframe") {
            return;
        }

        let viewport = self.iframe_viewport_from_host(node_id);
        let srcdoc = element
            .attr(local_name!("srcdoc"))
            .filter(|v| !v.is_empty())
            .map(html_escape::decode_html_entities)
            .map(|v| v.to_string());
        let src = element.attr(local_name!("src")).filter(|v| !v.is_empty()).map(str::to_string);

        if let Some(html) = srcdoc {
            let base_url = self.url.to_string();
            let sub_dom = self.build_iframe_dom(&html, &base_url, viewport);
            self.set_sub_dom(node_id, sub_dom);
            self.nodes[node_id].insert_damage(ALL_DAMAGE);
            self.nodes[node_id].mark_ancestors_dirty();
            self.shell_provider.request_redraw();
            return;
        }

        if let Some(raw_src) = src {
            if let Some(resolved) = self.url.resolve_relative(&raw_src) {
                self.net_provider.fetch(
                    self.id(),
                    Request::get(resolved),
                    ResourceHandler::boxed(
                        self.tx.clone(),
                        self.id(),
                        Some(node_id),
                        self.shell_provider.clone(),
                        HtmlDocumentHandler,
                    ),
                );
                return;
            }
        }

        // No src/srcdoc: load an empty same-origin document.
        println!("ERROR: Iframe with no src or srcdoc, loading empty document. Node ID: {}", node_id);
    }

    fn apply_iframe_html_response(&mut self, node_id: usize, resolved_url: &str, html: String) {
        let viewport = self.iframe_viewport_from_host(node_id);
        let sub_dom = self.build_iframe_dom(&html, resolved_url, viewport);
        self.set_sub_dom(node_id, sub_dom);

        self.nodes[node_id].cache.clear();
        self.nodes[node_id].insert_damage(ALL_DAMAGE);
        self.nodes[node_id].mark_ancestors_dirty();
        self.shell_provider.request_redraw();
    }

    pub(crate) fn load_resource(&mut self, res: ResourceLoadResponse) {
        let Ok(resource) = res.result else {
            eprintln!("Failed to load resource: {:?}", res.resolved_url);
            return;
        };

        match resource {
            Resource::Css(css) => {
                //println!("Loaded CSS resource: {:?}", res.resolved_url);
                let node_id = res.node_id.unwrap();
                self.add_stylesheet_for_node(css, node_id);
            }
            Resource::Image(kind, width, height, data) => {
                //println!("Loaded Image resource: {:?}", res.resolved_url);
                let image = ImageData::Raster(RasterImageData::new(width, height, data));

                let Some(url) = res.resolved_url.as_ref() else {
                    return;
                };

                let waiting = self.pending_images.remove(url).unwrap_or_default();

                self.image_cache.insert(url.clone(), image.clone());

                for (node_id, image_type) in waiting {
                    let Some(node) = self.get_node_mut(node_id) else {
                        continue;
                    };

                    match image_type {
                        ImageType::Image => {
                            node.element_data_mut().unwrap().special_data =
                                SpecialElementData::Image(Box::new(image.clone()));

                            node.cache.clear();
                            node.insert_damage(ALL_DAMAGE);
                        }
                        ImageType::Background(idx) => {
                            if let Some(Some(bg_image)) = node
                                .element_data_mut()
                                .and_then(|el| el.background_images.get_mut(idx))
                            {
                                bg_image.status = Status::Ok;
                                bg_image.image = image.clone();
                            }
                        }
                    }
                }
            },
            Resource::Svg(_kind, tree) => {
                //println!("Loaded SVG resource: {:?}", res.resolved_url);
                let image = ImageData::Svg(tree);

                let Some(url) = res.resolved_url.as_ref() else {
                    return;
                };

                let waiting = self.pending_images.remove(url).unwrap_or_default();

                self.image_cache.insert(url.clone(), image.clone());

                // Apply to all waiting nodes
                for (node_id, image_type) in waiting {
                    let Some(node) = self.get_node_mut(node_id) else {
                        continue;
                    };

                    match image_type {
                        ImageType::Image => {
                            node.element_data_mut().unwrap().special_data =
                                SpecialElementData::Image(Box::new(image.clone()));

                            // Clear layout cache
                            node.cache.clear();
                            node.insert_damage(ALL_DAMAGE);
                        }
                        ImageType::Background(idx) => {
                            if let Some(Some(bg_image)) = node
                                .element_data_mut()
                                .and_then(|el| el.background_images.get_mut(idx))
                            {
                                bg_image.status = Status::Ok;
                                bg_image.image = image.clone();
                            }
                        }
                    }
                }
            },
            Resource::Html(html) => {
                let Some(node_id) = res.node_id else {
                    return;
                };
                let Some(resolved_url) = res.resolved_url.as_deref() else {
                    return;
                };
                self.apply_iframe_html_response(node_id, resolved_url, html);
            }
            Resource::Font(bytes) => {
                //println!("Loaded Font resource: {:?}", res.resolved_url);
                let font = Blob::new(Arc::new(bytes));

                // TODO: Implement FontInfoOveride
                // TODO: Investigate eliminating double-box
                let mut global_font_ctx = self.font_ctx.lock().unwrap();
                global_font_ctx
                    .collection
                    .register_fonts(font.clone(), None);

                drop(global_font_ctx);

                // TODO: see if we can only invalidate if resolved fonts may have changed
                self.invalidate_inline_contexts();
            }
            Resource::None => {
                println!("Loaded resource with no data: {:?}", res.resolved_url);
                // Do nothing
            }
        }
    }
}

