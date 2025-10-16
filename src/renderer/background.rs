use crate::css::{BackgroundImage, ComputedValues};
use skia_safe::{Canvas, Color, FilterMode, MipmapMode, Paint, Rect, SamplingOptions};
use std::cell::RefCell;
// Background rendering (colors, images)
use std::collections::HashMap;

/// Background image cache manager
pub struct BackgroundImageCache {
    cache: RefCell<HashMap<String, Option<skia_safe::Image>>>,
}

impl BackgroundImageCache {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// Load a background image from URL (with caching)
    pub fn load_background_image(&self, url: &str) -> Option<skia_safe::Image> {
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
    canvas: &Canvas,
    rect: &Rect,
    styles: &ComputedValues,
    scale_factor: f64,
    image_cache: &BackgroundImageCache,
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
                let mut paint = Paint::default();
                paint.set_anti_alias(true);

                // Use high quality filtering for better scaling
                let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear);

                // Draw the background image to cover the entire rect
                canvas.draw_image_rect_with_sampling_options(
                    &image,
                    None, // Use entire source image
                    *rect,
                    sampling,
                    &paint
                );
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
        _ => Color::WHITE,
    };
    paint.set_color(color);
}

/// Load an image from a file path
fn load_image_from_path(path: &str) -> Option<skia_safe::Image> {
    use std::fs;

    // Try to read the file
    match fs::read(path) {
        Ok(data) => {
            // Decode the image data using Skia
            skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&data))
        }
        Err(_) => None,
    }
}
