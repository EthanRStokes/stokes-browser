//! JS event listener registry: stores pinned JS function callbacks keyed by
//! DOM node ID and fires them during event dispatch.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_char;

use keyboard_types::Modifiers;
use markup5ever::local_name;
use mozjs::rust::wrappers2::{
    AddRawValueRoot,
    JS_CallFunctionValue, JS_ClearPendingException, JS_DefineProperty,
    JS_GetProperty,
    JS_IsExceptionPending, JS_NewPlainObject, RemoveRawValueRoot,
};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{CallArgs, HandleValueArray, Heap, JSContext, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{DoubleValue, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::Runtime;
use tracing::warn;
use crate::dom::events::EventHandler;
use crate::dom::{Dom, NodeData};
use crate::events::{
    BlitzPointerId, BlitzWheelDelta, DomEvent, DomEventData, EventState,
};
use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::helpers::{define_function, set_bool_property, set_int_property, set_string_property, ToSafeCx};
use crate::js::runtime::RUNTIME;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Sentinel node ID used to store `window.addEventListener` listeners.
pub const WINDOW_NODE_ID: usize = usize::MAX;
/// Sentinel node ID used to store `document.addEventListener` listeners.
pub const DOCUMENT_NODE_ID: usize = usize::MAX - 1;

// ── PinnedCallback ─────────────────────────────────────────────────────────────

/// A JavaScript callable rooted / pinned from SpiderMonkey GC until dropped.
pub struct PinnedCallback {
    heap_obj: Box<Heap<*mut JSObject>>,
    permanent_root: Box<Heap<JSVal>>,
}

// Single-threaded (all access is via thread_local).
unsafe impl Send for PinnedCallback {}
unsafe impl Sync for PinnedCallback {}

impl PinnedCallback {
    /// # Safety
    /// `cx` must be the active JS context. `obj` must be a valid callable `JSObject`.
    pub unsafe fn new(cx: &mut SafeJSContext, obj: *mut JSObject) -> Self {
        let heap_obj: Box<Heap<*mut JSObject>> = Box::new(Heap::default());
        heap_obj.set(obj);
        let permanent_root: Box<Heap<JSVal>> = Box::new(Heap::default());
        permanent_root.set(ObjectValue(obj));
        let name = CString::new("PinnedCallback").unwrap();
        AddRawValueRoot(cx, permanent_root.get_unsafe(), name.as_ptr() as *const c_char);
        Self { heap_obj, permanent_root }
    }

    #[inline]
    pub fn get(&self) -> *mut JSObject {
        self.heap_obj.get()
    }
}

impl Drop for PinnedCallback {
    fn drop(&mut self) {
        // If the JS runtime is still alive on this thread, unroot the value.
        unsafe {
            if let Some(cx) = Runtime::get() {
                RemoveRawValueRoot(&cx.to_safe_cx(), self.permanent_root.get_unsafe());
            }
        }
    }
}

// ── Listener registry ──────────────────────────────────────────────────────────

pub struct JsEventListener {
    pub id: usize,
    pub event_type: String,
    pub callback: PinnedCallback,
    pub use_capture: bool,
}

thread_local! {
    static JS_EVENT_LISTENERS: RefCell<HashMap<usize, Vec<JsEventListener>>> =
        RefCell::new(HashMap::new());
    static NEXT_LISTENER_ID: Cell<usize> = const { Cell::new(1) };

    /// Set by `event.preventDefault()` during a JS listener call.
    pub(crate) static EVENT_DEFAULT_PREVENTED: Cell<bool>  = const { Cell::new(false) };
    /// Set by `event.stopPropagation()` / `stopImmediatePropagation()`.
    pub(crate) static EVENT_PROPAGATION_STOPPED: Cell<bool> = const { Cell::new(false) };
    /// Set by `event.stopImmediatePropagation()`.
    pub(crate) static EVENT_IMMEDIATE_STOPPED: Cell<bool>   = const { Cell::new(false) };
}

/// Register a JS function as an event listener for the given `node_id`.
///
/// # Safety
/// `cx` must be a valid JS context. `callback_obj` must be a callable JS object.
pub unsafe fn add_listener(
    cx: &mut SafeJSContext,
    node_id: usize,
    event_type: String,
    callback_obj: *mut JSObject,
    use_capture: bool,
) -> usize {
    let id = NEXT_LISTENER_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let callback = PinnedCallback::new(cx, callback_obj);
    JS_EVENT_LISTENERS.with(|m| {
        m.borrow_mut()
            .entry(node_id)
            .or_default()
            .push(JsEventListener { id, event_type, callback, use_capture });
    });
    id
}

/// Unregister an event listener from `node_id`.
pub fn remove_listener(
    node_id: usize,
    event_type: &str,
    callback_obj: *mut JSObject,
    use_capture: bool,
) {
    JS_EVENT_LISTENERS.with(|m| {
        if let Some(ls) = m.borrow_mut().get_mut(&node_id) {
            ls.retain(|l| {
                !(l.event_type == event_type
                    && l.use_capture == use_capture
                    && l.callback.get() == callback_obj)
            });
        }
    });
}

/// Drop every listener registered for `node_id` (e.g., on DOM node removal).
pub fn clear_listeners_for_node(node_id: usize) {
    JS_EVENT_LISTENERS.with(|m| { m.borrow_mut().remove(&node_id); });
}

/// Drop all JS event listeners (used when a new document replaces the old one).
pub fn clear_all_listeners() {
    JS_EVENT_LISTENERS.with(|m| m.borrow_mut().clear());
    NEXT_LISTENER_ID.with(|n| n.set(1));
    EVENT_DEFAULT_PREVENTED.with(|f| f.set(false));
    EVENT_PROPAGATION_STOPPED.with(|f| f.set(false));
    EVENT_IMMEDIATE_STOPPED.with(|f| f.set(false));
}

// ── JS event object construction ───────────────────────────────────────────────

/// Native implementation of `event.stopPropagation()`.
unsafe extern "C" fn js_stop_propagation(
    _cx: *mut JSContext, argc: u32, vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    EVENT_PROPAGATION_STOPPED.with(|f| f.set(true));
    args.rval().set(UndefinedValue());
    true
}

/// Native implementation of `event.stopImmediatePropagation()`.
unsafe extern "C" fn js_stop_immediate_propagation(
    _cx: *mut JSContext, argc: u32, vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    EVENT_PROPAGATION_STOPPED.set(true);
    EVENT_IMMEDIATE_STOPPED.set(true);
    args.rval().set(UndefinedValue());
    true
}

/// Native implementation of `event.preventDefault()`.
unsafe extern "C" fn js_prevent_default(
    _cx: *mut JSContext, argc: u32, vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    EVENT_DEFAULT_PREVENTED.set(true);
    args.rval().set(UndefinedValue());
    true
}

/// Helper: define a `f64` property on a JS object.
pub(crate) unsafe fn set_double_property(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    name: &str,
    value: f64,
) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let val = DoubleValue(value));
    rooted!(in(raw_cx) let obj_r = obj);
    let cname = CString::new(name).unwrap();
    JS_DefineProperty(
        cx, obj_r.handle().into(), cname.as_ptr(),
        val.handle().into(), JSPROP_ENUMERATE as u32,
    );
}

/// Map a `keyboard_types::Key` to a legacy DOM `keyCode` integer.
pub(crate) fn key_to_key_code(key: &keyboard_types::Key) -> u32 {
    use keyboard_types::Key;
    match key {
        Key::Backspace    => 8,
        Key::Tab          => 9,
        Key::Enter        => 13,
        Key::Shift        => 16,
        Key::Control      => 17,
        Key::Alt          => 18,
        Key::Pause        => 19,
        Key::CapsLock     => 20,
        Key::Escape       => 27,
        Key::PageUp       => 33,
        Key::PageDown     => 34,
        Key::End          => 35,
        Key::Home         => 36,
        Key::ArrowLeft    => 37,
        Key::ArrowUp      => 38,
        Key::ArrowRight   => 39,
        Key::ArrowDown    => 40,
        Key::Delete       => 46,
        Key::Character(s) if s.len() == 1 => {
            s.chars().next().unwrap().to_ascii_uppercase() as u32
        }
        Key::F1  => 112, Key::F2  => 113, Key::F3  => 114, Key::F4  => 115,
        Key::F5  => 116, Key::F6  => 117, Key::F7  => 118, Key::F8  => 119,
        Key::F9  => 120, Key::F10 => 121, Key::F11 => 122, Key::F12 => 123,
        _ => 0,
    }
}

/// Map a `keyboard_types::Key` to a browser-like DOM `event.key` string.
pub(crate) fn key_to_dom_key(key: &keyboard_types::Key) -> String {
    use keyboard_types::Key;
    match key {
        Key::Character(s) => s.clone(),
        Key::Backspace => "Backspace".to_string(),
        Key::Tab => "Tab".to_string(),
        Key::Enter => "Enter".to_string(),
        Key::Shift => "Shift".to_string(),
        Key::Control => "Control".to_string(),
        Key::Alt => "Alt".to_string(),
        Key::Escape => "Escape".to_string(),
        Key::PageUp => "PageUp".to_string(),
        Key::PageDown => "PageDown".to_string(),
        Key::End => "End".to_string(),
        Key::Home => "Home".to_string(),
        Key::ArrowLeft => "ArrowLeft".to_string(),
        Key::ArrowUp => "ArrowUp".to_string(),
        Key::ArrowRight => "ArrowRight".to_string(),
        Key::ArrowDown => "ArrowDown".to_string(),
        Key::Delete => "Delete".to_string(),
        _ => key.to_string(),
    }
}

/// Build a JS Event-like plain object from a Rust `DomEvent`.
pub unsafe fn build_event_object(cx: &mut SafeJSContext, event: &DomEvent) -> *mut JSObject {
    build_event_object_with_type(cx, event.name(), event.bubbles, event.cancelable, &event.data)
}

unsafe fn set_object_prototype_cached(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    proto: *mut JSObject,
) -> bool {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = mozjs::rust::wrappers2::CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return false;
    }

    let set_proto_cache_name = CString::new("__stokesSetPrototypeOfFn").unwrap();
    rooted!(in(raw_cx) let mut set_proto_fn = UndefinedValue());
    if !JS_GetProperty(
        cx,
        global.handle().into(),
        set_proto_cache_name.as_ptr(),
        set_proto_fn.handle_mut().into(),
    ) || !set_proto_fn.get().is_object() {
        rooted!(in(raw_cx) let mut object_ctor_val = UndefinedValue());
        let object_name = CString::new("Object").unwrap();
        if !JS_GetProperty(
            cx,
            global.handle().into(),
            object_name.as_ptr(),
            object_ctor_val.handle_mut().into(),
        ) || !object_ctor_val.get().is_object() {
            return false;
        }
        rooted!(in(raw_cx) let object_ctor_obj = object_ctor_val.get().to_object());
        let set_proto_name = CString::new("setPrototypeOf").unwrap();
        if !JS_GetProperty(
            cx,
            object_ctor_obj.handle().into(),
            set_proto_name.as_ptr(),
            set_proto_fn.handle_mut().into(),
        ) || !set_proto_fn.get().is_object() {
            return false;
        }

        JS_DefineProperty(
            cx,
            global.handle().into(),
            set_proto_cache_name.as_ptr(),
            set_proto_fn.handle().into(),
            0,
        );
    }

    rooted!(in(raw_cx) let args = mozjs::rust::ValueArray::<2usize>::new([
        ObjectValue(obj),
        ObjectValue(proto),
    ]));
    rooted!(in(raw_cx) let mut rval = UndefinedValue());
    JS_CallFunctionValue(
        cx,
        global.handle().into(),
        set_proto_fn.handle().into(),
        &HandleValueArray::from(&args),
        rval.handle_mut().into(),
    )
}

unsafe fn ensure_event_shared_prototype(cx: &mut SafeJSContext) -> *mut JSObject {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = mozjs::rust::wrappers2::CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return std::ptr::null_mut();
    }

    let key = CString::new("__stokesEventPrototype").unwrap();
    rooted!(in(raw_cx) let mut existing = UndefinedValue());
    if JS_GetProperty(
        cx,
        global.handle().into(),
        key.as_ptr(),
        existing.handle_mut().into(),
    ) && existing.get().is_object() {
        return existing.get().to_object();
    }

    rooted!(in(raw_cx) let proto = JS_NewPlainObject(cx));
    if proto.get().is_null() {
        return std::ptr::null_mut();
    }

    let _ = define_function(cx, proto.get(), "stopPropagation", Some(js_stop_propagation), 0);
    let _ = define_function(cx, proto.get(), "stopImmediatePropagation", Some(js_stop_immediate_propagation), 0);
    let _ = define_function(cx, proto.get(), "preventDefault", Some(js_prevent_default), 0);
    let _ = define_function(cx, proto.get(), "initEvent", Some(noop_init_event), 3);
    let _ = set_int_property(cx, proto.get(), "NONE", 0);
    let _ = set_int_property(cx, proto.get(), "CAPTURING_PHASE", 1);
    let _ = set_int_property(cx, proto.get(), "AT_TARGET", 2);
    let _ = set_int_property(cx, proto.get(), "BUBBLING_PHASE", 3);

    rooted!(in(raw_cx) let proto_val = ObjectValue(proto.get()));
    JS_DefineProperty(
        cx,
        global.handle().into(),
        key.as_ptr(),
        proto_val.handle().into(),
        0,
    );

    proto.get()
}

/// Build a JS Event-like object with fully specified parameters.
pub unsafe fn build_event_object_with_type(
    cx: &mut SafeJSContext,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    data: &DomEventData,
) -> *mut JSObject {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let obj = JS_NewPlainObject(cx));
    if obj.get().is_null() { return std::ptr::null_mut(); }

    let event_proto = ensure_event_shared_prototype(cx);
    if !event_proto.is_null() {
        let _ = set_object_prototype_cached(cx, obj.get(), event_proto);
    }

    let _ = set_string_property(cx, obj.get(), "type",            event_type);
    let _ = set_bool_property(cx, obj.get(),   "bubbles",         bubbles);
    let _ = set_bool_property(cx, obj.get(),   "cancelable",      cancelable);
    let _ = set_bool_property(cx, obj.get(),   "defaultPrevented",false);
    let _ = set_bool_property(cx, obj.get(),   "isTrusted",       true);
    set_double_property(cx, obj.get(), "timeStamp", 0.0);

    // Set target / currentTarget to null initially; updated during dispatch.
    rooted!(in(raw_cx) let null_v = NullValue());
    for prop in &["target", "currentTarget", "relatedTarget"] {
        let cname = CString::new(*prop).unwrap();
        rooted!(in(raw_cx) let obj_r = obj.get());
        JS_DefineProperty(cx, obj_r.handle().into(), cname.as_ptr(),
            null_v.handle().into(), JSPROP_ENUMERATE as u32);
    }

    let _ = set_int_property(cx, obj.get(), "eventPhase",      2); // updated during dispatch


    // Add event-specific properties based on DomEventData.
    match data {
        DomEventData::Click(ev)
        | DomEventData::PointerDown(ev)
        | DomEventData::PointerUp(ev)
        | DomEventData::PointerMove(ev)
        | DomEventData::MouseDown(ev)
        | DomEventData::MouseUp(ev)
        | DomEventData::MouseMove(ev)
        | DomEventData::MouseEnter(ev)
        | DomEventData::MouseLeave(ev)
        | DomEventData::MouseOver(ev)
        | DomEventData::MouseOut(ev)
        | DomEventData::PointerEnter(ev)
        | DomEventData::PointerLeave(ev)
        | DomEventData::PointerOver(ev)
        | DomEventData::PointerOut(ev)
        | DomEventData::ContextMenu(ev)
        | DomEventData::DoubleClick(ev) => {
            set_double_property(cx, obj.get(), "clientX",  ev.client_x() as f64);
            set_double_property(cx, obj.get(), "clientY",  ev.client_y() as f64);
            set_double_property(cx, obj.get(), "pageX",    ev.page_x() as f64);
            set_double_property(cx, obj.get(), "pageY",    ev.page_y() as f64);
            set_double_property(cx, obj.get(), "screenX",  ev.screen_x() as f64);
            set_double_property(cx, obj.get(), "screenY",  ev.screen_y() as f64);
            set_double_property(cx, obj.get(), "x",        ev.client_x() as f64);
            set_double_property(cx, obj.get(), "y",        ev.client_y() as f64);
            set_double_property(cx, obj.get(), "offsetX",  ev.client_x() as f64);
            set_double_property(cx, obj.get(), "offsetY",  ev.client_y() as f64);
            set_double_property(cx, obj.get(), "movementX", 0.0);
            set_double_property(cx, obj.get(), "movementY", 0.0);
            let _ = set_int_property(cx, obj.get(), "button",  ev.button as i32);
            let _ = set_int_property(cx, obj.get(), "buttons", ev.buttons.bits() as i32);
            let _ = set_bool_property(cx, obj.get(), "ctrlKey",  ev.mods.contains(Modifiers::CONTROL));
            let _ = set_bool_property(cx, obj.get(), "shiftKey", ev.mods.contains(Modifiers::SHIFT));
            let _ = set_bool_property(cx, obj.get(), "altKey",   ev.mods.contains(Modifiers::ALT));
            let _ = set_bool_property(cx, obj.get(), "metaKey",  ev.mods.contains(Modifiers::META));
            let pointer_id: i32 = match ev.id {
                BlitzPointerId::Mouse    => 1,
                BlitzPointerId::Pen      => 2,
                BlitzPointerId::Finger(id) => (id as i32).wrapping_add(10),
            };
            let _ = set_int_property(cx, obj.get(), "pointerId", pointer_id);
            let _ = set_bool_property(cx, obj.get(), "isPrimary", ev.is_primary);
            set_double_property(cx, obj.get(), "pressure", if ev.buttons.is_empty() { 0.0 } else { 0.5 });
            let _ = set_int_property(cx, obj.get(), "detail", 0);
        }
        DomEventData::KeyDown(kev) | DomEventData::KeyUp(kev) | DomEventData::KeyPress(kev) => {
            let key_str  = key_to_dom_key(&kev.key);
            let code_str = format!("{:?}", kev.code);
            let _ = set_string_property(cx, obj.get(), "key",  &key_str);
            let _ = set_string_property(cx, obj.get(), "code", &code_str);
            let kc = key_to_key_code(&kev.key);
            let _ = set_int_property(cx, obj.get(), "keyCode",  kc as i32);
            let _ = set_int_property(cx, obj.get(), "which",    kc as i32);
            let _ = set_int_property(cx, obj.get(), "charCode", 0);
            let _ = set_bool_property(cx, obj.get(), "ctrlKey",    kev.modifiers.contains(Modifiers::CONTROL));
            let _ = set_bool_property(cx, obj.get(), "shiftKey",   kev.modifiers.contains(Modifiers::SHIFT));
            let _ = set_bool_property(cx, obj.get(), "altKey",     kev.modifiers.contains(Modifiers::ALT));
            let _ = set_bool_property(cx, obj.get(), "metaKey",    kev.modifiers.contains(Modifiers::META));
            let _ = set_bool_property(cx, obj.get(), "repeat",     kev.is_auto_repeating);
            let _ = set_bool_property(cx, obj.get(), "isComposing",kev.is_composing);
            let location = match kev.location {
                keyboard_types::Location::Standard  => 0,
                keyboard_types::Location::Left      => 1,
                keyboard_types::Location::Right     => 2,
                keyboard_types::Location::Numpad    => 3,
            };
            let _ = set_int_property(cx, obj.get(), "location", location);
        }
        DomEventData::Wheel(wev) => {
            let (dx, dy) = match &wev.delta {
                BlitzWheelDelta::Pixels(x, y) => (*x, *y),
                BlitzWheelDelta::Lines(x, y)  => (x * 16.0, y * 16.0),
            };
            set_double_property(cx, obj.get(), "deltaX", dx);
            set_double_property(cx, obj.get(), "deltaY", dy);
            set_double_property(cx, obj.get(), "deltaZ", 0.0);
            let _ = set_int_property(cx, obj.get(), "deltaMode", 0); // DOM_DELTA_PIXEL
            set_double_property(cx, obj.get(), "clientX", wev.client_x() as f64);
            set_double_property(cx, obj.get(), "clientY", wev.client_y() as f64);
            set_double_property(cx, obj.get(), "pageX",   wev.page_x() as f64);
            set_double_property(cx, obj.get(), "pageY",   wev.page_y() as f64);
            let _ = set_int_property(cx, obj.get(), "buttons", wev.buttons.bits() as i32);
            let _ = set_bool_property(cx, obj.get(), "ctrlKey",  wev.mods.contains(Modifiers::CONTROL));
            let _ = set_bool_property(cx, obj.get(), "shiftKey", wev.mods.contains(Modifiers::SHIFT));
            let _ = set_bool_property(cx, obj.get(), "altKey",   wev.mods.contains(Modifiers::ALT));
            let _ = set_bool_property(cx, obj.get(), "metaKey",  wev.mods.contains(Modifiers::META));
        }
        DomEventData::Focus(_) | DomEventData::Blur(_)
        | DomEventData::FocusIn(_) | DomEventData::FocusOut(_) => {
            let _ = set_int_property(cx, obj.get(), "detail", 0);
        }
        DomEventData::Input(iev) => {
            let _ = set_string_property(cx, obj.get(), "data", &iev.value);
            let _ = set_bool_property(cx, obj.get(), "isComposing", false);
            let _ = set_string_property(cx, obj.get(), "inputType", "insertText");
        }
        _ => {}
    }

    obj.get()
}

/// Stub `event.initEvent(type, bubbles, cancelable)` – no-op for modern compatibility.
unsafe extern "C" fn noop_init_event(
    _cx: *mut JSContext, argc: u32, vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    // FIXME: event.initEvent() should mutate this Event instance's type/bubbles/cancelable
    // fields for legacy callers. It is currently a no-op.
    warn!("[JS] Event.initEvent() called on partial binding (no-op)");
    args.rval().set(UndefinedValue());
    true
}

// ── Low-level dispatch helpers ─────────────────────────────────────────────────

/// Set an object-valued property with `__nodeId` as a target/currentTarget stub.
unsafe fn make_target_proxy(cx: &mut SafeJSContext, node_id: usize) -> *mut JSObject {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let t = JS_NewPlainObject(cx));
    if t.get().is_null() { return std::ptr::null_mut(); }
    rooted!(in(raw_cx) let id_val = DoubleValue(node_id as f64));
    let cname = CString::new("__nodeId").unwrap();
    JS_DefineProperty(cx, t.handle().into(), cname.as_ptr(),
        id_val.handle().into(), 0);

    // Expose common target fields (notably `value`) expected by form/input listeners.
    DOM_REF.with(|dom_ref| {
        let Some(dom_ptr) = *dom_ref.borrow() else { return; };
        let dom = unsafe { &*dom_ptr };
        let Some(node) = dom.get_node(node_id) else { return; };

        if let NodeData::Element(ref elem_data) = node.data {
            let tag_name = elem_data.name.local.to_string();
            let _ = set_string_property(cx, t.get(), "localName", &tag_name);
            let _ = set_string_property(cx, t.get(), "tagName", &tag_name.to_uppercase());

            if let Some(id_attr) = elem_data.attr(local_name!("id")) {
                let _ = set_string_property(cx, t.get(), "id", id_attr);
            }

            if let Some(input_data) = elem_data.text_input_data() {
                let value = input_data.editor.raw_text().to_string();
                let _ = set_string_property(cx, t.get(), "value", &value);
            } else if let Some(value_attr) = elem_data.attr(local_name!("value")) {
                let _ = set_string_property(cx, t.get(), "value", value_attr);
            }
        }
    });

    t.get()
}

/// Update `event.target` on the JS event object.
unsafe fn set_event_target(cx: &mut SafeJSContext, event_obj: *mut JSObject, node_id: usize) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let ev = event_obj);
    let proxy = make_target_proxy(cx, node_id);
    if proxy.is_null() { return; }
    rooted!(in(raw_cx) let pv = ObjectValue(proxy));
    let tname = CString::new("target").unwrap();
    JS_DefineProperty(cx, ev.handle().into(), tname.as_ptr(),
        pv.handle().into(), JSPROP_ENUMERATE as u32);
}

/// Update `event.currentTarget` on the JS event object.
unsafe fn set_event_current_target(cx: &mut SafeJSContext, event_obj: *mut JSObject, node_id: usize) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let ev = event_obj);
    let proxy = make_target_proxy(cx, node_id);
    if proxy.is_null() { return; }
    rooted!(in(raw_cx) let pv = ObjectValue(proxy));
    let tname = CString::new("currentTarget").unwrap();
    JS_DefineProperty(cx, ev.handle().into(), tname.as_ptr(),
        pv.handle().into(), JSPROP_ENUMERATE as u32);
}

/// Update `event.eventPhase`.
unsafe fn set_event_phase(cx: &mut SafeJSContext, event_obj: *mut JSObject, phase: i32) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let ev = event_obj);
    rooted!(in(raw_cx) let pv = mozjs::jsval::Int32Value(phase));
    let tname = CString::new("eventPhase").unwrap();
    JS_DefineProperty(cx, ev.handle().into(), tname.as_ptr(),
        pv.handle().into(), JSPROP_ENUMERATE as u32);
}

/// Invoke all matching listeners on a single node.
///
/// Returns `true` if `stopImmediatePropagation()` was called by a listener.
unsafe fn fire_on_node(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    node_id: usize,
    event_obj: *mut JSObject,
    event_type: &str,
    capture: bool,
    at_target: bool,
) -> bool {
    let raw_cx = cx.raw_cx();
    // Snapshot raw callback pointers while briefly holding the borrow.
    // This avoids holding a borrow while JS is running (re-entrancy).
    let callbacks: Vec<*mut JSObject> = JS_EVENT_LISTENERS.with(|map| {
        let map = map.borrow();
        let Some(ls) = map.get(&node_id) else { return Vec::new(); };
        ls.iter()
            .filter(|l| {
                let phase_ok = at_target || (l.use_capture == capture);
                l.event_type == event_type && phase_ok
            })
            .map(|l| l.callback.get())
            .collect()
    });

    for cb in callbacks {
        // Root the callback on the current JS stack to protect it from GC.
        rooted!(in(raw_cx) let callable = ObjectValue(cb));
        rooted!(in(raw_cx) let this_v  = global);
        rooted!(in(raw_cx) let evt_v   = ObjectValue(event_obj));
        rooted!(in(raw_cx) let mut rv  = UndefinedValue());

        let args_arr: [JSVal; 1] = [*evt_v];
        let handle_arr = HandleValueArray { length_: 1, elements_: args_arr.as_ptr() };

        JS_CallFunctionValue(
            cx,
            this_v.handle().into(),
            callable.handle().into(),
            &handle_arr,
            rv.handle_mut().into(),
        );

        // Swallow any exception the callback threw.
        if JS_IsExceptionPending(cx) {
            JS_ClearPendingException(cx);
        }

        if EVENT_IMMEDIATE_STOPPED.with(|f| f.get()) {
            return true;
        }
    }

    false
}

// ── Public dispatch entrypoints ────────────────────────────────────────────────

/// Dispatch a Rust `DomEvent` through the JS listener chain.
///
/// `chain[0]` is the event target; subsequent elements are ancestors toward the root.
pub unsafe fn fire_js_event_on_chain(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    chain: &[usize],
    event: &DomEvent,
) {
    let event_obj = build_event_object(cx, event);
    if event_obj.is_null() { return; }
    dispatch_event_obj(cx, global, chain, event.name(), event.bubbles, event_obj);
}

/// Dispatch an *already-constructed* JS event object through the listener chain.
///
/// Used by `element.dispatchEvent(event)` where the event object comes from JS.
/// `event_type` should match `event_obj.type`, `bubbles` should match `event_obj.bubbles`.
pub unsafe fn dispatch_event_obj(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    chain: &[usize],
    event_type: &str,
    bubbles: bool,
    event_obj: *mut JSObject,
) {
    let raw_cx = cx.raw_cx();
    if event_obj.is_null() { return; }
    // Keep the event object rooted for the full dispatch. JS callbacks can
    // trigger GC/compaction, which can move objects and invalidate raw pointers.
    rooted!(in(raw_cx) let event_obj_r = event_obj);

    // Reset per-dispatch flags.
    EVENT_DEFAULT_PREVENTED.set(false);
    EVENT_PROPAGATION_STOPPED.set(false);
    EVENT_IMMEDIATE_STOPPED.set(false);

    let target_id = chain.first().copied().unwrap_or(0);
    set_event_target(cx, event_obj_r.get(), target_id);

    // ── Capture phase: root → parent-of-target ────────────────────────────
    if bubbles && chain.len() > 1 {
        set_event_phase(cx, event_obj_r.get(), 1); // CAPTURING_PHASE
        for &node_id in chain[1..].iter().rev() {
            if EVENT_PROPAGATION_STOPPED.with(|f| f.get()) { break; }
            set_event_current_target(cx, event_obj_r.get(), node_id);
            fire_on_node(cx, global, node_id, event_obj_r.get(), event_type, true, false);
        }
    }

    // ── At-target phase ───────────────────────────────────────────────────
    if !EVENT_PROPAGATION_STOPPED.get() {
        set_event_phase(cx, event_obj_r.get(), 2); // AT_TARGET
        set_event_current_target(cx, event_obj_r.get(), target_id);
        fire_on_node(cx, global, target_id, event_obj_r.get(), event_type, false, true);
    }

    // ── Bubble phase: parent-of-target → root ─────────────────────────────
    if bubbles {
        set_event_phase(cx, event_obj_r.get(), 3); // BUBBLING_PHASE
        for &node_id in chain[1..].iter() {
            if EVENT_PROPAGATION_STOPPED.get() { break; }
            set_event_current_target(cx, event_obj_r.get(), node_id);
            fire_on_node(cx, global, node_id, event_obj_r.get(), event_type, false, false);
        }
        // Bubble to document-level listeners.
        if !EVENT_PROPAGATION_STOPPED.get() {
            fire_on_node(cx, global, DOCUMENT_NODE_ID, event_obj_r.get(), event_type, false, false);
        }
        // Bubble to window-level listeners.
        if !EVENT_PROPAGATION_STOPPED.get() {
            fire_on_node(cx, global, WINDOW_NODE_ID, event_obj_r.get(), event_type, false, false);
        }
    }

    // Reset currentTarget to null when dispatch is complete.
    rooted!(in(raw_cx) let ev = event_obj_r.get());
    rooted!(in(raw_cx) let null_v = NullValue());
    let ct = CString::new("currentTarget").unwrap();
    JS_DefineProperty(cx, ev.handle().into(), ct.as_ptr(),
        null_v.handle().into(), JSPROP_ENUMERATE as u32);
}

/// Dispatch a window-level Promise rejection event (`unhandledrejection` /
/// `rejectionhandled`) with browser-like payload fields.
///
/// Returns `true` if the event was not canceled via `preventDefault()`.
pub unsafe fn dispatch_window_promise_rejection_event(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    event_type: &str,
    promise_obj: *mut JSObject,
    reason: JSVal,
    cancelable: bool,
) -> bool {
    EVENT_DEFAULT_PREVENTED.set(false);
    EVENT_PROPAGATION_STOPPED.set(false);
    EVENT_IMMEDIATE_STOPPED.set(false);

    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let event_obj = JS_NewPlainObject(cx));
    if event_obj.get().is_null() {
        return true;
    }

    let _ = set_string_property(cx, event_obj.get(), "type", event_type);
    let _ = set_bool_property(cx, event_obj.get(), "bubbles", false);
    let _ = set_bool_property(cx, event_obj.get(), "cancelable", cancelable);
    let _ = set_bool_property(cx, event_obj.get(), "isTrusted", true);
    let _ = define_function(cx, event_obj.get(), "stopPropagation", Some(js_stop_propagation), 0);
    let _ = define_function(cx, event_obj.get(), "stopImmediatePropagation", Some(js_stop_immediate_propagation), 0);
    let _ = define_function(cx, event_obj.get(), "preventDefault", Some(js_prevent_default), 0);

    rooted!(in(raw_cx) let promise_v = ObjectValue(promise_obj));
    rooted!(in(raw_cx) let reason_v = reason);
    let promise_name = CString::new("promise").unwrap();
    let reason_name = CString::new("reason").unwrap();
    JS_DefineProperty(
        cx,
        event_obj.handle().into(),
        promise_name.as_ptr(),
        promise_v.handle().into(),
        JSPROP_ENUMERATE as u32,
    );
    JS_DefineProperty(
        cx,
        event_obj.handle().into(),
        reason_name.as_ptr(),
        reason_v.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    set_event_target(cx, event_obj.get(), WINDOW_NODE_ID);
    set_event_phase(cx, event_obj.get(), 2);
    set_event_current_target(cx, event_obj.get(), WINDOW_NODE_ID);

    // Support window.onunhandledrejection / window.onrejectionhandled.
    // Many sites use the IDL event handler property instead of addEventListener.
    invoke_window_event_handler_property(cx, global, event_obj.get(), event_type);

    fire_on_node(
        cx,
        global,
        WINDOW_NODE_ID,
        event_obj.get(),
        event_type,
        false,
        true,
    );

    !EVENT_DEFAULT_PREVENTED.with(|f| f.get())
}

unsafe fn invoke_window_event_handler_property(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    event_obj: *mut JSObject,
    event_type: &str,
) {
    let raw_cx = cx.raw_cx();
    let prop_name = format!("on{event_type}");
    let c_name = CString::new(prop_name).unwrap();

    rooted!(in(raw_cx) let global_r = global);
    rooted!(in(raw_cx) let mut handler_val = UndefinedValue());
    if !JS_GetProperty(
        cx,
        global_r.handle().into(),
        c_name.as_ptr(),
        handler_val.handle_mut().into(),
    ) {
        return;
    }

    if !handler_val.get().is_object() {
        return;
    }

    rooted!(in(raw_cx) let handler = handler_val.get().to_object());
    rooted!(in(raw_cx) let this_v = global);
    rooted!(in(raw_cx) let callable = ObjectValue(handler.get()));
    rooted!(in(raw_cx) let evt_v = ObjectValue(event_obj));
    rooted!(in(raw_cx) let mut rv = UndefinedValue());
    let args_arr: [JSVal; 1] = [*evt_v];
    let handle_arr = HandleValueArray { length_: 1, elements_: args_arr.as_ptr() };

    JS_CallFunctionValue(
        cx,
        this_v.handle().into(),
        callable.handle().into(),
        &handle_arr,
        rv.handle_mut().into(),
    );

    if JS_IsExceptionPending(cx) {
        JS_ClearPendingException(cx);
    }
}

/// Fire `DOMContentLoaded` and `load` events on the document / window.
/// Call this once the page is fully loaded.
pub fn fire_load_events(dom: &Dom) {
    let rt_ptr = RUNTIME.with(|cell| *cell.borrow());
    let Some(rt_ptr) = rt_ptr else { return; };
    let rt = unsafe { &mut *rt_ptr };

    // Build the node chain for the root element.
    let root_id = dom.root_node().id;
    let chain = vec![root_id];

    rt.do_with_jsapi(|cx, global| unsafe {
        // DOMContentLoaded — fires on document, does not bubble to window in the
        // standard sense, but we fire on both DOCUMENT_NODE_ID and WINDOW_NODE_ID.
        EVENT_DEFAULT_PREVENTED.set(false);
        EVENT_PROPAGATION_STOPPED.set(false);
        EVENT_IMMEDIATE_STOPPED.set(false);
        let raw_cx = cx.raw_cx();
        rooted!(in(raw_cx) let dcl_obj = JS_NewPlainObject(cx));
        if !dcl_obj.get().is_null() {
            let _ = set_string_property(cx, dcl_obj.get(), "type",    "DOMContentLoaded");
            let _ = set_bool_property(cx, dcl_obj.get(),   "bubbles", true);
            let _ = set_bool_property(cx, dcl_obj.get(),   "cancelable", false);
            let _ = set_bool_property(cx, dcl_obj.get(),   "isTrusted", true);
            let _ = define_function(cx, dcl_obj.get(), "stopPropagation",         Some(js_stop_propagation), 0);
            let _ = define_function(cx, dcl_obj.get(), "stopImmediatePropagation",Some(js_stop_immediate_propagation), 0);
            let _ = define_function(cx, dcl_obj.get(), "preventDefault",          Some(js_prevent_default), 0);
            set_event_target(cx, dcl_obj.get(), DOCUMENT_NODE_ID);
            fire_on_node(cx, global.get(), DOCUMENT_NODE_ID, dcl_obj.get(), "DOMContentLoaded", false, true);
            fire_on_node(cx, global.get(), WINDOW_NODE_ID,   dcl_obj.get(), "DOMContentLoaded", false, false);
        }

        // load event — fires on window.
        EVENT_DEFAULT_PREVENTED.set(false);
        EVENT_PROPAGATION_STOPPED.set(false);
        EVENT_IMMEDIATE_STOPPED.set(false);
        rooted!(in(raw_cx) let load_obj = JS_NewPlainObject(cx));
        if !load_obj.get().is_null() {
            let _ = set_string_property(cx, load_obj.get(), "type",    "load");
            let _ = set_bool_property(cx, load_obj.get(),   "bubbles", false);
            let _ = set_bool_property(cx, load_obj.get(),   "cancelable", false);
            let _ = set_bool_property(cx, load_obj.get(),   "isTrusted", true);
            let _ = define_function(cx, load_obj.get(), "stopPropagation",         Some(js_stop_propagation), 0);
            let _ = define_function(cx, load_obj.get(), "stopImmediatePropagation",Some(js_stop_immediate_propagation), 0);
            let _ = define_function(cx, load_obj.get(), "preventDefault",          Some(js_prevent_default), 0);
            set_event_target(cx, load_obj.get(), WINDOW_NODE_ID);
            fire_on_node(cx, global.get(), WINDOW_NODE_ID, load_obj.get(), "load", false, true);
        }

        let _ = chain; // suppress unused warning
    });
}

// ── JsEventHandler ─────────────────────────────────────────────────────────────

/// An [`EventHandler`] that fires registered JavaScript event listeners for
/// every DOM event that passes through the [`EventDriver`] pipeline.
pub struct JsEventHandler;

impl EventHandler for JsEventHandler {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        _doc: &mut Dom,
        event_state: &mut EventState,
    ) {
        // Extract the runtime pointer without keeping the borrow alive.
        let rt_ptr = RUNTIME.with(|cell| *cell.borrow());
        let Some(rt_ptr) = rt_ptr else { return; };
        let rt = unsafe { &mut *rt_ptr };

        rt.do_with_jsapi(|cx, global| {
            unsafe {
                fire_js_event_on_chain(cx, global.get(), chain, event);
            }
        });

        // Propagate preventDefault() back to the Rust EventState.
        if EVENT_DEFAULT_PREVENTED.get() {
            event_state.prevent_default();
        }
    }
}
