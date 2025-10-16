use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::{Font, FontStyle, Typeface};
use std::cell::RefCell;
// Font management and caching for the renderer
use std::collections::HashMap;

/// Font manager with caching capabilities
pub struct FontManager {
    pub typeface: Typeface,
    pub italic_typeface: Typeface,
    pub font_mgr: skia_safe::FontMgr,
    // Cache for different sizes - wrapped in RefCell for interior mutability
    font_cache: RefCell<HashMap<u32, Font>>, // key is font size as u32 (rounded)
    // Cache for styled fonts: (family, size, weight, style) -> Font
    styled_font_cache: RefCell<HashMap<(String, u32, String, u8), Font>>, // u8: 0=normal, 1=italic, 2=oblique
    // Cache for typefaces by family and style to avoid recreating them
    typeface_cache: RefCell<HashMap<(String, i32, u8), Typeface>>,
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
            typeface_cache: RefCell::new(HashMap::new()),
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

        // Get typeface with proper font family matching
        let typeface = self.get_typeface_for_family(family, font_weight, style_key);

        // Create font with the typeface and size
        let mut font = Font::new(typeface, size);

        // Configure font properties for proper rendering
        // Enable embolden if we have a bold weight but the typeface doesn't support it natively
        if font_weight >= 600 {
            font.set_embolden(true);
        }

        // Set skew for italic/oblique if needed
        if style_key > 0 {
            font.set_skew_x(-0.25); // Standard italic skew
        }

        // Enable subpixel positioning for better text rendering
        font.set_subpixel(true);

        // Enable linear metrics for smoother scaling
        font.set_linear_metrics(true);

        // Cache the font
        self.styled_font_cache.borrow_mut().insert(cache_key, font.clone());

        font
    }

    /// Get a typeface for the specified font family with fallbacks
    fn get_typeface_for_family(&self, family: &str, weight: i32, style_key: u8) -> Typeface {
        let typeface_key = (family.to_string(), weight, style_key);

        // Check typeface cache first
        {
            let cache = self.typeface_cache.borrow();
            if let Some(typeface) = cache.get(&typeface_key) {
                return typeface.clone();
            }
        }

        let font_slant = match style_key {
            1 => Slant::Italic,
            2 => Slant::Oblique,
            _ => Slant::Upright,
        };

        let skia_style = FontStyle::new(Weight::from(weight), Width::NORMAL, font_slant);

        // Parse the font family list (CSS can have multiple fallbacks)
        let families: Vec<&str> = family.split(',')
            .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\''))
            .collect();

        // Try each font family in order
        let mut typeface = None;
        for font_family in &families {
            let family_name = normalize_font_family(font_family);

            // Try to match with the specified family name
            if let Some(tf) = self.font_mgr.match_family_style(&family_name, skia_style) {
                typeface = Some(tf);
                break;
            }
        }

        // If no match found, try generic family names as fallbacks
        if typeface.is_none() {
            for generic in &["Arial", "Helvetica", "Times New Roman", "Courier New"] {
                if let Some(tf) = self.font_mgr.match_family_style(generic, skia_style) {
                    typeface = Some(tf);
                    break;
                }
            }
        }

        // Final fallback to default typeface with style applied
        let typeface = typeface.unwrap_or_else(|| {
            // Try to create a styled version of the default typeface
            if let Some(tf) = self.font_mgr.legacy_make_typeface(None, skia_style) {
                tf
            } else if style_key > 0 {
                self.italic_typeface.clone()
            } else {
                self.typeface.clone()
            }
        });

        // Cache the typeface
        self.typeface_cache.borrow_mut().insert(typeface_key, typeface.clone());

        typeface
    }
}

/// Normalize font family names to handle common aliases and variations
fn normalize_font_family(family: &str) -> String {
    let normalized = family.to_lowercase();

    // Map common font family aliases to their actual names
    match normalized.as_str() {
        "arial" => "Arial".to_string(),
        "helvetica" => "Helvetica".to_string(),
        "times" | "times new roman" => "Times New Roman".to_string(),
        "courier" | "courier new" => "Courier New".to_string(),
        "georgia" => "Georgia".to_string(),
        "verdana" => "Verdana".to_string(),
        "tahoma" => "Tahoma".to_string(),
        "trebuchet ms" => "Trebuchet MS".to_string(),
        "comic sans ms" => "Comic Sans MS".to_string(),
        "impact" => "Impact".to_string(),
        "lucida console" => "Lucida Console".to_string(),
        "palatino" => "Palatino".to_string(),
        "garamond" => "Garamond".to_string(),
        "bookman" => "Bookman".to_string(),
        "arial black" => "Arial Black".to_string(),
        // Generic families - pass through as-is
        "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" => family.to_string(),
        // For unrecognized fonts, preserve the original capitalization
        _ => family.to_string(),
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
