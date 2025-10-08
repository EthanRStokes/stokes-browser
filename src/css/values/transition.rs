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

/// Single transition configuration for one property
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub property: TransitionProperty,
    pub duration: Duration,
    pub timing_function: TimingFunction,
    pub delay: Duration,
}

impl Transition {
    /// Create a new transition with default values
    pub fn new(property: TransitionProperty) -> Self {
        Self {
            property,
            duration: Duration(0.0),
            timing_function: TimingFunction::Ease,
            delay: Duration(0.0),
        }
    }

    /// Parse a single transition from CSS string
    /// Format: <property> <duration> <timing-function> <delay>
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();

        if value == "none" {
            return None;
        }

        let parts: Vec<&str> = value.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let mut property = TransitionProperty::All;
        let mut duration = Duration(0.0);
        let mut timing_function = TimingFunction::Ease;
        let mut delay = Duration(0.0);

        let mut duration_set = false;

        for part in parts {
            // Try to parse as property name
            if !duration_set && TransitionProperty::is_property_name(part) {
                property = TransitionProperty::parse(part);
            }
            // Try to parse as duration
            else if part.ends_with('s') || part.ends_with("ms") {
                let parsed_duration = Duration::parse(part);
                if !duration_set {
                    duration = parsed_duration;
                    duration_set = true;
                } else {
                    delay = parsed_duration;
                }
            }
            // Try to parse as timing function
            else if Self::is_timing_function(part) {
                timing_function = TimingFunction::parse(part);
            }
        }

        Some(Transition {
            property,
            duration,
            timing_function,
            delay,
        })
    }

    fn is_timing_function(value: &str) -> bool {
        matches!(value.to_lowercase().as_str(),
            "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" |
            "step-start" | "step-end"
        ) || value.starts_with("cubic-bezier(") || value.starts_with("steps(")
    }
}

/// Property that can be transitioned
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionProperty {
    All,
    BackgroundColor,
    Color,
    Opacity,
    Width,
    Height,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    Transform,
    Custom(String),
}

impl TransitionProperty {
    /// Parse property name from string
    pub fn parse(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "all" => TransitionProperty::All,
            "background-color" => TransitionProperty::BackgroundColor,
            "color" => TransitionProperty::Color,
            "opacity" => TransitionProperty::Opacity,
            "width" => TransitionProperty::Width,
            "height" => TransitionProperty::Height,
            "margin-top" => TransitionProperty::MarginTop,
            "margin-right" => TransitionProperty::MarginRight,
            "margin-bottom" => TransitionProperty::MarginBottom,
            "margin-left" => TransitionProperty::MarginLeft,
            "padding-top" => TransitionProperty::PaddingTop,
            "padding-right" => TransitionProperty::PaddingRight,
            "padding-bottom" => TransitionProperty::PaddingBottom,
            "padding-left" => TransitionProperty::PaddingLeft,
            "border-top-width" => TransitionProperty::BorderTopWidth,
            "border-right-width" => TransitionProperty::BorderRightWidth,
            "border-bottom-width" => TransitionProperty::BorderBottomWidth,
            "border-left-width" => TransitionProperty::BorderLeftWidth,
            "transform" => TransitionProperty::Transform,
            _ => TransitionProperty::Custom(value.to_string()),
        }
    }

    /// Check if a string is a valid property name for transitions
    fn is_property_name(value: &str) -> bool {
        // Common property names
        matches!(value.to_lowercase().as_str(),
            "all" | "background-color" | "color" | "opacity" | "width" | "height" |
            "margin-top" | "margin-right" | "margin-bottom" | "margin-left" |
            "padding-top" | "padding-right" | "padding-bottom" | "padding-left" |
            "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" |
            "transform"
        )
    }
}

/// Complete transition specification (can contain multiple transitions)
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionSpec {
    pub transitions: Vec<Transition>,
}

impl TransitionSpec {
    /// Parse transition specification from CSS string
    /// Format: <property> <duration> <timing-function> <delay>, ...
    pub fn parse(value: &str) -> Self {
        let value = value.trim();

        if value == "none" {
            return TransitionSpec {
                transitions: Vec::new(),
            };
        }

        // Split by commas for multiple transitions
        let transition_strings: Vec<&str> = value.split(',').map(|s| s.trim()).collect();
        let mut transitions = Vec::new();

        for transition_str in transition_strings {
            if let Some(transition) = Transition::parse(transition_str) {
                transitions.push(transition);
            }
        }

        TransitionSpec { transitions }
    }

    /// Check if any property is being transitioned
    pub fn has_transitions(&self) -> bool {
        !self.transitions.is_empty()
    }

    /// Find transition for a specific property
    pub fn get_transition_for_property(&self, property_name: &str) -> Option<&Transition> {
        for transition in &self.transitions {
            match &transition.property {
                TransitionProperty::All => return Some(transition),
                TransitionProperty::Custom(name) if name == property_name => return Some(transition),
                _ => {
                    // Check if the property matches
                    if Self::property_matches(property_name, &transition.property) {
                        return Some(transition);
                    }
                }
            }
        }
        None
    }

    fn property_matches(property_name: &str, transition_property: &TransitionProperty) -> bool {
        match transition_property {
            TransitionProperty::BackgroundColor => property_name == "background-color",
            TransitionProperty::Color => property_name == "color",
            TransitionProperty::Opacity => property_name == "opacity",
            TransitionProperty::Width => property_name == "width",
            TransitionProperty::Height => property_name == "height",
            TransitionProperty::MarginTop => property_name == "margin-top",
            TransitionProperty::MarginRight => property_name == "margin-right",
            TransitionProperty::MarginBottom => property_name == "margin-bottom",
            TransitionProperty::MarginLeft => property_name == "margin-left",
            TransitionProperty::PaddingTop => property_name == "padding-top",
            TransitionProperty::PaddingRight => property_name == "padding-right",
            TransitionProperty::PaddingBottom => property_name == "padding-bottom",
            TransitionProperty::PaddingLeft => property_name == "padding-left",
            TransitionProperty::BorderTopWidth => property_name == "border-top-width",
            TransitionProperty::BorderRightWidth => property_name == "border-right-width",
            TransitionProperty::BorderBottomWidth => property_name == "border-bottom-width",
            TransitionProperty::BorderLeftWidth => property_name == "border-left-width",
            TransitionProperty::Transform => property_name == "transform",
            _ => false,
        }
    }
}

impl Default for TransitionSpec {
    fn default() -> Self {
        TransitionSpec {
            transitions: Vec::new(),
        }
    }
}
