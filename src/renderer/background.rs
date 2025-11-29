use crate::dom::node::RasterImageData;
use crate::renderer::text::TextPainter;
use kurbo::{Affine, Rect};
use peniko::{ImageAlphaType, ImageData, ImageFormat, ImageSampler};
use skia_safe::Color;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::servo::url::ComputedUrl;
use style::servo_arc::Arc;
use style::values::computed::{Gradient, Image};
use style::values::generics::image::{GenericCrossFadeImage, GenericImage, GenericImageSetItem};
use style::values::specified::ImageRendering;

/// Background image cache manager
pub struct BackgroundImageCache {
    cache: RefCell<HashMap<String, Option<RasterImageData>>>,
}

impl BackgroundImageCache {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// Load a background image from URL (with caching)
    pub fn load_background_image(&self, url: &ComputedUrl) -> Option<RasterImageData> {
        // Check cache first
        let url = match url {
            ComputedUrl::Valid(u) => u.as_str(),
            ComputedUrl::Invalid(u) => return None,
        };
        {
            let cache = self.cache.borrow();
            if let Some(cached) = cache.get(url) {
                return cached.clone();
            }
        }

        // Try to load the image from file system
        let image_opt = load_image_from_path(url);

        // Cache the result (even if None)
        self.cache.borrow_mut().insert(url.to_string(), image_opt.clone());

        image_opt
    }

    /// Clear the cache to free memory
    pub fn clear_cache(&self) {
        self.cache.borrow_mut().clear();
    }
}

/// Render background image for an element
pub fn render_background_image(
    painter: &mut TextPainter,
    rect: &Rect,
    style: &Arc<StyloComputedValues>,
    scale_factor: f32,
    image_cache: &BackgroundImageCache,
    scroll_transform: kurbo::Affine,
) {
    // Check if background-image is specified
    let background = style.get_background();
    let images = &background.background_image;
    for image in images.0.iter() {
        match image {
            Image::None => {}
            Image::Url(url) => {
                // Try to load and render the background image
                if let Some(image) = image_cache.load_background_image(url) {
                    let inherited_box = style.get_inherited_box();
                    let image_rendering = inherited_box.image_rendering;
                    let quality = to_image_quality(image_rendering);

                    let transform = scroll_transform * Affine::translate((rect.x0, rect.y0));

                    // Draw the background image to cover the entire rect
                    painter.draw_image(to_peniko_image(&image, quality).as_ref(), transform);
                }
            }
            Image::Gradient(gradient) => {}
            Image::PaintWorklet(worklet) => {}
            Image::CrossFade(fade) => {
                for element in fade.elements.iter() {
                    let image = &element.image;
                    match image {
                        GenericCrossFadeImage::Image(image) => {
                            match image {
                                GenericImage::Url(url) => {
                                    // Try to load and render the background image
                                    if let Some(image) = image_cache.load_background_image(&url) {
                                        let inherited_box = style.get_inherited_box();
                                        let image_rendering = inherited_box.image_rendering;
                                        let quality = to_image_quality(image_rendering);

                                        let transform = scroll_transform * Affine::translate((rect.x0, rect.y0));

                                        // Draw the background image to cover the entire rect
                                        painter.draw_image(to_peniko_image(&image, quality).as_ref(), transform);
                                    }
                                }
                                GenericImage::Gradient(gradient) => {
                                    // Handle gradient rendering here
                                },
                                _ => todo!()
                            }
                        }
                        GenericCrossFadeImage::Color(color) => {
                            todo!()
                        }
                    }
                }
            }
            Image::ImageSet(set) => {
                // todo: handle image set rendering
            }
            Image::LightDark(lightdark) => {
                // TODO: handle light/dark mode images
            }
        }
    }
}

/// Get default background color for elements
pub fn get_default_background_color(tag_name: &str) -> Color {
     match tag_name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Color::from_rgb(240, 240, 250),
        "div" => Color::from_rgb(248, 248, 248),
        "p" => Color::WHITE,
        "a" => Color::from_rgb(230, 240, 255),
        "button" => Color::from_rgb(242, 242, 242),
        _ => Color::WHITE,
    }
}

/// Load an image from a file path
fn load_image_from_path(path: &str) -> Option<RasterImageData> {
    use std::fs;

    // Try to read the file
    match fs::read(path) {
        Ok(data) => {
            if let Ok(image) = image::ImageReader::new(Cursor::new(data))
                .with_guessed_format()
                .expect("Failed to guess image format")
                .decode()
            {
                let rgba_image = image.to_rgba8();
                let (width, height) = rgba_image.dimensions();
                let rgba_data = rgba_image.into_raw();

                return Some(RasterImageData::new(
                    width,
                    height,
                    std::sync::Arc::new(rgba_data)
                ));
            }

            None
        }
        Err(err) => None,
    }
}

pub(crate) fn to_image_quality(image_rendering: ImageRendering) -> peniko::ImageQuality {
    match image_rendering {
        ImageRendering::Auto => peniko::ImageQuality::Medium,
        ImageRendering::CrispEdges => peniko::ImageQuality::Low,
        ImageRendering::Pixelated => peniko::ImageQuality::Low,
    }
}

pub(crate) fn to_peniko_image(image: &RasterImageData, quality: peniko::ImageQuality) -> peniko::ImageBrush {
    peniko::ImageBrush {
        image: ImageData {
            data: image.data.clone(),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: image.width,
            height: image.height,
        },
        sampler: ImageSampler {
            x_extend: peniko::Extend::Repeat,
            y_extend: peniko::Extend::Repeat,
            quality,
            alpha: 1.0,
        },
    }
}
