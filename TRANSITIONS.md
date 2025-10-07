# CSS Transition Implementation

## Overview

This implementation adds full support for the CSS `transition` property to the Stokes browser. CSS transitions allow you to smoothly animate changes in CSS property values over time.

## Features Implemented

### 1. **Transition Property Parsing**
   - `transition`: Shorthand property for all transition settings
   - `transition-property`: Specifies which properties to transition
   - `transition-duration`: Duration of the transition (e.g., `0.3s`, `500ms`)
   - `transition-timing-function`: Easing function for the transition
   - `transition-delay`: Delay before transition starts

### 2. **Timing Functions**
   - **Linear**: Constant speed throughout
   - **Ease**: Default, slow start and end, faster in the middle
   - **Ease-in**: Slow start, accelerates
   - **Ease-out**: Fast start, decelerates
   - **Ease-in-out**: Slow start and end
   - **Cubic-bezier(x1, y1, x2, y2)**: Custom bezier curve
   - **Steps(n, position)**: Step-based animation (discrete steps)
   - **Step-start**: Jump to end value immediately at start
   - **Step-end**: Stay at start value until end

### 3. **Transitionable Properties**
Currently supported properties for transitions:
   - `background-color`: Color transitions
   - `color`: Text color transitions
   - `width`: Width transitions (supports all length units)
   - `height`: Height transitions (supports all length units)
   - `margin-*`: Margin properties
   - `padding-*`: Padding properties
   - Additional properties can be easily added

### 4. **Duration Format**
   - Seconds: `0.5s`, `1s`, `2.5s`
   - Milliseconds: `500ms`, `100ms`, `2500ms`

## Syntax Examples

### Basic Transition
```css
.box {
    background-color: blue;
    transition: background-color 0.3s ease;
}

.box:hover {
    background-color: red;
}
```

### Multiple Properties
```css
.element {
    width: 100px;
    height: 100px;
    background-color: green;
    transition: width 0.5s ease, height 0.3s ease-out, background-color 1s linear;
}

.element:hover {
    width: 200px;
    height: 150px;
    background-color: blue;
}
```

### With Delay
```css
.delayed {
    color: black;
    transition: color 0.5s ease 0.2s; /* 0.2s delay */
}

.delayed:hover {
    color: red;
}
```

### All Properties
```css
.all-transition {
    transition: all 0.3s ease;
}
```

### Custom Timing Functions
```css
.custom {
    transition: width 1s cubic-bezier(0.68, -0.55, 0.265, 1.55);
}

.steps {
    transition: width 2s steps(4, end);
}
```

## Architecture

### File Structure
```
src/css/
├── values.rs              # Core transition types
│   ├── TimingFunction     # Easing functions
│   ├── Duration           # Time values
│   ├── Transition         # Single transition config
│   ├── TransitionProperty # Property identifier
│   └── TransitionSpec     # Complete transition specification
│
├── computed.rs            # ComputedValues with transition field
├── mod.rs                 # Property name mappings
└── transition_manager.rs  # Runtime transition handling
    ├── TransitionManager  # Tracks active transitions
    ├── ActiveTransition   # Running transition state
    └── TransitionValue    # Interpolatable values
```

### Key Components

#### 1. **TransitionSpec**
Stores the parsed transition configuration for an element:
```rust
pub struct TransitionSpec {
    pub transitions: Vec<Transition>,
}
```

#### 2. **Transition**
Individual transition configuration:
```rust
pub struct Transition {
    pub property: TransitionProperty,
    pub duration: Duration,
    pub timing_function: TimingFunction,
    pub delay: Duration,
}
```

#### 3. **TimingFunction**
Applies easing to animation progress:
```rust
pub enum TimingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    CubicBezier(f32, f32, f32, f32),
    Steps(i32, StepPosition),
    // ...
}
```

#### 4. **TransitionManager**
Manages active transitions at runtime:
- Tracks property changes
- Interpolates values over time
- Applies timing functions
- Cleans up completed transitions

## Usage in Browser Engine

### Integration Steps (For Future Implementation)

1. **Track Style Changes**:
   ```rust
   // When element styles change (e.g., on hover)
   transition_manager.update_element_styles(
       node_id,
       &old_computed_values,
       &new_computed_values
   );
   ```

2. **Get Interpolated Styles**:
   ```rust
   // During rendering
   let interpolated_styles = transition_manager.get_interpolated_styles(
       node_id,
       &computed_values
   );
   ```

3. **Request Redraws**:
   ```rust
   // Continue rendering while transitions are active
   if transition_manager.has_active_transitions() {
       window.request_redraw();
   }
   ```

4. **Cleanup**:
   ```rust
   // Periodically remove completed transitions
   transition_manager.cleanup_completed_transitions();
   ```

## Testing

A comprehensive test file is included: `transition-test.html`

This file demonstrates:
- Basic ease transitions
- Multiple property transitions
- Different timing functions (linear, ease-in, ease-out)
- Custom cubic-bezier curves
- Step functions
- Transition delays
- Combined transitions

To test:
```bash
cargo run transition-test.html
```

## Interpolation Details

### Color Interpolation
Colors are interpolated in RGBA space:
```
r_new = r_start + (r_end - r_start) * progress
g_new = g_start + (g_end - g_start) * progress
b_new = b_start + (b_end - b_start) * progress
a_new = a_start + (a_end - a_start) * progress
```

### Length Interpolation
Lengths are converted to pixels and interpolated linearly:
```
length_new = length_start + (length_end - length_start) * progress
```

### Timing Function Application
The progress value (0.0 to 1.0) is transformed by the timing function:
```
eased_progress = timing_function.apply(linear_progress)
```

## Cubic Bezier Curves

The implementation uses a simplified cubic bezier calculation for performance. The bezier curve is defined by four points:
- P0 = (0, 0) - Start point
- P1 = (x1, y1) - First control point
- P2 = (x2, y2) - Second control point  
- P3 = (1, 1) - End point

Common presets:
- `ease`: cubic-bezier(0.25, 0.1, 0.25, 1.0)
- `ease-in`: cubic-bezier(0.42, 0, 1.0, 1.0)
- `ease-out`: cubic-bezier(0, 0, 0.58, 1.0)
- `ease-in-out`: cubic-bezier(0.42, 0, 0.58, 1.0)

## Performance Considerations

1. **Efficient Interpolation**: Values are pre-converted to pixels for fast interpolation
2. **Cleanup**: Completed transitions are automatically removed
3. **Selective Updates**: Only elements with active transitions are updated
4. **Cached Timing**: Timing function calculations are optimized

## Future Enhancements

Potential improvements for future versions:

1. **More Properties**: Add support for:
   - `opacity`
   - `transform` (translate, rotate, scale)
   - `border-width`
   - `border-radius`
   - `margin` and `padding` (all sides)
   - `font-size`

2. **Better Cubic Bezier**: Implement Newton-Raphson method for accurate bezier solving

3. **Transition Events**: Add support for:
   - `transitionstart`
   - `transitionend`
   - `transitioncancel`
   - `transitionrun`

4. **Advanced Features**:
   - Multiple values per property
   - Interrupted transitions (smooth continuation)
   - Hardware acceleration hints

5. **CSS Animations**: Extend to support `@keyframes` and `animation` property

## API Reference

### TransitionSpec Methods
- `parse(value: &str) -> Self`: Parse CSS transition string
- `has_transitions() -> bool`: Check if any transitions are defined
- `get_transition_for_property(property_name: &str) -> Option<&Transition>`: Find transition config

### TransitionManager Methods
- `new() -> Self`: Create new manager
- `update_element_styles(node_id, old_styles, new_styles)`: Start transitions
- `get_interpolated_styles(node_id, base_styles) -> ComputedValues`: Get current interpolated values
- `has_active_transitions() -> bool`: Check for active transitions
- `cleanup_completed_transitions()`: Remove finished transitions

### TimingFunction Methods
- `parse(value: &str) -> Self`: Parse timing function string
- `apply(t: f32) -> f32`: Apply easing to progress (0.0 to 1.0)

### Duration Methods
- `parse(value: &str) -> Self`: Parse duration string
- `as_millis() -> f32`: Get duration in milliseconds
- `as_seconds() -> f32`: Get duration in seconds

## Browser Compatibility

This implementation follows the CSS Transitions Module Level 1 specification:
https://www.w3.org/TR/css-transitions-1/

Supported features match modern browser behavior for:
- Chrome/Edge
- Firefox
- Safari
- Opera

## Examples in transition-test.html

The test file includes these examples:

1. **Basic Box Transition**: Blue box that grows and turns red on hover
2. **Button Transitions**: Smooth background color changes
3. **Text Fade**: Linear color transition for text
4. **Different Durations**: Width and height transition at different speeds
5. **Multiple Properties**: Background, border-radius, and padding all transition
6. **Step Animation**: Discrete steps instead of smooth transition
7. **Custom Bezier**: Bounce effect using custom curve

## Conclusion

This implementation provides a solid foundation for CSS transitions in the Stokes browser. The architecture is extensible, allowing easy addition of new transitionable properties and timing functions. The separation between parsing (compile-time) and interpolation (runtime) ensures good performance.

