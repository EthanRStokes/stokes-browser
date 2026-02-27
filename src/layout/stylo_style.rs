//! A wrapper around Stylo's ComputedValues that implements Taffy's layout traits.
//!
//! This allows style-to-layout conversion to happen lazily during Taffy's layout traversal,
//! rather than requiring a separate pre-traversal pass.

use atomic_refcell::AtomicRef;
use std::ops::Deref;
use style::properties::ComputedValues;
use stylo_atoms::Atom;

/// A wrapper that holds a reference to Stylo's ComputedValues (via AtomicRef)
/// and implements all of Taffy's style traits for use with Taffy's layout algorithms.
///
/// This wrapper delegates to `stylo_taffy::TaffyStyloStyle` for core, flexbox, and grid
/// style traits, and additionally implements float/clear support.
pub struct StyloStyleRef<'a> {
    inner: stylo_taffy::TaffyStyloStyle<StyloComputedRef<'a>>,
}

/// A newtype wrapper around AtomicRef<ComputedValues> that implements Deref<Target = ComputedValues>
pub struct StyloComputedRef<'a>(pub AtomicRef<'a, ComputedValues>);

impl Deref for StyloComputedRef<'_> {
    type Target = ComputedValues;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> StyloStyleRef<'a> {
    /// Create a new StyloStyleRef from an AtomicRef to ComputedValues
    pub fn new(style: AtomicRef<'a, ComputedValues>) -> Self {
        Self {
            inner: stylo_taffy::TaffyStyloStyle(StyloComputedRef(style)),
        }
    }

    /// Get a reference to the underlying ComputedValues
    pub fn computed_values(&self) -> &ComputedValues {
        &self.inner.0
    }
}

// Delegate CoreStyle to TaffyStyloStyle
impl taffy::CoreStyle for StyloStyleRef<'_> {
    type CustomIdent = Atom;

    #[inline]
    fn box_generation_mode(&self) -> taffy::BoxGenerationMode {
        self.inner.box_generation_mode()
    }

    #[inline]
    fn is_block(&self) -> bool {
        self.inner.is_block()
    }

    #[inline]
    fn box_sizing(&self) -> taffy::BoxSizing {
        self.inner.box_sizing()
    }

    #[inline]
    fn overflow(&self) -> taffy::Point<taffy::Overflow> {
        self.inner.overflow()
    }

    #[inline]
    fn scrollbar_width(&self) -> f32 {
        self.inner.scrollbar_width()
    }

    #[inline]
    fn position(&self) -> taffy::Position {
        self.inner.position()
    }

    #[inline]
    fn inset(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.inner.inset()
    }

    #[inline]
    fn size(&self) -> taffy::Size<taffy::Dimension> {
        self.inner.size()
    }

    #[inline]
    fn min_size(&self) -> taffy::Size<taffy::Dimension> {
        self.inner.min_size()
    }

    #[inline]
    fn max_size(&self) -> taffy::Size<taffy::Dimension> {
        self.inner.max_size()
    }

    #[inline]
    fn aspect_ratio(&self) -> Option<f32> {
        self.inner.aspect_ratio()
    }

    #[inline]
    fn margin(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.inner.margin()
    }

    #[inline]
    fn padding(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.inner.padding()
    }

    #[inline]
    fn border(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.inner.border()
    }
}

// BlockContainerStyle
impl taffy::BlockContainerStyle for StyloStyleRef<'_> {
    #[inline]
    fn text_align(&self) -> taffy::TextAlign {
        self.inner.text_align()
    }
}

// BlockItemStyle with float/clear support
impl taffy::BlockItemStyle for StyloStyleRef<'_> {
    #[inline]
    fn is_table(&self) -> bool {
        self.inner.is_table()
    }

    #[inline]
    fn float(&self) -> taffy::Float {
        stylo_taffy::convert::float(self.computed_values().clone_float())
    }

    #[inline]
    fn clear(&self) -> taffy::Clear {
        stylo_taffy::convert::clear(self.computed_values().clone_clear())
    }
}

// FlexboxContainerStyle
impl taffy::FlexboxContainerStyle for StyloStyleRef<'_> {
    #[inline]
    fn flex_direction(&self) -> taffy::FlexDirection {
        self.inner.flex_direction()
    }

    #[inline]
    fn flex_wrap(&self) -> taffy::FlexWrap {
        self.inner.flex_wrap()
    }

    #[inline]
    fn gap(&self) -> taffy::Size<taffy::LengthPercentage> {
        taffy::FlexboxContainerStyle::gap(&self.inner)
    }

    #[inline]
    fn align_content(&self) -> Option<taffy::AlignContent> {
        taffy::FlexboxContainerStyle::align_content(&self.inner)
    }

    #[inline]
    fn align_items(&self) -> Option<taffy::AlignItems> {
        taffy::FlexboxContainerStyle::align_items(&self.inner)
    }

    #[inline]
    fn justify_content(&self) -> Option<taffy::JustifyContent> {
        taffy::FlexboxContainerStyle::justify_content(&self.inner)
    }
}

// FlexboxItemStyle
impl taffy::FlexboxItemStyle for StyloStyleRef<'_> {
    #[inline]
    fn flex_basis(&self) -> taffy::Dimension {
        self.inner.flex_basis()
    }

    #[inline]
    fn flex_grow(&self) -> f32 {
        self.inner.flex_grow()
    }

    #[inline]
    fn flex_shrink(&self) -> f32 {
        self.inner.flex_shrink()
    }

    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        taffy::FlexboxItemStyle::align_self(&self.inner)
    }
}

// GridContainerStyle
impl taffy::GridContainerStyle for StyloStyleRef<'_> {
    type Repetition<'a>
        = <stylo_taffy::TaffyStyloStyle<StyloComputedRef<'a>> as taffy::GridContainerStyle>::Repetition<'a>
    where
        Self: 'a;
    type TemplateTrackList<'a>
        = <stylo_taffy::TaffyStyloStyle<StyloComputedRef<'a>> as taffy::GridContainerStyle>::TemplateTrackList<'a>
    where
        Self: 'a;
    type AutoTrackList<'a>
        = <stylo_taffy::TaffyStyloStyle<StyloComputedRef<'a>> as taffy::GridContainerStyle>::AutoTrackList<'a>
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = <stylo_taffy::TaffyStyloStyle<StyloComputedRef<'a>> as taffy::GridContainerStyle>::TemplateLineNames<'a>
    where
        Self: 'a;
    type GridTemplateAreas<'a>
        = <stylo_taffy::TaffyStyloStyle<StyloComputedRef<'a>> as taffy::GridContainerStyle>::GridTemplateAreas<'a>
    where
        Self: 'a;

    #[inline]
    fn grid_template_rows(&self) -> Option<Self::TemplateTrackList<'_>> {
        self.inner.grid_template_rows()
    }

    #[inline]
    fn grid_template_columns(&self) -> Option<Self::TemplateTrackList<'_>> {
        self.inner.grid_template_columns()
    }

    #[inline]
    fn grid_auto_rows(&self) -> Self::AutoTrackList<'_> {
        self.inner.grid_auto_rows()
    }

    #[inline]
    fn grid_auto_columns(&self) -> Self::AutoTrackList<'_> {
        self.inner.grid_auto_columns()
    }

    #[inline]
    fn grid_template_areas(&self) -> Option<Self::GridTemplateAreas<'_>> {
        self.inner.grid_template_areas()
    }

    #[inline]
    fn grid_template_column_names(&self) -> Option<Self::TemplateLineNames<'_>> {
        self.inner.grid_template_column_names()
    }

    #[inline]
    fn grid_template_row_names(&self) -> Option<Self::TemplateLineNames<'_>> {
        self.inner.grid_template_row_names()
    }

    #[inline]
    fn grid_auto_flow(&self) -> taffy::GridAutoFlow {
        self.inner.grid_auto_flow()
    }

    #[inline]
    fn gap(&self) -> taffy::Size<taffy::LengthPercentage> {
        taffy::GridContainerStyle::gap(&self.inner)
    }

    #[inline]
    fn align_content(&self) -> Option<taffy::AlignContent> {
        taffy::GridContainerStyle::align_content(&self.inner)
    }

    #[inline]
    fn justify_content(&self) -> Option<taffy::JustifyContent> {
        taffy::GridContainerStyle::justify_content(&self.inner)
    }

    #[inline]
    fn align_items(&self) -> Option<taffy::AlignItems> {
        taffy::GridContainerStyle::align_items(&self.inner)
    }

    #[inline]
    fn justify_items(&self) -> Option<taffy::AlignItems> {
        self.inner.justify_items()
    }
}

// GridItemStyle
impl taffy::GridItemStyle for StyloStyleRef<'_> {
    #[inline]
    fn grid_row(&self) -> taffy::Line<taffy::GridPlacement<Atom>> {
        self.inner.grid_row()
    }

    #[inline]
    fn grid_column(&self) -> taffy::Line<taffy::GridPlacement<Atom>> {
        self.inner.grid_column()
    }

    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        taffy::GridItemStyle::align_self(&self.inner)
    }

    #[inline]
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.inner.justify_self()
    }
}

