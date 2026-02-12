// Event system for DOM nodes using mozjs
use crate::dom::DomNode;
use crate::js::JsRuntime;
use mozjs::jsval::{JSVal, UndefinedValue, ObjectValue, Int32Value, BooleanValue, DoubleValue, StringValue};
use mozjs::rooted;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use mozjs::context::JSContext;
use mozjs::jsapi::{JSObject, JS_DefineProperty, JS_NewPlainObject, JS_NewUCStringCopyN, JSPROP_ENUMERATE};

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

/// Event listener callback - stores a JavaScript function code or reference
#[derive(Clone)]
pub struct EventListener {
    /// JavaScript function to call
    pub callback: JSObject,
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
    pub fn add_listener(&mut self, event_type: EventType, callback: JSObject, use_capture: bool) -> usize {
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
    pub fn to_js_object(&self, context: &mut JSContext) -> Result<JSVal, String> {
        let raw_cx = unsafe { context.raw_cx() };

        unsafe {
            rooted!(in(raw_cx) let event_obj = JS_NewPlainObject(raw_cx));
            if event_obj.get().is_null() {
                return Err("Failed to create event object".to_string());
            }

            // Set type
            let type_name = std::ffi::CString::new("type").unwrap();
            let type_utf16: Vec<u16> = self.event_type.as_str().encode_utf16().collect();
            rooted!(in(raw_cx) let type_str = JS_NewUCStringCopyN(raw_cx, type_utf16.as_ptr(), type_utf16.len()));
            rooted!(in(raw_cx) let type_val = StringValue(&*type_str.get()));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), type_name.as_ptr(), type_val.handle().into(), JSPROP_ENUMERATE as u32);

            // Set bubbles
            let bubbles_name = std::ffi::CString::new("bubbles").unwrap();
            rooted!(in(raw_cx) let bubbles_val = BooleanValue(self.bubbles));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), bubbles_name.as_ptr(), bubbles_val.handle().into(), JSPROP_ENUMERATE as u32);

            // Set cancelable
            let cancelable_name = std::ffi::CString::new("cancelable").unwrap();
            rooted!(in(raw_cx) let cancelable_val = BooleanValue(self.cancelable));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), cancelable_name.as_ptr(), cancelable_val.handle().into(), JSPROP_ENUMERATE as u32);

            // Set defaultPrevented
            let default_prevented_name = std::ffi::CString::new("defaultPrevented").unwrap();
            rooted!(in(raw_cx) let default_prevented_val = BooleanValue(self.default_prevented));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), default_prevented_name.as_ptr(), default_prevented_val.handle().into(), JSPROP_ENUMERATE as u32);

            // Set timestamp
            let timestamp_name = std::ffi::CString::new("timestamp").unwrap();
            rooted!(in(raw_cx) let timestamp_val = DoubleValue(self.timestamp));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), timestamp_name.as_ptr(), timestamp_val.handle().into(), JSPROP_ENUMERATE as u32);

            // Set eventPhase
            let phase_name = std::ffi::CString::new("eventPhase").unwrap();
            rooted!(in(raw_cx) let phase_val = Int32Value(self.phase as i32));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), phase_name.as_ptr(), phase_val.handle().into(), JSPROP_ENUMERATE as u32);

            // Set mouse position if applicable
            if let Some(x) = self.client_x {
                let client_x_name = std::ffi::CString::new("clientX").unwrap();
                rooted!(in(raw_cx) let client_x_val = DoubleValue(x));
                JS_DefineProperty(raw_cx, event_obj.handle().into(), client_x_name.as_ptr(), client_x_val.handle().into(), JSPROP_ENUMERATE as u32);
            }

            if let Some(y) = self.client_y {
                let client_y_name = std::ffi::CString::new("clientY").unwrap();
                rooted!(in(raw_cx) let client_y_val = DoubleValue(y));
                JS_DefineProperty(raw_cx, event_obj.handle().into(), client_y_name.as_ptr(), client_y_val.handle().into(), JSPROP_ENUMERATE as u32);
            }

            // Set keyboard properties if applicable
            if let Some(ref key) = self.key {
                let key_name = std::ffi::CString::new("key").unwrap();
                let key_utf16: Vec<u16> = key.encode_utf16().collect();
                rooted!(in(raw_cx) let key_str = JS_NewUCStringCopyN(raw_cx, key_utf16.as_ptr(), key_utf16.len()));
                rooted!(in(raw_cx) let key_val = StringValue(&*key_str.get()));
                JS_DefineProperty(raw_cx, event_obj.handle().into(), key_name.as_ptr(), key_val.handle().into(), JSPROP_ENUMERATE as u32);
            }

            if let Some(key_code) = self.key_code {
                let key_code_name = std::ffi::CString::new("keyCode").unwrap();
                rooted!(in(raw_cx) let key_code_val = Int32Value(key_code as i32));
                JS_DefineProperty(raw_cx, event_obj.handle().into(), key_code_name.as_ptr(), key_code_val.handle().into(), JSPROP_ENUMERATE as u32);
            }

            // Set modifier keys
            let ctrl_name = std::ffi::CString::new("ctrlKey").unwrap();
            rooted!(in(raw_cx) let ctrl_val = BooleanValue(self.ctrl_key));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), ctrl_name.as_ptr(), ctrl_val.handle().into(), JSPROP_ENUMERATE as u32);

            let shift_name = std::ffi::CString::new("shiftKey").unwrap();
            rooted!(in(raw_cx) let shift_val = BooleanValue(self.shift_key));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), shift_name.as_ptr(), shift_val.handle().into(), JSPROP_ENUMERATE as u32);

            let alt_name = std::ffi::CString::new("altKey").unwrap();
            rooted!(in(raw_cx) let alt_val = BooleanValue(self.alt_key));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), alt_name.as_ptr(), alt_val.handle().into(), JSPROP_ENUMERATE as u32);

            let meta_name = std::ffi::CString::new("metaKey").unwrap();
            rooted!(in(raw_cx) let meta_val = BooleanValue(self.meta_key));
            JS_DefineProperty(raw_cx, event_obj.handle().into(), meta_name.as_ptr(), meta_val.handle().into(), JSPROP_ENUMERATE as u32);

            Ok(ObjectValue(event_obj.get()))
        }
    }
}

/// Event dispatcher for firing events on DOM nodes
pub struct EventDispatcher;

impl EventDispatcher {
    /// Dispatch an event on a target node with event bubbling
    pub fn dispatch_event(
        target: &DomNode,
        mut event: Event,
        context: &mut JSContext,
    ) -> Result<(), String> {
        // Set target
        event.target = Some(target.id);

        // Phase 1: Capturing phase
        let ancestors = Self::get_ancestors(target);
        event.phase = EventPhase::Capturing;

        for ancestor in ancestors.iter().rev() {
            if event.propagation_stopped {
                break;
            }
            event.current_target = Some(target.id);
            Self::fire_listeners(ancestor, &event, true, context)?;
        }

        // Phase 2: At target
        if !event.propagation_stopped {
            event.phase = EventPhase::AtTarget;
            event.current_target = Some(target.id);
            Self::fire_listeners(target, &event, false, context)?;
        }

        // Phase 3: Bubbling phase
        if event.bubbles && !event.propagation_stopped {
            event.phase = EventPhase::Bubbling;
            for ancestor in ancestors.iter() {
                if event.propagation_stopped {
                    break;
                }
                event.current_target = Some(ancestor.id);
                Self::fire_listeners(ancestor, &event, false, context)?;
            }
        }

        Ok(())
    }

    /// Get all ancestors of a node
    fn get_ancestors(node: &DomNode) -> Vec<&DomNode> {
        use std::collections::HashSet;

        let mut ancestors: Vec<&DomNode> = Vec::new();
        let mut current = node.id;
        let mut visited = HashSet::new();

        // Track the starting node to prevent infinite loops
        visited.insert(current);

        loop {
            let parent = match &node.parent {
                Some(parent) => parent,
                None => break,
            };

            // Check for circular reference
            if visited.contains(parent) {
                //eprintln!("Warning: Circular reference detected in DOM tree parent chain");
                break;
            }

            visited.insert(*parent);
            ancestors.push(node.get_node(*parent));
            current = *parent;
        }

        ancestors
    }

    /// Fire event listeners on a specific node
    fn fire_listeners(
        node: &DomNode,
        event: &Event,
        capture_phase: bool,
        context: &mut JSContext,
    ) -> Result<(), String> {
        // Get listeners for this event type
        if let Some(listeners) = node.event_listeners.get_listeners(&event.event_type) {
            // Clone the listeners to avoid borrow issues
            let listeners_to_fire: Vec<_> = listeners
                .iter()
                .filter(|l| l.use_capture == capture_phase)
                .map(|l| l.callback.clone())
                .collect();

            // Convert event to JS object
            let js_event = event.to_js_object(context)
                .map_err(|e| format!("Failed to convert event to JS object: {}", e))?;

            // Fire each listener
            for callback in listeners_to_fire {
                if event.immediate_propagation_stopped {
                    break;
                }

                // TODOExecute the callback code
                //if let Err(e) = context.execute(&callback_code) {
                //    eprintln!("Error executing event listener: {}", e);
                //}
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
        context: &mut JSContext,
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
        context: &mut JSContext,
    ) -> Result<(), String> {
        let event = Event::new_keyboard_event(event_type, key, key_code);
        Self::dispatch_event(target, event, context)
    }

    /// Dispatch a simple event (no mouse/keyboard data)
    pub fn dispatch_simple_event(
        target: &DomNode,
        event_type: EventType,
        context: &mut JSContext,
    ) -> Result<(), String> {
        let event = Event::new(event_type);
        Self::dispatch_event(target, event, context)
    }
}
