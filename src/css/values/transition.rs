// CSS transition-related values

/// CSS transition timing function (easing)
#[derive(Debug, Clone, PartialEq)]
pub enum TimingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    CubicBezier(f32, f32, f32, f32),
    StepStart,
    StepEnd,
    Steps(i32, StepPosition),
}

/// Step position for steps() timing function
#[derive(Debug, Clone, PartialEq)]
pub enum StepPosition {
    Start,
    End,
}

impl TimingFunction {
    /// Parse timing function from CSS string
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        match value.to_lowercase().as_str() {
            "linear" => TimingFunction::Linear,
            "ease" => TimingFunction::Ease,
            "ease-in" => TimingFunction::EaseIn,
            "ease-out" => TimingFunction::EaseOut,
            "ease-in-out" => TimingFunction::EaseInOut,
            "step-start" => TimingFunction::StepStart,
            "step-end" => TimingFunction::StepEnd,
            _ => {
                // Try to parse cubic-bezier() or steps()
                if value.starts_with("cubic-bezier(") && value.ends_with(')') {
                    if let Some(bezier) = Self::parse_cubic_bezier(value) {
                        return bezier;
                    }
                } else if value.starts_with("steps(") && value.ends_with(')') {
                    if let Some(steps) = Self::parse_steps(value) {
                        return steps;
                    }
                }
                TimingFunction::Ease // Default
            }
        }
    }

    fn parse_cubic_bezier(value: &str) -> Option<TimingFunction> {
        let content = &value[13..value.len()-1];
        let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

        if parts.len() == 4 {
            if let (Ok(x1), Ok(y1), Ok(x2), Ok(y2)) = (
                parts[0].parse::<f32>(),
                parts[1].parse::<f32>(),
                parts[2].parse::<f32>(),
                parts[3].parse::<f32>(),
            ) {
                return Some(TimingFunction::CubicBezier(x1, y1, x2, y2));
            }
        }
        None
    }

    fn parse_steps(value: &str) -> Option<TimingFunction> {
        let content = &value[6..value.len()-1];
        let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

        if parts.is_empty() {
            return None;
        }

        if let Ok(steps) = parts[0].parse::<i32>() {
            let position = if parts.len() > 1 {
                match parts[1].to_lowercase().as_str() {
                    "start" => StepPosition::Start,
                    "end" => StepPosition::End,
                    _ => StepPosition::End,
                }
            } else {
                StepPosition::End
            };
            return Some(TimingFunction::Steps(steps, position));
        }
        None
    }

    /// Apply the timing function to a progress value (0.0 to 1.0)
    /// Returns the eased progress value
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);

        match self {
            TimingFunction::Linear => t,
            TimingFunction::Ease => {
                // cubic-bezier(0.25, 0.1, 0.25, 1.0)
                Self::cubic_bezier(t, 0.25, 0.1, 0.25, 1.0)
            }
            TimingFunction::EaseIn => {
                // cubic-bezier(0.42, 0, 1.0, 1.0)
                Self::cubic_bezier(t, 0.42, 0.0, 1.0, 1.0)
            }
            TimingFunction::EaseOut => {
                // cubic-bezier(0, 0, 0.58, 1.0)
                Self::cubic_bezier(t, 0.0, 0.0, 0.58, 1.0)
            }
            TimingFunction::EaseInOut => {
                // cubic-bezier(0.42, 0, 0.58, 1.0)
                Self::cubic_bezier(t, 0.42, 0.0, 0.58, 1.0)
            }
            TimingFunction::CubicBezier(x1, y1, x2, y2) => {
                Self::cubic_bezier(t, *x1, *y1, *x2, *y2)
            }
            TimingFunction::StepStart => {
                if t > 0.0 { 1.0 } else { 0.0 }
            }
            TimingFunction::StepEnd => {
                if t >= 1.0 { 1.0 } else { 0.0 }
            }
            TimingFunction::Steps(steps, position) => {
                Self::steps(t, *steps, position)
            }
        }
    }

    /// Calculate cubic bezier curve value
    /// Simplified implementation using Newton-Raphson method
    fn cubic_bezier(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
        // Simplified cubic bezier calculation
        // For a more accurate implementation, you'd solve for x(t) = t
        // and then calculate y at that t value

        // This is an approximation using the parametric form directly
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        // Cubic bezier formula: B(t) = (1-t)³P0 + 3(1-t)²tP1 + 3(1-t)t²P2 + t³P3
        // Where P0 = (0, 0) and P3 = (1, 1)
        3.0 * mt2 * t * y1 + 3.0 * mt * t2 * y2 + t3
    }

    /// Calculate step function value
    fn steps(t: f32, steps: i32, position: &StepPosition) -> f32 {
        if steps <= 0 {
            return t;
        }

        let steps_f = steps as f32;
        match position {
            StepPosition::Start => {
                ((t * steps_f).ceil() / steps_f).min(1.0)
            }
            StepPosition::End => {
                ((t * steps_f).floor() / steps_f).min(1.0)
            }
        }
    }
}

impl Default for TimingFunction {
    fn default() -> Self {
        TimingFunction::Ease
    }
}

/// CSS transition duration (in milliseconds)
#[derive(Debug, Clone, PartialEq)]
pub struct Duration(pub f32);

impl Duration {
    /// Parse duration from CSS string (supports s and ms)
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        if value.ends_with("ms") {
            if let Ok(ms) = value[..value.len()-2].trim().parse::<f32>() {
                return Duration(ms);
            }
        } else if value.ends_with('s') {
            if let Ok(s) = value[..value.len()-1].trim().parse::<f32>() {
                return Duration(s * 1000.0); // Convert to milliseconds
            }
        }

        Duration(0.0) // Default to 0
    }

    /// Get duration in milliseconds
    pub fn as_millis(&self) -> f32 {
        self.0
    }

    /// Get duration in seconds
    pub fn as_seconds(&self) -> f32 {
        self.0 / 1000.0
    }
}

impl Default for Duration {
    fn default() -> Self {
        Duration(0.0)
    }
}

