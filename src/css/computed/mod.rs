// Computed CSS values and style resolution
//
// This module is responsible for computing final CSS values for DOM nodes
// by applying the CSS cascade (specificity, inheritance, etc.).
//
// Module structure:
// - values: ComputedValues struct and DisplayType enum
// - resolver: StyleResolver for matching rules and computing styles
// - applicator: Logic for applying CSS declarations to computed values
// - shorthands: Parsers for CSS shorthand properties (font, background, etc.)

mod values;
mod resolver;
mod applicator;
mod shorthands;

pub use resolver::StyleResolver;
// Re-export main types
pub use values::{ComputedValues, DisplayType};

