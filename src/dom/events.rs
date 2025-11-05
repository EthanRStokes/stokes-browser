use crate::dom::DomNode;
use boa_engine::{object::JsObject, Context, JsValue};
use std::cell::RefCell;
// Event system for DOM nodes
use std::collections::HashMap;
use std::rc::Rc;

/// Event types supported by the browser
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventType {
    // Mouse events
    Click,
    DblClick,
    MouseDown,
    MouseUp,
    MouseMove,
    MouseEnter,
    MouseLeave,
    MouseOver,
    MouseOut,
    ContextMenu,

    // Keyboard events
    KeyDown,
    KeyUp,
    KeyPress,

    // Focus events
    Focus,
    Blur,
    FocusIn,
    FocusOut,

    // Form events
    Submit,
    Change,
    Input,
    Invalid,
    Reset,
    Select,

    // Drag events
    Drag,
    DragStart,
    DragEnd,
    DragEnter,
    DragLeave,
    DragOver,
    Drop,

    // Touch events
    TouchStart,
    TouchEnd,
    TouchMove,
    TouchCancel,

    // UI events
    Load,
    Unload,
    Resize,
    Scroll,

    // Custom/Unknown event
    Custom(String),
}

impl EventType {
    /// Parse an event type from a string
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "click" => EventType::Click,
            "dblclick" => EventType::DblClick,
            "mousedown" => EventType::MouseDown,
            "mouseup" => EventType::MouseUp,
            "mousemove" => EventType::MouseMove,
            "mouseenter" => EventType::MouseEnter,
            "mouseleave" => EventType::MouseLeave,
            "mouseover" => EventType::MouseOver,
            "mouseout" => EventType::MouseOut,
            "contextmenu" => EventType::ContextMenu,
            "keydown" => EventType::KeyDown,
            "keyup" => EventType::KeyUp,
            "keypress" => EventType::KeyPress,
            "focus" => EventType::Focus,
            "blur" => EventType::Blur,
            "focusin" => EventType::FocusIn,
            "focusout" => EventType::FocusOut,
            "submit" => EventType::Submit,
            "change" => EventType::Change,
            "input" => EventType::Input,
            "invalid" => EventType::Invalid,
            "reset" => EventType::Reset,
            "select" => EventType::Select,
            "drag" => EventType::Drag,
            "dragstart" => EventType::DragStart,
            "dragend" => EventType::DragEnd,
            "dragenter" => EventType::DragEnter,
            "dragleave" => EventType::DragLeave,
            "dragover" => EventType::DragOver,
            "drop" => EventType::Drop,
            "touchstart" => EventType::TouchStart,
            "touchend" => EventType::TouchEnd,
            "touchmove" => EventType::TouchMove,
            "touchcancel" => EventType::TouchCancel,
            "load" => EventType::Load,
            "unload" => EventType::Unload,
            "resize" => EventType::Resize,
            "scroll" => EventType::Scroll,
            _ => EventType::Custom(s.to_string()),
        }
    }

    /// Convert event type to string
    pub fn as_str(&self) -> &str {
        match self {
            EventType::Click => "click",
            EventType::DblClick => "dblclick",
            EventType::MouseDown => "mousedown",
            EventType::MouseUp => "mouseup",
            EventType::MouseMove => "mousemove",
            EventType::MouseEnter => "mouseenter",
            EventType::MouseLeave => "mouseleave",
            EventType::MouseOver => "mouseover",
            EventType::MouseOut => "mouseout",
            EventType::ContextMenu => "contextmenu",
            EventType::KeyDown => "keydown",
            EventType::KeyUp => "keyup",
            EventType::KeyPress => "keypress",
            EventType::Focus => "focus",
            EventType::Blur => "blur",
            EventType::FocusIn => "focusin",
            EventType::FocusOut => "focusout",
            EventType::Submit => "submit",
            EventType::Change => "change",
            EventType::Input => "input",
            EventType::Invalid => "invalid",
            EventType::Reset => "reset",
            EventType::Select => "select",
            EventType::Drag => "drag",
            EventType::DragStart => "dragstart",
            EventType::DragEnd => "dragend",
            EventType::DragEnter => "dragenter",
            EventType::DragLeave => "dragleave",
            EventType::DragOver => "dragover",
            EventType::Drop => "drop",
            EventType::TouchStart => "touchstart",
            EventType::TouchEnd => "touchend",
            EventType::TouchMove => "touchmove",
            EventType::TouchCancel => "touchcancel",
            EventType::Load => "load",
            EventType::Unload => "unload",
            EventType::Resize => "resize",
            EventType::Scroll => "scroll",
            EventType::Custom(s) => s.as_str(),
        }
    }
}

/// Event listener callback - stores a JavaScript function
#[derive(Clone)]
pub struct EventListener {
    /// JavaScript function to call
    pub callback: JsObject,
    /// Whether to capture the event
    pub use_capture: bool,
    /// Unique ID for this listener
    pub id: usize,
}

/// Event listener registry for a DOM node
#[derive(Clone, Default)]
pub struct EventListenerRegistry {
    /// Map of event type to list of listeners
    listeners: HashMap<EventType, Vec<EventListener>>,
    /// Counter for generating unique listener IDs
    next_id: usize,
}

impl EventListenerRegistry {
    /// Create a new event listener registry
    pub fn new() -> Self {
        Self {
            listeners: HashMap::new(),
            next_id: 0,
        }
    }

    /// Add an event listener
    pub fn add_listener(&mut self, event_type: EventType, callback: JsObject, use_capture: bool) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        let listener = EventListener {
            callback,
            use_capture,
            id,
        };

        self.listeners
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(listener);

        id
    }

    /// Remove an event listener by callback (for removeEventListener)
    /// Note: This is simplified - in a real implementation, we'd need to compare function references
    pub fn remove_listener(&mut self, event_type: &EventType, callback: &JsObject) -> bool {
        if let Some(listeners) = self.listeners.get_mut(event_type) {
            let initial_len = listeners.len();
            listeners.retain(|listener| !JsObject::equals(&listener.callback, callback));
            listeners.len() < initial_len
        } else {
            false
        }
    }

    /// Remove an event listener by ID
    pub fn remove_listener_by_id(&mut self, event_type: &EventType, id: usize) -> bool {
        if let Some(listeners) = self.listeners.get_mut(event_type) {
            let initial_len = listeners.len();
            listeners.retain(|listener| listener.id != id);
            listeners.len() < initial_len
        } else {
            false
        }
    }

    /// Get all listeners for an event type
    pub fn get_listeners(&self, event_type: &EventType) -> Option<&Vec<EventListener>> {
        self.listeners.get(event_type)
    }

    /// Check if there are any listeners for an event type
    pub fn has_listeners(&self, event_type: &EventType) -> bool {
        self.listeners.get(event_type).map_or(false, |l| !l.is_empty())
    }

    /// Clear all listeners for an event type
    pub fn clear_event_type(&mut self, event_type: &EventType) {
        self.listeners.remove(event_type);
    }

    /// Clear all listeners
    pub fn clear_all(&mut self) {
        self.listeners.clear();
    }
}

/// Event object that gets passed to event handlers
#[derive(Debug, Clone)]
pub struct Event {
    /// Type of the event
    pub event_type: EventType,
    /// Target element
    pub target: Option<usize>, // Node pointer as ID
    /// Current target (element with the listener)
    pub current_target: Option<usize>,
    /// Timestamp when event was created
    pub timestamp: f64,
    /// Whether the event bubbles
    pub bubbles: bool,
    /// Whether the event is cancelable
    pub cancelable: bool,
    /// Whether preventDefault has been called
    pub default_prevented: bool,
    /// Whether stopPropagation has been called
    pub propagation_stopped: bool,
    /// Whether stopImmediatePropagation has been called
    pub immediate_propagation_stopped: bool,
    /// Event phase (capturing, at target, bubbling)
    pub phase: EventPhase,
    /// Mouse position (if applicable)
    pub client_x: Option<f64>,
    pub client_y: Option<f64>,
    /// Keyboard key (if applicable)
    pub key: Option<String>,
    pub key_code: Option<u32>,
    /// Modifier keys
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub alt_key: bool,
    pub meta_key: bool,
}

/// Event phase
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    None = 0,
    Capturing = 1,
    AtTarget = 2,
    Bubbling = 3,
}

impl Event {
    /// Create a new event
    pub fn new(event_type: EventType) -> Self {
        Self {
            event_type,
            target: None,
            current_target: None,
            timestamp: 0.0, // Would be set by the event system
            bubbles: true,
            cancelable: true,
            default_prevented: false,
            propagation_stopped: false,
            immediate_propagation_stopped: false,
            phase: EventPhase::None,
            client_x: None,
            client_y: None,
            key: None,
            key_code: None,
            ctrl_key: false,
            shift_key: false,
            alt_key: false,
            meta_key: false,
        }
    }

    /// Create a mouse event
    pub fn new_mouse_event(event_type: EventType, x: f64, y: f64) -> Self {
        let mut event = Self::new(event_type);
        event.client_x = Some(x);
        event.client_y = Some(y);
        event
    }

    /// Create a keyboard event
    pub fn new_keyboard_event(event_type: EventType, key: String, key_code: u32) -> Self {
        let mut event = Self::new(event_type);
        event.key = Some(key);
        event.key_code = Some(key_code);
        event
    }

    /// Prevent default action
    pub fn prevent_default(&mut self) {
        if self.cancelable {
            self.default_prevented = true;
        }
    }

    /// Stop event propagation
    pub fn stop_propagation(&mut self) {
        self.propagation_stopped = true;
    }

    /// Stop immediate propagation
    pub fn stop_immediate_propagation(&mut self) {
        self.propagation_stopped = true;
        self.immediate_propagation_stopped = true;
    }

    /// Convert to JavaScript object
    pub fn to_js_object(&self, context: &mut Context) -> boa_engine::JsResult<JsValue> {
        use boa_engine::object::ObjectInitializer;
        use boa_engine::JsString;

        let mut js_event = ObjectInitializer::new(context);

        let mut js_event = js_event
            .property(
                JsString::from("type"),
                JsValue::from(JsString::from(self.event_type.as_str())),
                boa_engine::property::Attribute::all(),
            )
            .property(
                JsString::from("bubbles"),
                JsValue::from(self.bubbles),
                boa_engine::property::Attribute::all(),
            )
            .property(
                JsString::from("cancelable"),
                JsValue::from(self.cancelable),
                boa_engine::property::Attribute::all(),
            )
            .property(
                JsString::from("defaultPrevented"),
                JsValue::from(self.default_prevented),
                boa_engine::property::Attribute::all(),
            )
            .property(
                JsString::from("timestamp"),
                JsValue::from(self.timestamp),
                boa_engine::property::Attribute::all(),
            )
            .property(
                JsString::from("eventPhase"),
                JsValue::from(self.phase as i32),
                boa_engine::property::Attribute::all(),
            );

        // Add mouse event properties if applicable
        if let Some(x) = self.client_x {
            js_event = js_event.property(
                JsString::from("clientX"),
                JsValue::from(x),
                boa_engine::property::Attribute::all(),
            );
        }

        if let Some(y) = self.client_y {
            js_event = js_event.property(
                JsString::from("clientY"),
                JsValue::from(y),
                boa_engine::property::Attribute::all(),
            );
        }

        // Add keyboard event properties if applicable
        if let Some(ref key) = self.key {
            js_event = js_event.property(
                JsString::from("key"),
                JsValue::from(JsString::from(key.clone())),
                boa_engine::property::Attribute::all(),
            );
        }

        if let Some(key_code) = self.key_code {
            js_event = js_event.property(
                JsString::from("keyCode"),
                JsValue::from(key_code),
                boa_engine::property::Attribute::all(),
            );
        }

        // Add modifier keys
        js_event = js_event
            .property(
                JsString::from("ctrlKey"),
                JsValue::from(self.ctrl_key),
                boa_engine::property::Attribute::all(),
            )
            .property(
                JsString::from("shiftKey"),
                JsValue::from(self.shift_key),
                boa_engine::property::Attribute::all(),
            )
            .property(
                JsString::from("altKey"),
                JsValue::from(self.alt_key),
                boa_engine::property::Attribute::all(),
            )
            .property(
                JsString::from("metaKey"),
                JsValue::from(self.meta_key),
                boa_engine::property::Attribute::all(),
            );

        Ok(js_event.build().into())
    }
}

/// Event dispatcher for firing events on DOM nodes
pub struct EventDispatcher;

impl EventDispatcher {
    /// Dispatch an event on a target node with event bubbling
    pub fn dispatch_event(
        target: &Rc<RefCell<DomNode>>,
        mut event: Event,
        context: &mut Context,
    ) -> Result<(), String> {
        // Set target
        event.target = Some(Rc::as_ptr(target) as usize);

        // Phase 1: Capturing phase
        let ancestors = Self::get_ancestors(target);
        event.phase = EventPhase::Capturing;

        for ancestor in ancestors.iter().rev() {
            if event.propagation_stopped {
                break;
            }
            event.current_target = Some(**ancestor);
            Self::fire_listeners(ancestor, &event, true, context)?;
        }

        // Phase 2: At target
        if !event.propagation_stopped {
            event.phase = EventPhase::AtTarget;
            event.current_target = Some(Rc::as_ptr(target) as usize);
            Self::fire_listeners(target, &event, false, context)?;
        }

        // Phase 3: Bubbling phase
        if event.bubbles && !event.propagation_stopped {
            event.phase = EventPhase::Bubbling;
            for ancestor in ancestors.iter() {
                if event.propagation_stopped {
                    break;
                }
                event.current_target = Some(Rc::as_ptr(ancestor) as usize);
                Self::fire_listeners(ancestor, &event, false, context)?;
            }
        }

        Ok(())
    }

    /// Get all ancestors of a node
    fn get_ancestors(node: &Rc<RefCell<DomNode>>) -> Vec<&usize> {
        use std::collections::HashSet;

        let mut ancestors: Vec<&usize> = Vec::new();
        let mut current = node.borrow().id;
        let mut visited = HashSet::new();

        // Track the starting node to prevent infinite loops
        visited.insert(current);

        loop {
            let parent_rc = {
                let current_borrowed = node.borrow();
                let node_borrowed = current_borrowed;
                match &node_borrowed.parent {
                    Some(parent_weak) => Some(parent_weak),
                    None => None,
                }
            };

            match parent_rc {
                Some(parent) => {

                    // Check for circular reference
                    if visited.contains(parent) {
                        eprintln!("Warning: Circular reference detected in DOM tree parent chain");
                        break;
                    }

                    visited.insert(*parent);
                    ancestors.push(parent);
                    current = *parent;
                }
                None => break,
            }
        }

        ancestors
    }

    /// Fire event listeners on a specific node
    fn fire_listeners(
        node: &Rc<RefCell<DomNode>>,
        event: &Event,
        capture_phase: bool,
        context: &mut Context,
    ) -> Result<(), String> {
        let node_borrowed = node.borrow();

        // Get listeners for this event type
        if let Some(listeners) = node_borrowed.event_listeners.get_listeners(&event.event_type) {
            // Clone the listeners to avoid borrow issues
            let listeners_to_fire: Vec<_> = listeners
                .iter()
                .filter(|l| l.use_capture == capture_phase)
                .map(|l| l.callback.clone())
                .collect();

            drop(node_borrowed); // Release the borrow

            // Convert event to JS object
            let js_event = event.to_js_object(context)
                .map_err(|e| format!("Failed to convert event to JS object: {}", e))?;

            // Fire each listener
            for callback in listeners_to_fire {
                if event.immediate_propagation_stopped {
                    break;
                }

                // Call the callback function
                let _ = callback.call(&JsValue::undefined(), &[js_event.clone()], context)
                    .map_err(|e| {
                        eprintln!("Error executing event listener: {}", e);
                        e
                    });
            }
        }

        Ok(())
    }

    /// Dispatch a mouse event
    pub fn dispatch_mouse_event(
        target: &DomNode,
        event_type: EventType,
        x: f64,
        y: f64,
        context: &mut Context,
    ) -> Result<(), String> {
        let event = Event::new_mouse_event(event_type, x, y);
        Self::dispatch_event(target, event, context)
    }

    /// Dispatch a keyboard event
    pub fn dispatch_keyboard_event(
        target: &DomNode,
        event_type: EventType,
        key: String,
        key_code: u32,
        context: &mut Context,
    ) -> Result<(), String> {
        let event = Event::new_keyboard_event(event_type, key, key_code);
        Self::dispatch_event(target, event, context)
    }

    /// Dispatch a simple event (no mouse/keyboard data)
    pub fn dispatch_simple_event(
        target: &DomNode,
        event_type: EventType,
        context: &mut Context,
    ) -> Result<(), String> {
        let event = Event::new(event_type);
        Self::dispatch_event(target, event, context)
    }
}
