// Shared helper functions for JavaScript bindings
use mozjs::conversions::jsstr_to_string;
use mozjs::gc::Handle;
use mozjs::rust::wrappers2::{CurrentGlobalOrNull, JS_CallFunctionValue, JS_DefineFunction, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject, JS_NewUCStringCopyN, JS_SetProperty, NewArrayObject};
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, ObjectValue, StringValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers::JS_ValueToSource;
use std::ptr::NonNull;
use mozjs::rust::ValueArray;
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{CallArgs, HandleValueArray, JSContext, JSNative, JSObject, JSPROP_ENUMERATE};

/// Create an empty JavaScript array
pub unsafe fn create_empty_array(cx: &mut SafeJSContext) -> *mut JSObject {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let array = ValueArray::<0usize>::new([]));

    NewArrayObject(cx, &HandleValueArray::from(&array))
}

/// Define a function on a JavaScript object
pub unsafe fn define_function(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    name: &str,
    func: JSNative,
    nargs: u32,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    if JS_DefineFunction(
        cx,
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
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    name: &str,
    value: &str,
) -> Result<(), String> {
    if obj.is_null() {
        return Err(format!("Failed to set property {}: null JS object", name));
    }

    let raw_cx = cx.raw_cx();
    // Root object first: JS_NewUCStringCopyN can trigger GC and move objects.
    rooted!(in(raw_cx) let obj_rooted = obj);

    let utf16: Vec<u16> = value.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(cx, utf16.as_ptr(), utf16.len()));
    if str_val.get().is_null() {
        return Err(format!("Failed to allocate JS string for property {}", name));
    }

    rooted!(in(raw_cx) let val = StringValue(&*str_val.get()));
    let cname = std::ffi::CString::new(name)
        .map_err(|_| format!("Property name contains NUL byte: {}", name))?;
    if !JS_DefineProperty(
        cx,
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
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    name: &str,
    value: i32,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let val = Int32Value(value));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        cx,
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
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    name: &str,
    value: bool,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let val = BooleanValue(value));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        cx,
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
pub unsafe fn js_value_to_string(cx: &mut SafeJSContext, val: JSVal) -> String {
    let raw_cx = cx.raw_cx();
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
pub unsafe fn create_js_string(cx: &mut SafeJSContext, s: &str) -> JSVal {
    let raw_cx = cx.raw_cx();
    let utf16: Vec<u16> = s.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(cx, utf16.as_ptr(), utf16.len()));
    if str_val.get().is_null() {
        return UndefinedValue();
    }
    StringValue(&*str_val.get())
}

/// Get the node ID from a JS element object's `this` value
pub unsafe fn get_node_id_from_this(cx: &mut SafeJSContext, args: &CallArgs) -> Option<usize> {
    let raw_cx = cx.raw_cx();
    let this_val = args.thisv();
    if !this_val.get().is_object() || this_val.get().is_null() {
        return None;
    }

    rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
    rooted!(in(raw_cx) let mut ptr_val = UndefinedValue());

    let cname = std::ffi::CString::new("__nodeId").unwrap();
    if !JS_GetProperty(
        cx,
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
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    prop_name: &str,
    getter_name: &str,
    setter_name: &str,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let obj_rooted = obj);

    // Resolve Object.defineProperty once per call without compiling helper scripts.
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return Err("No global object".to_string());
    }

    rooted!(in(raw_cx) let mut object_ctor_val = UndefinedValue());
    let object_name = std::ffi::CString::new("Object").unwrap();
    if !JS_GetProperty(
        cx,
        global.handle().into(),
        object_name.as_ptr(),
        object_ctor_val.handle_mut().into(),
    ) || !object_ctor_val.get().is_object() {
        return Err("Failed to resolve global Object constructor".to_string());
    }
    rooted!(in(raw_cx) let object_ctor_obj = object_ctor_val.get().to_object());

    rooted!(in(raw_cx) let mut define_property_fn = UndefinedValue());
    let define_property_name = std::ffi::CString::new("defineProperty").unwrap();
    if !JS_GetProperty(
        cx,
        object_ctor_obj.handle().into(),
        define_property_name.as_ptr(),
        define_property_fn.handle_mut().into(),
    ) || !define_property_fn.get().is_object() {
        return Err("Failed to resolve Object.defineProperty".to_string());
    }

    rooted!(in(raw_cx) let mut getter_val = UndefinedValue());
    let getter_cname = std::ffi::CString::new(getter_name).unwrap();
    if !JS_GetProperty(
        cx,
        obj_rooted.handle().into(),
        getter_cname.as_ptr(),
        getter_val.handle_mut().into(),
    ) || !getter_val.get().is_object() {
        return Err(format!("Failed to resolve getter function {}", getter_name));
    }

    rooted!(in(raw_cx) let mut setter_val = UndefinedValue());
    let setter_cname = std::ffi::CString::new(setter_name).unwrap();
    if !JS_GetProperty(
        cx,
        obj_rooted.handle().into(),
        setter_cname.as_ptr(),
        setter_val.handle_mut().into(),
    ) || !setter_val.get().is_object() {
        return Err(format!("Failed to resolve setter function {}", setter_name));
    }

    rooted!(in(raw_cx) let descriptor = JS_NewPlainObject(cx));
    if descriptor.get().is_null() {
        return Err("Failed to create property descriptor object".to_string());
    }

    rooted!(in(raw_cx) let getter_obj = getter_val.get());
    let get_key = std::ffi::CString::new("get").unwrap();
    if !JS_SetProperty(cx, descriptor.handle(), get_key.as_ptr(), getter_obj.handle()) {
        return Err("Failed to define descriptor.get".to_string());
    }

    rooted!(in(raw_cx) let setter_obj = setter_val.get());
    let set_key = std::ffi::CString::new("set").unwrap();
    if !JS_SetProperty(cx, descriptor.handle(), set_key.as_ptr(), setter_obj.handle()) {
        return Err("Failed to define descriptor.set".to_string());
    }

    rooted!(in(raw_cx) let enumerable = BooleanValue(true));
    let enumerable_key = std::ffi::CString::new("enumerable").unwrap();
    if !JS_SetProperty(cx, descriptor.handle(), enumerable_key.as_ptr(), enumerable.handle()) {
        return Err("Failed to define descriptor.enumerable".to_string());
    }

    rooted!(in(raw_cx) let configurable = BooleanValue(true));
    let configurable_key = std::ffi::CString::new("configurable").unwrap();
    if !JS_SetProperty(cx, descriptor.handle(), configurable_key.as_ptr(), configurable.handle()) {
        return Err("Failed to define descriptor.configurable".to_string());
    }

    rooted!(in(raw_cx) let prop_name_val = create_js_string(cx, prop_name));
    if prop_name_val.get().is_undefined() {
        return Err(format!("Failed to create JS string for property {}", prop_name));
    }

    rooted!(in(raw_cx) let define_args = ValueArray::<3usize>::new([
        ObjectValue(obj_rooted.get()),
        prop_name_val.get(),
        ObjectValue(descriptor.get()),
    ]));
    rooted!(in(raw_cx) let mut rval = UndefinedValue());

    if !JS_CallFunctionValue(
        cx,
        object_ctor_obj.handle().into(),
        define_property_fn.handle().into(),
        &HandleValueArray::from(&define_args),
        rval.handle_mut().into(),
    ) {
        Err(format!("Failed to define accessor property {}", prop_name))
    } else {
        Ok(())
    }
}

/// Get the node ID from an arbitrary JS value (e.g. an argument object) by reading its `__nodeId` property.
pub unsafe fn get_node_id_from_value(cx: &mut SafeJSContext, val: JSVal) -> Option<usize> {
    let raw_cx = cx.raw_cx();
    if !val.is_object() || val.is_null() {
        return None;
    }

    rooted!(in(raw_cx) let obj = val.to_object());
    rooted!(in(raw_cx) let mut ptr_val = UndefinedValue());

    let cname = std::ffi::CString::new("__nodeId").unwrap();
    if !JS_GetProperty(cx, obj.handle(), cname.as_ptr(), ptr_val.handle_mut()) {
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

pub trait ToSafeCx {
    fn to_safe_cx(self) -> SafeJSContext;
}

impl ToSafeCx for *mut JSContext {
    #[inline]
    fn to_safe_cx(self) -> SafeJSContext {
        unsafe { SafeJSContext::from_ptr(NonNull::new(self).expect("Failed to unwrap safe JSContext")) }
    }
}

impl ToSafeCx for NonNull<JSContext> {
    #[inline]
    fn to_safe_cx(self) -> SafeJSContext {
        unsafe { SafeJSContext::from_ptr(self) }
    }
}