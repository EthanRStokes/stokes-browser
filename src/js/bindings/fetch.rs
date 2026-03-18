// Fetch API implementation for JavaScript using mozjs
// Provides the global fetch() function and Response object

use crate::js::bindings::dom_bindings::{DOM_REF, USER_AGENT};
use crate::js::helpers::{js_value_to_string, ToSafeCx};
use crate::js::jsapi::js_promise::JsPromiseHandle;
use crate::js::JsRuntime;
use curl::easy::{Easy, List};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::conversions::jsstr_to_string;
use mozjs::gc::Handle;
use mozjs::jsapi::{
    CallArgs, JSContext, JSObject,
    JSPROP_ENUMERATE,
};
use mozjs::jsval::{Int32Value, JSVal, ObjectValue, StringValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers2::{JS_DefineFunction, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject, JS_NewUCStringCopyN, JS_ParseJSON, NewArrayBuffer, NewPromiseObject, RejectPromise, ResolvePromise};
use mozjs::rust::MutableHandleValue;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_uint;
use std::ptr::NonNull;
use std::time::Duration;
use url::Url;

/// Thread-local storage for the pending response data
/// This is used to pass response data between fetch and Response methods
thread_local! {
    static PENDING_RESPONSE: RefCell<Option<FetchResponse>> = RefCell::new(None);
}

/// Represents an HTTP response
#[derive(Clone, Debug)]
struct FetchResponse {
    status: u32,
    status_text: String,
    headers: HashMap<String, String>,
    body: String,
    url: String,
    ok: bool,
}

impl Default for FetchResponse {
    fn default() -> Self {
        Self {
            status: 0,
            status_text: String::new(),
            headers: HashMap::new(),
            body: String::new(),
            url: String::new(),
            ok: false,
        }
    }
}

/// Set up the fetch API in the JavaScript context
pub fn setup_fetch(runtime: &mut JsRuntime, user_agent: String) -> Result<(), String> {
    // Store user agent for fetch requests
    USER_AGENT.with(|ua| {
        *ua.borrow_mut() = user_agent;
    });

    runtime.do_with_jsapi(|cx, global| unsafe {
        // Define global fetch function
        let fetch_name = CString::new("fetch").unwrap();
        if JS_DefineFunction(
            cx,
            global.into(),
            fetch_name.as_ptr(),
            Some(js_fetch),
            1, // 1 argument (url), options is optional
            JSPROP_ENUMERATE as u32,
        ).is_null() {
            return Err("Failed to define fetch function".to_string());
        }

        Ok(())
    })
}

/// The global fetch() function implementation
/// fetch(url, options?) -> Promise<Response>
unsafe extern "C" fn js_fetch(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    // Must have at least one argument (the URL)
    if argc < 1 {
        // Create rejected promise with error
        return create_rejected_promise(safe_cx, MutableHandleValue::from_raw(args.rval().into()), "fetch requires at least 1 argument");
    }

    let input_url = match extract_fetch_input_url(safe_cx, *args.get(0)) {
        Some(v) => v,
        None => return create_rejected_promise(safe_cx, MutableHandleValue::from_raw(args.rval()), "fetch: invalid request input"),
    };

    let url = match resolve_fetch_url(&input_url) {
        Ok(v) => v,
        Err(msg) => return create_rejected_promise(safe_cx, MutableHandleValue::from_raw(args.rval()), &msg),
    };

    // Parse request options (method, headers, body)
    let mut method = String::from("GET");
    let request_headers: HashMap<String, String> = HashMap::new();
    let mut request_body: Option<String> = None;

    if argc > 1 && args.get(1).is_object() {
        let options_obj = args.get(1).to_object();
        rooted!(in(raw_cx) let options = options_obj);

        // Get method
        if let Some(m) = get_string_property(safe_cx, options.handle(), "method") {
            method = m.to_uppercase();
        }

        // Get headers
        if let Some(headers_val) = get_property_value(safe_cx, options.handle(), "headers") {
            if headers_val.is_object() {
                // FIXME: Request headers object is parsed here but never applied to the curl request.
                // Should iterate over the object's own enumerable properties and add each as an
                // HTTP header via curl's header_list.
            }
        }

        // Get body
        if let Some(body) = get_string_property(safe_cx, options.handle(), "body") {
            request_body = Some(body);
        }
    }

    // Create a Promise for the fetch operation
    let promise_ptr = match JsPromiseHandle::create_direct(safe_cx) {
        Ok((ptr, _handle)) => ptr,
        Err(e) => {
            eprintln!("fetch: failed to create promise: {}", e.message);
            args.rval().set(UndefinedValue());
            return false;
        }
    };

    // Get user agent
    let user_agent = USER_AGENT.with(|ua| ua.borrow().clone());

    // Perform the fetch synchronously (for now - could be made async later)
    let result = perform_fetch(&url, &method, &request_headers, request_body.as_deref(), &user_agent);

    // Resolve or reject the promise using the current context
    match result {
        Ok(response) => {
            // Store response for Response methods to access
            PENDING_RESPONSE.with(|pr| {
                *pr.borrow_mut() = Some(response.clone());
            });

            // Create Response object
            let response_obj = create_response_object(safe_cx, &response);
            rooted!(in(raw_cx) let promise = promise_ptr);
            rooted!(in(raw_cx) let response_val = ObjectValue(response_obj));
            ResolvePromise(safe_cx, promise.handle().into(), response_val.handle().into());
        }
        Err(err) => {
            rooted!(in(raw_cx) let promise = promise_ptr);
            let error_utf16: Vec<u16> = err.encode_utf16().collect();
            rooted!(in(raw_cx) let error_str = JS_NewUCStringCopyN(safe_cx, error_utf16.as_ptr(), error_utf16.len()));
            rooted!(in(raw_cx) let error_val = StringValue(&*error_str.get()));
            RejectPromise(safe_cx, promise.handle().into(), error_val.handle().into());
        }
    }


    // Return the promise
    args.rval().set(ObjectValue(promise_ptr));
    true
}

fn resolve_fetch_url(input: &str) -> Result<String, String> {
    DOM_REF.with(|dom_ref| {
        let dom_borrow = dom_ref.borrow();
        let dom_ptr = *dom_borrow
            .as_ref()
            .ok_or_else(|| "fetch: document base URL unavailable".to_string())?;
        if dom_ptr.is_null() {
            return Err("fetch: document base URL unavailable".to_string());
        }

        let dom = unsafe { &*dom_ptr };

        dom.url
            .join(input)
            .map(|u| u.to_string())
            .map_err(|e| format!("fetch: invalid URL: {e}"))
    })
}

unsafe fn extract_fetch_input_url(cx: &mut SafeJSContext, request_info: JSVal) -> Option<String> {
    let raw_cx = cx.raw_cx();
    if request_info.is_undefined() || request_info.is_null() {
        return None;
    }

    if request_info.is_string() {
        let js_str = request_info.to_string();
        if js_str.is_null() {
            return None;
        }
        return Some(jsstr_to_string(raw_cx, NonNull::new(js_str).unwrap()));
    }

    if request_info.is_object() {
        rooted!(in(raw_cx) let obj = request_info.to_object());
        // Request and URL objects expose a string `url`/`href` we can consume.
        if let Some(url) = get_string_property(cx, obj.handle().into(), "url") {
            return Some(url);
        }
        if let Some(href) = get_string_property(cx, obj.handle().into(), "href") {
            return Some(href);
        }
    }

    Some(js_value_to_string(cx, request_info))
}

/// Perform the actual HTTP fetch operation
fn perform_fetch(
    url: &str,
    method: &str,
    headers: &HashMap<String, String>,
    body: Option<&str>,
    user_agent: &str,
) -> Result<FetchResponse, String> {
    // Parse URL
    let parsed_url = Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;

    // Handle file:// URLs
    if parsed_url.scheme() == "file" {
        let path = parsed_url.to_file_path()
            .map_err(|_| "Invalid file URL")?;

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read file: {}", e))?;

        return Ok(FetchResponse {
            status: 200,
            status_text: "OK".to_string(),
            headers: HashMap::new(),
            body: content,
            url: url.to_string(),
            ok: true,
        });
    }

    // Perform HTTP request using curl
    let mut easy = Easy::new();
    let mut response_data = Vec::new();
    let mut response_headers: HashMap<String, String> = HashMap::new();

    // Configure curl
    easy.url(url).map_err(|e| format!("Curl error: {}", e))?;
    easy.useragent(user_agent).map_err(|e| format!("Curl error: {}", e))?;
    easy.timeout(Duration::from_secs(30)).map_err(|e| format!("Curl error: {}", e))?;
    easy.follow_location(true).map_err(|e| format!("Curl error: {}", e))?;
    easy.max_redirections(5).map_err(|e| format!("Curl error: {}", e))?;

    // Set method
    match method {
        "GET" => {},
        "POST" => {
            easy.post(true).map_err(|e| format!("Curl error: {}", e))?;
        },
        "PUT" => {
            easy.put(true).map_err(|e| format!("Curl error: {}", e))?;
        },
        "DELETE" => {
            easy.custom_request("DELETE").map_err(|e| format!("Curl error: {}", e))?;
        },
        "HEAD" => {
            easy.nobody(true).map_err(|e| format!("Curl error: {}", e))?;
        },
        "OPTIONS" => {
            easy.custom_request("OPTIONS").map_err(|e| format!("Curl error: {}", e))?;
        },
        "PATCH" => {
            easy.custom_request("PATCH").map_err(|e| format!("Curl error: {}", e))?;
        },
        _ => {
            easy.custom_request(method).map_err(|e| format!("Curl error: {}", e))?;
        }
    }

    // Set request headers
    let mut header_list = List::new();
    for (key, value) in headers {
        let header = format!("{}: {}", key, value);
        header_list.append(&header).map_err(|e| format!("Curl error: {}", e))?;
    }
    easy.http_headers(header_list).map_err(|e| format!("Curl error: {}", e))?;

    // Set request body
    if let Some(body_data) = body {
        let body_bytes = body_data.as_bytes().to_vec();
        easy.post_field_size(body_bytes.len() as u64).map_err(|e| format!("Curl error: {}", e))?;
    }

    // Perform the transfer
    {
        let mut transfer = easy.transfer();

        transfer.write_function(|data| {
            response_data.extend_from_slice(data);
            Ok(data.len())
        }).map_err(|e| format!("Curl error: {}", e))?;

        transfer.header_function(|header| {
            let header_str = String::from_utf8_lossy(header).trim().to_string();
            if let Some(colon_pos) = header_str.find(':') {
                let key = header_str[..colon_pos].trim().to_lowercase();
                let value = header_str[colon_pos + 1..].trim().to_string();
                response_headers.insert(key, value);
            }
            true
        }).map_err(|e| format!("Curl error: {}", e))?;

        transfer.perform().map_err(|e| format!("Network error: {}", e))?;
    }

    // Get response status
    let status = easy.response_code().map_err(|e| format!("Curl error: {}", e))? as u32;
    let status_text = get_status_text(status);

    // Convert body to string
    let body = String::from_utf8(response_data)
        .unwrap_or_else(|e| format!("[Binary data: {} bytes]", e.as_bytes().len()));

    Ok(FetchResponse {
        status,
        status_text,
        headers: response_headers,
        body,
        url: url.to_string(),
        ok: status >= 200 && status < 300,
    })
}

/// Create a Response object from FetchResponse
unsafe fn create_response_object(cx: &mut SafeJSContext, response: &FetchResponse) -> *mut JSObject {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let obj = JS_NewPlainObject(cx));
    if obj.get().is_null() {
        return std::ptr::null_mut();
    }

    // Set status property
    let status_name = CString::new("status").unwrap();
    rooted!(in(raw_cx) let status_val = Int32Value(response.status as i32));
    JS_DefineProperty(
        cx,
        obj.handle().into(),
        status_name.as_ptr(),
        status_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Set statusText property
    let status_text_name = CString::new("statusText").unwrap();
    let status_text_utf16: Vec<u16> = response.status_text.encode_utf16().collect();
    rooted!(in(raw_cx) let status_text_str = JS_NewUCStringCopyN(cx, status_text_utf16.as_ptr(), status_text_utf16.len()));
    rooted!(in(raw_cx) let status_text_val = StringValue(&*status_text_str.get()));
    JS_DefineProperty(
        cx,
        obj.handle().into(),
        status_text_name.as_ptr(),
        status_text_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Set ok property
    let ok_name = CString::new("ok").unwrap();
    rooted!(in(raw_cx) let ok_val = mozjs::jsval::BooleanValue(response.ok));
    JS_DefineProperty(
        cx,
        obj.handle().into(),
        ok_name.as_ptr(),
        ok_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Set url property
    let url_name = CString::new("url").unwrap();
    let url_utf16: Vec<u16> = response.url.encode_utf16().collect();
    rooted!(in(raw_cx) let url_str = JS_NewUCStringCopyN(cx, url_utf16.as_ptr(), url_utf16.len()));
    rooted!(in(raw_cx) let url_val = StringValue(&*url_str.get()));
    JS_DefineProperty(
        cx,
        obj.handle().into(),
        url_name.as_ptr(),
        url_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Define text() method - returns Promise<string>
    let text_name = CString::new("text").unwrap();
    JS_DefineFunction(
        cx,
        obj.handle().into(),
        text_name.as_ptr(),
        Some(response_text),
        0,
        JSPROP_ENUMERATE as u32,
    );

    // Define json() method - returns Promise<any>
    let json_name = CString::new("json").unwrap();
    JS_DefineFunction(
        cx,
        obj.handle().into(),
        json_name.as_ptr(),
        Some(response_json),
        0,
        JSPROP_ENUMERATE as u32,
    );

    // Define blob() method - returns Promise<Blob> (simplified)
    let blob_name = CString::new("blob").unwrap();
    JS_DefineFunction(
        cx,
        obj.handle().into(),
        blob_name.as_ptr(),
        Some(response_blob),
        0,
        JSPROP_ENUMERATE as u32,
    );

    // Define arrayBuffer() method - returns Promise<ArrayBuffer>
    let array_buffer_name = CString::new("arrayBuffer").unwrap();
    JS_DefineFunction(
        cx,
        obj.handle().into(),
        array_buffer_name.as_ptr(),
        Some(response_array_buffer),
        0,
        JSPROP_ENUMERATE as u32,
    );

    // Create headers object
    rooted!(in(raw_cx) let headers_obj = JS_NewPlainObject(cx));
    for (key, value) in &response.headers {
        let key_cstr = CString::new(key.as_str()).unwrap_or_else(|_| CString::new("").unwrap());
        let value_utf16: Vec<u16> = value.encode_utf16().collect();
        rooted!(in(raw_cx) let value_str = JS_NewUCStringCopyN(cx, value_utf16.as_ptr(), value_utf16.len()));
        rooted!(in(raw_cx) let value_val = StringValue(&*value_str.get()));
        JS_DefineProperty(
            cx,
            headers_obj.handle().into(),
            key_cstr.as_ptr(),
            value_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    let headers_name = CString::new("headers").unwrap();
    rooted!(in(raw_cx) let headers_val = ObjectValue(headers_obj.get()));
    JS_DefineProperty(
        cx,
        obj.handle().into(),
        headers_name.as_ptr(),
        headers_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    obj.get()
}

/// Response.text() - Returns a Promise that resolves to the body as text
unsafe extern "C" fn response_text(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    // Get the stored response body
    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Create a resolved promise with the body text
    rooted!(in(raw_cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(raw_cx) let promise = NewPromiseObject(safe_cx, null_obj.handle()));

    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    let body_utf16: Vec<u16> = body.encode_utf16().collect();
    rooted!(in(raw_cx) let body_str = JS_NewUCStringCopyN(safe_cx, body_utf16.as_ptr(), body_utf16.len()));
    rooted!(in(raw_cx) let body_val = StringValue(&*body_str.get()));

    ResolvePromise(safe_cx, promise.handle().into(), body_val.handle().into());
    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Response.json() - Returns a Promise that resolves to the body parsed as JSON
unsafe extern "C" fn response_json(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    // Get the stored response body
    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Create a promise
    rooted!(in(raw_cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(raw_cx) let promise = NewPromiseObject(safe_cx, null_obj.handle()));

    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    // Parse the JSON
    let body_utf16: Vec<u16> = body.encode_utf16().collect();
    rooted!(in(raw_cx) let mut json_val = UndefinedValue());

    if JS_ParseJSON(safe_cx, body_utf16.as_ptr(), body_utf16.len() as u32, json_val.handle_mut().into()) {
        ResolvePromise(safe_cx, promise.handle().into(), json_val.handle().into());
    } else {
        // JSON parse error - reject the promise
        let error_msg = "JSON parse error";
        let error_utf16: Vec<u16> = error_msg.encode_utf16().collect();
        rooted!(in(raw_cx) let error_str = JS_NewUCStringCopyN(safe_cx, error_utf16.as_ptr(), error_utf16.len()));
        rooted!(in(raw_cx) let error_val = StringValue(&*error_str.get()));
        RejectPromise(safe_cx, promise.handle().into(), error_val.handle().into());
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Response.blob() - Returns a Promise that resolves to a Blob (simplified implementation)
// FIXME: This implementation consumes the global PENDING_RESPONSE store (taking the value out),
// which means calling .blob() after .text() or .json() on the same Response will return empty
// data.  A proper implementation should give each Response its own body store that can be
// consumed only once (per the ReadableStream / Body mixin spec).
unsafe extern "C" fn response_blob(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let response = PENDING_RESPONSE.with(|pr| {
        pr.borrow_mut().take().unwrap_or_default()
    });

    let headers = response.headers;
    let body = &response.body;

    // Create a promise
    rooted!(in(raw_cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(raw_cx) let promise = NewPromiseObject(safe_cx, null_obj.handle()));

    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    // Create a simple blob-like object (real Blob API would be more complex)
    rooted!(in(raw_cx) let blob = JS_NewPlainObject(safe_cx));

    // Set size property
    let size_name = CString::new("size").unwrap();
    rooted!(in(raw_cx) let size_val = Int32Value(body.len() as i32));
    JS_DefineProperty(
        safe_cx,
        blob.handle().into(),
        size_name.as_ptr(),
        size_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Set type from Content-Type header, stripping parameters (e.g. "; charset=utf-8")
    let content_type = headers
        .get("content-type")
        .and_then(|ct| ct.split(';').next())
        .map(|ct| ct.trim().to_string())
        .unwrap_or_else(|| "text/plain".to_string());
    let type_name = CString::new("type").unwrap();
    let type_utf16: Vec<u16> = content_type.encode_utf16().collect();
    rooted!(in(raw_cx) let type_js_str = JS_NewUCStringCopyN(safe_cx, type_utf16.as_ptr(), type_utf16.len()));
    rooted!(in(raw_cx) let type_val = StringValue(&*type_js_str.get()));
    JS_DefineProperty(
        safe_cx,
        blob.handle().into(),
        type_name.as_ptr(),
        type_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    rooted!(in(raw_cx) let blob_val = ObjectValue(blob.get()));
    ResolvePromise(safe_cx, promise.handle().into(), blob_val.handle().into());
    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Response.arrayBuffer() - Returns a Promise that resolves to an ArrayBuffer
/// Note: This is a simplified implementation that creates an empty ArrayBuffer
/// A full implementation would need to properly copy the response body data
unsafe extern "C" fn response_array_buffer(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    // Get the stored response body
    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Create a promise
    rooted!(in(raw_cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(raw_cx) let promise = NewPromiseObject(safe_cx, null_obj.handle()));

    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    // Create an ArrayBuffer with the body size
    // Note: We're using NewArrayBuffer which creates an uninitialized buffer
    // FIXME: The ArrayBuffer is allocated with the correct size but the body bytes are never
    // copied into it. Use JS_GetArrayBufferData (or similar) to obtain a mutable pointer to the
    // buffer's backing store and write body_bytes into it so callers receive actual response data.
    let body_bytes = body.as_bytes();
    rooted!(in(raw_cx) let array_buffer = NewArrayBuffer(safe_cx, body_bytes.len()));

    if !array_buffer.get().is_null() {
        rooted!(in(raw_cx) let ab_val = ObjectValue(array_buffer.get()));
        ResolvePromise(safe_cx, promise.handle().into(), ab_val.handle().into());
    } else {
        // Failed to create ArrayBuffer
        let error_msg = "Failed to create ArrayBuffer";
        let error_utf16: Vec<u16> = error_msg.encode_utf16().collect();
        rooted!(in(raw_cx) let error_str = JS_NewUCStringCopyN(safe_cx, error_utf16.as_ptr(), error_utf16.len()));
        rooted!(in(raw_cx) let error_val = StringValue(&*error_str.get()));
        RejectPromise(safe_cx, promise.handle().into(), error_val.handle().into());
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Create a rejected promise with an error message
unsafe fn create_rejected_promise(cx: &mut SafeJSContext, mut rval: MutableHandleValue, error_msg: &str) -> bool {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(raw_cx) let promise = NewPromiseObject(cx, null_obj.handle()));

    if promise.get().is_null() {
        rval.set(UndefinedValue());
        return false;
    }

    let error_utf16: Vec<u16> = error_msg.encode_utf16().collect();
    rooted!(in(raw_cx) let error_str = JS_NewUCStringCopyN(cx, error_utf16.as_ptr(), error_utf16.len()));
    rooted!(in(raw_cx) let error_val = StringValue(&*error_str.get()));

    RejectPromise(cx, promise.handle().into(), error_val.handle().into());
    rval.set(ObjectValue(promise.get()));
    true
}

/// Get HTTP status text from status code
fn get_status_text(status: u32) -> String {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }.to_string()
}

/// Helper: Get a string property from a JS object
unsafe fn get_string_property(cx: &mut SafeJSContext, obj: Handle<*mut JSObject>, name: &str) -> Option<String> {
    let raw_cx = cx.raw_cx();
    let name_cstr = CString::new(name).ok()?;
    rooted!(in(raw_cx) let mut val = UndefinedValue());

    if !JS_GetProperty(cx, obj, name_cstr.as_ptr(), val.handle_mut().into()) {
        return None;
    }

    if val.is_undefined() || val.is_null() {
        return None;
    }

    if val.is_string() {
        let js_str = val.to_string();
        if js_str.is_null() {
            return None;
        }
        Some(jsstr_to_string(raw_cx, NonNull::new(js_str).unwrap()))
    } else {
        None
    }
}

/// Helper: Get a property value from a JS object
unsafe fn get_property_value(cx: &mut SafeJSContext, obj: Handle<*mut JSObject>, name: &str) -> Option<JSVal> {
    let raw_cx = cx.raw_cx();
    let name_cstr = CString::new(name).ok()?;
    rooted!(in(raw_cx) let mut val = UndefinedValue());

    if !JS_GetProperty(cx, obj, name_cstr.as_ptr(), val.handle_mut().into()) {
        return None;
    }

    if val.is_undefined() {
        None
    } else {
        Some(*val)
    }
}

