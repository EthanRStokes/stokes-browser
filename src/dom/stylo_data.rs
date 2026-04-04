// Copyright DioxusLabs
// Licensed under the Apache License, Version 2.0 or the MIT license.

use crate::dom::damage::ALL_DAMAGE;
use std::fmt;
use std::ops::Deref;
use std::{cell::UnsafeCell, ops::DerefMut};
use style::data::{ElementDataMut, ElementDataRef, ElementDataWrapper};
use style::properties::ComputedValues;
use style::values::computed::{BorderSideWidth, BorderStyle, PositionProperty, LengthPercentage, GridTemplateAreas};
use style::values::generics::grid::{GenericTrackSize, TrackListValue};
use style::values::specified::{GenericGridTemplateComponent};
use style::values::specified::position::NamedArea;
use stylo_atoms::Atom;
use stylo_taffy::convert;
use stylo_taffy::wrapper::{RepetitionWrapper, SliceMapIter, StyloLineNameIter};
use taffy::style_helpers::TaffyAuto;
use taffy::TrackSizingFunction;

/// Interior-mutable wrapper around `Option<ElementDataWrapper>`.
///
/// Encapsulates the `UnsafeCell` so that access sites don't need raw `unsafe` blocks.
///
/// Safety relies on:
///   - Regular static-borrow checking for regular access, and on:
///   - Stylo having exclusive access to nodes during style traversals
///   - Stylo's thread-safe traversal model: `init`/`clear` only happen during exclusive-access phases
pub struct StyloData {
    inner: UnsafeCell<Option<ElementDataWrapper>>,
}

impl Default for StyloData {
    fn default() -> Self {
        Self {
            inner: UnsafeCell::new(None),
        }
    }
}

impl fmt::Debug for StyloData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StyloData").finish_non_exhaustive()
    }
}

impl Deref for StyloData {
    type Target = Option<ElementDataWrapper>;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner.get() }
    }
}

impl DerefMut for StyloData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.get_mut()
    }
}

impl StyloData {
    /// Whether element data has been initialized.
    pub fn has_data(&self) -> bool {
        unsafe { &*self.inner.get() }.is_some()
    }

    /// Borrow the element data immutably, if present.
    pub fn get(&self) -> Option<ElementDataRef<'_>> {
        self.as_ref().map(|w| w.borrow())
    }

    /// Borrow the element data mutably, if present.
    pub fn get_mut(&mut self) -> Option<ElementDataMut<'_>> {
        self.as_mut().map(|w| w.borrow_mut())
    }

    /// Initialize the element data ready for use (if it is not already initialized)
    pub fn ensure_init_mut(&mut self) -> ElementDataMut<'_> {
        // SAFETY:
        // If we have exclusive access to self (implied by &mut self) then it safe to mutate self.
        unsafe { self.ensure_init() }
    }

    pub fn primary_styles(&self) -> Option<StyleDataRef<'_>> {
        let stylo_element_data = self.get();
        if stylo_element_data
            .as_ref()
            .and_then(|d| d.styles.get_primary())
            .is_some()
        {
            Some(StyleDataRef(self.get().unwrap()))
        } else {
            None
        }
    }

    /// Get a mutable reference to the data
    pub unsafe fn unsafe_stylo_only_mut(&self) -> Option<ElementDataMut<'_>> {
        let opt = unsafe { &mut *self.inner.get() };
        opt.as_mut().map(|w| w.borrow_mut())
    }

    /// Initialize the element data ready for use (if it is not already initialized)
    ///
    /// SAFETY:
    /// There must be no outstanding borrows to this container or anything contained within it
    /// when this method is called
    pub unsafe fn ensure_init(&self) -> ElementDataMut<'_> {
        if !self.has_data() {
            unsafe { *self.inner.get() = Some(ElementDataWrapper::default()) };
            let mut data_mut = unsafe { self.unsafe_stylo_only_mut() }.unwrap();
            data_mut.damage = ALL_DAMAGE;
            data_mut
        } else {
            unsafe { self.unsafe_stylo_only_mut() }.unwrap()
        }
    }

    /// Clear the element data, returning to the uninitialized state.
    ///
    /// SAFETY:
    /// There must be no outstanding borrows to this container or anything contained within it
    /// when this method is called
    pub unsafe fn clear(&self) {
        unsafe { *self.inner.get() = None };
    }
}

pub struct StyleDataRef<'a>(ElementDataRef<'a>);

impl Deref for StyleDataRef<'_> {
    type Target = ComputedValues;

    fn deref(&self) -> &Self::Target {
        &**self.0.styles.get_primary().unwrap()
    }
}

fn line_names_iter(names: &Vec<Atom>) -> core::slice::Iter<'_, Atom> {
    names.iter()
}

impl taffy::CoreStyle for StyleDataRef<'_> {
    type CustomIdent = Atom;

    #[inline]
    fn box_generation_mode(&self) -> taffy::BoxGenerationMode {
        convert::box_generation_mode(self.get_box().display)
    }

    #[inline]
    fn is_block(&self) -> bool {
        convert::is_block(self.get_box().display)
    }

    #[inline]
    fn box_sizing(&self) -> taffy::BoxSizing {
        convert::box_sizing(self.get_position().box_sizing)
    }

    #[inline]
    fn overflow(&self) -> taffy::Point<taffy::Overflow> {
        let box_styles = self.get_box();
        taffy::Point {
            x: convert::overflow(box_styles.overflow_x),
            y: convert::overflow(box_styles.overflow_y),
        }
    }

    #[inline]
    fn scrollbar_width(&self) -> f32 {
        0.0
    }

    #[inline]
    fn position(&self) -> taffy::Position {
        convert::position(self.get_box().position)
    }

    #[inline]
    fn inset(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        if matches!(
            self.get_box().position,
            PositionProperty::Static | PositionProperty::Sticky
        ) {
            return taffy::Rect {
                left: taffy::LengthPercentageAuto::AUTO,
                right: taffy::LengthPercentageAuto::AUTO,
                top: taffy::LengthPercentageAuto::AUTO,
                bottom: taffy::LengthPercentageAuto::AUTO,
            }
        }
        let position_styles = self.get_position();
        taffy::Rect {
            left: convert::inset(&position_styles.left),
            right: convert::inset(&position_styles.right),
            top: convert::inset(&position_styles.top),
            bottom: convert::inset(&position_styles.bottom),
        }
    }

    #[inline]
    fn size(&self) -> taffy::Size<taffy::Dimension> {
        let position_styles = self.get_position();
        taffy::Size {
            width: convert::dimension(&position_styles.width),
            height: convert::dimension(&position_styles.height),
        }
    }

    #[inline]
    fn min_size(&self) -> taffy::Size<taffy::Dimension> {
        let position_styles = self.get_position();
        taffy::Size {
            width: convert::dimension(&position_styles.min_width),
            height: convert::dimension(&position_styles.min_height),
        }
    }

    #[inline]
    fn max_size(&self) -> taffy::Size<taffy::Dimension> {
        let position_styles = self.get_position();
        taffy::Size {
            width: convert::max_size_dimension(&position_styles.max_width),
            height: convert::max_size_dimension(&position_styles.max_height),
        }
    }

    #[inline]
    fn aspect_ratio(&self) -> Option<f32> {
        convert::aspect_ratio(self.get_position().aspect_ratio)
    }

    #[inline]
    fn margin(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        let margin_styles = self.get_margin();
        taffy::Rect {
            left: convert::margin(&margin_styles.margin_left),
            right: convert::margin(&margin_styles.margin_right),
            top: convert::margin(&margin_styles.margin_top),
            bottom: convert::margin(&margin_styles.margin_bottom),
        }
    }

    #[inline]
    fn padding(&self) -> taffy::Rect<taffy::LengthPercentage> {
        let padding_styles = self.get_padding();
        taffy::Rect {
            left: convert::length_percentage(&padding_styles.padding_left.0),
            right: convert::length_percentage(&padding_styles.padding_right.0),
            top: convert::length_percentage(&padding_styles.padding_top.0),
            bottom: convert::length_percentage(&padding_styles.padding_bottom.0),
        }
    }

    #[inline]
    fn border(&self) -> taffy::Rect<taffy::LengthPercentage> {
        let border = self.get_border();
        let resolve = |width: &BorderSideWidth, style: BorderStyle| {
            taffy::LengthPercentage::length(if style.none_or_hidden() {
                0.0
            } else {
                width.0.to_f32_px()
            })
        };
        taffy::Rect {
            left: resolve(&border.border_left_width, border.border_left_style),
            right: resolve(&border.border_right_width, border.border_right_style),
            top: resolve(&border.border_top_width, border.border_top_style),
            bottom: resolve(&border.border_bottom_width, border.border_bottom_style),
        }
    }
}

impl taffy::BlockContainerStyle for StyleDataRef<'_> {
    #[inline]
    fn text_align(&self) -> taffy::TextAlign {
        convert::text_align(self.get_inherited_text().text_align)
    }
}

impl taffy::BlockItemStyle for StyleDataRef<'_> {
    #[inline]
    fn is_table(&self) -> bool {
        convert::is_table(self.get_box().display)
    }

    #[inline]
    fn float(&self) -> taffy::Float {
        convert::float(self.get_box().float)
    }

    #[inline]
    fn clear(&self) -> taffy::Clear {
        convert::clear(self.get_box().clear)
    }
}

impl taffy::FlexboxContainerStyle for StyleDataRef<'_> {
    #[inline]
    fn flex_direction(&self) -> taffy::FlexDirection {
        convert::flex_direction(self.get_position().flex_direction)
    }

    #[inline]
    fn flex_wrap(&self) -> taffy::FlexWrap {
        convert::flex_wrap(self.get_position().flex_wrap)
    }

    #[inline]
    fn gap(&self) -> taffy::Size<taffy::LengthPercentage> {
        let position_styles = self.get_position();
        taffy::Size {
            width: convert::gap(&position_styles.column_gap),
            height: convert::gap(&position_styles.row_gap),
        }
    }

    #[inline]
    fn align_content(&self) -> Option<taffy::AlignContent> {
        convert::content_alignment(self.get_position().align_content)
    }

    #[inline]
    fn align_items(&self) -> Option<taffy::AlignItems> {
        convert::item_alignment(self.get_position().align_items.0)
    }

    #[inline]
    fn justify_content(&self) -> Option<taffy::JustifyContent> {
        convert::content_alignment(self.get_position().justify_content)
    }
}

impl taffy::FlexboxItemStyle for StyleDataRef<'_> {
    #[inline]
    fn flex_basis(&self) -> taffy::Dimension {
        convert::flex_basis(&self.get_position().flex_basis)
    }

    #[inline]
    fn flex_grow(&self) -> f32 {
        self.get_position().flex_grow.0
    }

    #[inline]
    fn flex_shrink(&self) -> f32 {
        self.get_position().flex_shrink.0
    }

    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        convert::item_alignment(self.get_position().align_self.0)
    }
}

impl taffy::GridContainerStyle for StyleDataRef<'_> {
    type Repetition<'a>
        = RepetitionWrapper<'a>
    where
        Self: 'a;
    type TemplateTrackList<'a>
    = core::iter::Map<
        core::slice::Iter<'a, TrackListValue<LengthPercentage, i32>>,
        fn(
            &'a TrackListValue<LengthPercentage, i32>,
        ) -> taffy::GenericGridTemplateComponent<Atom, RepetitionWrapper<'a>>,
    >
    where
        Self: 'a;
    type AutoTrackList<'a>
    = SliceMapIter<'a, GenericTrackSize<LengthPercentage>, taffy::TrackSizingFunction>
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = StyloLineNameIter<'a>
    where
        Self: 'a;
    type GridTemplateAreas<'a>
        = SliceMapIter<'a, NamedArea, taffy::GridTemplateArea<Atom>>
    where
        Self: 'a;

    #[inline]
    fn grid_template_rows(&self) -> Option<Self::TemplateTrackList<'_>> {
        match &self.get_position().grid_template_rows {
            GenericGridTemplateComponent::None => None,
            GenericGridTemplateComponent::TrackList(list) => {
                Some(list.values.iter().map(|track| match track {
                    TrackListValue::TrackSize(size) => {
                        taffy::GenericGridTemplateComponent::Single(convert::track_size(size))
                    },
                    TrackListValue::TrackRepeat(repeat) => {
                        taffy::GenericGridTemplateComponent::Repeat(RepetitionWrapper(repeat))
                    },
                }))
            },

            // TODO: Implement subgrid and masonry
            GenericGridTemplateComponent::Subgrid(_) => None,
            GenericGridTemplateComponent::Masonry => None,
        }
    }

    #[inline]
    fn grid_template_columns(&self) -> Option<Self::TemplateTrackList<'_>> {
        match &self.get_position().grid_template_columns {
            GenericGridTemplateComponent::None => None,
            GenericGridTemplateComponent::TrackList(list) => {
                Some(list.values.iter().map(|track| match track {
                    TrackListValue::TrackSize(size) => {
                        taffy::GenericGridTemplateComponent::Single(convert::track_size(size))
                    },
                    TrackListValue::TrackRepeat(repeat) => {
                        taffy::GenericGridTemplateComponent::Repeat(RepetitionWrapper(repeat))
                    },
                }))
            },

            // TODO: Implement subgrid and masonry
            GenericGridTemplateComponent::Subgrid(_) => None,
            GenericGridTemplateComponent::Masonry => None,
        }
    }

    #[inline]
    fn grid_auto_rows(&self) -> Self::AutoTrackList<'_> {
        self.get_position()
            .grid_auto_rows
            .0
            .iter()
            .map(convert::track_size)
    }

    #[inline]
    fn grid_auto_columns(&self) -> Self::AutoTrackList<'_> {
        self.get_position()
            .grid_auto_columns
            .0
            .iter()
            .map(convert::track_size)
    }

    #[inline]
    fn grid_template_areas(&self) -> Option<Self::GridTemplateAreas<'_>> {
        match &self.get_position().grid_template_areas {
            GridTemplateAreas::Areas(areas) => {
                Some(areas.0.areas.iter().map(|area| taffy::GridTemplateArea {
                    name: area.name.clone(),
                    row_start: area.rows.start as u16,
                    row_end: area.rows.end as u16,
                    column_start: area.columns.start as u16,
                    column_end: area.columns.end as u16,
                }))
            }
            GridTemplateAreas::None => None,
        }
    }

    #[inline]
    fn grid_template_column_names(&self) -> Option<Self::TemplateLineNames<'_>> {
        match &self.get_position().grid_template_columns {
            GenericGridTemplateComponent::None => None,
            GenericGridTemplateComponent::TrackList(list) => {
                Some(StyloLineNameIter::new(&list.line_names))
            }
            // TODO: Implement subgrid and masonry
            GenericGridTemplateComponent::Subgrid(_) => None,
            GenericGridTemplateComponent::Masonry => None,
        }
    }

    #[inline]
    fn grid_template_row_names(&self) -> Option<Self::TemplateLineNames<'_>> {
        match &self.get_position().grid_template_rows {
            GenericGridTemplateComponent::None => None,
            GenericGridTemplateComponent::TrackList(list) => {
                Some(StyloLineNameIter::new(&list.line_names))
            }
            // TODO: Implement subgrid and masonry
            GenericGridTemplateComponent::Subgrid(_) => None,
            GenericGridTemplateComponent::Masonry => None,
        }
    }

    #[inline]
    fn grid_auto_flow(&self) -> taffy::GridAutoFlow {
        convert::grid_auto_flow(self.get_position().grid_auto_flow)
    }

    #[inline]
    fn gap(&self) -> taffy::Size<taffy::LengthPercentage> {
        let position_styles = self.get_position();
        taffy::Size {
            width: convert::gap(&position_styles.column_gap),
            height: convert::gap(&position_styles.row_gap),
        }
    }

    #[inline]
    fn align_content(&self) -> Option<taffy::AlignContent> {
        convert::content_alignment(self.get_position().align_content)
    }

    #[inline]
    fn justify_content(&self) -> Option<taffy::JustifyContent> {
        convert::content_alignment(self.get_position().justify_content)
    }

    #[inline]
    fn align_items(&self) -> Option<taffy::AlignItems> {
        convert::item_alignment(self.get_position().align_items.0)
    }

    #[inline]
    fn justify_items(&self) -> Option<taffy::AlignItems> {
        convert::item_alignment((self.get_position().justify_items.computed.0).0)
    }
}

impl taffy::GridItemStyle for StyleDataRef<'_> {
    #[inline]
    fn grid_row(&self) -> taffy::Line<taffy::GridPlacement<Atom>> {
        let position_styles = self.get_position();
        taffy::Line {
            start: convert::grid_line(&position_styles.grid_row_start),
            end: convert::grid_line(&position_styles.grid_row_end),
        }
    }

    #[inline]
    fn grid_column(&self) -> taffy::Line<taffy::GridPlacement<Atom>> {
        let position_styles = self.get_position();
        taffy::Line {
            start: convert::grid_line(&position_styles.grid_column_start),
            end: convert::grid_line(&position_styles.grid_column_end),
        }
    }

    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        convert::item_alignment(self.get_position().align_self.0)
    }

    #[inline]
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        convert::item_alignment(self.get_position().justify_self.0)
    }
}