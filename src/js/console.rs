// Console API implementation for JavaScript using mozjs
use super::runtime::JsRuntime;
use mozjs::jsval::{JSVal, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::HandleValue;
use std::os::raw::c_uint;
use std::ptr;
use mozjs::gc::Handle;
use mozjs::jsapi::{CallArgs, CurrentGlobalOrNull, JSContext, JSNative, JSObject, JS_DefineFunction, JS_DefineProperty, JS_GetTwoByteStringCharsAndLength, JS_NewPlainObject, JSPROP_ENUMERATE};
use mozjs::rust::wrappers::JS_ValueToSource;

/// Set up the console object in the JavaScript context
pub fn setup_console(runtime: &mut JsRuntime) -> Result<(), String> {
    let raw_cx = unsafe { runtime.cx().raw_cx() };

    unsafe {
        rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
        if global.get().is_null() {
            return Err("No global object for console setup".to_string());
        }

        // Create console object
        rooted!(in(raw_cx) let console = JS_NewPlainObject(raw_cx));
        if console.get().is_null() {
            return Err("Failed to create console object".to_string());
        }

        // Define console.log
        define_console_method(raw_cx, console.handle().get(), "log", Some(console_log))?;

        // Define console.error
        define_console_method(raw_cx, console.handle().get(), "error", Some(console_error))?;

        // Define console.warn
        define_console_method(raw_cx, console.handle().get(), "warn", Some(console_warn))?;

        // Define console.info
        define_console_method(raw_cx, console.handle().get(), "info", Some(console_info))?;

        // Define console.debug
        define_console_method(raw_cx, console.handle().get(), "debug", Some(console_debug))?;

        // Set console on global object
        rooted!(in(raw_cx) let console_val = mozjs::jsval::ObjectValue(console.get()));
        let name = std::ffi::CString::new("console").unwrap();
        if !JS_DefineProperty(
            raw_cx,
            global.handle().into(),
            name.as_ptr(),
            console_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        ) {
            return Err("Failed to define console property".to_string());
        }
    }

    Ok(())
}

unsafe fn define_console_method(
    raw_cx: *mut JSContext,
    console: *mut JSObject,
    name: &str,
    func: JSNative,
) -> Result<(), String> {
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let console_rooted = console);

    if !JS_DefineFunction(
        raw_cx,
        console_rooted.handle().into(),
        cname.as_ptr(),
        func,
        0,
        JSPROP_ENUMERATE as u32,
    ).is_null() {
        Ok(())
    } else {
        Err(format!("Failed to define console.{}", name))
    }
}

/// Format arguments for console output
unsafe fn format_args(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> String {
    let args = CallArgs::from_vp(vp, argc);
    let mut parts = Vec::new();

    for i in 0..argc {
        rooted!(in(raw_cx) let arg = *args.get(i as u32));
        let arg_str = js_value_to_string(raw_cx, arg.handle().get());
        parts.push(arg_str);
    }

    parts.join(" ")
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

    rooted!(in(raw_cx) let str_val = unsafe { JS_ValueToSource(raw_cx, Handle::from_marked_location(&val)) });
    if str_val.get().is_null() {
        return "[object]".to_string();
    }

    let mut length = 0;
    let chars = unsafe { JS_GetTwoByteStringCharsAndLength(raw_cx, ptr::null(), *str_val.handle(), &mut length) };
    if chars.is_null() {
        return "[string conversion failed]".to_string();
    }

    let slice = unsafe { std::slice::from_raw_parts(chars, length) };
    String::from_utf16_lossy(slice)
}

/// console.log implementation
unsafe extern "C" fn console_log(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let message = unsafe { format_args(raw_cx, argc, vp) };
    println!("[JS] {}", message);

    let args = unsafe { CallArgs::from_vp(vp, argc) };
    args.rval().set(UndefinedValue());
    true
}

/// console.error implementation
unsafe extern "C" fn console_error(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let message = unsafe { format_args(raw_cx, argc, vp) };
    eprintln!("[JS Error] {}", message);

    let args = unsafe { CallArgs::from_vp(vp, argc) };
    args.rval().set(UndefinedValue());
    true
}

/// console.warn implementation
unsafe extern "C" fn console_warn(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let message = unsafe { format_args(raw_cx, argc, vp) };
    println!("[JS Warning] {}", message);

    let args = unsafe { CallArgs::from_vp(vp, argc) };
    args.rval().set(UndefinedValue());
    true
}

/// console.info implementation
unsafe extern "C" fn console_info(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let message = unsafe { format_args(raw_cx, argc, vp) };
    println!("[JS Info] {}", message);

    let args = unsafe { CallArgs::from_vp(vp, argc) };
    args.rval().set(UndefinedValue());
    true
}

/// console.debug implementation
unsafe extern "C" fn console_debug(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let message = unsafe { format_args(raw_cx, argc, vp) };
    println!("[JS Debug] {}", message);

    let args = unsafe { CallArgs::from_vp(vp, argc) };
    args.rval().set(UndefinedValue());
    true
}
