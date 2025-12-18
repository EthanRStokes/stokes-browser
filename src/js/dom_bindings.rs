use super::element_bindings;
// DOM bindings for JavaScript using mozjs
use super::runtime::JsRuntime;
use crate::dom::Dom;
use mozjs::gc::Handle;
use mozjs::jsapi::{
    CallArgs, CurrentGlobalOrNull, HandleValueArray, JSContext, JSNative, JSObject, JS_DefineFunction,
    JS_DefineProperty, JS_NewPlainObject, JS_NewUCStringCopyN, NewArrayObject,
    JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, ObjectValue, StringValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers::JS_ValueToSource;
use std::cell::RefCell;
use std::os::raw::c_uint;
use std::ptr::NonNull;

// Thread-local storage for DOM reference
thread_local! {
    static DOM_REF: RefCell<Option<*mut Dom>> = RefCell::new(None);
    static USER_AGENT: RefCell<String> = RefCell::new(String::new());
}

/// Set up DOM bindings in the JavaScript context
pub fn setup_dom_bindings(runtime: &mut JsRuntime, document_root: *mut Dom, user_agent: String) -> Result<(), String> {
    let raw_cx = unsafe { runtime.cx().raw_cx() };

    // Store DOM reference in thread-local storage
    DOM_REF.with(|dom| {
        *dom.borrow_mut() = Some(document_root);
    });
    USER_AGENT.with(|ua| {
        *ua.borrow_mut() = user_agent.clone();
    });

    unsafe {
        rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
        if global.get().is_null() {
            return Err("No global object for DOM setup".to_string());
        }

        // Create and set up document object
        setup_document(raw_cx, global.handle().get())?;

        // Set up window object (as alias to global)
        setup_window(raw_cx, global.handle().get(), &user_agent)?;

        // Set up navigator object
        setup_navigator(raw_cx, global.handle().get(), &user_agent)?;

        // Set up location object
        setup_location(raw_cx, global.handle().get())?;

        // Set up localStorage and sessionStorage
        setup_storage(raw_cx, global.handle().get())?;

        // Set up Node constructor with constants
        setup_node_constructor(raw_cx, global.handle().get())?;

        // Set up Element and HTMLElement constructors
        setup_element_constructors(raw_cx, global.handle().get())?;

        // Set up Event and CustomEvent constructors
        setup_event_constructors(raw_cx, global.handle().get())?;

        // Set up XMLHttpRequest constructor
        setup_xhr_constructor(raw_cx, global.handle().get())?;

        // Set up atob/btoa functions
        setup_base64_functions(raw_cx, global.handle().get())?;

        // Set up dataLayer for Google Analytics compatibility
        setup_data_layer(raw_cx, global.handle().get())?;
    }

    println!("[JS] DOM bindings initialized");
    Ok(())
}

/// Set up the document object
unsafe fn setup_document(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let document = JS_NewPlainObject(raw_cx));
    if document.get().is_null() {
        return Err("Failed to create document object".to_string());
    }

    // Define document methods
    define_function(raw_cx, document.get(), "getElementById", Some(document_get_element_by_id), 1)?;
    define_function(raw_cx, document.get(), "getElementsByTagName", Some(document_get_elements_by_tag_name), 1)?;
    define_function(raw_cx, document.get(), "getElementsByClassName", Some(document_get_elements_by_class_name), 1)?;
    define_function(raw_cx, document.get(), "querySelector", Some(document_query_selector), 1)?;
    define_function(raw_cx, document.get(), "querySelectorAll", Some(document_query_selector_all), 1)?;
    define_function(raw_cx, document.get(), "createElement", Some(document_create_element), 1)?;
    define_function(raw_cx, document.get(), "createTextNode", Some(document_create_text_node), 1)?;
    define_function(raw_cx, document.get(), "createDocumentFragment", Some(document_create_document_fragment), 0)?;

    // Create documentElement (represents <html>)
    rooted!(in(raw_cx) let document_element = JS_NewPlainObject(raw_cx));
    if !document_element.get().is_null() {
        set_string_property(raw_cx, document_element.get(), "tagName", "HTML")?;
        set_string_property(raw_cx, document_element.get(), "nodeName", "HTML")?;
        set_int_property(raw_cx, document_element.get(), "nodeType", 1)?;

        rooted!(in(raw_cx) let doc_elem_val = ObjectValue(document_element.get()));
        let name = std::ffi::CString::new("documentElement").unwrap();
        rooted!(in(raw_cx) let document_rooted = document.get());
        JS_DefineProperty(
            raw_cx,
            document_rooted.handle().into(),
            name.as_ptr(),
            doc_elem_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Set document on global
    rooted!(in(raw_cx) let document_val = ObjectValue(document.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("document").unwrap();
    if !JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        document_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        return Err("Failed to define document property".to_string());
    }

    Ok(())
}

/// Set up the window object (as alias to global)
unsafe fn setup_window(raw_cx: *mut JSContext, global: *mut JSObject, user_agent: &str) -> Result<(), String> {
    rooted!(in(raw_cx) let global_val = ObjectValue(global));
    rooted!(in(raw_cx) let global_rooted = global);

    // window, self, top, parent, globalThis all point to global
    for name in &["window", "self", "top", "parent", "globalThis"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            raw_cx,
            global_rooted.handle().into(),
            cname.as_ptr(),
            global_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Define window functions on global
    define_function(raw_cx, global, "alert", Some(window_alert), 1)?;
    define_function(raw_cx, global, "confirm", Some(window_confirm), 1)?;
    define_function(raw_cx, global, "prompt", Some(window_prompt), 2)?;
    define_function(raw_cx, global, "requestAnimationFrame", Some(window_request_animation_frame), 1)?;
    define_function(raw_cx, global, "cancelAnimationFrame", Some(window_cancel_animation_frame), 1)?;
    define_function(raw_cx, global, "getComputedStyle", Some(window_get_computed_style), 1)?;
    define_function(raw_cx, global, "addEventListener", Some(window_add_event_listener), 3)?;
    define_function(raw_cx, global, "removeEventListener", Some(window_remove_event_listener), 3)?;
    define_function(raw_cx, global, "scrollTo", Some(window_scroll_to), 2)?;
    define_function(raw_cx, global, "scrollBy", Some(window_scroll_by), 2)?;

    // Set innerWidth/innerHeight properties
    set_int_property(raw_cx, global, "innerWidth", 1920)?;
    set_int_property(raw_cx, global, "innerHeight", 1080)?;
    set_int_property(raw_cx, global, "outerWidth", 1920)?;
    set_int_property(raw_cx, global, "outerHeight", 1080)?;
    set_int_property(raw_cx, global, "screenX", 0)?;
    set_int_property(raw_cx, global, "screenY", 0)?;
    set_int_property(raw_cx, global, "scrollX", 0)?;
    set_int_property(raw_cx, global, "scrollY", 0)?;
    set_int_property(raw_cx, global, "pageXOffset", 0)?;
    set_int_property(raw_cx, global, "pageYOffset", 0)?;
    set_int_property(raw_cx, global, "devicePixelRatio", 1)?;

    Ok(())
}

/// Set up the navigator object
unsafe fn setup_navigator(raw_cx: *mut JSContext, global: *mut JSObject, user_agent: &str) -> Result<(), String> {
    rooted!(in(raw_cx) let navigator = JS_NewPlainObject(raw_cx));
    if navigator.get().is_null() {
        return Err("Failed to create navigator object".to_string());
    }

    set_string_property(raw_cx, navigator.get(), "userAgent", user_agent)?;
    set_string_property(raw_cx, navigator.get(), "language", "en-US")?;
    set_string_property(raw_cx, navigator.get(), "platform", std::env::consts::OS)?;
    set_string_property(raw_cx, navigator.get(), "appName", "Stokes Browser")?;
    set_string_property(raw_cx, navigator.get(), "appVersion", "1.0")?;
    set_string_property(raw_cx, navigator.get(), "vendor", "Stokes")?;
    set_bool_property(raw_cx, navigator.get(), "onLine", true)?;
    set_bool_property(raw_cx, navigator.get(), "cookieEnabled", true)?;

    // Set navigator on global
    rooted!(in(raw_cx) let navigator_val = ObjectValue(navigator.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("navigator").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        navigator_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up the location object
unsafe fn setup_location(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let location = JS_NewPlainObject(raw_cx));
    if location.get().is_null() {
        return Err("Failed to create location object".to_string());
    }

    set_string_property(raw_cx, location.get(), "href", "about:blank")?;
    set_string_property(raw_cx, location.get(), "protocol", "about:")?;
    set_string_property(raw_cx, location.get(), "host", "")?;
    set_string_property(raw_cx, location.get(), "hostname", "")?;
    set_string_property(raw_cx, location.get(), "port", "")?;
    set_string_property(raw_cx, location.get(), "pathname", "blank")?;
    set_string_property(raw_cx, location.get(), "search", "")?;
    set_string_property(raw_cx, location.get(), "hash", "")?;
    set_string_property(raw_cx, location.get(), "origin", "null")?;

    define_function(raw_cx, location.get(), "reload", Some(location_reload), 0)?;
    define_function(raw_cx, location.get(), "assign", Some(location_assign), 1)?;
    define_function(raw_cx, location.get(), "replace", Some(location_replace), 1)?;

    // Set location on global
    rooted!(in(raw_cx) let location_val = ObjectValue(location.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("location").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        location_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up localStorage and sessionStorage
unsafe fn setup_storage(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    // Create storage object with getItem, setItem, removeItem, clear methods
    rooted!(in(raw_cx) let storage = JS_NewPlainObject(raw_cx));
    if storage.get().is_null() {
        return Err("Failed to create storage object".to_string());
    }

    define_function(raw_cx, storage.get(), "getItem", Some(storage_get_item), 1)?;
    define_function(raw_cx, storage.get(), "setItem", Some(storage_set_item), 2)?;
    define_function(raw_cx, storage.get(), "removeItem", Some(storage_remove_item), 1)?;
    define_function(raw_cx, storage.get(), "clear", Some(storage_clear), 0)?;
    define_function(raw_cx, storage.get(), "key", Some(storage_key), 1)?;
    set_int_property(raw_cx, storage.get(), "length", 0)?;

    rooted!(in(raw_cx) let storage_val = ObjectValue(storage.get()));
    rooted!(in(raw_cx) let global_rooted = global);

    // localStorage
    let name = std::ffi::CString::new("localStorage").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        storage_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // sessionStorage (same object for now)
    let name = std::ffi::CString::new("sessionStorage").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        storage_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up Node constructor with node type constants
unsafe fn setup_node_constructor(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let node = JS_NewPlainObject(raw_cx));
    if node.get().is_null() {
        return Err("Failed to create Node constructor".to_string());
    }

    set_int_property(raw_cx, node.get(), "ELEMENT_NODE", 1)?;
    set_int_property(raw_cx, node.get(), "ATTRIBUTE_NODE", 2)?;
    set_int_property(raw_cx, node.get(), "TEXT_NODE", 3)?;
    set_int_property(raw_cx, node.get(), "CDATA_SECTION_NODE", 4)?;
    set_int_property(raw_cx, node.get(), "ENTITY_REFERENCE_NODE", 5)?;
    set_int_property(raw_cx, node.get(), "ENTITY_NODE", 6)?;
    set_int_property(raw_cx, node.get(), "PROCESSING_INSTRUCTION_NODE", 7)?;
    set_int_property(raw_cx, node.get(), "COMMENT_NODE", 8)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_NODE", 9)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_TYPE_NODE", 10)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_FRAGMENT_NODE", 11)?;
    set_int_property(raw_cx, node.get(), "NOTATION_NODE", 12)?;

    rooted!(in(raw_cx) let node_val = ObjectValue(node.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("Node").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        node_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up Element and HTMLElement constructors
unsafe fn setup_element_constructors(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    // Element constructor
    rooted!(in(raw_cx) let element = JS_NewPlainObject(raw_cx));
    if element.get().is_null() {
        return Err("Failed to create Element constructor".to_string());
    }
    set_int_property(raw_cx, element.get(), "ELEMENT_NODE", 1)?;

    rooted!(in(raw_cx) let element_val = ObjectValue(element.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("Element").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // HTMLElement constructor (alias for now)
    let name = std::ffi::CString::new("HTMLElement").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up Event and CustomEvent constructors
unsafe fn setup_event_constructors(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let event = JS_NewPlainObject(raw_cx));
    if event.get().is_null() {
        return Err("Failed to create Event constructor".to_string());
    }

    rooted!(in(raw_cx) let event_val = ObjectValue(event.get()));
    rooted!(in(raw_cx) let global_rooted = global);

    let name = std::ffi::CString::new("Event").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        event_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    let name = std::ffi::CString::new("CustomEvent").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        event_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up XMLHttpRequest constructor
unsafe fn setup_xhr_constructor(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let xhr = JS_NewPlainObject(raw_cx));
    if xhr.get().is_null() {
        return Err("Failed to create XMLHttpRequest constructor".to_string());
    }

    rooted!(in(raw_cx) let xhr_val = ObjectValue(xhr.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("XMLHttpRequest").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        xhr_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up atob/btoa functions
unsafe fn setup_base64_functions(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    define_function(raw_cx, global, "atob", Some(window_atob), 1)?;
    define_function(raw_cx, global, "btoa", Some(window_btoa), 1)?;
    Ok(())
}

/// Set up dataLayer for Google Analytics compatibility
unsafe fn setup_data_layer(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    // Create an empty array for dataLayer
    rooted!(in(raw_cx) let data_layer = create_empty_array(raw_cx));
    if data_layer.get().is_null() {
        return Err("Failed to create dataLayer array".to_string());
    }

    rooted!(in(raw_cx) let data_layer_val = ObjectValue(data_layer.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("dataLayer").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        data_layer_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

// ============================================================================
// Helper functions
// ============================================================================

/// Create an empty JavaScript array
unsafe fn create_empty_array(raw_cx: *mut JSContext) -> *mut JSObject {
    NewArrayObject(raw_cx, &HandleValueArray::empty())
}

unsafe fn define_function(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    func: JSNative,
    nargs: u32,
) -> Result<(), String> {
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    if JS_DefineFunction(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        func,
        nargs,
        JSPROP_ENUMERATE as u32,
    ).is_null() {
        Err(format!("Failed to define function {}", name))
    } else {
        Ok(())
    }
}

unsafe fn set_string_property(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    value: &str,
) -> Result<(), String> {
    let utf16: Vec<u16> = value.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(raw_cx, utf16.as_ptr(), utf16.len()));
    rooted!(in(raw_cx) let val = StringValue(&*str_val.get()));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        Err(format!("Failed to set property {}", name))
    } else {
        Ok(())
    }
}

unsafe fn set_int_property(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    value: i32,
) -> Result<(), String> {
    rooted!(in(raw_cx) let val = Int32Value(value));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        Err(format!("Failed to set property {}", name))
    } else {
        Ok(())
    }
}

unsafe fn set_bool_property(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    value: bool,
) -> Result<(), String> {
    rooted!(in(raw_cx) let val = BooleanValue(value));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        Err(format!("Failed to set property {}", name))
    } else {
        Ok(())
    }
}

/// Convert a JS value to a Rust string
unsafe fn js_value_to_string(raw_cx: *mut JSContext, val: JSVal) -> String {
    if val.is_undefined() {
        return "undefined".to_string();
    }
    if val.is_null() {
        return "null".to_string();
    }
    if val.is_boolean() {
        return val.to_boolean().to_string();
    }
    if val.is_int32() {
        return val.to_int32().to_string();
    }
    if val.is_double() {
        return val.to_double().to_string();
    }
    if val.is_string() {
        rooted!(in(raw_cx) let str_val = val.to_string());
        if str_val.get().is_null() {
            return String::new();
        }
        return unsafe { mozjs::conversions::jsstr_to_string(raw_cx, NonNull::new(str_val.get()).unwrap()) };
    }

    // For objects, try to convert to source
    rooted!(in(raw_cx) let str_val = JS_ValueToSource(raw_cx, Handle::from_marked_location(&val)));
    if str_val.get().is_null() {
        return "[object]".to_string();
    }
    unsafe { mozjs::conversions::jsstr_to_string(raw_cx, NonNull::new(str_val.get()).unwrap()) }
}

/// Create a JS string from a Rust string
unsafe fn create_js_string(raw_cx: *mut JSContext, s: &str) -> JSVal {
    let utf16: Vec<u16> = s.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(raw_cx, utf16.as_ptr(), utf16.len()));
    StringValue(&*str_val.get())
}

// ============================================================================
// Document methods
// ============================================================================

/// document.getElementById implementation
unsafe extern "C" fn document_get_element_by_id(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let id = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    if id.is_empty() {
        args.rval().set(mozjs::jsval::NullValue());
        return true;
    }

    println!("[JS] document.getElementById('{}') called", id);

    // Try to find the element in the DOM
    let element = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            // Search for element with matching id
            if let Some(&node_id) = dom.nodes_to_id.get(&id) {
                Some(node_id)
            } else {
                None
            }
        } else {
            None
        }
    });

    if let Some(node_id) = element {
        // Create a JS element wrapper
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, node_id) {
            args.rval().set(js_elem);
        } else {
            args.rval().set(mozjs::jsval::NullValue());
        }
    } else {
        println!("[JS] Element with id '{}' not found", id);
        args.rval().set(mozjs::jsval::NullValue());
    }

    true
}

/// document.getElementsByTagName implementation
unsafe extern "C" fn document_get_elements_by_tag_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let tag_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.getElementsByTagName('{}') called", tag_name);

    // Return an empty array for now
    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));
    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.getElementsByClassName implementation
unsafe extern "C" fn document_get_elements_by_class_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.getElementsByClassName('{}') called", class_name);

    // Return an empty array for now
    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));
    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.querySelector implementation
unsafe extern "C" fn document_query_selector(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.querySelector('{}') called", selector);

    // Return null for now
    args.rval().set(mozjs::jsval::NullValue());
    true
}

/// document.querySelectorAll implementation
unsafe extern "C" fn document_query_selector_all(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.querySelectorAll('{}') called", selector);

    // Return an empty array for now
    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));
    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.createElement implementation
unsafe extern "C" fn document_create_element(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let tag_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    if tag_name.is_empty() {
        args.rval().set(mozjs::jsval::NullValue());
        return true;
    }

    println!("[JS] document.createElement('{}') called", tag_name);

    // Create a stub element
    match element_bindings::create_stub_element(raw_cx, &tag_name) {
        Ok(elem) => args.rval().set(elem),
        Err(_) => args.rval().set(mozjs::jsval::NullValue()),
    }
    true
}

/// document.createTextNode implementation
unsafe extern "C" fn document_create_text_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let text = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.createTextNode('{}') called", text);

    // Create a text node object
    rooted!(in(raw_cx) let text_node = JS_NewPlainObject(raw_cx));
    if !text_node.get().is_null() {
        let _ = set_int_property(raw_cx, text_node.get(), "nodeType", 3);
        let _ = set_string_property(raw_cx, text_node.get(), "nodeName", "#text");
        let _ = set_string_property(raw_cx, text_node.get(), "textContent", &text);
        let _ = set_string_property(raw_cx, text_node.get(), "nodeValue", &text);
        args.rval().set(ObjectValue(text_node.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// document.createDocumentFragment implementation
unsafe extern "C" fn document_create_document_fragment(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] document.createDocumentFragment() called");

    // Create a document fragment object
    rooted!(in(raw_cx) let fragment = JS_NewPlainObject(raw_cx));
    if !fragment.get().is_null() {
        let _ = set_int_property(raw_cx, fragment.get(), "nodeType", 11);
        let _ = set_string_property(raw_cx, fragment.get(), "nodeName", "#document-fragment");
        let _ = define_function(raw_cx, fragment.get(), "appendChild", Some(element_append_child), 1);
        let _ = define_function(raw_cx, fragment.get(), "querySelector", Some(document_query_selector), 1);
        let _ = define_function(raw_cx, fragment.get(), "querySelectorAll", Some(document_query_selector_all), 1);
        args.rval().set(ObjectValue(fragment.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

// ============================================================================
// Window methods
// ============================================================================

/// window.alert implementation
unsafe extern "C" fn window_alert(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    super::alert_callback::trigger_alert(message);
    args.rval().set(UndefinedValue());
    true
}

/// window.confirm implementation
unsafe extern "C" fn window_confirm(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.confirm('{}') called - returning false", message);
    args.rval().set(BooleanValue(false));
    true
}

/// window.prompt implementation
unsafe extern "C" fn window_prompt(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.prompt('{}') called - returning null", message);
    args.rval().set(mozjs::jsval::NullValue());
    true
}

/// window.requestAnimationFrame implementation
unsafe extern "C" fn window_request_animation_frame(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] requestAnimationFrame called");
    args.rval().set(Int32Value(1)); // Return a dummy request ID
    true
}

/// window.cancelAnimationFrame implementation
unsafe extern "C" fn window_cancel_animation_frame(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] cancelAnimationFrame called");
    args.rval().set(UndefinedValue());
    true
}

/// window.getComputedStyle implementation
unsafe extern "C" fn window_get_computed_style(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] getComputedStyle called");

    // Return an empty style object
    rooted!(in(raw_cx) let style = JS_NewPlainObject(raw_cx));
    if !style.get().is_null() {
        let _ = define_function(raw_cx, style.get(), "getPropertyValue", Some(style_get_property_value), 1);
        args.rval().set(ObjectValue(style.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// window.addEventListener implementation
unsafe extern "C" fn window_add_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.addEventListener('{}') called", event_type);
    args.rval().set(UndefinedValue());
    true
}

/// window.removeEventListener implementation
unsafe extern "C" fn window_remove_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.removeEventListener('{}') called", event_type);
    args.rval().set(UndefinedValue());
    true
}

/// window.scrollTo implementation
unsafe extern "C" fn window_scroll_to(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] window.scrollTo called");
    args.rval().set(UndefinedValue());
    true
}

/// window.scrollBy implementation
unsafe extern "C" fn window_scroll_by(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] window.scrollBy called");
    args.rval().set(UndefinedValue());
    true
}

/// window.atob implementation (base64 decode)
unsafe extern "C" fn window_atob(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let encoded = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    use base64::Engine;
    match base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) {
        Ok(decoded) => {
            if let Ok(s) = String::from_utf8(decoded) {
                args.rval().set(create_js_string(raw_cx, &s));
            } else {
                args.rval().set(create_js_string(raw_cx, ""));
            }
        }
        Err(_) => {
            args.rval().set(create_js_string(raw_cx, ""));
        }
    }
    true
}

/// window.btoa implementation (base64 encode)
unsafe extern "C" fn window_btoa(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let data = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
    args.rval().set(create_js_string(raw_cx, &encoded));
    true
}

// ============================================================================
// Location methods
// ============================================================================

/// location.reload implementation
unsafe extern "C" fn location_reload(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] location.reload() called");
    args.rval().set(UndefinedValue());
    true
}

/// location.assign implementation
unsafe extern "C" fn location_assign(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let url = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] location.assign('{}') called", url);
    args.rval().set(UndefinedValue());
    true
}

/// location.replace implementation
unsafe extern "C" fn location_replace(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let url = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] location.replace('{}') called", url);
    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// Storage methods
// ============================================================================

/// Storage.getItem implementation
unsafe extern "C" fn storage_get_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] Storage.getItem('{}') called", key);
    args.rval().set(mozjs::jsval::NullValue());
    true
}

/// Storage.setItem implementation
unsafe extern "C" fn storage_set_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    println!("[JS] Storage.setItem('{}', '{}') called", key, value);
    args.rval().set(UndefinedValue());
    true
}

/// Storage.removeItem implementation
unsafe extern "C" fn storage_remove_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] Storage.removeItem('{}') called", key);
    args.rval().set(UndefinedValue());
    true
}

/// Storage.clear implementation
unsafe extern "C" fn storage_clear(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] Storage.clear() called");
    args.rval().set(UndefinedValue());
    true
}

/// Storage.key implementation
unsafe extern "C" fn storage_key(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] Storage.key() called");
    args.rval().set(mozjs::jsval::NullValue());
    true
}

// ============================================================================
// Element methods (shared)
// ============================================================================

/// element.appendChild implementation
unsafe extern "C" fn element_append_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.appendChild() called");

    if argc > 0 {
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// style.getPropertyValue implementation
unsafe extern "C" fn style_get_property_value(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let property = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] style.getPropertyValue('{}') called", property);
    args.rval().set(create_js_string(raw_cx, ""));
    true
}

