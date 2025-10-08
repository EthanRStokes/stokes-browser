// Background rendering (colors, images)
use std::collections::HashMap;
use std::cell::RefCell;
use skia_safe::{Canvas, Paint, Color, Rect};
use crate::css::{ComputedValues, BackgroundImage};

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
            let image_opt = image_cache.load_background_image(url);

            if let Some(image) = image_opt {
                let mut paint = Paint::default();
                paint.set_anti_alias(true);

                // Draw the background image to cover the entire rect
                // For now, we'll use a simple "cover" behavior
                canvas.draw_image_rect(
                    &image,
                    None, // Use entire source image
                    *rect,
                    &paint
                );
            }
        }
    }
}

/// Set default background color for elements
pub fn set_default_background_color(paint: &mut Paint, tag_name: &str) {
    match tag_name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            paint.set_color(Color::from_rgb(240, 240, 250));
        },
        "div" => {
            paint.set_color(Color::from_rgb(248, 248, 248));
        },
        "p" => {
            paint.set_color(Color::WHITE);
        },
        "a" => {
            paint.set_color(Color::from_rgb(230, 240, 255));
        },
        _ => {
            paint.set_color(Color::WHITE);
        }
    }
}

/// Load an image from a file path
fn load_image_from_path(path: &str) -> Option<skia_safe::Image> {
    use std::fs;

    // Try to read the file
    match fs::read(path) {
        Ok(data) => {
            // Try to decode the image data
            match skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&data)) {
                Some(image) => Some(image),
                None => {
                    println!("Failed to decode background image: {}", path);
                    None
                }
            }
        }
        Err(e) => {
            println!("Failed to load background image {}: {}", path, e);
            None
        }
    }
}

