// Shared helper functions for JavaScript bindings
use mozjs::conversions::jsstr_to_string;
use mozjs::gc::Handle;
use mozjs::jsapi::{
    CallArgs, HandleValueArray, JSContext, JSNative, JSObject, JS_DefineFunction,
    JS_DefineProperty, JS_NewUCStringCopyN, NewArrayObject, JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, StringValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers::JS_ValueToSource;
use std::ptr::NonNull;

/// Create an empty JavaScript array
pub unsafe fn create_empty_array(raw_cx: *mut JSContext) -> *mut JSObject {
    NewArrayObject(raw_cx, &HandleValueArray::empty())
}

/// Define a function on a JavaScript object
pub unsafe fn define_function(
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
    )
    .is_null()
    {
        Err(format!("Failed to define function {}", name))
    } else {
        Ok(())
    }
}

/// Set a string property on a JavaScript object
pub unsafe fn set_string_property(
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

/// Set an integer property on a JavaScript object
pub unsafe fn set_int_property(
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

/// Set a boolean property on a JavaScript object
pub unsafe fn set_bool_property(
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
pub unsafe fn js_value_to_string(raw_cx: *mut JSContext, val: JSVal) -> String {
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
        return jsstr_to_string(raw_cx, NonNull::new(str_val.get()).unwrap());
    }

    // For objects, try to convert to source
    rooted!(in(raw_cx) let str_val = JS_ValueToSource(raw_cx, Handle::from_marked_location(&val)));
    if str_val.get().is_null() {
        return "[object]".to_string();
    }
    jsstr_to_string(raw_cx, NonNull::new(str_val.get()).unwrap())
}

/// Create a JS string from a Rust string
pub unsafe fn create_js_string(raw_cx: *mut JSContext, s: &str) -> JSVal {
    let utf16: Vec<u16> = s.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(raw_cx, utf16.as_ptr(), utf16.len()));
    StringValue(&*str_val.get())
}

/// Get the node ID from a JS element object's `this` value
pub unsafe fn get_node_id_from_this(raw_cx: *mut JSContext, args: &CallArgs) -> Option<usize> {
    let this_val = args.thisv();
    if !this_val.get().is_object() || this_val.get().is_null() {
        return None;
    }

    rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
    rooted!(in(raw_cx) let mut ptr_val = UndefinedValue());

    let cname = std::ffi::CString::new("__nodeId").unwrap();
    if !mozjs::jsapi::JS_GetProperty(
        raw_cx,
        this_obj.handle().into(),
        cname.as_ptr(),
        ptr_val.handle_mut().into(),
    ) {
        return None;
    }

    if ptr_val.get().is_double() {
        Some(ptr_val.get().to_double() as usize)
    } else if ptr_val.get().is_int32() {
        Some(ptr_val.get().to_int32() as usize)
    } else {
        None
    }
}

/// Define a property with getter/setter on a JavaScript object using Object.defineProperty
pub unsafe fn define_property_accessor(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    prop_name: &str,
    getter_name: &str,
    setter_name: &str,
) -> Result<(), String> {
    use mozjs::jsapi::{CurrentGlobalOrNull, Compile1, JS_ExecuteScript, Handle, MutableHandleValue};
    use mozjs::context::JSContext as SafeJSContext;
    use mozjs::rust::{CompileOptionsWrapper, transform_str_to_source_text};

    // We'll use a well-known temporary variable name
    let temp_var_name = "__definePropertyTarget__";

    rooted!(in(raw_cx) let obj_val = mozjs::jsval::ObjectValue(obj));

    // Get the global object
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
    if global.is_null() {
        return Err("No global object".to_string());
    }

    // Set temporary global variable
    let temp_cname = std::ffi::CString::new(temp_var_name).unwrap();
    if !mozjs::jsapi::JS_SetProperty(
        raw_cx,
        global.handle().into(),
        temp_cname.as_ptr(),
        obj_val.handle().into(),
    ) {
        return Err("Failed to set temporary variable".to_string());
    }

    // Execute script to define the property
    let script = format!(
        r#"(function() {{
            Object.defineProperty(__definePropertyTarget__, '{prop}', {{
                get: function() {{
                    return this.{getter}();
                }},
                set: function(value) {{
                    this.{setter}(value);
                }},
                configurable: true,
                enumerable: true
            }});
        }})();"#,
        prop = prop_name,
        getter = getter_name,
        setter = setter_name
    );

    // Create a safe JSContext wrapper for the compile options
    // SAFETY: We're within a valid JSContext scope, and the raw pointer is valid
    let safe_cx: &SafeJSContext = std::mem::transmute(&raw_cx);
    let options = CompileOptionsWrapper::new(safe_cx, "define_property_accessor".parse().unwrap(), 1);

    // Compile the script
    let compiled = Compile1(raw_cx, options.ptr, &mut transform_str_to_source_text(&script));
    if compiled.is_null() {
        // Clean up the temporary variable
        rooted!(in(raw_cx) let undefined = UndefinedValue());
        mozjs::jsapi::JS_SetProperty(
            raw_cx,
            global.handle().into(),
            temp_cname.as_ptr(),
            undefined.handle().into(),
        );
        return Err("Failed to compile property definition script".to_string());
    }

    rooted!(in(raw_cx) let script_root = compiled);
    rooted!(in(raw_cx) let mut rval = UndefinedValue());

    // Execute the script
    let success = JS_ExecuteScript(raw_cx, Handle::from(script_root.handle()), MutableHandleValue::from(rval.handle_mut()));

    // Clean up the temporary variable
    rooted!(in(raw_cx) let undefined = UndefinedValue());
    mozjs::jsapi::JS_SetProperty(
        raw_cx,
        global.handle().into(),
        temp_cname.as_ptr(),
        undefined.handle().into(),
    );

    if !success {
        Err("Failed to execute property definition script".to_string())
    } else {
        Ok(())
    }
}

/// Get the node ID from an arbitrary JS value (e.g. an argument object) by reading its `__nodeId` property.
pub unsafe fn get_node_id_from_value(raw_cx: *mut JSContext, val: JSVal) -> Option<usize> {
    if !val.is_object() || val.is_null() {
        return None;
    }

    rooted!(in(raw_cx) let obj = val.to_object());
    rooted!(in(raw_cx) let mut ptr_val = UndefinedValue());

    let cname = std::ffi::CString::new("__nodeId").unwrap();
    if !mozjs::jsapi::JS_GetProperty(raw_cx, obj.handle().into(), cname.as_ptr(), ptr_val.handle_mut().into()) {
        return None;
    }

    if ptr_val.get().is_double() {
        Some(ptr_val.get().to_double() as usize)
    } else if ptr_val.get().is_int32() {
        Some(ptr_val.get().to_int32() as usize)
    } else {
        None
    }
}

/// Convert JavaScript camelCase property name to CSS kebab-case
pub fn to_css_property_name(js_name: &str) -> String {
    let mut result = String::with_capacity(js_name.len() + 5);
    for ch in js_name.chars() {
        if ch.is_uppercase() {
            result.push('-');
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

