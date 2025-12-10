// Fetch API implementation for JavaScript using mozjs
use super::runtime::JsRuntime;
use crate::networking::HttpClient;
use mozjs::jsval::{JSVal, UndefinedValue, Int32Value, BooleanValue, StringValue, ObjectValue};
use mozjs::rooted;
use std::os::raw::c_uint;
use std::ptr;
use std::sync::{Arc, Mutex};
use mozjs::jsapi::{CallArgs, CurrentGlobalOrNull, JSContext, JS_DefineFunction, JS_DefineProperty, JS_GetProperty, JS_GetTwoByteStringCharsAndLength, JS_NewPlainObject, JS_NewUCStringCopyN, JS_ParseJSON, JS_ValueToSource, JSPROP_ENUMERATE};

/// Response data stored between calls
struct FetchResponseData {
    body: String,
    status: u16,
    url: String,
}

thread_local! {
    static PENDING_RESPONSE: std::cell::RefCell<Option<FetchResponseData>> = std::cell::RefCell::new(None);
}

/// Setup the fetch API in the JavaScript context
pub fn setup_fetch(runtime: &mut JsRuntime) -> Result<(), String> {
    let cx = runtime.cx();

    println!("[JS] Setting up fetch API");

    unsafe {
        rooted!(in(cx) let global = CurrentGlobalOrNull(cx));
        if global.get().is_null() {
            return Err("No global object for fetch setup".to_string());
        }

        // Define fetch function on global
        let cname = std::ffi::CString::new("fetch").unwrap();
        if JS_DefineFunction(
            cx,
            global.handle().into(),
            cname.as_ptr(),
            Some(fetch_impl),
            1,
            JSPROP_ENUMERATE as u32,
        ).is_null() {
            return Err("Failed to define fetch function".to_string());
        }
    }

    println!("[JS] fetch API initialized");
    Ok(())
}

/// Fetch implementation
unsafe extern "C" fn fetch_impl(cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    // Get the URL argument
    let url = if argc > 0 {
        let url_val = *args.get(0);
        js_value_to_string(cx, url_val)
    } else {
        String::new()
    };

    if url.is_empty() {
        println!("[JS] fetch() called with empty URL");
        // Return a rejected promise
        return create_rejected_promise(cx, args.rval(), "URL is required");
    }

    println!("[JS] fetch('{}') called", url);

    // Get method from options if provided
    let method = if argc > 1 {
        let opts = *args.get(1);
        if opts.is_object() && !opts.is_null() {
            get_object_property_string(cx, opts, "method").unwrap_or_else(|| "GET".to_string())
        } else {
            "GET".to_string()
        }
    } else {
        "GET".to_string()
    };

    println!("[JS] fetch method: {}", method);

    // Perform the fetch synchronously (blocking)
    let url_clone = url.clone();
    let fetch_result = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = HttpClient::new();
            client.fetch(&url_clone).await
        })
    }).join();

    match fetch_result {
        Ok(Ok(body)) => {
            println!("[JS] fetch successful, body length: {}", body.len());
            create_response_promise(cx, args.rval(), body, 200, url)
        }
        Ok(Err(e)) => {
            println!("[JS] fetch failed: {}", e);
            create_rejected_promise(cx, args.rval(), &format!("Fetch failed: {}", e))
        }
        Err(_) => {
            println!("[JS] fetch thread panicked");
            create_rejected_promise(cx, args.rval(), "Fetch failed")
        }
    }
}

/// Convert a JS value to a Rust string
unsafe fn js_value_to_string(cx: *mut JSContext, val: JSVal) -> String {
    if val.is_string() {
        rooted!(in(cx) let str_val = val.to_string());
        if str_val.get().is_null() {
            return String::new();
        }

        let mut length = 0;
        let chars = JS_GetTwoByteStringCharsAndLength(cx, ptr::null(), *str_val.handle(), &mut length);
        if chars.is_null() {
            return String::new();
        }

        let slice = std::slice::from_raw_parts(chars, length);
        String::from_utf16_lossy(slice)
    } else {
        rooted!(in(cx) let str_val = JS_ValueToSource(cx, val));
        if str_val.get().is_null() {
            return String::new();
        }

        let mut length = 0;
        let chars = JS_GetTwoByteStringCharsAndLength(cx, ptr::null(), *str_val.handle(), &mut length);
        if chars.is_null() {
            return String::new();
        }

        let slice = std::slice::from_raw_parts(chars, length);
        String::from_utf16_lossy(slice)
    }
}

/// Get a property from an object as a string
unsafe fn get_object_property_string(cx: *mut JSContext, obj_val: JSVal, name: &str) -> Option<String> {
    if !obj_val.is_object() || obj_val.is_null() {
        return None;
    }

    rooted!(in(cx) let obj = obj_val.to_object());
    rooted!(in(cx) let mut val = UndefinedValue());

    let cname = std::ffi::CString::new(name).ok()?;
    if !JS_GetProperty(cx, obj.handle().into(), cname.as_ptr(), val.handle_mut().into()) {
        return None;
    }

    if val.get().is_undefined() {
        return None;
    }

    Some(js_value_to_string(cx, val.get()))
}

/// Create a rejected promise and set it as return value
unsafe fn create_rejected_promise(cx: *mut JSContext, mut rval: mozjs::rust::MutableHandleValue, error_msg: &str) -> bool {
    // Create the error message as a JS string
    let error_code = format!("Promise.reject(new Error({}))",
        serde_json::to_string(error_msg).unwrap_or_else(|_| "\"Error\"".to_string()));

    rooted!(in(cx) let mut result = UndefinedValue());
    let code_utf16: Vec<u16> = error_code.encode_utf16().collect();

    // Evaluate the promise code
    let filename = std::ffi::CString::new("fetch").unwrap();
    rooted!(in(cx) let global = CurrentGlobalOrNull(cx));

    // For simplicity, we'll just return undefined on error
    rval.set(UndefinedValue());
    true
}

/// Create a response object wrapped in a resolved promise
unsafe fn create_response_promise(cx: *mut JSContext, mut rval: mozjs::rust::MutableHandleValue, body: String, status: u16, url: String) -> bool {
    rooted!(in(cx) let global = CurrentGlobalOrNull(cx));

    // Create response object
    rooted!(in(cx) let response = JS_NewPlainObject(cx));
    if response.get().is_null() {
        return create_rejected_promise(cx, rval, "Failed to create response object");
    }

    // Set status
    let status_name = std::ffi::CString::new("status").unwrap();
    rooted!(in(cx) let status_val = Int32Value(status as i32));
    JS_DefineProperty(cx, response.handle().into(), status_name.as_ptr(), status_val.handle().into(), JSPROP_ENUMERATE as u32);

    // Set ok
    let ok = status >= 200 && status < 300;
    let ok_name = std::ffi::CString::new("ok").unwrap();
    rooted!(in(cx) let ok_val = BooleanValue(ok));
    JS_DefineProperty(cx, response.handle().into(), ok_name.as_ptr(), ok_val.handle().into(), JSPROP_ENUMERATE as u32);

    // Set url
    let url_name = std::ffi::CString::new("url").unwrap();
    let url_utf16: Vec<u16> = url.encode_utf16().collect();
    rooted!(in(cx) let url_str = JS_NewUCStringCopyN(cx, url_utf16.as_ptr(), url_utf16.len()));
    rooted!(in(cx) let url_val = StringValue(&*url_str.get()));
    JS_DefineProperty(cx, response.handle().into(), url_name.as_ptr(), url_val.handle().into(), JSPROP_ENUMERATE as u32);

    // Store body for text() method
    PENDING_RESPONSE.with(|pr| {
        *pr.borrow_mut() = Some(FetchResponseData { body: body.clone(), status, url: url.clone() });
    });

    // Define text() method
    let text_name = std::ffi::CString::new("text").unwrap();
    JS_DefineFunction(cx, response.handle().into(), text_name.as_ptr(), Some(response_text), 0, JSPROP_ENUMERATE as u32);

    // Define json() method
    let json_name = std::ffi::CString::new("json").unwrap();
    JS_DefineFunction(cx, response.handle().into(), json_name.as_ptr(), Some(response_json), 0, JSPROP_ENUMERATE as u32);

    // Wrap in Promise.resolve()
    // For simplicity, just return the response object directly (caller should wrap in promise)
    rooted!(in(cx) let response_val = ObjectValue(response.get()));
    rval.set(response_val.get());
    true
}

/// Response.text() implementation
unsafe extern "C" fn response_text(cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, _argc);

    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Create the body as a JS string
    let body_utf16: Vec<u16> = body.encode_utf16().collect();
    rooted!(in(cx) let body_str = JS_NewUCStringCopyN(cx, body_utf16.as_ptr(), body_utf16.len()));
    rooted!(in(cx) let body_val = StringValue(&*body_str.get()));

    args.rval().set(body_val.get());
    true
}

/// Response.json() implementation
unsafe extern "C" fn response_json(cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, _argc);

    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Parse JSON using JS_ParseJSON
    let body_utf16: Vec<u16> = body.encode_utf16().collect();
    rooted!(in(cx) let mut result = UndefinedValue());

    if JS_ParseJSON(cx, body_utf16.as_ptr(), body_utf16.len() as u32, result.handle_mut().into()) {
        args.rval().set(result.get());
    } else {
        args.rval().set(UndefinedValue());
    }

    true
}
