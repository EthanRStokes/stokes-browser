use std::sync::Arc;
use blitz_traits::net::Request;
use markup5ever::local_name;
use peniko::Blob;
use crate::dom::damage::ALL_DAMAGE;
use crate::dom::{Dom, ImageData};
use crate::dom::node::{CanvasData, RasterImageData, SpecialElementData, Status};
use crate::networking::{ImageHandler, ImageType, Resource, ResourceHandler, ResourceLoadResponse, StylesheetHandler};

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
        println!("Loading image for node_id: {}", node_id);
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
        if let Some(raw_src) = node.attr(local_name!("src")) {
            if let Ok(custom_paint_source_id) = raw_src.parse::<u64>() {
                // todo animation stuff
                let canvas_data = SpecialElementData::Canvas(CanvasData {
                    custom_paint_source_id,
                });
                node.element_data_mut().unwrap().special_data = canvas_data;
            }
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