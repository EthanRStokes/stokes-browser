// Font management and caching for the renderer
use std::collections::HashMap;
use std::cell::RefCell;
use skia_safe::{Font, FontStyle, Typeface};
use skia_safe::font_style::{Weight, Width, Slant};

/// Font manager with caching capabilities
pub struct FontManager {
    pub typeface: Typeface,
    pub italic_typeface: Typeface,
    pub font_mgr: skia_safe::FontMgr,
    // Cache for different sizes - wrapped in RefCell for interior mutability
    font_cache: RefCell<HashMap<u32, Font>>, // key is font size as u32 (rounded)
    // Cache for styled fonts: (family, size, weight, style) -> Font
    styled_font_cache: RefCell<HashMap<(String, u32, String, u8), Font>>, // u8: 0=normal, 1=italic, 2=oblique
}

impl FontManager {
    pub fn new() -> Self {
        let font_mgr = skia_safe::FontMgr::new();
        let typeface = font_mgr.legacy_make_typeface(None, FontStyle::default())
            .expect("Failed to create default typeface");

        // Create italic typeface for styled text
        let italic_typeface = font_mgr.legacy_make_typeface(None, FontStyle::italic())
            .unwrap_or_else(|| typeface.clone());

        Self {
            typeface,
            italic_typeface,
            font_mgr,
            font_cache: RefCell::new(HashMap::new()),
            styled_font_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Get or create a font for the specified size
    pub fn get_font_for_size(&self, size: f32) -> Font {
        let size_key = size.round() as u32;

        // Check cache first
        {
            let cache = self.font_cache.borrow();
            if let Some(font) = cache.get(&size_key) {
                return font.clone();
            }
        }

        // Create new font and cache it
        let font = Font::new(self.typeface.clone(), size);
        self.font_cache.borrow_mut().insert(size_key, font.clone());
        font
    }

    /// Get or create a font for the specified size and style
    pub fn get_font_for_size_and_style(&self, size: f32, css_font_style: &crate::css::FontStyle) -> Font {
        // Use default font family and weight
        self.get_font("Arial", size, "normal", css_font_style)
    }

    /// Get or create a font with specified family, size, weight, and style
    pub fn get_font(&self, family: &str, size: f32, weight: &str, css_font_style: &crate::css::FontStyle) -> Font {
        let size_key = size.round() as u32;

        // Determine style key: 0=normal, 1=italic, 2=oblique
        let style_key = match css_font_style {
            crate::css::FontStyle::Normal => 0,
            crate::css::FontStyle::Italic => 1,
            crate::css::FontStyle::Oblique => 2,
        };

        let cache_key = (family.to_string(), size_key, weight.to_string(), style_key);

        // Check cache first
        {
            let cache = self.styled_font_cache.borrow();
            if let Some(font) = cache.get(&cache_key) {
                return font.clone();
            }
        }

        // Convert CSS font properties to Skia FontStyle
        let font_weight = parse_font_weight(weight);
        let font_width = Width::NORMAL;
        let font_slant = match css_font_style {
            crate::css::FontStyle::Normal => Slant::Upright,
            crate::css::FontStyle::Italic => Slant::Italic,
            crate::css::FontStyle::Oblique => Slant::Oblique,
        };

        let skia_style = FontStyle::new(Weight::from(font_weight), font_width, font_slant);
        // Try to match a typeface with the specified family and style
        let typeface = self.font_mgr.match_family_style(family, skia_style)
            .or_else(|| {
                // Fallback: try without family name (use system default)
                self.font_mgr.legacy_make_typeface(None, skia_style)
            })
            .unwrap_or_else(|| self.typeface.clone());

        // Create font with the typeface and size
        let font = Font::new(typeface, size);

        // Cache the font
        self.styled_font_cache.borrow_mut().insert(cache_key, font.clone());

        font
    }
}

/// Parse CSS font-weight string to Skia font weight value
pub fn parse_font_weight(weight: &str) -> i32 {
    match weight.to_lowercase().as_str() {
        "normal" => 400,
        "bold" => 700,
        "bolder" => 900,
        "lighter" => 300,
        "100" | "thin" => 100,
        "200" | "extra-light" | "ultra-light" => 200,
        "300" | "light" => 300,
        "400" => 400,
        "500" | "medium" => 500,
        "600" | "semi-bold" | "demi-bold" => 600,
        "700" => 700,
        "800" | "extra-bold" | "ultra-bold" => 800,
        "900" | "black" | "heavy" => 900,
        _ => {
            // Try to parse as a number
            weight.parse::<i32>().unwrap_or(400).clamp(100, 900)
        }
    }
}

