// Fetch API implementation for JavaScript using mozjs
use crate::networking::{HttpClient, NetworkError};
use mozjs::conversions::jsstr_to_string;
use mozjs::jsapi::{CallArgs, CurrentGlobalOrNull, Handle, JSContext, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject, JS_NewUCStringCopyN, JS_ParseJSON, JS_ValueToSource, MutableHandleValue, JSPROP_ENUMERATE, Compile1, JS_ExecuteScript, JSObject, NewPromiseObject, ResolvePromise, RejectPromise, HandleObject, JS_DefineFunction};
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, ObjectValue, StringValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::{CompileOptionsWrapper, transform_str_to_source_text};
use std::os::raw::c_uint;
use std::ptr::NonNull;
use crate::js::JsRuntime;
use mozjs::context::JSContext as SafeJSContext;
use crate::engine::USER_AGENT_REF;
use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::jsapi::promise::resolve_promise;

/// Helper function to evaluate JavaScript code and return the result
unsafe fn eval_js(cx: *mut JSContext, code: &str, rval: MutableHandleValue) -> bool {
    rooted!(in(cx) let global = CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return false;
    }

    // Create a safe JSContext wrapper for the compile options
    // SAFETY: We're within a valid JSContext scope, and the raw pointer is valid
    let safe_cx: &SafeJSContext = std::mem::transmute(&cx);
    let options = CompileOptionsWrapper::new(safe_cx, "fetch_eval".parse().unwrap(), 1);

    // Compile the script
    let script = Compile1(cx, options.ptr, &mut transform_str_to_source_text(code));
    if script.is_null() {
        return false;
    }

    rooted!(in(cx) let script_root = script);

    // Execute the script
    JS_ExecuteScript(cx, Handle::from(script_root.handle()), rval)
}

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
    println!("[JS] Setting up fetch API");

    runtime.add_global_function("fetch", |cx, args| {
        unsafe {
            let argc = args.argc_;

            // Get the URL argument
            let url = if argc > 0 {
                let url_val = *args.get(0);
                js_value_to_string(cx, url_val)
            } else {
                String::new()
            };

            let url = DOM_REF.with(|dom_ref| {
                if let Some(dom) = dom_ref.borrow().as_ref() {
                    let dom = &**dom;
                    match dom.url.join(&url) {
                        Ok(resolved_url) => {
                            println!("[JS] Resolved URL: {}", resolved_url);
                            resolved_url
                        }
                        Err(e) => {
                            panic!("[JS] URL resolution error: {} for url {url} on dom url {}", e, dom.url.as_str());
                        }
                    }
                } else {
                    panic!("[JS] DOM ref not available for URL resolution");
                }
            });

            if url.as_str().is_empty() {
                println!("[JS] fetch() called with empty URL");
                // Return a rejected promise
                return create_rejected_promise(cx, args.rval().into(), "URL is required");
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
            let fetch_result = USER_AGENT_REF.with(|user_agent_ref| {
                if let Some(user_agent) = user_agent_ref.borrow().as_ref() {
                    let client = HttpClient::new();
                    client.fetch(&url.as_str(), user_agent)
                } else {
                    Err(NetworkError::Engine("User agent ref not available".to_string()))
                }
            });

            match fetch_result {
                Ok(body) => {
                    println!("[JS] fetch successful, body length: {}", body.len());
                    create_response_promise(cx, args.rval(), body, 200, url.to_string())
                }
                Err(e) => {
                    println!("[JS] fetch failed: {}", e);
                    create_rejected_promise(cx, args.rval(), &format!("Fetch failed: {}", e))
                }
            }
        }
    });

    println!("[JS] fetch API initialized");
    Ok(())
}

/// Convert a JS value to a Rust string
unsafe fn js_value_to_string(cx: *mut JSContext, val: JSVal) -> String {
    if val.is_string() {
        rooted!(in(cx) let str_val = val.to_string());
        if str_val.get().is_null() {
            return String::new();
        }

        jsstr_to_string(cx, NonNull::new(str_val.handle().get()).unwrap())
    } else {
        rooted!(in(cx) let str_val = JS_ValueToSource(cx, Handle::from_marked_location(&val)));
        if str_val.get().is_null() {
            return String::new();
        }

        jsstr_to_string(cx, NonNull::new(str_val.handle().get()).unwrap())
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
unsafe fn create_rejected_promise(cx: *mut JSContext, mut rval: MutableHandleValue, error_msg: &str) -> bool {
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, HandleObject::from(null_obj.handle())));

    if promise.get().is_null() {
        println!("[JS] Failed to create promise object");
        rval.set(UndefinedValue());
        return false;
    }

    // Create error string
    // TODO: Construct a real Error object instead of just a string
    let error_utf16: Vec<u16> = error_msg.encode_utf16().collect();
    rooted!(in(cx) let str_obj = JS_NewUCStringCopyN(cx, error_utf16.as_ptr(), error_utf16.len()));
    rooted!(in(cx) let reason = StringValue(&*str_obj.get()));

    // Reject promise
    RejectPromise(cx, promise.handle().into(), reason.handle().into());

    rval.set(ObjectValue(promise.get()));
    true
}

/// Create a response object wrapped in a resolved promise
unsafe fn create_response_promise(cx: *mut JSContext, mut rval: MutableHandleValue, body: String, status: u16, url: String) -> bool {
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
    PENDING_RESPONSE.set(Some(FetchResponseData { body: body.clone(), status, url: url.clone() }));

    // Define text() method
    let text_name = std::ffi::CString::new("text").unwrap();
    JS_DefineFunction(cx, response.handle().into(), text_name.as_ptr(), Some(response_text), 0, JSPROP_ENUMERATE as u32);

    // Define json() method
    let json_name = std::ffi::CString::new("json").unwrap();
    JS_DefineFunction(cx, response.handle().into(), json_name.as_ptr(), Some(response_json), 0, JSPROP_ENUMERATE as u32);

    // Create Promise
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, null_obj.handle().into()));

    if promise.get().is_null() {
        // Fallback: return the raw response object if Promise creation failed
        println!("ERROR: Promise creation failed");
        rval.set(ObjectValue(response.get()));
        return true;
    }

    rooted!(in(cx) let response_val = ObjectValue(response.get()));
    match resolve_promise(cx, promise.handle().into(), response_val.handle().into()) {
        Ok(_) => (),
        Err(e) => {
            println!("ERROR: Failed to resolve promise: {:?}", e);
            rval.set(ObjectValue(response.get()));
            return true;
        }
    }

    rval.set(ObjectValue(promise.get()));
    true
}

/// Response.text() implementation - returns a Promise that resolves to the body text
unsafe extern "C" fn response_text(cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, _argc);

    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    let body_utf16: Vec<u16> = body.encode_utf16().collect();
    rooted!(in(cx) let body_str = JS_NewUCStringCopyN(cx, body_utf16.as_ptr(), body_utf16.len()));
    rooted!(in(cx) let body_val = StringValue(&*body_str.get()));

    // Create Promise
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, null_obj.handle().into()));

    if promise.get().is_null() {
        // Fallback: return the raw string if Promise creation failed
        args.rval().set(body_val.get());
        return true;
    }

    ResolvePromise(cx, promise.handle().into(), body_val.handle().into());

    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Response.json() implementation - returns a Promise that resolves to the parsed JSON
unsafe extern "C" fn response_json(cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, _argc);

    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Parse JSON using JS_ParseJSON
    let body_utf16: Vec<u16> = body.encode_utf16().collect();
    rooted!(in(cx) let mut result = UndefinedValue());

    // Create Promise
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, null_obj.handle().into()));

    if promise.get().is_null() {
        // Fallback if Promise creation failed
         if JS_ParseJSON(cx, body_utf16.as_ptr(), body_utf16.len() as u32, result.handle_mut().into()) {
            args.rval().set(result.get());
         } else {
            args.rval().set(UndefinedValue());
         }
        return true;
    }

    if JS_ParseJSON(cx, body_utf16.as_ptr(), body_utf16.len() as u32, result.handle_mut().into()) {
        ResolvePromise(cx, promise.handle().into(), result.handle().into());
    } else {
        let error_msg = "JSON parse error";
        let error_utf16: Vec<u16> = error_msg.encode_utf16().collect();
        rooted!(in(cx) let str_obj = JS_NewUCStringCopyN(cx, error_utf16.as_ptr(), error_utf16.len()));
        rooted!(in(cx) let reason = StringValue(&*str_obj.get()));

        RejectPromise(cx, promise.handle().into(), reason.handle().into());
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}
