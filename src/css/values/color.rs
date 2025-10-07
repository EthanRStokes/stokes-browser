// CSS color representation

/// CSS color representation
#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    Rgb { r: u8, g: u8, b: u8 },
    Rgba { r: u8, g: u8, b: u8, a: f32 },
    Named(String),
    Hex(String),
}

impl Color {
    /// Convert to Skia color
    pub fn to_skia_color(&self) -> skia_safe::Color {
        match self {
            Color::Rgb { r, g, b } => skia_safe::Color::from_rgb(*r, *g, *b),
            Color::Rgba { r, g, b, a } => skia_safe::Color::from_argb((*a * 255.0) as u8, *r, *g, *b),
            Color::Named(name) => match name.to_lowercase().as_str() {
                "black" => skia_safe::Color::BLACK,
                "white" => skia_safe::Color::WHITE,
                "red" => skia_safe::Color::RED,
                "green" => skia_safe::Color::GREEN,
                "blue" => skia_safe::Color::BLUE,
                "yellow" => skia_safe::Color::YELLOW,
                "cyan" => skia_safe::Color::CYAN,
                "magenta" => skia_safe::Color::MAGENTA,
                "gray" | "grey" => skia_safe::Color::GRAY,
                _ => skia_safe::Color::BLACK, // Default fallback
            },
            Color::Hex(hex) => {
                // Parse hex color (e.g., "#ff0000" or "#f00")
                let hex = hex.trim_start_matches('#');
                match hex.len() {
                    3 => {
                        // Short hex (#f00 -> #ff0000)
                        if let (Ok(r), Ok(g), Ok(b)) = (
                            u8::from_str_radix(&hex[0..1].repeat(2), 16),
                            u8::from_str_radix(&hex[1..2].repeat(2), 16),
                            u8::from_str_radix(&hex[2..3].repeat(2), 16),
                        ) {
                            skia_safe::Color::from_rgb(r, g, b)
                        } else {
                            skia_safe::Color::BLACK
                        }
                    },
                    6 => {
                        // Full hex (#ff0000)
                        if let (Ok(r), Ok(g), Ok(b)) = (
                            u8::from_str_radix(&hex[0..2], 16),
                            u8::from_str_radix(&hex[2..4], 16),
                            u8::from_str_radix(&hex[4..6], 16),
                        ) {
                            skia_safe::Color::from_rgb(r, g, b)
                        } else {
                            skia_safe::Color::BLACK
                        }
                    },
                    _ => skia_safe::Color::BLACK,
                }
            }
        }
    }
}

