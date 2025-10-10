// CSS length units and values

/// CSS length units
#[derive(Debug, Clone, PartialEq)]
pub enum Unit {
    // Absolute length units
    Px,
    Cm,  // Centimeters (1cm = 96px/2.54 ≈ 37.8px)
    Mm,  // Millimeters (1mm = 1/10 cm)
    Q,   // Quarter-millimeters (1Q = 1/40 cm)
    In,  // Inches (1in = 2.54cm = 96px)
    Pc,  // Picas (1pc = 1/6 inch = 12pt)
    Pt,  // Points (1pt = 1/72 inch, typically 1pt ≈ 1.33px at 96 DPI)

    // Font-relative length units
    Em,  // Relative to font size of the element
    Rem, // Relative to font size of root element
    Ex,  // x-height of the element's font (typically 0.5em)
    Ch,  // Width of the "0" character in the element's font
    Cap, // Cap-height (height of capital letters) of the element's font
    Ic,  // Width of the "水" (CJK water ideograph) character
    Lh,  // Line height of the element
    Rlh, // Line height of the root element

    // Viewport-percentage length units
    Vw,   // Viewport width (1vw = 1% of viewport width)
    Vh,   // Viewport height (1vh = 1% of viewport height)
    Vmin, // Minimum of vw and vh
    Vmax, // Maximum of vw and vh
    Vi,   // 1% of viewport size in the inline axis
    Vb,   // 1% of viewport size in the block axis

    // Small, Large, and Dynamic viewport units
    Svw,  // Small viewport width (1svw = 1% of small viewport width)
    Svh,  // Small viewport height (1svh = 1% of small viewport height)
    Svi,  // Small viewport inline (1svi = 1% of small viewport inline size)
    Svb,  // Small viewport block (1svb = 1% of small viewport block size)
    Svmin, // Smaller of svi or svb
    Svmax, // Larger of svi or svb
    Lvw,  // Large viewport width (1lvw = 1% of large viewport width)
    Lvh,  // Large viewport height (1lvh = 1% of large viewport height)
    Lvi,  // Large viewport inline (1lvi = 1% of large viewport inline size)
    Lvb,  // Large viewport block (1lvb = 1% of large viewport block size)
    Lvmin, // Smaller of lvi or lvb
    Lvmax, // Larger of lvi or lvb
    Dvw,  // Dynamic viewport width (1dvw = 1% of dynamic viewport width)
    Dvh,  // Dynamic viewport height (1dvh = 1% of dynamic viewport height)
    Dvi,  // Dynamic viewport inline (1dvi = 1% of dynamic viewport inline size)
    Dvb,  // Dynamic viewport block (1dvb = 1% of dynamic viewport block size)
    Dvmin, // Smaller of dvi or dvb
    Dvmax, // Larger of dvi or dvb

    // Container query length units
    Cqw,    // 1% of container's width
    Cqh,    // 1% of container's height
    Cqi,    // 1% of container's inline size
    Cqb,    // 1% of container's block size
    Cqmin,  // Smaller of cqi or cqb
    Cqmax,  // Larger of cqi or cqb

    // Special
    Percent,
    Auto,
}

/// CSS length value
#[derive(Debug, Clone, PartialEq)]
pub struct Length {
    pub value: f32,
    pub unit: Unit,
}

impl Length {
    // Absolute length units
    pub fn px(value: f32) -> Self {
        Self { value, unit: Unit::Px }
    }

    pub fn cm(value: f32) -> Self {
        Self { value, unit: Unit::Cm }
    }

    pub fn mm(value: f32) -> Self {
        Self { value, unit: Unit::Mm }
    }

    pub fn q(value: f32) -> Self {
        Self { value, unit: Unit::Q }
    }

    pub fn inch(value: f32) -> Self {
        Self { value, unit: Unit::In }
    }

    pub fn pc(value: f32) -> Self {
        Self { value, unit: Unit::Pc }
    }

    pub fn pt(value: f32) -> Self {
        Self { value, unit: Unit::Pt }
    }

    // Font-relative length units
    pub fn em(value: f32) -> Self {
        Self { value, unit: Unit::Em }
    }

    pub fn rem(value: f32) -> Self {
        Self { value, unit: Unit::Rem }
    }

    pub fn ex(value: f32) -> Self {
        Self { value, unit: Unit::Ex }
    }

    pub fn ch(value: f32) -> Self {
        Self { value, unit: Unit::Ch }
    }

    pub fn cap(value: f32) -> Self {
        Self { value, unit: Unit::Cap }
    }

    pub fn ic(value: f32) -> Self {
        Self { value, unit: Unit::Ic }
    }

    pub fn lh(value: f32) -> Self {
        Self { value, unit: Unit::Lh }
    }

    pub fn rlh(value: f32) -> Self {
        Self { value, unit: Unit::Rlh }
    }

    // Viewport-percentage length units
    pub fn vw(value: f32) -> Self {
        Self { value, unit: Unit::Vw }
    }

    pub fn vh(value: f32) -> Self {
        Self { value, unit: Unit::Vh }
    }

    pub fn vmin(value: f32) -> Self {
        Self { value, unit: Unit::Vmin }
    }

    pub fn vmax(value: f32) -> Self {
        Self { value, unit: Unit::Vmax }
    }

    pub fn vi(value: f32) -> Self {
        Self { value, unit: Unit::Vi }
    }

    pub fn vb(value: f32) -> Self {
        Self { value, unit: Unit::Vb }
    }

    // Small, Large, and Dynamic viewport units
    pub fn svw(value: f32) -> Self {
        Self { value, unit: Unit::Svw }
    }

    pub fn svh(value: f32) -> Self {
        Self { value, unit: Unit::Svh }
    }

    pub fn svi(value: f32) -> Self {
        Self { value, unit: Unit::Svi }
    }

    pub fn svb(value: f32) -> Self {
        Self { value, unit: Unit::Svb }
    }

    pub fn svmin(value: f32) -> Self {
        Self { value, unit: Unit::Svmin }
    }

    pub fn svmax(value: f32) -> Self {
        Self { value, unit: Unit::Svmax }
    }

    pub fn lvw(value: f32) -> Self {
        Self { value, unit: Unit::Lvw }
    }

    pub fn lvh(value: f32) -> Self {
        Self { value, unit: Unit::Lvh }
    }

    pub fn lvi(value: f32) -> Self {
        Self { value, unit: Unit::Lvi }
    }

    pub fn lvb(value: f32) -> Self {
        Self { value, unit: Unit::Lvb }
    }

    pub fn lvmin(value: f32) -> Self {
        Self { value, unit: Unit::Lvmin }
    }

    pub fn lvmax(value: f32) -> Self {
        Self { value, unit: Unit::Lvmax }
    }

    pub fn dvw(value: f32) -> Self {
        Self { value, unit: Unit::Dvw }
    }

    pub fn dvh(value: f32) -> Self {
        Self { value, unit: Unit::Dvh }
    }

    pub fn dvi(value: f32) -> Self {
        Self { value, unit: Unit::Dvi }
    }

    pub fn dvb(value: f32) -> Self {
        Self { value, unit: Unit::Dvb }
    }

    pub fn dvmin(value: f32) -> Self {
        Self { value, unit: Unit::Dvmin }
    }

    pub fn dvmax(value: f32) -> Self {
        Self { value, unit: Unit::Dvmax }
    }

    // Container query length units
    pub fn cqw(value: f32) -> Self {
        Self { value, unit: Unit::Cqw }
    }

    pub fn cqh(value: f32) -> Self {
        Self { value, unit: Unit::Cqh }
    }

    pub fn cqi(value: f32) -> Self {
        Self { value, unit: Unit::Cqi }
    }

    pub fn cqb(value: f32) -> Self {
        Self { value, unit: Unit::Cqb }
    }

    pub fn cqmin(value: f32) -> Self {
        Self { value, unit: Unit::Cqmin }
    }

    pub fn cqmax(value: f32) -> Self {
        Self { value, unit: Unit::Cqmax }
    }

    // Special
    pub fn percent(value: f32) -> Self {
        Self { value, unit: Unit::Percent }
    }

    pub fn auto() -> Self {
        Self { value: 0.0, unit: Unit::Auto }
    }

    /// Convert to pixels given a context
    pub fn to_px(&self, font_size: f32, parent_size: f32) -> f32 {
        match self.unit {
            // Absolute length units (all based on CSS spec with 96 DPI)
            Unit::Px => self.value,
            Unit::Cm => self.value * 96.0 / 2.54, // 1cm = 96px/2.54 ≈ 37.795px
            Unit::Mm => self.value * 96.0 / 25.4, // 1mm = 1/10 cm ≈ 3.7795px
            Unit::Q => self.value * 96.0 / 101.6, // 1Q = 1/40 cm ≈ 0.945px
            Unit::In => self.value * 96.0, // 1in = 96px (CSS standard)
            Unit::Pc => self.value * 16.0, // 1pc = 12pt = 16px
            Unit::Pt => self.value * 96.0 / 72.0, // 1pt = 1/72 inch = 96/72 = 1.333...px

            // Font-relative length units
            Unit::Em => self.value * font_size,
            Unit::Rem => self.value * 16.0, // Default root font size
            Unit::Ex => self.value * font_size * 0.5, // Approximate x-height as 0.5em
            Unit::Ch => self.value * font_size * 0.5, // Approximate width of "0" as 0.5em
            Unit::Cap => self.value * font_size * 0.7, // Approximate cap-height as 0.7em
            Unit::Ic => self.value * font_size, // Approximate CJK ideograph width as 1em
            Unit::Lh => self.value * font_size * 1.2, // Default line height (typically 1.2)
            Unit::Rlh => self.value * 16.0 * 1.2, // Root element line height

            // Viewport-percentage length units
            Unit::Vw => self.value / 100.0 * parent_size, // 1vw = 1% of viewport width
            Unit::Vh => self.value / 100.0 * parent_size, // 1vh = 1% of viewport height
            Unit::Vmin => self.value / 100.0 * parent_size, // Use parent_size as approximation
            Unit::Vmax => self.value / 100.0 * parent_size, // Use parent_size as approximation
            Unit::Vi => self.value / 100.0 * parent_size, // Inline axis (typically width)
            Unit::Vb => self.value / 100.0 * parent_size, // Block axis (typically height)

            // Small, Large, and Dynamic viewport units
            Unit::Svw => self.value / 100.0 * parent_size, // Small viewport width
            Unit::Svh => self.value / 100.0 * parent_size, // Small viewport height
            Unit::Svi => self.value / 100.0 * parent_size, // Small viewport inline
            Unit::Svb => self.value / 100.0 * parent_size, // Small viewport block
            Unit::Svmin => self.value / 100.0 * parent_size, // Smaller of svi or svb
            Unit::Svmax => self.value / 100.0 * parent_size, // Larger of svi or svb
            Unit::Lvw => self.value / 100.0 * parent_size, // Large viewport width
            Unit::Lvh => self.value / 100.0 * parent_size, // Large viewport height
            Unit::Lvi => self.value / 100.0 * parent_size, // Large viewport inline
            Unit::Lvb => self.value / 100.0 * parent_size, // Large viewport block
            Unit::Lvmin => self.value / 100.0 * parent_size, // Smaller of lvi or lvb
            Unit::Lvmax => self.value / 100.0 * parent_size, // Larger of lvi or lvb
            Unit::Dvw => self.value / 100.0 * parent_size, // Dynamic viewport width
            Unit::Dvh => self.value / 100.0 * parent_size, // Dynamic viewport height
            Unit::Dvi => self.value / 100.0 * parent_size, // Dynamic viewport inline
            Unit::Dvb => self.value / 100.0 * parent_size, // Dynamic viewport block
            Unit::Dvmin => self.value / 100.0 * parent_size, // Smaller of dvi or dvb
            Unit::Dvmax => self.value / 100.0 * parent_size, // Larger of dvi or dvb

            // Container query length units
            Unit::Cqw => self.value / 100.0 * parent_size, // 1% of container's width
            Unit::Cqh => self.value / 100.0 * parent_size, // 1% of container's height
            Unit::Cqi => self.value / 100.0 * parent_size, // 1% of container's inline size
            Unit::Cqb => self.value / 100.0 * parent_size, // 1% of container's block size
            Unit::Cqmin => self.value / 100.0 * parent_size, // Smaller of cqi or cqb
            Unit::Cqmax => self.value / 100.0 * parent_size, // Larger of cqi or cqb

            // Special
            Unit::Percent => self.value / 100.0 * parent_size,
            Unit::Auto => 0.0, // Auto should be handled by layout algorithm
        }
    }
}

impl Default for Length {
    fn default() -> Self {
        Length::px(0.0)
    }
}
