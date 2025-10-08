// Fetch API implementation for JavaScript using V8
use std::sync::{Arc, Mutex};

/// Set up the fetch API in the JavaScript context
pub fn setup_fetch(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
) -> Result<(), String> {
    // Create fetch function
    let fetch_fn = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut retval: v8::ReturnValue| {
            let url = if args.length() > 0 {
                args.get(0).to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            
            println!("[JS] fetch('{}') called (not fully implemented)", url);
            
            // Return a promise-like object for now
            // In a full implementation, this would make an actual HTTP request
            let promise_obj = v8::Object::new(scope);
            
            // Add then method
            let then_fn = v8::Function::new(
                scope,
                |scope: &mut v8::PinScope,
                 _args: v8::FunctionCallbackArguments,
                 mut retval: v8::ReturnValue| {
                    let promise_obj = v8::Object::new(scope);
                    retval.set(promise_obj.into());
                },
            ).unwrap();
            
            let key = v8::String::new(scope, "then").unwrap();
            promise_obj.set(scope, key.into(), then_fn.into());
            
            // Add catch method
            let catch_fn = v8::Function::new(
                scope,
                |scope: &mut v8::PinScope,
                 _args: v8::FunctionCallbackArguments,
                 mut retval: v8::ReturnValue| {
                    let promise_obj = v8::Object::new(scope);
                    retval.set(promise_obj.into());
                },
            ).unwrap();
            
            let key = v8::String::new(scope, "catch").unwrap();
            promise_obj.set(scope, key.into(), catch_fn.into());
            
            retval.set(promise_obj.into());
        },
    ).unwrap();
    
    // Add fetch to global
    let name = v8::String::new(scope, "fetch").unwrap();
    global.set(scope, name.into(), fetch_fn.into());
    
    Ok(())
}

/// Response object for fetch API
pub struct FetchResponse {
    body: Arc<Mutex<Option<String>>>,
    status: u16,
    status_text: String,
    ok: bool,
    url: String,
}

impl FetchResponse {
    pub fn new(body: String, status: u16, url: String) -> Self {
        let ok = status >= 200 && status < 300;
        let status_text = match status {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            301 => "Moved Permanently",
            302 => "Found",
            304 => "Not Modified",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            500 => "Internal Server Error",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            _ => "Unknown",
        }.to_string();

        Self {
            body: Arc::new(Mutex::new(Some(body))),
            status,
            status_text,
            ok,
            url,
        }
    }

    /// Create a JavaScript object representing this Response
    pub fn to_js_object(&self, scope: &mut v8::PinScope) -> Result<v8::Local<v8::Object>, String> {
        let response_obj = v8::Object::new(scope);

        // status
        let key = v8::String::new(scope, "status").unwrap();
        let val = v8::Integer::new(scope, self.status as i32);
        response_obj.set(scope, key.into(), val.into());

        // statusText
        let key = v8::String::new(scope, "statusText").unwrap();
        let val = v8::String::new(scope, &self.status_text).unwrap();
        response_obj.set(scope, key.into(), val.into());

        // ok
        let key = v8::String::new(scope, "ok").unwrap();
        let val = v8::Boolean::new(scope, self.ok);
        response_obj.set(scope, key.into(), val.into());

        // url
        let key = v8::String::new(scope, "url").unwrap();
        let val = v8::String::new(scope, &self.url).unwrap();
        response_obj.set(scope, key.into(), val.into());

        // text() method
        let body_clone = self.body.clone();
        let text_fn = v8::Function::new(
            scope,
            move |scope: &mut v8::PinScope,
                  _args: v8::FunctionCallbackArguments,
                  mut retval: v8::ReturnValue| {
                let body_guard = body_clone.lock().unwrap();
                let body_text = body_guard.as_ref().map(|s| s.clone()).unwrap_or_default();
                drop(body_guard);

                let promise_obj = v8::Object::new(scope);
                let val = v8::String::new(scope, &body_text).unwrap();

                let key = v8::String::new(scope, "[[PromiseResult]]").unwrap();
                promise_obj.set(scope, key.into(), val.into());

                retval.set(promise_obj.into());
            },
        ).unwrap();

        let key = v8::String::new(scope, "text").unwrap();
        response_obj.set(scope, key.into(), text_fn.into());

        Ok(response_obj)
    }
}

