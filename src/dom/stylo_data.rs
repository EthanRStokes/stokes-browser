// Copyright DioxusLabs
// Licensed under the Apache License, Version 2.0 or the MIT license.

use std::fmt;
use std::ops::Deref;
use std::{cell::UnsafeCell, ops::DerefMut};
use style::data::{ElementDataMut, ElementDataRef, ElementDataWrapper};
use style::properties::ComputedValues;
use crate::dom::damage::ALL_DAMAGE;
use stylo_atoms::Atom;

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

    fn taffy_style(&self) -> Option<stylo_taffy::TaffyStyloStyle<StyleDataRef<'_>>> {
        self.primary_styles().map(stylo_taffy::TaffyStyloStyle)
    }

}

pub struct StyleDataRef<'a>(ElementDataRef<'a>);

impl Deref for StyleDataRef<'_> {
    type Target = ComputedValues;

    fn deref(&self) -> &Self::Target {
        &**self.0.styles.get_primary().unwrap()
    }
}

#[derive(Clone)]
pub struct OwnedGridTrackList {
    items: Vec<taffy::GridTemplateComponent<Atom>>,
    idx: usize,
}

impl OwnedGridTrackList {
    fn new(items: Vec<taffy::GridTemplateComponent<Atom>>) -> Self {
        Self { items, idx: 0 }
    }
}

impl Iterator for OwnedGridTrackList {
    type Item = taffy::GenericGridTemplateComponent<Atom, OwnedGridRepetition>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.items.get(self.idx)?.clone();
        self.idx += 1;
        Some(match item {
            taffy::GridTemplateComponent::Single(size) => {
                taffy::GenericGridTemplateComponent::Single(size)
            }
            taffy::GridTemplateComponent::Repeat(repetition) => {
                taffy::GenericGridTemplateComponent::Repeat(OwnedGridRepetition::from(repetition))
            }
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.items.len().saturating_sub(self.idx);
        (len, Some(len))
    }
}

impl ExactSizeIterator for OwnedGridTrackList {
    fn len(&self) -> usize {
        self.items.len().saturating_sub(self.idx)
    }
}

#[derive(Clone)]
pub struct OwnedGridAutoTrackList {
    items: Vec<taffy::TrackSizingFunction>,
    idx: usize,
}

impl OwnedGridAutoTrackList {
    fn new(items: Vec<taffy::TrackSizingFunction>) -> Self {
        Self { items, idx: 0 }
    }
}

impl Iterator for OwnedGridAutoTrackList {
    type Item = taffy::TrackSizingFunction;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.items.get(self.idx).copied();
        if item.is_some() {
            self.idx += 1;
        }
        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.items.len().saturating_sub(self.idx);
        (len, Some(len))
    }
}

impl ExactSizeIterator for OwnedGridAutoTrackList {
    fn len(&self) -> usize {
        self.items.len().saturating_sub(self.idx)
    }
}

#[derive(Clone)]
pub struct OwnedGridAreas {
    items: Vec<taffy::GridTemplateArea<Atom>>,
    idx: usize,
}

impl OwnedGridAreas {
    fn new(items: Vec<taffy::GridTemplateArea<Atom>>) -> Self {
        Self { items, idx: 0 }
    }
}

impl Iterator for OwnedGridAreas {
    type Item = taffy::GridTemplateArea<Atom>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.items.get(self.idx).cloned();
        if item.is_some() {
            self.idx += 1;
        }
        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.items.len().saturating_sub(self.idx);
        (len, Some(len))
    }
}

impl ExactSizeIterator for OwnedGridAreas {
    fn len(&self) -> usize {
        self.items.len().saturating_sub(self.idx)
    }
}

fn line_names_iter(names: &Vec<Atom>) -> core::slice::Iter<'_, Atom> {
    names.iter()
}

#[derive(Clone)]
pub struct OwnedGridRepetition {
    count: taffy::RepetitionCount,
    tracks: Vec<taffy::TrackSizingFunction>,
    line_names: Vec<Vec<Atom>>,
}

impl From<taffy::GridTemplateRepetition<Atom>> for OwnedGridRepetition {
    fn from(value: taffy::GridTemplateRepetition<Atom>) -> Self {
        Self {
            count: value.count,
            tracks: value.tracks,
            line_names: value.line_names,
        }
    }
}

impl taffy::GenericRepetition for OwnedGridRepetition {
    type CustomIdent = Atom;

    type RepetitionTrackList<'a>
        = OwnedGridAutoTrackList
    where
        Self: 'a;

    type TemplateLineNames<'a>
        = core::iter::Map<
        core::slice::Iter<'a, Vec<Atom>>,
        fn(&Vec<Atom>) -> core::slice::Iter<'_, Atom>,
    >
    where
        Self: 'a;

    fn count(&self) -> taffy::RepetitionCount {
        self.count
    }

    fn tracks(&self) -> Self::RepetitionTrackList<'_> {
        OwnedGridAutoTrackList::new(self.tracks.clone())
    }

    fn lines_names(&self) -> Self::TemplateLineNames<'_> {
        self.line_names.iter().map(line_names_iter)
    }
}

impl taffy::CoreStyle for StyloData {
    type CustomIdent = Atom;

    #[inline]
    fn box_generation_mode(&self) -> taffy::BoxGenerationMode {
        self.taffy_style().unwrap().box_generation_mode()
    }

    #[inline]
    fn is_block(&self) -> bool {
        self.taffy_style().unwrap().is_block()
    }

    #[inline]
    fn box_sizing(&self) -> taffy::BoxSizing {
        self.taffy_style().unwrap().box_sizing()
    }

    #[inline]
    fn overflow(&self) -> taffy::Point<taffy::Overflow> {
        self.taffy_style().unwrap().overflow()
    }

    #[inline]
    fn scrollbar_width(&self) -> f32 {
        self.taffy_style().unwrap().scrollbar_width()
    }

    #[inline]
    fn position(&self) -> taffy::Position {
        self.taffy_style().unwrap().position()
    }

    #[inline]
    fn inset(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.taffy_style().unwrap().inset()
    }

    #[inline]
    fn size(&self) -> taffy::Size<taffy::Dimension> {
        self.taffy_style().unwrap().size()
    }

    #[inline]
    fn min_size(&self) -> taffy::Size<taffy::Dimension> {
        self.taffy_style().unwrap().min_size()
    }

    #[inline]
    fn max_size(&self) -> taffy::Size<taffy::Dimension> {
        self.taffy_style().unwrap().max_size()
    }

    #[inline]
    fn aspect_ratio(&self) -> Option<f32> {
        self.taffy_style().unwrap().aspect_ratio()
    }

    #[inline]
    fn margin(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.taffy_style().unwrap().margin()
    }

    #[inline]
    fn padding(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.taffy_style().unwrap().padding()
    }

    #[inline]
    fn border(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.taffy_style().unwrap().border()
    }
}

impl taffy::BlockContainerStyle for StyloData {
    #[inline]
    fn text_align(&self) -> taffy::TextAlign {
        self.taffy_style().unwrap().text_align()
    }
}

impl taffy::BlockItemStyle for StyloData {
    #[inline]
    fn is_table(&self) -> bool {
        self.taffy_style().unwrap().is_table()
    }

    #[inline]
    fn float(&self) -> taffy::Float {
        let style = self.taffy_style().unwrap();
        stylo_taffy::convert::float(style.0.clone_float())
    }

    #[inline]
    fn clear(&self) -> taffy::Clear {
        let style = self.taffy_style().unwrap();
        stylo_taffy::convert::clear(style.0.clone_clear())
    }
}

impl taffy::FlexboxContainerStyle for StyloData {
    #[inline]
    fn flex_direction(&self) -> taffy::FlexDirection {
        self.taffy_style().unwrap().flex_direction()
    }

    #[inline]
    fn flex_wrap(&self) -> taffy::FlexWrap {
        self.taffy_style().unwrap().flex_wrap()
    }

    #[inline]
    fn gap(&self) -> taffy::Size<taffy::LengthPercentage> {
        let style = self.taffy_style().unwrap();
        taffy::FlexboxContainerStyle::gap(&style)
    }

    #[inline]
    fn align_content(&self) -> Option<taffy::AlignContent> {
        let style = self.taffy_style().unwrap();
        taffy::FlexboxContainerStyle::align_content(&style)
    }

    #[inline]
    fn align_items(&self) -> Option<taffy::AlignItems> {
        let style = self.taffy_style().unwrap();
        taffy::FlexboxContainerStyle::align_items(&style)
    }

    #[inline]
    fn justify_content(&self) -> Option<taffy::JustifyContent> {
        let style = self.taffy_style().unwrap();
        taffy::FlexboxContainerStyle::justify_content(&style)
    }
}

impl taffy::FlexboxItemStyle for StyloData {
    #[inline]
    fn flex_basis(&self) -> taffy::Dimension {
        self.taffy_style().unwrap().flex_basis()
    }

    #[inline]
    fn flex_grow(&self) -> f32 {
        self.taffy_style().unwrap().flex_grow()
    }

    #[inline]
    fn flex_shrink(&self) -> f32 {
        self.taffy_style().unwrap().flex_shrink()
    }

    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        let style = self.taffy_style().unwrap();
        taffy::FlexboxItemStyle::align_self(&style)
    }
}

impl taffy::GridContainerStyle for StyloData {
    type Repetition<'a>
        = OwnedGridRepetition
    where
        Self: 'a;
    type TemplateTrackList<'a>
        = OwnedGridTrackList
    where
        Self: 'a;
    type AutoTrackList<'a>
        = OwnedGridAutoTrackList
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = core::iter::Map<
        core::slice::Iter<'a, Vec<Atom>>,
        fn(&Vec<Atom>) -> core::slice::Iter<'_, Atom>,
    >
    where
        Self: 'a;
    type GridTemplateAreas<'a>
        = OwnedGridAreas
    where
        Self: 'a;

    #[inline]
    fn grid_template_rows(&self) -> Option<Self::TemplateTrackList<'_>> {
        let style = stylo_taffy::to_taffy_style(&*self.primary_styles().unwrap());
        Some(OwnedGridTrackList::new(style.grid_template_rows))
    }

    #[inline]
    fn grid_template_columns(&self) -> Option<Self::TemplateTrackList<'_>> {
        let style = stylo_taffy::to_taffy_style(&*self.primary_styles().unwrap());
        Some(OwnedGridTrackList::new(style.grid_template_columns))
    }

    #[inline]
    fn grid_auto_rows(&self) -> Self::AutoTrackList<'_> {
        let style = stylo_taffy::to_taffy_style(&*self.primary_styles().unwrap());
        OwnedGridAutoTrackList::new(style.grid_auto_rows)
    }

    #[inline]
    fn grid_auto_columns(&self) -> Self::AutoTrackList<'_> {
        let style = stylo_taffy::to_taffy_style(&*self.primary_styles().unwrap());
        OwnedGridAutoTrackList::new(style.grid_auto_columns)
    }

    #[inline]
    fn grid_template_areas(&self) -> Option<Self::GridTemplateAreas<'_>> {
        let style = stylo_taffy::to_taffy_style(&*self.primary_styles().unwrap());
        Some(OwnedGridAreas::new(style.grid_template_areas))
    }

    #[inline]
    fn grid_template_column_names(&self) -> Option<Self::TemplateLineNames<'_>> {
        let style = stylo_taffy::to_taffy_style(&*self.primary_styles().unwrap());
        let line_names = Box::leak(style.grid_template_column_names.into_boxed_slice());
        Some(line_names.iter().map(line_names_iter))
    }

    #[inline]
    fn grid_template_row_names(&self) -> Option<Self::TemplateLineNames<'_>> {
        let style = stylo_taffy::to_taffy_style(&*self.primary_styles().unwrap());
        let line_names = Box::leak(style.grid_template_row_names.into_boxed_slice());
        Some(line_names.iter().map(line_names_iter))
    }

    #[inline]
    fn grid_auto_flow(&self) -> taffy::GridAutoFlow {
        self.taffy_style().unwrap().grid_auto_flow()
    }

    #[inline]
    fn gap(&self) -> taffy::Size<taffy::LengthPercentage> {
        let style = self.taffy_style().unwrap();
        taffy::GridContainerStyle::gap(&style)
    }

    #[inline]
    fn align_content(&self) -> Option<taffy::AlignContent> {
        let style = self.taffy_style().unwrap();
        taffy::GridContainerStyle::align_content(&style)
    }

    #[inline]
    fn justify_content(&self) -> Option<taffy::JustifyContent> {
        let style = self.taffy_style().unwrap();
        taffy::GridContainerStyle::justify_content(&style)
    }

    #[inline]
    fn align_items(&self) -> Option<taffy::AlignItems> {
        let style = self.taffy_style().unwrap();
        taffy::GridContainerStyle::align_items(&style)
    }

    #[inline]
    fn justify_items(&self) -> Option<taffy::AlignItems> {
        self.taffy_style().unwrap().justify_items()
    }
}

impl taffy::GridItemStyle for StyloData {
    #[inline]
    fn grid_row(&self) -> taffy::Line<taffy::GridPlacement<Atom>> {
        self.taffy_style().unwrap().grid_row()
    }

    #[inline]
    fn grid_column(&self) -> taffy::Line<taffy::GridPlacement<Atom>> {
        self.taffy_style().unwrap().grid_column()
    }

    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        let style = self.taffy_style().unwrap();
        taffy::GridItemStyle::align_self(&style)
    }

    #[inline]
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.taffy_style().unwrap().justify_self()
    }
}