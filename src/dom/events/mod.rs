pub mod pointer;
pub mod focus;
pub mod keyboard;
mod ime;

use crate::dom::events::ime::handle_ime_event;
use crate::dom::events::keyboard::handle_keypress;
use crate::dom::events::pointer::{handle_click, handle_pointerdown, handle_pointermove, handle_pointerup, handle_wheel};
// Event system for DOM nodes using mozjs
use crate::dom::{Dom, DomNode, NodeData};
use crate::events::{BlitzPointerEvent, BlitzPointerId, DomEvent, DomEventData, EventState, UiEvent};
use crate::js::bindings::element_bindings::create_js_element_by_id;
use crate::js::with_runtime_mut;
use blitz_traits::shell::ShellProvider;
use crate::shell_provider::ShellProviderMessage;
use mozjs::jsapi::{
    CallArgs, HandleValueArray, JSContext, JSObject, JS_CallFunctionValue, JS_DefineFunction,
    JS_DefineProperty, JS_GetProperty, JS_NewPlainObject, JS_NewUCStringCopyN, JS_SetProperty,
    JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, DoubleValue, Int32Value, JSVal, ObjectValue, StringValue, UndefinedValue};
use std::collections::{HashMap, VecDeque};
use std::os::raw::c_uint;
use mozjs::rooted;
use mozjs::rust::ValueArray;

impl Dom {
    pub(crate) fn handle_dom_event<F: FnMut(DomEvent)>(
        &mut self,
        event: &mut DomEvent,
        mut dispatch_event: F,
    ) {
        let target_node_id = event.target;

        // Handle forwarding event sub-document

        match &event.data {
            DomEventData::PointerMove(event) => {
                let changed = handle_pointermove(self, target_node_id, event, dispatch_event);
                if changed {
                    let _ = self.shell_provider.sender.send(ShellProviderMessage::RequestRedraw);
                }
            }
            DomEventData::MouseMove(_) => {
                // Do nothing (handled in PointerMove)
            }
            DomEventData::PointerDown(event) => {
                handle_pointerdown(
                    self,
                    target_node_id,
                    event.page_x(),
                    event.page_y(),
                    event.mods,
                    &mut dispatch_event,
                );
            }
            DomEventData::MouseDown(_) => {
                // Do nothing (handled in PointerDown)
            }
            DomEventData::PointerUp(event) => {
                handle_pointerup(self, target_node_id, event, dispatch_event);
            }
            DomEventData::MouseUp(_) => {
                // Do nothing (handled in PointerUp)
            }
            DomEventData::Click(event) => {
                handle_click(self, target_node_id, event, &mut dispatch_event);
            }
            DomEventData::KeyDown(event) => {
                handle_keypress(self, target_node_id, event.clone(), dispatch_event);
            }
            DomEventData::KeyPress(_) => {
                // Do nothing (no default action)
            }
            DomEventData::KeyUp(_) => {
                // Do nothing (no default action)
            }
            DomEventData::Ime(event) => {
                handle_ime_event(self, event.clone(), dispatch_event);
            }
            DomEventData::Input(_) => {
                // Do nothing (no default action)
            }
            DomEventData::ContextMenu(_) => {
                // TODO: Open context menu
            }
            DomEventData::DoubleClick(_) => {
                // Do nothing (no default action)
            }
            DomEventData::PointerEnter(_) => {
                // Do nothing (no default action)
            }
            DomEventData::PointerLeave(_) => {
                // Do nothing (no default action)
            }
            DomEventData::PointerOver(_) => {
                // Do nothing (no default action)
            }
            DomEventData::PointerOut(_) => {
                // Do nothing (no default action)
            }
            DomEventData::MouseEnter(_) => {
                // Do nothing (no default action)
            }
            DomEventData::MouseLeave(_) => {
                // Do nothing (no default action)
            }
            DomEventData::MouseOver(_) => {
                // Do nothing (no default action)
            }
            DomEventData::MouseOut(_) => {
                // Do nothing (no default action)
            }
            DomEventData::Scroll(_) => {
                // Handled elsewhere
            }
            DomEventData::Wheel(event) => {
                handle_wheel(self, target_node_id, event.clone(), dispatch_event);
            }
            DomEventData::Focus(_) => {
                // Do nothing (no default action)
            }
            DomEventData::Blur(_) => {
                // Do nothing (no default action)
            }
            DomEventData::FocusIn(_) => {
                // Do nothing (no default action)
            }
            DomEventData::FocusOut(_) => {
                // Do nothing (no default action)
            }
        }
    }
}

// EVERYTHING UNDER THIS IS OLD AND PROBABLY NEEDS TO BE REWRITTEN

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
    pub callback: *mut JSObject,
    /// Stable pointer identity for removeEventListener
    pub callback_ptr: usize,
    /// Whether to capture the event
    pub use_capture: bool,
    /// Unique ID for this listener
    pub id: usize,
}

/// Event listener registry for a DOM node
#[derive(Clone, Default)]
pub struct EventListenerRegistry {
    /// Map of event type name to list of listeners
    listeners: HashMap<String, Vec<EventListener>>,
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

    /// Add an event listener using a raw event type name.
    pub fn add_listener_by_name(&mut self, event_name: &str, callback: *mut JSObject, use_capture: bool) -> usize {
        let key = event_name.to_ascii_lowercase();
        if self.has_callback_listener(&key, callback, use_capture) {
            return usize::MAX;
        }

        let id = self.next_id;
        self.next_id += 1;

        let listener = EventListener {
            callback,
            callback_ptr: callback as usize,
            use_capture,
            id,
        };

        self.listeners
            .entry(key)
            .or_insert_with(Vec::new)
            .push(listener);

        id
    }

    /// Add an event listener
    pub fn add_listener(&mut self, event_type: EventType, callback: *mut JSObject, use_capture: bool) -> usize {
        self.add_listener_by_name(event_type.as_str(), callback, use_capture)
    }

    /// Remove an event listener by callback identity.
    pub fn remove_listener_by_callback(&mut self, event_name: &str, callback: *mut JSObject, use_capture: bool) -> bool {
        let key = event_name.to_ascii_lowercase();
        if let Some(listeners) = self.listeners.get_mut(&key) {
            let initial_len = listeners.len();
            let callback_ptr = callback as usize;
            listeners.retain(|listener| {
                listener.callback_ptr != callback_ptr || listener.use_capture != use_capture
            });
            return listeners.len() < initial_len;
        }
        false
    }

    /// Remove an event listener by ID
    pub fn remove_listener_by_id(&mut self, event_type: &EventType, id: usize) -> bool {
        if let Some(listeners) = self.listeners.get_mut(event_type.as_str()) {
            let initial_len = listeners.len();
            listeners.retain(|listener| listener.id != id);
            listeners.len() < initial_len
        } else {
            false
        }
    }

    /// Get all listeners for an event type
    pub fn get_listeners(&self, event_type: &EventType) -> Option<&Vec<EventListener>> {
        self.listeners.get(event_type.as_str())
    }

    /// Get all listeners for an event name.
    pub fn get_listeners_by_name(&self, event_name: &str) -> Option<&Vec<EventListener>> {
        self.listeners.get(&event_name.to_ascii_lowercase())
    }

    /// Check if there are any listeners for an event type
    pub fn has_listeners(&self, event_type: &EventType) -> bool {
        self.listeners.get(event_type.as_str()).is_some_and(|l| !l.is_empty())
    }

    /// Clear all listeners for an event type
    pub fn clear_event_type(&mut self, event_type: &EventType) {
        self.listeners.remove(event_type.as_str());
    }

    /// Clear all listeners
    pub fn clear_all(&mut self) {
        self.listeners.clear();
    }

    fn has_callback_listener(&self, event_name: &str, callback: *mut JSObject, use_capture: bool) -> bool {
        let callback_ptr = callback as usize;
        self.listeners.get(event_name).is_some_and(|listeners| {
            listeners
                .iter()
                .any(|listener| listener.callback_ptr == callback_ptr && listener.use_capture == use_capture)
        })
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
    pub fn to_js_object(&self, raw_cx: &mut JSContext) -> Result<JSVal, String> {

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

// Copyright DioxusLabs
// Licensed under the Apache License, Version 2.0 or the MIT license.

pub trait EventHandler {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        doc: &mut Dom,
        event_state: &mut EventState,
    );
}

pub struct NoopEventHandler;
impl EventHandler for NoopEventHandler {
    fn handle_event(
        &mut self,
        _chain: &[usize],
        _event: &mut DomEvent,
        _doc: &mut Dom,
        _event_state: &mut EventState,
    ) {
        // Do nothing
    }
}

pub struct EventDriver<'doc, Handler: EventHandler> {
    doc: &'doc mut Dom,
    handler: Handler,
    queue: VecDeque<DomEvent>,
}

impl<'doc, Handler: EventHandler> EventDriver<'doc, Handler> {
    pub fn new(doc: &'doc mut Dom, handler: Handler) -> Self {
        EventDriver {
            doc,
            handler,
            queue: VecDeque::with_capacity(4),
        }
    }

    pub fn handle_pointer_move(&mut self, event: &BlitzPointerEvent) -> Option<usize> {
        let doc = &mut self.doc;

        let prev_hover_node_id = doc.hover_node_id;
        let changed = doc.set_hover(event.page_x(), event.page_y());
        let hover_node_id = doc.hover_node_id;

        if !changed {
            return prev_hover_node_id;
        }

        let mut old_chain = prev_hover_node_id
            .map(|id| doc.node_chain(id))
            .unwrap_or_default();
        let mut new_chain = hover_node_id
            .map(|id| doc.node_chain(id))
            .unwrap_or_default();
        old_chain.reverse();
        new_chain.reverse();

        // Find the difference in the node chain of the last hovered objected and the newest
        let old_len = old_chain.len();
        let new_len = new_chain.len();

        let first_difference_index = old_chain
            .iter()
            .zip(&new_chain)
            .position(|(old, new)| old != new)
            .unwrap_or_else(|| old_len.min(new_len));

        let is_mouse = event.is_mouse();

        if let Some(target) = prev_hover_node_id {
            self.handle_dom_event(DomEvent::new(
                target,
                DomEventData::PointerOut(event.clone()),
            ));
            if is_mouse {
                self.handle_dom_event(DomEvent::new(target, DomEventData::MouseOut(event.clone())));
            }

            // Send an mouseleave event to all old elements on the chain
            for node_id in old_chain
                .get(first_difference_index..)
                .unwrap_or(&[])
                .iter()
            {
                self.handle_dom_event(DomEvent::new(
                    *node_id,
                    DomEventData::PointerLeave(event.clone()),
                ));
                if is_mouse {
                    self.handle_dom_event(DomEvent::new(
                        *node_id,
                        DomEventData::MouseLeave(event.clone()),
                    ));
                }
            }
        }

        if let Some(target) = hover_node_id {
            self.handle_dom_event(DomEvent::new(
                target,
                DomEventData::PointerOver(event.clone()),
            ));

            if is_mouse {
                self.handle_dom_event(DomEvent::new(
                    target,
                    DomEventData::MouseOver(event.clone()),
                ));
            }

            // Send an mouseenter event to all new elements on the chain
            for node_id in new_chain
                .get(first_difference_index..)
                .unwrap_or(&[])
                .iter()
            {
                self.handle_dom_event(DomEvent::new(
                    *node_id,
                    DomEventData::PointerEnter(event.clone()),
                ));

                if is_mouse {
                    self.handle_dom_event(DomEvent::new(
                        *node_id,
                        DomEventData::MouseEnter(event.clone()),
                    ));
                }
            }
        }

        hover_node_id
    }

    pub fn handle_ui_event(&mut self, event: UiEvent) {
        let mut should_clear_hover = false;
        let mut hover_node_id = self.doc.hover_node_id;
        let focussed_node_id = self.doc.focus_node_id;

        // Update document input state (hover, focus, active, etc)
        match &event {
            UiEvent::PointerMove(event) => {
                hover_node_id = self.handle_pointer_move(event);
            }
            UiEvent::PointerDown(event) => {
                hover_node_id = self.handle_pointer_move(event);
                self.doc.active_node();
                self.doc.set_mousedown_node_id(hover_node_id);
            }
            UiEvent::PointerUp(event) => {
                hover_node_id = self.handle_pointer_move(event);
                self.doc.unactive_node();

                if event.is_primary && matches!(event.id, BlitzPointerId::Finger(_)) {
                    should_clear_hover = true;
                }
            }
            _ => {}
        };

        let target = match event {
            UiEvent::PointerMove(_) => hover_node_id,
            UiEvent::PointerUp(_) => hover_node_id,
            UiEvent::PointerDown(_) => hover_node_id,
            UiEvent::Wheel(_) => hover_node_id,
            UiEvent::KeyUp(_) => focussed_node_id,
            UiEvent::KeyDown(_) => focussed_node_id,
            UiEvent::Ime(_) => focussed_node_id,
        };
        let target = target.unwrap_or_else(|| self.doc.root_element().id);

        match event {
            UiEvent::PointerMove(data) => {
                self.handle_pointer_event(
                    target,
                    data,
                    DomEventData::PointerMove,
                    DomEventData::MouseMove,
                );
            }
            UiEvent::PointerUp(data) => {
                self.handle_pointer_event(
                    target,
                    data,
                    DomEventData::PointerUp,
                    DomEventData::MouseUp,
                );
            }
            UiEvent::PointerDown(data) => {
                self.handle_pointer_event(
                    target,
                    data,
                    DomEventData::PointerDown,
                    DomEventData::MouseDown,
                );
            }
            UiEvent::Wheel(data) => {
                self.handle_dom_event(DomEvent::new(target, DomEventData::Wheel(data)))
            }
            UiEvent::KeyUp(data) => {
                self.handle_dom_event(DomEvent::new(target, DomEventData::KeyUp(data)))
            }
            UiEvent::KeyDown(data) => {
                self.handle_dom_event(DomEvent::new(target, DomEventData::KeyDown(data)))
            }
            UiEvent::Ime(data) => {
                self.handle_dom_event(DomEvent::new(target, DomEventData::Ime(data)))
            }
        };

        // Update document input state (hover, focus, active, etc)
        if should_clear_hover {
            self.doc.clear_hover();
        }
    }

    pub fn handle_dom_event(&mut self, event: DomEvent) {
        self.queue.push_back(event);
        self.process_queue();
    }

    fn handle_pointer_event(
        &mut self,
        target: usize,
        data: BlitzPointerEvent,
        make_ptr_data: impl FnOnce(BlitzPointerEvent) -> DomEventData,
        make_mouse_data: impl FnOnce(BlitzPointerEvent) -> DomEventData,
    ) {
        let mut ptr_event = DomEvent::new(target, make_ptr_data(data.clone()));
        let mut event_state = EventState::default();
        event_state = self.run_handler_event(&mut ptr_event, event_state);
        if !event_state.is_cancelled() && data.is_mouse() {
            let mut mouse_event = DomEvent::new(target, make_mouse_data(data));
            event_state = self.run_handler_event(&mut mouse_event, event_state);
        }
        if !event_state.is_cancelled() {
            self.run_default_action(&mut ptr_event);
        }
        self.process_queue();
    }

    fn process_queue(&mut self) {
        while let Some(mut event) = self.queue.pop_front() {
            let event_state = self.run_handler_event(&mut event, EventState::default());
            if !event_state.is_cancelled() {
                self.run_default_action(&mut event);
            }
        }
    }

    fn run_handler_event(
        &mut self,
        event: &mut DomEvent,
        initial_event_state: EventState,
    ) -> EventState {
        let chain = if event.bubbles {
            let doc = &mut self.doc;
            doc.node_chain(event.target)
        } else {
            vec![event.target]
        };

        let mut event_state = initial_event_state;
        self.handler
            .handle_event(&chain, event, self.doc, &mut event_state);

        event_state
    }

    fn run_default_action(&mut self, event: &mut DomEvent) {
        let mut doc = &mut self.doc;
        doc.handle_dom_event(event, |new_evt| self.queue.push_back(new_evt));
    }
}

pub struct JsEventHandler;
impl EventHandler for JsEventHandler {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        doc: &mut Dom,
        event_state: &mut EventState,
    ) {
        dispatch_js_event(chain, event, doc, event_state);
    }
}

fn dispatch_js_event(chain: &[usize], event: &DomEvent, doc: &mut Dom, event_state: &mut EventState) {
    let target = event.target;
    let ancestors: Vec<usize> = chain.iter().copied().skip(1).collect();

    with_runtime_mut(|runtime| {
        runtime.do_with_jsapi(|_rt, raw_cx, _global| unsafe {
            for node_id in ancestors.iter().rev() {
                if event_state.propagation_is_stopped() {
                    return;
                }
                fire_js_listeners_for_node(raw_cx, doc, *node_id, target, event, true, event_state);
            }

            if !event_state.propagation_is_stopped() {
                fire_js_listeners_for_node(raw_cx, doc, target, target, event, true, event_state);
            }
            if !event_state.propagation_is_stopped() {
                fire_js_listeners_for_node(raw_cx, doc, target, target, event, false, event_state);
            }

            if event.bubbles {
                for node_id in ancestors {
                    if event_state.propagation_is_stopped() {
                        return;
                    }
                    fire_js_listeners_for_node(raw_cx, doc, node_id, target, event, false, event_state);
                }
            }
        });
        runtime.run_pending_jobs();
    });
}

unsafe fn fire_js_listeners_for_node(
    raw_cx: *mut JSContext,
    doc: &Dom,
    node_id: usize,
    target_id: usize,
    event: &DomEvent,
    capture: bool,
    event_state: &mut EventState,
) {
    let listeners = match doc
        .get_node(node_id)
        .and_then(|node| node.event_listeners.get_listeners_by_name(event.name()).cloned())
    {
        Some(listeners) => listeners,
        None => return,
    };

    for listener in listeners {
        if listener.use_capture != capture {
            continue;
        }

        let Some(target_obj) = create_js_node_object(raw_cx, doc, target_id) else {
            continue;
        };
        let Some(current_target_obj) = create_js_node_object(raw_cx, doc, node_id) else {
            continue;
        };
        let Some(event_obj) = create_js_event_object(raw_cx, event, target_obj, current_target_obj, event_state) else {
            continue;
        };

        rooted!(in(raw_cx) let mut rval = UndefinedValue());
        rooted!(in(raw_cx) let callable = ObjectValue(listener.callback));
        rooted!(in(raw_cx) let mut one_arg = ValueArray::new([ObjectValue(event_obj)]));
        rooted!(in(raw_cx) let this_obj = current_target_obj);

        let ok = JS_CallFunctionValue(
            raw_cx,
            this_obj.handle().into(),
            callable.handle().into(),
            &HandleValueArray::from(&one_arg),
            rval.handle_mut().into(),
        );
        if !ok {
            continue;
        }

        rooted!(in(raw_cx) let event_obj_rooted = event_obj);
        rooted!(in(raw_cx) let mut prevent_default = UndefinedValue());
        rooted!(in(raw_cx) let mut stop_propagation = UndefinedValue());

        let prevent_default_name = std::ffi::CString::new("__defaultPrevented").unwrap();
        let stop_propagation_name = std::ffi::CString::new("__propagationStopped").unwrap();

        let _ = JS_GetProperty(
            raw_cx,
            event_obj_rooted.handle().into(),
            prevent_default_name.as_ptr(),
            prevent_default.handle_mut().into(),
        );
        let _ = JS_GetProperty(
            raw_cx,
            event_obj_rooted.handle().into(),
            stop_propagation_name.as_ptr(),
            stop_propagation.handle_mut().into(),
        );

        if prevent_default.get().is_boolean() && prevent_default.get().to_boolean() {
            event_state.prevent_default();
        }
        if stop_propagation.get().is_boolean() && stop_propagation.get().to_boolean() {
            event_state.stop_propagation();
        }

        if event_state.propagation_is_stopped() {
            return;
        }
    }
}

unsafe fn create_js_node_object(raw_cx: *mut JSContext, doc: &Dom, node_id: usize) -> Option<*mut JSObject> {
    let node = doc.get_node(node_id)?;
    match &node.data {
        NodeData::Element(element_data) | NodeData::AnonymousBlock(element_data) => {
            let tag_name = element_data.name.local.to_string();
            let js_val = create_js_element_by_id(raw_cx, node_id, &tag_name, &element_data.attributes).ok()?;
            if js_val.is_object() && !js_val.is_null() {
                Some(js_val.to_object())
            } else {
                None
            }
        }
        _ => {
            rooted!(in(raw_cx) let obj = JS_NewPlainObject(raw_cx));
            if obj.get().is_null() {
                return None;
            }
            rooted!(in(raw_cx) let node_id_val = DoubleValue(node_id as f64));
            let key = std::ffi::CString::new("__nodeId").unwrap();
            JS_DefineProperty(
                raw_cx,
                obj.handle().into(),
                key.as_ptr(),
                node_id_val.handle().into(),
                0,
            );
            Some(obj.get())
        }
    }
}

unsafe fn create_js_event_object(
    raw_cx: *mut JSContext,
    event: &DomEvent,
    target_obj: *mut JSObject,
    current_target_obj: *mut JSObject,
    event_state: &EventState,
) -> Option<*mut JSObject> {
    rooted!(in(raw_cx) let event_obj = JS_NewPlainObject(raw_cx));
    if event_obj.get().is_null() {
        return None;
    }

    define_event_method(raw_cx, event_obj.get(), "preventDefault", js_event_prevent_default, 0);
    define_event_method(raw_cx, event_obj.get(), "stopPropagation", js_event_stop_propagation, 0);

    set_js_string_property(raw_cx, event_obj.get(), "type", event.name());
    set_js_bool_property(raw_cx, event_obj.get(), "bubbles", event.bubbles);
    set_js_bool_property(raw_cx, event_obj.get(), "cancelable", event.cancelable);
    set_js_bool_property(raw_cx, event_obj.get(), "defaultPrevented", event_state.is_cancelled());
    set_js_bool_property(raw_cx, event_obj.get(), "__defaultPrevented", event_state.is_cancelled());
    set_js_bool_property(raw_cx, event_obj.get(), "__propagationStopped", event_state.propagation_is_stopped());

    rooted!(in(raw_cx) let target_val = ObjectValue(target_obj));
    rooted!(in(raw_cx) let current_target_val = ObjectValue(current_target_obj));
    let target_name = std::ffi::CString::new("target").unwrap();
    let current_target_name = std::ffi::CString::new("currentTarget").unwrap();
    JS_DefineProperty(
        raw_cx,
        event_obj.handle().into(),
        target_name.as_ptr(),
        target_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );
    JS_DefineProperty(
        raw_cx,
        event_obj.handle().into(),
        current_target_name.as_ptr(),
        current_target_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    match &event.data {
        DomEventData::PointerMove(e)
        | DomEventData::PointerDown(e)
        | DomEventData::PointerUp(e)
        | DomEventData::PointerEnter(e)
        | DomEventData::PointerLeave(e)
        | DomEventData::PointerOver(e)
        | DomEventData::PointerOut(e)
        | DomEventData::MouseMove(e)
        | DomEventData::MouseDown(e)
        | DomEventData::MouseUp(e)
        | DomEventData::MouseEnter(e)
        | DomEventData::MouseLeave(e)
        | DomEventData::MouseOver(e)
        | DomEventData::MouseOut(e)
        | DomEventData::Click(e)
        | DomEventData::ContextMenu(e)
        | DomEventData::DoubleClick(e) => {
            set_js_double_property(raw_cx, event_obj.get(), "clientX", e.client_x() as f64);
            set_js_double_property(raw_cx, event_obj.get(), "clientY", e.client_y() as f64);
        }
        DomEventData::Wheel(e) => {
            set_js_double_property(raw_cx, event_obj.get(), "clientX", e.client_x() as f64);
            set_js_double_property(raw_cx, event_obj.get(), "clientY", e.client_y() as f64);
        }
        DomEventData::KeyPress(e) | DomEventData::KeyDown(e) | DomEventData::KeyUp(e) => {
            let key = e.key.to_string();
            set_js_string_property(raw_cx, event_obj.get(), "key", &key);
            set_js_int_property(raw_cx, event_obj.get(), "keyCode", 0);
        }
        _ => {}
    }

    Some(event_obj.get())
}

unsafe fn define_event_method(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    method: unsafe extern "C" fn(*mut JSContext, c_uint, *mut JSVal) -> bool,
    argc: u32,
) {
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    JS_DefineFunction(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        Some(method),
        argc,
        JSPROP_ENUMERATE as u32,
    );
}

unsafe fn set_js_string_property(raw_cx: *mut JSContext, obj: *mut JSObject, name: &str, value: &str) {
    let utf16: Vec<u16> = value.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(raw_cx, utf16.as_ptr(), utf16.len()));
    rooted!(in(raw_cx) let js_val = StringValue(&*str_val.get()));
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        js_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );
}

unsafe fn set_js_bool_property(raw_cx: *mut JSContext, obj: *mut JSObject, name: &str, value: bool) {
    rooted!(in(raw_cx) let js_val = BooleanValue(value));
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        js_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );
}

unsafe fn set_js_int_property(raw_cx: *mut JSContext, obj: *mut JSObject, name: &str, value: i32) {
    rooted!(in(raw_cx) let js_val = Int32Value(value));
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        js_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );
}

unsafe fn set_js_double_property(raw_cx: *mut JSContext, obj: *mut JSObject, name: &str, value: f64) {
    rooted!(in(raw_cx) let js_val = DoubleValue(value));
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        js_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );
}

unsafe extern "C" fn js_event_prevent_default(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let this_val = args.thisv();
    if this_val.get().is_object() && !this_val.get().is_null() {
        rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
        rooted!(in(raw_cx) let true_val = BooleanValue(true));
        let hidden_name = std::ffi::CString::new("__defaultPrevented").unwrap();
        let public_name = std::ffi::CString::new("defaultPrevented").unwrap();
        let _ = JS_SetProperty(raw_cx, this_obj.handle().into(), hidden_name.as_ptr(), true_val.handle().into());
        let _ = JS_SetProperty(raw_cx, this_obj.handle().into(), public_name.as_ptr(), true_val.handle().into());
    }
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn js_event_stop_propagation(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let this_val = args.thisv();
    if this_val.get().is_object() && !this_val.get().is_null() {
        rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
        rooted!(in(raw_cx) let true_val = BooleanValue(true));
        let hidden_name = std::ffi::CString::new("__propagationStopped").unwrap();
        let _ = JS_SetProperty(raw_cx, this_obj.handle().into(), hidden_name.as_ptr(), true_val.handle().into());
    }
    args.rval().set(UndefinedValue());
    true
}
