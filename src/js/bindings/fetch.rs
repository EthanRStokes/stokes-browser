// Fetch API implementation for JavaScript using mozjs
// Provides the global fetch() function and Response object

use crate::js::bindings::dom_bindings::{DOM_REF, USER_AGENT};
use crate::js::jsapi::js_promise::JsPromiseHandle;
use crate::js::JsRuntime;
use curl::easy::{Easy, List};
use mozjs::conversions::jsstr_to_string;
use mozjs::jsapi::{
    CallArgs, HandleObject, JSContext, JSObject,
    JS_DefineFunction, JS_DefineProperty, JS_NewPlainObject, JS_NewUCStringCopyN,
    JS_ParseJSON, NewPromiseObject, RejectPromise, ResolvePromise, JSPROP_ENUMERATE,
};
use mozjs::jsval::{Int32Value, JSVal, ObjectValue, StringValue, UndefinedValue};
use mozjs::rooted;
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

    runtime.do_with_jsapi(|_rt, cx, global| unsafe {
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
unsafe extern "C" fn js_fetch(cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    // Must have at least one argument (the URL)
    if argc < 1 {
        // Create rejected promise with error
        return create_rejected_promise(cx, args.rval(), "fetch requires at least 1 argument");
    }

    // Get the URL argument
    let url_arg = args.get(0);
    if !url_arg.is_string() {
        return create_rejected_promise(cx, args.rval(), "fetch: first argument must be a string URL");
    }

    // Convert JS string to Rust string
    let url_str = {
        let js_str = url_arg.to_string();
        if js_str.is_null() {
            return create_rejected_promise(cx, args.rval(), "fetch: failed to read URL string");
        }
        jsstr_to_string(cx, NonNull::new(js_str).unwrap())
    };
    let url_str = DOM_REF.with(|dom_ref| {
        if let Some(dom) = dom_ref.borrow().as_ref() {
            let dom = &**dom;
            match dom.url.join(&url_str) {
                Ok(resolved_url) => {
                    println!("[JS] Resolved URL: {}", resolved_url);
                    resolved_url
                }
                Err(e) => {
                    panic!("[JS] URL resolution error: {} for url {url_str} on dom url {}", e, dom.url.as_str());
                }
            }
        } else {
            panic!("[JS] DOM ref not available for URL resolution");
        }
    });
    let url_str = url_str.as_str();

    // Parse request options (method, headers, body)
    let mut method = String::from("GET");
    let request_headers: HashMap<String, String> = HashMap::new();
    let mut request_body: Option<String> = None;

    if argc > 1 && args.get(1).is_object() {
        let options_obj = args.get(1).to_object();
        rooted!(in(cx) let options = options_obj);

        // Get method
        if let Some(m) = get_string_property(cx, options.handle().into(), "method") {
            method = m.to_uppercase();
        }

        // Get headers
        if let Some(headers_val) = get_property_value(cx, options.handle().into(), "headers") {
            if headers_val.is_object() {
                // TODO: Parse headers object
            }
        }

        // Get body
        if let Some(body) = get_string_property(cx, options.handle().into(), "body") {
            request_body = Some(body);
        }
    }

    // Create a Promise for the fetch operation
    let promise_ptr = match JsPromiseHandle::create_direct(cx) {
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
    let url = url_str.clone();
    let result = perform_fetch(&url, &method, &request_headers, request_body.as_deref(), &user_agent);

    // Resolve or reject the promise using the current context
    match result {
        Ok(response) => {
            // Store response for Response methods to access
            PENDING_RESPONSE.with(|pr| {
                *pr.borrow_mut() = Some(response.clone());
            });

            // Create Response object
            let response_obj = create_response_object(cx, &response);
            rooted!(in(cx) let promise = promise_ptr);
            rooted!(in(cx) let response_val = ObjectValue(response_obj));
            ResolvePromise(cx, promise.handle().into(), response_val.handle().into());
        }
        Err(err) => {
            rooted!(in(cx) let promise = promise_ptr);
            let error_utf16: Vec<u16> = err.encode_utf16().collect();
            rooted!(in(cx) let error_str = JS_NewUCStringCopyN(cx, error_utf16.as_ptr(), error_utf16.len()));
            rooted!(in(cx) let error_val = StringValue(&*error_str.get()));
            RejectPromise(cx, promise.handle().into(), error_val.handle().into());
        }
    }

    // Run any promise jobs that were queued (like .then() callbacks)
    crate::js::jsapi::promise::run_promise_jobs(cx);

    // Return the promise
    args.rval().set(ObjectValue(promise_ptr));
    true
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
unsafe fn create_response_object(cx: *mut JSContext, response: &FetchResponse) -> *mut JSObject {
    rooted!(in(cx) let obj = JS_NewPlainObject(cx));
    if obj.get().is_null() {
        return std::ptr::null_mut();
    }

    // Set status property
    let status_name = CString::new("status").unwrap();
    rooted!(in(cx) let status_val = Int32Value(response.status as i32));
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
    rooted!(in(cx) let status_text_str = JS_NewUCStringCopyN(cx, status_text_utf16.as_ptr(), status_text_utf16.len()));
    rooted!(in(cx) let status_text_val = StringValue(&*status_text_str.get()));
    JS_DefineProperty(
        cx,
        obj.handle().into(),
        status_text_name.as_ptr(),
        status_text_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Set ok property
    let ok_name = CString::new("ok").unwrap();
    rooted!(in(cx) let ok_val = mozjs::jsval::BooleanValue(response.ok));
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
    rooted!(in(cx) let url_str = JS_NewUCStringCopyN(cx, url_utf16.as_ptr(), url_utf16.len()));
    rooted!(in(cx) let url_val = StringValue(&*url_str.get()));
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
    rooted!(in(cx) let headers_obj = JS_NewPlainObject(cx));
    for (key, value) in &response.headers {
        let key_cstr = CString::new(key.as_str()).unwrap_or_else(|_| CString::new("").unwrap());
        let value_utf16: Vec<u16> = value.encode_utf16().collect();
        rooted!(in(cx) let value_str = JS_NewUCStringCopyN(cx, value_utf16.as_ptr(), value_utf16.len()));
        rooted!(in(cx) let value_val = StringValue(&*value_str.get()));
        JS_DefineProperty(
            cx,
            headers_obj.handle().into(),
            key_cstr.as_ptr(),
            value_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    let headers_name = CString::new("headers").unwrap();
    rooted!(in(cx) let headers_val = ObjectValue(headers_obj.get()));
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
unsafe extern "C" fn response_text(cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    // Get the stored response body
    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Create a resolved promise with the body text
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, HandleObject::from(null_obj.handle())));

    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    let body_utf16: Vec<u16> = body.encode_utf16().collect();
    rooted!(in(cx) let body_str = JS_NewUCStringCopyN(cx, body_utf16.as_ptr(), body_utf16.len()));
    rooted!(in(cx) let body_val = StringValue(&*body_str.get()));

    ResolvePromise(cx, promise.handle().into(), body_val.handle().into());
    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Response.json() - Returns a Promise that resolves to the body parsed as JSON
unsafe extern "C" fn response_json(cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    // Get the stored response body
    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Create a promise
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, HandleObject::from(null_obj.handle())));

    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    // Parse the JSON
    let body_utf16: Vec<u16> = body.encode_utf16().collect();
    rooted!(in(cx) let mut json_val = UndefinedValue());

    if JS_ParseJSON(cx, body_utf16.as_ptr(), body_utf16.len() as u32, json_val.handle_mut().into()) {
        ResolvePromise(cx, promise.handle().into(), json_val.handle().into());
    } else {
        // JSON parse error - reject the promise
        let error_msg = "JSON parse error";
        let error_utf16: Vec<u16> = error_msg.encode_utf16().collect();
        rooted!(in(cx) let error_str = JS_NewUCStringCopyN(cx, error_utf16.as_ptr(), error_utf16.len()));
        rooted!(in(cx) let error_val = StringValue(&*error_str.get()));
        RejectPromise(cx, promise.handle().into(), error_val.handle().into());
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Response.blob() - Returns a Promise that resolves to a Blob (simplified implementation)
unsafe extern "C" fn response_blob(cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    // Get the stored response body
    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Create a promise
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, HandleObject::from(null_obj.handle())));

    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    // Create a simple blob-like object (real Blob API would be more complex)
    rooted!(in(cx) let blob = JS_NewPlainObject(cx));

    // Set size property
    let size_name = CString::new("size").unwrap();
    rooted!(in(cx) let size_val = Int32Value(body.len() as i32));
    JS_DefineProperty(
        cx,
        blob.handle().into(),
        size_name.as_ptr(),
        size_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Set type property
    let type_name = CString::new("type").unwrap();
    let type_str = "text/plain";
    let type_utf16: Vec<u16> = type_str.encode_utf16().collect();
    rooted!(in(cx) let type_js_str = JS_NewUCStringCopyN(cx, type_utf16.as_ptr(), type_utf16.len()));
    rooted!(in(cx) let type_val = StringValue(&*type_js_str.get()));
    JS_DefineProperty(
        cx,
        blob.handle().into(),
        type_name.as_ptr(),
        type_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    rooted!(in(cx) let blob_val = ObjectValue(blob.get()));
    ResolvePromise(cx, promise.handle().into(), blob_val.handle().into());
    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Response.arrayBuffer() - Returns a Promise that resolves to an ArrayBuffer
/// Note: This is a simplified implementation that creates an empty ArrayBuffer
/// A full implementation would need to properly copy the response body data
unsafe extern "C" fn response_array_buffer(cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    // Get the stored response body
    let body = PENDING_RESPONSE.with(|pr| {
        pr.borrow().as_ref().map(|r| r.body.clone()).unwrap_or_default()
    });

    // Create a promise
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, HandleObject::from(null_obj.handle())));

    if promise.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    // Create an ArrayBuffer with the body size
    // Note: We're using NewArrayBuffer which creates an uninitialized buffer
    let body_bytes = body.as_bytes();
    rooted!(in(cx) let array_buffer = mozjs::jsapi::NewArrayBuffer(cx, body_bytes.len()));

    if !array_buffer.get().is_null() {
        rooted!(in(cx) let ab_val = ObjectValue(array_buffer.get()));
        ResolvePromise(cx, promise.handle().into(), ab_val.handle().into());
    } else {
        // Failed to create ArrayBuffer
        let error_msg = "Failed to create ArrayBuffer";
        let error_utf16: Vec<u16> = error_msg.encode_utf16().collect();
        rooted!(in(cx) let error_str = JS_NewUCStringCopyN(cx, error_utf16.as_ptr(), error_utf16.len()));
        rooted!(in(cx) let error_val = StringValue(&*error_str.get()));
        RejectPromise(cx, promise.handle().into(), error_val.handle().into());
    }

    args.rval().set(ObjectValue(promise.get()));
    true
}

/// Create a rejected promise with an error message
unsafe fn create_rejected_promise(cx: *mut JSContext, rval: mozjs::jsapi::MutableHandleValue, error_msg: &str) -> bool {
    rooted!(in(cx) let null_obj = std::ptr::null_mut::<JSObject>());
    rooted!(in(cx) let promise = NewPromiseObject(cx, HandleObject::from(null_obj.handle())));

    if promise.get().is_null() {
        rval.set(UndefinedValue());
        return false;
    }

    let error_utf16: Vec<u16> = error_msg.encode_utf16().collect();
    rooted!(in(cx) let error_str = JS_NewUCStringCopyN(cx, error_utf16.as_ptr(), error_utf16.len()));
    rooted!(in(cx) let error_val = StringValue(&*error_str.get()));

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
unsafe fn get_string_property(cx: *mut JSContext, obj: HandleObject, name: &str) -> Option<String> {
    let name_cstr = CString::new(name).ok()?;
    rooted!(in(cx) let mut val = UndefinedValue());

    if !mozjs::jsapi::JS_GetProperty(cx, obj, name_cstr.as_ptr(), val.handle_mut().into()) {
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
        Some(jsstr_to_string(cx, NonNull::new(js_str).unwrap()))
    } else {
        None
    }
}

/// Helper: Get a property value from a JS object
unsafe fn get_property_value(cx: *mut JSContext, obj: HandleObject, name: &str) -> Option<JSVal> {
    let name_cstr = CString::new(name).ok()?;
    rooted!(in(cx) let mut val = UndefinedValue());

    if !mozjs::jsapi::JS_GetProperty(cx, obj, name_cstr.as_ptr(), val.handle_mut().into()) {
        return None;
    }

    if val.is_undefined() {
        None
    } else {
        Some(*val)
    }
}

