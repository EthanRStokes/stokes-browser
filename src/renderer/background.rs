use crate::css::{BackgroundImage, ComputedValues};
use crate::dom::node::RasterImageData;
use crate::renderer::text::TextPainter;
use kurbo::Affine;
use peniko::{ImageAlphaType, ImageData, ImageFormat, ImageSampler};
use skia_safe::{Color, Paint, Rect};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use style::properties::generated::ComputedValues as StyloComputedValues;
use style::servo_arc::Arc;
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
    pub fn load_background_image(&self, url: &str) -> Option<RasterImageData> {
        // Check cache first
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
    styles: &ComputedValues,
    style: &Arc<StyloComputedValues>,
    scale_factor: f32,
    image_cache: &BackgroundImageCache,
    scroll_transform: kurbo::Affine,
) {
    // Check if background-image is specified
    match &styles.background_image {
        BackgroundImage::None => {
            // No background image, nothing to do
            return;
        }
        BackgroundImage::Url(url) => {
            // Try to load and render the background image
            if let Some(image) = image_cache.load_background_image(url) {
                let inherited_box = style.get_inherited_box();
                let image_rendering = inherited_box.image_rendering;
                let quality = to_image_quality(image_rendering);

                let transform = scroll_transform * Affine::translate((rect.left as f64, rect.top as f64));

                // Draw the background image to cover the entire rect
                painter.draw_image(to_peniko_image(&image, quality).as_ref(), transform);
            }
        }
    }
}

/// Set default background color for elements
pub fn set_default_background_color(paint: &mut Paint, tag_name: &str) {
    let color = match tag_name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Color::from_rgb(240, 240, 250),
        "div" => Color::from_rgb(248, 248, 248),
        "p" => Color::WHITE,
        "a" => Color::from_rgb(230, 240, 255),
        "button" => Color::from_rgb(242, 242, 242),
        _ => Color::WHITE,
    };
    paint.set_color(color);
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
