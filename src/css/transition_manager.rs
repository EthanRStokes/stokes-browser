// CSS transition manager for handling animated property changes
use std::collections::HashMap;
use std::time::Instant;
use super::{TransitionSpec, ComputedValues, Color, Length};

/// Tracks active transitions for elements
pub struct TransitionManager {
    active_transitions: HashMap<usize, Vec<ActiveTransition>>,
}

/// An active transition for a specific property
#[derive(Debug, Clone)]
pub struct ActiveTransition {
    pub property_name: String,
    pub start_value: TransitionValue,
    pub end_value: TransitionValue,
    pub start_time: Instant,
    pub duration_ms: f32,
    pub delay_ms: f32,
    pub timing_function: super::TimingFunction,
}

/// Values that can be transitioned
#[derive(Debug, Clone)]
pub enum TransitionValue {
    Color(Color),
    Length(f32), // Already converted to pixels
    Number(f32),
}

impl TransitionManager {
    pub fn new() -> Self {
        Self {
            active_transitions: HashMap::new(),
        }
    }

    /// Start transitions when computed styles change for an element
    pub fn update_element_styles(
        &mut self,
        node_id: usize,
        old_styles: &ComputedValues,
        new_styles: &ComputedValues,
    ) {
        // Check if the element has transitions defined
        if !new_styles.transition.has_transitions() {
            return;
        }

        let mut new_transitions = Vec::new();
        let now = Instant::now();

        // Check each property that might have changed
        self.check_property_change(
            "background-color",
            &old_styles.background_color,
            &new_styles.background_color,
            &new_styles.transition,
            now,
            &mut new_transitions,
        );

        self.check_property_change(
            "color",
            &old_styles.color,
            &new_styles.color,
            &new_styles.transition,
            now,
            &mut new_transitions,
        );

        // Check dimensional properties
        self.check_length_property(
            "width",
            old_styles.width.as_ref(),
            new_styles.width.as_ref(),
            &new_styles.transition,
            now,
            &mut new_transitions,
            new_styles.font_size,
        );

        self.check_length_property(
            "height",
            old_styles.height.as_ref(),
            new_styles.height.as_ref(),
            &new_styles.transition,
            now,
            &mut new_transitions,
            new_styles.font_size,
        );

        // Check opacity property
        self.check_number_property(
            "opacity",
            old_styles.opacity,
            new_styles.opacity,
            &new_styles.transition,
            now,
            &mut new_transitions,
        );

        // Store active transitions for this element
        if !new_transitions.is_empty() {
            self.active_transitions.insert(node_id, new_transitions);
        }
    }

    /// Check if a color property has changed and start transition if needed
    fn check_property_change(
        &self,
        property_name: &str,
        old_value: &Option<Color>,
        new_value: &Option<Color>,
        transition_spec: &TransitionSpec,
        now: Instant,
        transitions: &mut Vec<ActiveTransition>,
    ) {
        // Check if values are different
        if old_value == new_value {
            return;
        }

        // Check if this property should be transitioned
        if let Some(transition) = transition_spec.get_transition_for_property(property_name) {
            if let (Some(old), Some(new)) = (old_value, new_value) {
                if transition.duration.as_millis() > 0.0 {
                    transitions.push(ActiveTransition {
                        property_name: property_name.to_string(),
                        start_value: TransitionValue::Color(old.clone()),
                        end_value: TransitionValue::Color(new.clone()),
                        start_time: now,
                        duration_ms: transition.duration.as_millis(),
                        delay_ms: transition.delay.as_millis(),
                        timing_function: transition.timing_function.clone(),
                    });
                }
            }
        }
    }

    /// Check if a length property has changed and start transition if needed
    fn check_length_property(
        &self,
        property_name: &str,
        old_value: Option<&Length>,
        new_value: Option<&Length>,
        transition_spec: &TransitionSpec,
        now: Instant,
        transitions: &mut Vec<ActiveTransition>,
        font_size: f32,
    ) {
        // Check if values are different
        if old_value == new_value {
            return;
        }

        // Check if this property should be transitioned
        if let Some(transition) = transition_spec.get_transition_for_property(property_name) {
            if let (Some(old), Some(new)) = (old_value, new_value) {
                if transition.duration.as_millis() > 0.0 {
                    // Convert to pixels for interpolation
                    let old_px = old.to_px(font_size, 400.0);
                    let new_px = new.to_px(font_size, 400.0);

                    if old_px != new_px {
                        transitions.push(ActiveTransition {
                            property_name: property_name.to_string(),
                            start_value: TransitionValue::Length(old_px),
                            end_value: TransitionValue::Length(new_px),
                            start_time: now,
                            duration_ms: transition.duration.as_millis(),
                            delay_ms: transition.delay.as_millis(),
                            timing_function: transition.timing_function.clone(),
                        });
                    }
                }
            }
        }
    }

    /// Check if a number property has changed and start transition if needed
    fn check_number_property(
        &self,
        property_name: &str,
        old_value: f32,
        new_value: f32,
        transition_spec: &TransitionSpec,
        now: Instant,
        transitions: &mut Vec<ActiveTransition>,
    ) {
        // Check if values are different
        if old_value == new_value {
            return;
        }

        // Check if this property should be transitioned
        if let Some(transition) = transition_spec.get_transition_for_property(property_name) {
            if transition.duration.as_millis() > 0.0 {
                transitions.push(ActiveTransition {
                    property_name: property_name.to_string(),
                    start_value: TransitionValue::Number(old_value),
                    end_value: TransitionValue::Number(new_value),
                    start_time: now,
                    duration_ms: transition.duration.as_millis(),
                    delay_ms: transition.delay.as_millis(),
                    timing_function: transition.timing_function.clone(),
                });
            }
        }
    }

    /// Get interpolated styles for an element at the current time
    pub fn get_interpolated_styles(
        &self,
        node_id: usize,
        base_styles: &ComputedValues,
    ) -> ComputedValues {
        let mut interpolated = base_styles.clone();

        if let Some(transitions) = self.active_transitions.get(&node_id) {
            let now = Instant::now();

            for transition in transitions {
                // Calculate elapsed time since start
                let elapsed = now.duration_since(transition.start_time).as_millis() as f32;

                // Check if we're past the delay
                if elapsed < transition.delay_ms {
                    continue; // Still in delay phase
                }

                let progress_time = elapsed - transition.delay_ms;

                // Check if transition is complete
                if progress_time >= transition.duration_ms {
                    // Transition complete, use end value
                    self.apply_transition_value(
                        &transition.property_name,
                        &transition.end_value,
                        &mut interpolated,
                    );
                    continue;
                }

                // Calculate progress (0.0 to 1.0)
                let linear_progress = progress_time / transition.duration_ms;

                // Apply timing function
                let eased_progress = transition.timing_function.apply(linear_progress);

                // Interpolate value
                let interpolated_value = Self::interpolate_value(
                    &transition.start_value,
                    &transition.end_value,
                    eased_progress,
                );

                // Apply to computed styles
                self.apply_transition_value(
                    &transition.property_name,
                    &interpolated_value,
                    &mut interpolated,
                );
            }
        }

        interpolated
    }

    /// Interpolate between two transition values
    fn interpolate_value(
        start: &TransitionValue,
        end: &TransitionValue,
        progress: f32,
    ) -> TransitionValue {
        match (start, end) {
            (TransitionValue::Color(start_color), TransitionValue::Color(end_color)) => {
                TransitionValue::Color(Self::interpolate_color(start_color, end_color, progress))
            }
            (TransitionValue::Length(start_len), TransitionValue::Length(end_len)) => {
                TransitionValue::Length(start_len + (end_len - start_len) * progress)
            }
            (TransitionValue::Number(start_num), TransitionValue::Number(end_num)) => {
                TransitionValue::Number(start_num + (end_num - start_num) * progress)
            }
            _ => end.clone(), // Type mismatch, use end value
        }
    }

    /// Interpolate between two colors
    fn interpolate_color(start: &Color, end: &Color, progress: f32) -> Color {
        let (sr, sg, sb, sa) = Self::color_to_rgba(start);
        let (er, eg, eb, ea) = Self::color_to_rgba(end);

        let r = (sr as f32 + (er as f32 - sr as f32) * progress) as u8;
        let g = (sg as f32 + (eg as f32 - sg as f32) * progress) as u8;
        let b = (sb as f32 + (eb as f32 - sb as f32) * progress) as u8;
        let a = sa + (ea - sa) * progress;

        Color::Rgba { r, g, b, a }
    }

    /// Convert color to RGBA components
    fn color_to_rgba(color: &Color) -> (u8, u8, u8, f32) {
        match color {
            Color::Rgb { r, g, b } => (*r, *g, *b, 1.0),
            Color::Rgba { r, g, b, a } => (*r, *g, *b, *a),
            Color::Named(_) => {
                // Convert named colors to RGB
                let skia_color = color.to_skia_color();
                (skia_color.r(), skia_color.g(), skia_color.b(), skia_color.a() as f32 / 255.0)
            }
            Color::Hex(_) => {
                // Convert hex colors to RGB
                let skia_color = color.to_skia_color();
                (skia_color.r(), skia_color.g(), skia_color.b(), skia_color.a() as f32 / 255.0)
            }
        }
    }

    /// Apply a transition value to computed styles
    fn apply_transition_value(
        &self,
        property_name: &str,
        value: &TransitionValue,
        styles: &mut ComputedValues,
    ) {
        match property_name {
            "background-color" => {
                if let TransitionValue::Color(color) = value {
                    styles.background_color = Some(color.clone());
                }
            }
            "color" => {
                if let TransitionValue::Color(color) = value {
                    styles.color = Some(color.clone());
                }
            }
            "width" => {
                if let TransitionValue::Length(px) = value {
                    styles.width = Some(Length::px(*px));
                }
            }
            "height" => {
                if let TransitionValue::Length(px) = value {
                    styles.height = Some(Length::px(*px));
                }
            }
            "opacity" => {
                if let TransitionValue::Number(opacity) = value {
                    styles.opacity = *opacity;
                }
            }
            _ => {
                // Property not yet supported for transitions
            }
        }
    }

    /// Check if any transitions are still active
    pub fn has_active_transitions(&self) -> bool {
        !self.active_transitions.is_empty()
    }

    /// Remove completed transitions for an element
    pub fn cleanup_completed_transitions(&mut self) {
        let now = Instant::now();

        self.active_transitions.retain(|_, transitions| {
            transitions.retain(|transition| {
                let elapsed = now.duration_since(transition.start_time).as_millis() as f32;
                let total_duration = transition.delay_ms + transition.duration_ms;
                elapsed < total_duration
            });
            !transitions.is_empty()
        });
    }
}

impl Default for TransitionManager {
    fn default() -> Self {
        Self::new()
    }
}
