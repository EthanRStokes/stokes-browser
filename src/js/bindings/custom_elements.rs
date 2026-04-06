use crate::dom::NodeData;
use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::bindings::element_bindings;
use crate::js::helpers::{create_js_string, define_function, get_node_id_from_value, js_value_to_string, ToSafeCx};
use crate::js::jsapi::promise::PersistentRooted;
use crate::js::JsRuntime;
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{
    CallArgs, HandleValueArray, JSContext, JSObject, JS_DefineProperty, JS_GetProperty,
    JS_NewPlainObject, JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, JSVal, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::ValueArray;
use mozjs::rust::wrappers2::{
    JS_CallFunctionValue, JS_ClearPendingException, NewPromiseObject, ResolvePromise,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::os::raw::c_uint;
use std::ptr;

const CE_DEF_MARKER_PROP: &str = "__stokesCustomElementDefinition";
const CE_CONNECTED_MARKER_PROP: &str = "__stokesCustomElementConnected";

struct CustomElementDefinition {
    name: String,
    extends_tag: Option<String>,
    ctor: PersistentRooted,
    prototype: PersistentRooted,
}

struct CustomElementsState {
    definitions_by_name: HashMap<String, CustomElementDefinition>,
    ctor_ptrs: HashSet<usize>,
    pending_when_defined: HashMap<String, Vec<PersistentRooted>>,
}

impl CustomElementsState {
    fn new() -> Self {
        Self {
            definitions_by_name: HashMap::new(),
            ctor_ptrs: HashSet::new(),
            pending_when_defined: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        self.definitions_by_name.clear();
        self.ctor_ptrs.clear();
        self.pending_when_defined.clear();
    }
}

thread_local! {
    static CUSTOM_ELEMENTS_STATE: RefCell<CustomElementsState> = RefCell::new(CustomElementsState::new());
}

fn normalize_custom_element_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn is_valid_custom_element_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if matches!(
        name,
        "annotation-xml"
            | "color-profile"
            | "font-face"
            | "font-face-src"
            | "font-face-uri"
            | "font-face-format"
            | "font-face-name"
            | "missing-glyph"
    ) {
        return false;
    }
    if !name.contains('-') || name.starts_with('-') {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.'))
}

fn collect_element_subtree_ids(root_id: usize) -> Vec<usize> {
    DOM_REF.with(|dom_ref| {
        let mut out = Vec::new();
        let Some(dom_ptr) = *dom_ref.borrow() else {
            return out;
        };
        let dom = unsafe { &*dom_ptr };
        let mut stack = vec![root_id];
        while let Some(node_id) = stack.pop() {
            let Some(node) = dom.get_node(node_id) else {
                continue;
            };
            if matches!(node.data, NodeData::Element(_) | NodeData::AnonymousBlock(_)) {
                out.push(node_id);
            }
            for child in node.children.iter().rev() {
                stack.push(*child);
            }
            if let Some(shadow_root_id) = node.shadow_root {
                stack.push(shadow_root_id);
            }
        }
        out
    })
}

fn node_local_name_and_is_attr(node_id: usize) -> Option<(String, Option<String>)> {
    DOM_REF.with(|dom_ref| {
        let dom_ptr = (*dom_ref.borrow())?;
        let dom = unsafe { &*dom_ptr };
        let node = dom.get_node(node_id)?;
        let elem = node.data.element()?;
        let local = elem.name.local.to_string().to_ascii_lowercase();
        let is_attr = elem
            .attributes
            .iter()
            .find(|attr| attr.name.local.as_ref() == "is")
            .map(|attr| attr.value.to_ascii_lowercase());
        Some((local, is_attr))
    })
}

fn node_is_connected(node_id: usize) -> bool {
    DOM_REF.with(|dom_ref| {
        let Some(dom_ptr) = *dom_ref.borrow() else {
            return false;
        };
        let dom = unsafe { &*dom_ptr };
        let mut current = Some(node_id);
        let mut depth = 0usize;
        while let Some(id) = current {
            if id == 0 {
                return true;
            }
            current = dom.get_node(id).and_then(|n| n.parent);
            depth += 1;
            if depth > 2048 {
                break;
            }
        }
        false
    })
}

fn definition_name_for_node(node_id: usize) -> Option<String> {
    let (local_name, is_attr) = node_local_name_and_is_attr(node_id)?;
    CUSTOM_ELEMENTS_STATE.with(|state| {
        let state = state.borrow();
        if let Some(def) = state.definitions_by_name.get(&local_name) {
            if def.extends_tag.is_none() {
                return Some(def.name.clone());
            }
        }
        if let Some(is_name) = is_attr {
            if let Some(def) = state.definitions_by_name.get(&is_name) {
                if def.extends_tag.as_deref() == Some(local_name.as_str()) {
                    return Some(def.name.clone());
                }
            }
        }
        None
    })
}

unsafe fn set_object_prototype(cx: &mut SafeJSContext, obj: *mut JSObject, proto: *mut JSObject) {
    use mozjs::jsapi::CurrentGlobalOrNull;

    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
    if global.get().is_null() {
        return;
    }

    rooted!(in(raw_cx) let mut object_ctor_val = UndefinedValue());
    let object_name = std::ffi::CString::new("Object").unwrap();
    if !JS_GetProperty(raw_cx, global.handle().into(), object_name.as_ptr(), object_ctor_val.handle_mut().into()) || !object_ctor_val.get().is_object() {
        return;
    }
    rooted!(in(raw_cx) let object_ctor_obj = object_ctor_val.get().to_object());

    rooted!(in(raw_cx) let mut set_proto_val = UndefinedValue());
    let set_proto_name = std::ffi::CString::new("setPrototypeOf").unwrap();
    if !JS_GetProperty(raw_cx, object_ctor_obj.handle().into(), set_proto_name.as_ptr(), set_proto_val.handle_mut().into()) || !set_proto_val.get().is_object() {
        return;
    }

    rooted!(in(raw_cx) let args = ValueArray::<2usize>::new([ObjectValue(obj), ObjectValue(proto)]));
    rooted!(in(raw_cx) let mut rval = UndefinedValue());
    if !JS_CallFunctionValue(
        cx,
        global.handle().into(),
        set_proto_val.handle().into(),
        &HandleValueArray::from(&args),
        rval.handle_mut().into(),
    ) {
        JS_ClearPendingException(cx);
    }
}

unsafe fn bridge_custom_prototype_to_wrapper_prototype(
    cx: &mut SafeJSContext,
    element_obj: *mut JSObject,
    custom_proto: *mut JSObject,
) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let element_rooted = element_obj);
    rooted!(in(raw_cx) let mut wrapper_proto_val = UndefinedValue());
    let proto_name = std::ffi::CString::new("__proto__").unwrap();
    if !JS_GetProperty(
        raw_cx,
        element_rooted.handle().into(),
        proto_name.as_ptr(),
        wrapper_proto_val.handle_mut().into(),
    ) || !wrapper_proto_val.get().is_object() {
        return;
    }

    let wrapper_proto = wrapper_proto_val.get().to_object();
    if wrapper_proto == custom_proto {
        return;
    }

    set_object_prototype(cx, custom_proto, wrapper_proto);

    for method in ["setAttribute", "getAttribute", "removeAttribute", "hasAttribute"] {
        let method_name = std::ffi::CString::new(method).unwrap();
        rooted!(in(raw_cx) let custom_proto_rooted = custom_proto);
        rooted!(in(raw_cx) let wrapper_proto_rooted = wrapper_proto);
        rooted!(in(raw_cx) let mut existing = UndefinedValue());
        let has_existing = JS_GetProperty(
            raw_cx,
            custom_proto_rooted.handle().into(),
            method_name.as_ptr(),
            existing.handle_mut().into(),
        );
        if has_existing && existing.get().is_object() {
            continue;
        }

        rooted!(in(raw_cx) let mut wrapper_method = UndefinedValue());
        if !JS_GetProperty(
            raw_cx,
            wrapper_proto_rooted.handle().into(),
            method_name.as_ptr(),
            wrapper_method.handle_mut().into(),
        ) || !wrapper_method.get().is_object() {
            continue;
        }

        JS_DefineProperty(
            raw_cx,
            custom_proto_rooted.handle().into(),
            method_name.as_ptr(),
            wrapper_method.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }
}

unsafe fn ensure_instance_attribute_methods(cx: &mut SafeJSContext, element_obj: *mut JSObject) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let element_rooted = element_obj);

    for method in ["setAttribute", "getAttribute", "removeAttribute", "hasAttribute"] {
        let method_name = std::ffi::CString::new(method).unwrap();
        rooted!(in(raw_cx) let mut value = UndefinedValue());
        if !JS_GetProperty(
            raw_cx,
            element_rooted.handle().into(),
            method_name.as_ptr(),
            value.handle_mut().into(),
        ) || !value.get().is_object() {
            continue;
        }

        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            method_name.as_ptr(),
            value.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }
}

unsafe fn set_hidden_connected(cx: &mut SafeJSContext, obj: *mut JSObject, connected: bool) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let rooted_obj = obj);
    rooted!(in(raw_cx) let connected_val = BooleanValue(connected));
    let prop = std::ffi::CString::new(CE_CONNECTED_MARKER_PROP).unwrap();
    JS_DefineProperty(raw_cx, rooted_obj.handle().into(), prop.as_ptr(), connected_val.handle().into(), 0);
}

unsafe fn get_hidden_connected(cx: &mut SafeJSContext, obj: *mut JSObject) -> bool {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let rooted_obj = obj);
    rooted!(in(raw_cx) let mut value = UndefinedValue());
    let prop = std::ffi::CString::new(CE_CONNECTED_MARKER_PROP).unwrap();
    if JS_GetProperty(raw_cx, rooted_obj.handle().into(), prop.as_ptr(), value.handle_mut().into()) {
        return value.get().is_boolean() && value.get().to_boolean();
    }
    false
}

unsafe fn get_hidden_definition_name(cx: &mut SafeJSContext, obj: *mut JSObject) -> Option<String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let rooted_obj = obj);
    rooted!(in(raw_cx) let mut value = UndefinedValue());
    let prop = std::ffi::CString::new(CE_DEF_MARKER_PROP).unwrap();
    if JS_GetProperty(raw_cx, rooted_obj.handle().into(), prop.as_ptr(), value.handle_mut().into()) {
        return value.get().is_string().then(|| js_value_to_string(cx, value.get()));
    }
    None
}

unsafe fn set_hidden_definition_name(cx: &mut SafeJSContext, obj: *mut JSObject, name: &str) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let rooted_obj = obj);
    rooted!(in(raw_cx) let name_val = create_js_string(cx, name));
    let prop = std::ffi::CString::new(CE_DEF_MARKER_PROP).unwrap();
    JS_DefineProperty(raw_cx, rooted_obj.handle().into(), prop.as_ptr(), name_val.handle().into(), 0);
}

unsafe fn invoke_connected_callback(cx: &mut SafeJSContext, element: *mut JSObject) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let element_rooted = element);
    rooted!(in(raw_cx) let mut callback = UndefinedValue());
    let callback_name = std::ffi::CString::new("connectedCallback").unwrap();
    if !JS_GetProperty(raw_cx, element_rooted.handle().into(), callback_name.as_ptr(), callback.handle_mut().into()) || !callback.get().is_object() {
        return;
    }
    rooted!(in(raw_cx) let args = ValueArray::<0usize>::new([]));
    rooted!(in(raw_cx) let mut rval = UndefinedValue());
    if !JS_CallFunctionValue(
        cx,
        element_rooted.handle().into(),
        callback.handle().into(),
        &HandleValueArray::from(&args),
        rval.handle_mut().into(),
    ) {
        JS_ClearPendingException(cx);
    }
}

unsafe fn upgrade_node_by_id(cx: &mut SafeJSContext, node_id: usize, forced_definition: Option<&str>) {
    let def_name = forced_definition
        .map(|v| v.to_string())
        .or_else(|| definition_name_for_node(node_id));
    let Some(def_name) = def_name else {
        return;
    };

    let def_data = CUSTOM_ELEMENTS_STATE.with(|state| {
        let state = state.borrow();
        state.definitions_by_name.get(&def_name).map(|def| {
            (def.name.clone(), def.ctor.get(), def.prototype.get())
        })
    });
    let Some((resolved_name, ctor_obj, prototype_obj)) = def_data else {
        return;
    };
    if ctor_obj.is_null() || prototype_obj.is_null() {
        return;
    }

    let raw_cx = cx.raw_cx();
    let Ok(element_val) = element_bindings::create_js_element_by_dom_id(cx, node_id) else {
        return;
    };
    if !element_val.is_object() {
        return;
    }

    rooted!(in(raw_cx) let element_obj = element_val.to_object());
    ensure_instance_attribute_methods(cx, element_obj.get());
    let connected_now = node_is_connected(node_id);
    let previous_name = get_hidden_definition_name(cx, element_obj.get());
    if previous_name.as_deref() == Some(resolved_name.as_str()) {
        let was_connected = get_hidden_connected(cx, element_obj.get());
        if connected_now && !was_connected {
            invoke_connected_callback(cx, element_obj.get());
            set_hidden_connected(cx, element_obj.get(), true);
        } else if !connected_now && was_connected {
            set_hidden_connected(cx, element_obj.get(), false);
        }
        return;
    }

    bridge_custom_prototype_to_wrapper_prototype(cx, element_obj.get(), prototype_obj);
    set_object_prototype(cx, element_obj.get(), prototype_obj);

    rooted!(in(raw_cx) let ctor_val = ObjectValue(ctor_obj));
    let ctor_prop = std::ffi::CString::new("constructor").unwrap();
    JS_DefineProperty(
        raw_cx,
        element_obj.handle().into(),
        ctor_prop.as_ptr(),
        ctor_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    rooted!(in(raw_cx) let args = ValueArray::<0usize>::new([]));
    rooted!(in(raw_cx) let mut call_rval = UndefinedValue());
    if !JS_CallFunctionValue(
        cx,
        element_obj.handle().into(),
        ctor_val.handle().into(),
        &HandleValueArray::from(&args),
        call_rval.handle_mut().into(),
    ) {
        JS_ClearPendingException(cx);
    }

    set_hidden_definition_name(cx, element_obj.get(), &resolved_name);
    invoke_connected_callback(cx, element_obj.get());
    set_hidden_connected(cx, element_obj.get(), connected_now);
}

unsafe fn upgrade_subtree_by_node_id(cx: &mut SafeJSContext, root_id: usize, forced_definition: Option<&str>) {
    let ids = collect_element_subtree_ids(root_id);
    for node_id in ids {
        upgrade_node_by_id(cx, node_id, forced_definition);
    }
}

pub(crate) unsafe fn custom_elements_upgrade_for_node(cx: &mut SafeJSContext, root_id: usize) {
    upgrade_subtree_by_node_id(cx, root_id, None);
}

unsafe extern "C" fn custom_element_registry_constructor(_raw_cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, 0);
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn custom_elements_define(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc < 2 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let name = normalize_custom_element_name(&js_value_to_string(safe_cx, *args.get(0)));
    if !is_valid_custom_element_name(&name) || !args.get(1).is_object() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let raw = safe_cx.raw_cx();
    rooted!(in(raw) let ctor_obj = args.get(1).to_object());
    rooted!(in(raw) let mut prototype_val = UndefinedValue());
    let prototype_name = std::ffi::CString::new("prototype").unwrap();
    if !JS_GetProperty(raw, ctor_obj.handle().into(), prototype_name.as_ptr(), prototype_val.handle_mut().into()) || !prototype_val.get().is_object() {
        args.rval().set(UndefinedValue());
        return true;
    }
    rooted!(in(raw) let prototype_obj = prototype_val.get().to_object());

    let mut extends_tag = None;
    if argc > 2 && args.get(2).is_object() {
        rooted!(in(raw) let options_obj = args.get(2).to_object());
        rooted!(in(raw) let mut extends_val = UndefinedValue());
        let extends_name = std::ffi::CString::new("extends").unwrap();
        if JS_GetProperty(raw, options_obj.handle().into(), extends_name.as_ptr(), extends_val.handle_mut().into())
            && !extends_val.get().is_undefined()
            && !extends_val.get().is_null()
        {
            let value = js_value_to_string(safe_cx, extends_val.get()).to_ascii_lowercase();
            if !value.is_empty() {
                extends_tag = Some(value);
            }
        }
    }

    let ctor_ptr = ctor_obj.get() as usize;
    let mut pending_promises: Vec<PersistentRooted> = Vec::new();
    let mut did_insert = false;

    CUSTOM_ELEMENTS_STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.definitions_by_name.contains_key(&name) || state.ctor_ptrs.contains(&ctor_ptr) {
            return;
        }

        let mut ctor_root = PersistentRooted::new();
        unsafe { ctor_root.init(safe_cx, ctor_obj.get()) };
        let mut prototype_root = PersistentRooted::new();
        unsafe { prototype_root.init(safe_cx, prototype_obj.get()) };

        state.definitions_by_name.insert(
            name.clone(),
            CustomElementDefinition {
                name: name.clone(),
                extends_tag,
                ctor: ctor_root,
                prototype: prototype_root,
            },
        );
        state.ctor_ptrs.insert(ctor_ptr);
        pending_promises = state.pending_when_defined.remove(&name).unwrap_or_default();
        did_insert = true;
    });

    if did_insert {
        rooted!(in(raw) let ctor_val = ObjectValue(ctor_obj.get()));
        for promise_root in pending_promises {
            rooted!(in(raw) let promise_obj = promise_root.get());
            ResolvePromise(safe_cx, promise_obj.handle().into(), ctor_val.handle().into());
        }
        upgrade_subtree_by_node_id(safe_cx, 0, None);
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn custom_elements_get(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc == 0 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let name = normalize_custom_element_name(&js_value_to_string(safe_cx, *args.get(0)));
    let ctor_obj = CUSTOM_ELEMENTS_STATE.with(|state| {
        let state = state.borrow();
        state.definitions_by_name.get(&name).map(|def| def.ctor.get())
    });

    if let Some(ctor) = ctor_obj {
        args.rval().set(ObjectValue(ctor));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

unsafe extern "C" fn custom_elements_when_defined(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let raw = safe_cx.raw_cx();

    let name = if argc > 0 {
        normalize_custom_element_name(&js_value_to_string(safe_cx, *args.get(0)))
    } else {
        String::new()
    };

    rooted!(in(raw) let null_executor = ptr::null_mut::<JSObject>());
    rooted!(in(raw) let promise = NewPromiseObject(safe_cx, null_executor.handle()));
    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let existing_ctor = CUSTOM_ELEMENTS_STATE.with(|state| {
        let state = state.borrow();
        state.definitions_by_name.get(&name).map(|def| def.ctor.get())
    });

    if let Some(ctor_obj) = existing_ctor {
        rooted!(in(raw) let ctor_val = ObjectValue(ctor_obj));
        ResolvePromise(safe_cx, promise.handle().into(), ctor_val.handle().into());
    } else {
        let mut promise_root = PersistentRooted::new();
        promise_root.init(safe_cx, promise.get());
        CUSTOM_ELEMENTS_STATE.with(|state| {
            state
                .borrow_mut()
                .pending_when_defined
                .entry(name)
                .or_default()
                .push(promise_root);
        });
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

unsafe extern "C" fn custom_elements_upgrade(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc > 0 {
        if let Some(root_id) = get_node_id_from_value(safe_cx, *args.get(0)) {
            upgrade_subtree_by_node_id(safe_cx, root_id, None);
        }
    }

    args.rval().set(UndefinedValue());
    true
}

pub(crate) fn setup_custom_elements(runtime: &mut JsRuntime) -> Result<(), String> {
    runtime.do_with_jsapi(|cx, global| unsafe {
        let raw_cx = cx.raw_cx();
        let global_obj = global.get();

        CUSTOM_ELEMENTS_STATE.with(|state| state.borrow_mut().clear());

        define_function(cx, global_obj, "CustomElementRegistry", Some(custom_element_registry_constructor), 0)?;

        rooted!(in(raw_cx) let registry = JS_NewPlainObject(raw_cx));
        if registry.get().is_null() {
            return Err("Failed to create customElements object".to_string());
        }

        define_function(cx, registry.get(), "define", Some(custom_elements_define), 3)?;
        define_function(cx, registry.get(), "get", Some(custom_elements_get), 1)?;
        define_function(cx, registry.get(), "whenDefined", Some(custom_elements_when_defined), 1)?;
        define_function(cx, registry.get(), "upgrade", Some(custom_elements_upgrade), 1)?;

        rooted!(in(raw_cx) let registry_val = ObjectValue(registry.get()));
        rooted!(in(raw_cx) let global_rooted = global_obj);
        let custom_elements_name = std::ffi::CString::new("customElements").unwrap();
        JS_DefineProperty(
            raw_cx,
            global_rooted.handle().into(),
            custom_elements_name.as_ptr(),
            registry_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );

        upgrade_subtree_by_node_id(cx, 0, None);

        Ok::<(), String>(())
    })
}

