// CSS length units and values

/// CSS length units
#[derive(Debug, Clone, PartialEq)]
pub enum Unit {
    Px,
    Pt, // Points (1pt = 1/72 inch, typically 1pt ≈ 1.33px at 96 DPI)
    Em,
    Rem,
    Percent,
    Vw, // Viewport width (1vw = 1% of viewport width)
    Vh, // Viewport height (1vh = 1% of viewport height)
    Auto,
}

/// CSS length value
#[derive(Debug, Clone, PartialEq)]
pub struct Length {
    pub value: f32,
    pub unit: Unit,
}

impl Length {
    pub fn px(value: f32) -> Self {
        Self { value, unit: Unit::Px }
    }

    pub fn pt(value: f32) -> Self {
        Self { value, unit: Unit::Pt }
    }

    pub fn em(value: f32) -> Self {
        Self { value, unit: Unit::Em }
    }

    pub fn percent(value: f32) -> Self {
        Self { value, unit: Unit::Percent }
    }

    pub fn vw(value: f32) -> Self {
        Self { value, unit: Unit::Vw }
    }

    pub fn vh(value: f32) -> Self {
        Self { value, unit: Unit::Vh }
    }

    /// Convert to pixels given a context
    pub fn to_px(&self, font_size: f32, parent_size: f32) -> f32 {
        match self.unit {
            Unit::Px => self.value,
            Unit::Pt => self.value * 4.0 / 3.0, // Convert points to pixels (1pt = 4/3 px at standard 96 DPI)
            Unit::Em => self.value * font_size,
            Unit::Rem => self.value * 16.0, // Default root font size
            Unit::Percent => self.value / 100.0 * parent_size,
            Unit::Auto => 0.0, // Auto should be handled by layout algorithm
            Unit::Vw => self.value / 100.0 * parent_size, // 1vw = 1% of viewport width
            Unit::Vh => self.value / 100.0 * parent_size, // 1vh = 1% of viewport height
        }
    }
}

impl Default for Length {
    fn default() -> Self {
        Length::px(0.0)
    }
}
