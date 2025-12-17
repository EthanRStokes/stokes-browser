// Element bindings for JavaScript using mozjs
use mozjs::jsval::{JSVal, UndefinedValue, ObjectValue, Int32Value, BooleanValue, StringValue, NullValue};
use mozjs::rooted;
use mozjs::gc::Handle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_uint;
use std::ptr;
use mozjs::jsapi::{
    CallArgs, HandleValueArray, JSContext, JSObject, NewArrayObject,
    JS_DefineFunction, JS_DefineProperty, JS_GetTwoByteStringCharsAndLength, JS_NewPlainObject, JS_NewUCStringCopyN,
    JSPROP_ENUMERATE,
};
use mozjs::rust::wrappers::JS_ValueToSource;

// Thread-local storage for element attributes (keyed by node pointer)
thread_local! {
    static ELEMENT_ATTRIBUTES: RefCell<HashMap<i64, HashMap<String, String>>> = RefCell::new(HashMap::new());
    static ELEMENT_CHILDREN: RefCell<HashMap<i64, Vec<i64>>> = RefCell::new(HashMap::new());
    static NEXT_NODE_PTR: RefCell<i64> = RefCell::new(1);
}

/// Generate a unique node pointer
fn generate_node_ptr() -> i64 {
    NEXT_NODE_PTR.with(|ptr| {
        let mut ptr = ptr.borrow_mut();
        let val = *ptr;
        *ptr += 1;
        val
    })
}

/// Create a JS element wrapper for a DOM node by its ID
pub unsafe fn create_js_element_by_id(raw_cx: *mut JSContext, node_id: usize) -> Result<JSVal, String> {
    // For now, create a stub element
    // In the future, this should look up the actual DOM node and create a proper wrapper
    create_stub_element(raw_cx, "div")
}

/// Create a stub element (for document.createElement and similar)
pub unsafe fn create_stub_element(raw_cx: *mut JSContext, tag_name: &str) -> Result<JSVal, String> {
    rooted!(in(raw_cx) let element = JS_NewPlainObject(raw_cx));
    if element.get().is_null() {
        return Err("Failed to create element object".to_string());
    }

    // Generate a unique pointer for this element
    let node_ptr = generate_node_ptr();

    // Initialize attributes storage for this element
    ELEMENT_ATTRIBUTES.with(|attrs| {
        attrs.borrow_mut().insert(node_ptr, HashMap::new());
    });
    ELEMENT_CHILDREN.with(|children| {
        children.borrow_mut().insert(node_ptr, Vec::new());
    });

    // Set basic properties
    set_string_property(raw_cx, element.get(), "tagName", &tag_name.to_uppercase())?;
    set_string_property(raw_cx, element.get(), "nodeName", &tag_name.to_uppercase())?;
    set_int_property(raw_cx, element.get(), "nodeType", 1)?; // ELEMENT_NODE
    set_string_property(raw_cx, element.get(), "id", "")?;
    set_string_property(raw_cx, element.get(), "className", "")?;
    set_string_property(raw_cx, element.get(), "innerHTML", "")?;
    set_string_property(raw_cx, element.get(), "outerHTML", &format!("<{0}></{0}>", tag_name.to_lowercase()))?;
    set_string_property(raw_cx, element.get(), "textContent", "")?;

    // Store the node pointer for reference
    rooted!(in(raw_cx) let ptr_val = mozjs::jsval::DoubleValue(node_ptr as f64));
    rooted!(in(raw_cx) let element_rooted = element.get());
    let cname = std::ffi::CString::new("__nodePtr").unwrap();
    JS_DefineProperty(
        raw_cx,
        element_rooted.handle().into(),
        cname.as_ptr(),
        ptr_val.handle().into(),
        0, // Hidden property
    );

    // Define element methods
    define_function(raw_cx, element.get(), "getAttribute", Some(element_get_attribute), 1)?;
    define_function(raw_cx, element.get(), "setAttribute", Some(element_set_attribute), 2)?;
    define_function(raw_cx, element.get(), "removeAttribute", Some(element_remove_attribute), 1)?;
    define_function(raw_cx, element.get(), "hasAttribute", Some(element_has_attribute), 1)?;
    define_function(raw_cx, element.get(), "appendChild", Some(element_append_child), 1)?;
    define_function(raw_cx, element.get(), "removeChild", Some(element_remove_child), 1)?;
    define_function(raw_cx, element.get(), "insertBefore", Some(element_insert_before), 2)?;
    define_function(raw_cx, element.get(), "replaceChild", Some(element_replace_child), 2)?;
    define_function(raw_cx, element.get(), "cloneNode", Some(element_clone_node), 1)?;
    define_function(raw_cx, element.get(), "querySelector", Some(element_query_selector), 1)?;
    define_function(raw_cx, element.get(), "querySelectorAll", Some(element_query_selector_all), 1)?;
    define_function(raw_cx, element.get(), "addEventListener", Some(element_add_event_listener), 3)?;
    define_function(raw_cx, element.get(), "removeEventListener", Some(element_remove_event_listener), 3)?;
    define_function(raw_cx, element.get(), "dispatchEvent", Some(element_dispatch_event), 1)?;
    define_function(raw_cx, element.get(), "focus", Some(element_focus), 0)?;
    define_function(raw_cx, element.get(), "blur", Some(element_blur), 0)?;
    define_function(raw_cx, element.get(), "click", Some(element_click), 0)?;
    define_function(raw_cx, element.get(), "getBoundingClientRect", Some(element_get_bounding_client_rect), 0)?;
    define_function(raw_cx, element.get(), "getClientRects", Some(element_get_client_rects), 0)?;
    define_function(raw_cx, element.get(), "closest", Some(element_closest), 1)?;
    define_function(raw_cx, element.get(), "matches", Some(element_matches), 1)?;
    define_function(raw_cx, element.get(), "contains", Some(element_contains), 1)?;

    // Create style object
    rooted!(in(raw_cx) let style = JS_NewPlainObject(raw_cx));
    if !style.get().is_null() {
        define_function(raw_cx, style.get(), "getPropertyValue", Some(style_get_property_value), 1)?;
        define_function(raw_cx, style.get(), "setProperty", Some(style_set_property), 3)?;
        define_function(raw_cx, style.get(), "removeProperty", Some(style_remove_property), 1)?;

        rooted!(in(raw_cx) let style_val = ObjectValue(style.get()));
        let cname = std::ffi::CString::new("style").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            style_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Create classList object
    rooted!(in(raw_cx) let class_list = JS_NewPlainObject(raw_cx));
    if !class_list.get().is_null() {
        define_function(raw_cx, class_list.get(), "add", Some(class_list_add), 1)?;
        define_function(raw_cx, class_list.get(), "remove", Some(class_list_remove), 1)?;
        define_function(raw_cx, class_list.get(), "toggle", Some(class_list_toggle), 2)?;
        define_function(raw_cx, class_list.get(), "contains", Some(class_list_contains), 1)?;
        define_function(raw_cx, class_list.get(), "replace", Some(class_list_replace), 2)?;
        set_int_property(raw_cx, class_list.get(), "length", 0)?;

        rooted!(in(raw_cx) let class_list_val = ObjectValue(class_list.get()));
        let cname = std::ffi::CString::new("classList").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            class_list_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Create dataset object
    rooted!(in(raw_cx) let dataset = JS_NewPlainObject(raw_cx));
    if !dataset.get().is_null() {
        rooted!(in(raw_cx) let dataset_val = ObjectValue(dataset.get()));
        let cname = std::ffi::CString::new("dataset").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            dataset_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Set parentNode, parentElement, children, childNodes to null/empty initially
    rooted!(in(raw_cx) let null_val = NullValue());
    for name in &["parentNode", "parentElement", "firstChild", "lastChild", "previousSibling", "nextSibling"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            null_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Create empty children and childNodes arrays
    rooted!(in(raw_cx) let children_array = create_empty_array(raw_cx));
    if !children_array.get().is_null() {
        rooted!(in(raw_cx) let children_val = ObjectValue(children_array.get()));
        let cname = std::ffi::CString::new("children").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            children_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
        let cname = std::ffi::CString::new("childNodes").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            children_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Set dimension properties
    set_int_property(raw_cx, element.get(), "offsetWidth", 0)?;
    set_int_property(raw_cx, element.get(), "offsetHeight", 0)?;
    set_int_property(raw_cx, element.get(), "offsetLeft", 0)?;
    set_int_property(raw_cx, element.get(), "offsetTop", 0)?;
    set_int_property(raw_cx, element.get(), "clientWidth", 0)?;
    set_int_property(raw_cx, element.get(), "clientHeight", 0)?;
    set_int_property(raw_cx, element.get(), "scrollWidth", 0)?;
    set_int_property(raw_cx, element.get(), "scrollHeight", 0)?;
    set_int_property(raw_cx, element.get(), "scrollLeft", 0)?;
    set_int_property(raw_cx, element.get(), "scrollTop", 0)?;

    Ok(ObjectValue(element.get()))
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
    func: mozjs::jsapi::JSNative,
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
        let mut length = 0;
        let chars = JS_GetTwoByteStringCharsAndLength(raw_cx, ptr::null(), *str_val.handle(), &mut length);
        if chars.is_null() {
            return String::new();
        }
        let slice = std::slice::from_raw_parts(chars, length);
        return String::from_utf16_lossy(slice);
    }

    // For objects, try to convert to source
    rooted!(in(raw_cx) let str_val = JS_ValueToSource(raw_cx, Handle::from_marked_location(&val)));
    if str_val.get().is_null() {
        return "[object]".to_string();
    }
    let mut length = 0;
    let chars = JS_GetTwoByteStringCharsAndLength(raw_cx, ptr::null(), *str_val.handle(), &mut length);
    if chars.is_null() {
        return "[string conversion failed]".to_string();
    }
    let slice = std::slice::from_raw_parts(chars, length);
    String::from_utf16_lossy(slice)
}

/// Create a JS string from a Rust string
unsafe fn create_js_string(raw_cx: *mut JSContext, s: &str) -> JSVal {
    let utf16: Vec<u16> = s.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(raw_cx, utf16.as_ptr(), utf16.len()));
    StringValue(&*str_val.get())
}

/// Get the node pointer from a JS element object
unsafe fn get_node_ptr_from_this(raw_cx: *mut JSContext, args: &CallArgs) -> Option<i64> {
    let this_val = args.thisv();
    if !this_val.get().is_object() || this_val.get().is_null() {
        return None;
    }

    rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
    rooted!(in(raw_cx) let mut ptr_val = UndefinedValue());

    let cname = std::ffi::CString::new("__nodePtr").unwrap();
    if !mozjs::jsapi::JS_GetProperty(raw_cx, this_obj.handle().into(), cname.as_ptr(), ptr_val.handle_mut().into()) {
        return None;
    }

    if ptr_val.get().is_double() {
        Some(ptr_val.get().to_double() as i64)
    } else if ptr_val.get().is_int32() {
        Some(ptr_val.get().to_int32() as i64)
    } else {
        None
    }
}

// ============================================================================
// Element methods
// ============================================================================

/// element.getAttribute implementation
unsafe extern "C" fn element_get_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let attr_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.getAttribute('{}') called", attr_name);

    if let Some(node_ptr) = get_node_ptr_from_this(raw_cx, &args) {
        let value = ELEMENT_ATTRIBUTES.with(|attrs| {
            attrs.borrow().get(&node_ptr)
                .and_then(|a| a.get(&attr_name).cloned())
        });

        if let Some(val) = value {
            println!("[JS] getAttribute('{}') = '{}'", attr_name, val);
            args.rval().set(create_js_string(raw_cx, &val));
        } else {
            args.rval().set(NullValue());
        }
    } else {
        args.rval().set(NullValue());
    }
    true
}

/// element.setAttribute implementation
unsafe extern "C" fn element_set_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let attr_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let attr_value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    println!("[JS] element.setAttribute('{}', '{}') called", attr_name, attr_value);

    if let Some(node_ptr) = get_node_ptr_from_this(raw_cx, &args) {
        ELEMENT_ATTRIBUTES.with(|attrs| {
            let mut attrs = attrs.borrow_mut();
            if let Some(attr_map) = attrs.get_mut(&node_ptr) {
                attr_map.insert(attr_name, attr_value);
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.removeAttribute implementation
unsafe extern "C" fn element_remove_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let attr_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.removeAttribute('{}') called", attr_name);

    if let Some(node_ptr) = get_node_ptr_from_this(raw_cx, &args) {
        ELEMENT_ATTRIBUTES.with(|attrs| {
            let mut attrs = attrs.borrow_mut();
            if let Some(attr_map) = attrs.get_mut(&node_ptr) {
                attr_map.remove(&attr_name);
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.hasAttribute implementation
unsafe extern "C" fn element_has_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let attr_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.hasAttribute('{}') called", attr_name);

    let has_attr = if let Some(node_ptr) = get_node_ptr_from_this(raw_cx, &args) {
        ELEMENT_ATTRIBUTES.with(|attrs| {
            attrs.borrow().get(&node_ptr)
                .map(|a| a.contains_key(&attr_name))
                .unwrap_or(false)
        })
    } else {
        false
    };

    args.rval().set(BooleanValue(has_attr));
    true
}

/// element.appendChild implementation
unsafe extern "C" fn element_append_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.appendChild() called");

    if argc > 0 {
        // Return the child that was appended
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.removeChild implementation
unsafe extern "C" fn element_remove_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.removeChild() called");

    if argc > 0 {
        // Return the child that was removed
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.insertBefore implementation
unsafe extern "C" fn element_insert_before(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.insertBefore() called");

    if argc > 0 {
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.replaceChild implementation
unsafe extern "C" fn element_replace_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.replaceChild() called");

    if argc > 1 {
        // Return the old child that was replaced
        args.rval().set(*args.get(1));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.cloneNode implementation
unsafe extern "C" fn element_clone_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let deep = if argc > 0 {
        let val = *args.get(0);
        val.is_boolean() && val.to_boolean()
    } else {
        false
    };

    println!("[JS] element.cloneNode({}) called", deep);

    // Create a new stub element (simplified implementation)
    match create_stub_element(raw_cx, "div") {
        Ok(elem) => args.rval().set(elem),
        Err(_) => args.rval().set(NullValue()),
    }
    true
}

/// element.querySelector implementation
unsafe extern "C" fn element_query_selector(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.querySelector('{}') called", selector);
    args.rval().set(NullValue());
    true
}

/// element.querySelectorAll implementation
unsafe extern "C" fn element_query_selector_all(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.querySelectorAll('{}') called", selector);

    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));
    args.rval().set(ObjectValue(array.get()));
    true
}

/// element.addEventListener implementation
unsafe extern "C" fn element_add_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.addEventListener('{}') called", event_type);
    args.rval().set(UndefinedValue());
    true
}

/// element.removeEventListener implementation
unsafe extern "C" fn element_remove_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.removeEventListener('{}') called", event_type);
    args.rval().set(UndefinedValue());
    true
}

/// element.dispatchEvent implementation
unsafe extern "C" fn element_dispatch_event(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.dispatchEvent() called");
    args.rval().set(BooleanValue(true));
    true
}

/// element.focus implementation
unsafe extern "C" fn element_focus(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.focus() called");
    args.rval().set(UndefinedValue());
    true
}

/// element.blur implementation
unsafe extern "C" fn element_blur(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.blur() called");
    args.rval().set(UndefinedValue());
    true
}

/// element.click implementation
unsafe extern "C" fn element_click(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.click() called");
    args.rval().set(UndefinedValue());
    true
}

/// element.getBoundingClientRect implementation
unsafe extern "C" fn element_get_bounding_client_rect(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.getBoundingClientRect() called");

    // Return a DOMRect-like object
    rooted!(in(raw_cx) let rect = JS_NewPlainObject(raw_cx));
    if !rect.get().is_null() {
        let _ = set_int_property(raw_cx, rect.get(), "x", 0);
        let _ = set_int_property(raw_cx, rect.get(), "y", 0);
        let _ = set_int_property(raw_cx, rect.get(), "width", 0);
        let _ = set_int_property(raw_cx, rect.get(), "height", 0);
        let _ = set_int_property(raw_cx, rect.get(), "top", 0);
        let _ = set_int_property(raw_cx, rect.get(), "right", 0);
        let _ = set_int_property(raw_cx, rect.get(), "bottom", 0);
        let _ = set_int_property(raw_cx, rect.get(), "left", 0);
        args.rval().set(ObjectValue(rect.get()));
    } else {
        args.rval().set(NullValue());
    }
    true
}

/// element.getClientRects implementation
unsafe extern "C" fn element_get_client_rects(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.getClientRects() called");

    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));
    args.rval().set(ObjectValue(array.get()));
    true
}

/// element.closest implementation
unsafe extern "C" fn element_closest(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.closest('{}') called", selector);
    args.rval().set(NullValue());
    true
}

/// element.matches implementation
unsafe extern "C" fn element_matches(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.matches('{}') called", selector);
    args.rval().set(BooleanValue(false));
    true
}

/// element.contains implementation
unsafe extern "C" fn element_contains(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.contains() called");
    args.rval().set(BooleanValue(false));
    true
}

// ============================================================================
// Style methods
// ============================================================================

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

/// style.setProperty implementation
unsafe extern "C" fn style_set_property(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let property = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    println!("[JS] style.setProperty('{}', '{}') called", property, value);
    args.rval().set(UndefinedValue());
    true
}

/// style.removeProperty implementation
unsafe extern "C" fn style_remove_property(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let property = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] style.removeProperty('{}') called", property);
    args.rval().set(create_js_string(raw_cx, ""));
    true
}

// ============================================================================
// ClassList methods
// ============================================================================

/// classList.add implementation
unsafe extern "C" fn class_list_add(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] classList.add('{}') called", class_name);
    args.rval().set(UndefinedValue());
    true
}

/// classList.remove implementation
unsafe extern "C" fn class_list_remove(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] classList.remove('{}') called", class_name);
    args.rval().set(UndefinedValue());
    true
}

/// classList.toggle implementation
unsafe extern "C" fn class_list_toggle(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] classList.toggle('{}') called", class_name);
    args.rval().set(BooleanValue(true)); // Assume it was added
    true
}

/// classList.contains implementation
unsafe extern "C" fn class_list_contains(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] classList.contains('{}') called", class_name);
    args.rval().set(BooleanValue(false));
    true
}

/// classList.replace implementation
unsafe extern "C" fn class_list_replace(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let old_class = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let new_class = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    println!("[JS] classList.replace('{}', '{}') called", old_class, new_class);
    args.rval().set(BooleanValue(false));
    true
}

