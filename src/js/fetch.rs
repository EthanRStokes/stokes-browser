// Fetch API implementation for JavaScript
use boa_engine::{Context, JsResult as BoaResult, JsValue, NativeFunction, object::ObjectInitializer, JsString, property::Attribute};
use boa_gc::{Finalize, Trace};
use std::sync::{Arc, Mutex};
use crate::networking::HttpClient;

/// Response object wrapper for fetch API
#[derive(Debug, Clone, Trace, Finalize)]
struct FetchResponse {
    #[unsafe_ignore_trace]
    body: Arc<Mutex<Option<String>>>,
    #[unsafe_ignore_trace]
    status: u16,
    #[unsafe_ignore_trace]
    status_text: String,
    #[unsafe_ignore_trace]
    ok: bool,
    #[unsafe_ignore_trace]
    url: String,
}

impl FetchResponse {
    fn new(body: String, status: u16, url: String) -> Self {
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
    fn to_js_object(&self, context: &mut Context) -> BoaResult<JsValue> {
        let response_clone = self.clone();
        let text_fn = unsafe {
            NativeFunction::from_closure(move |_this: &JsValue, _args: &[JsValue], context: &mut Context| {
                // Get the body text
                let body_guard = response_clone.body.lock().unwrap();
                let body_text = body_guard.as_ref().map(|s| s.clone()).unwrap_or_default();
                drop(body_guard);

                // Create a resolved promise with the text
                let promise = context.eval(boa_engine::Source::from_bytes("Promise.resolve('')"))?;
                if let Some(promise_obj) = promise.as_object() {
                    promise_obj.set(JsString::from("[[PromiseResult]]"), JsValue::from(JsString::from(body_text.clone())), false, context)?;
                }

                // For now, just return the text directly wrapped in a resolved promise
                // This is a simplification - ideally we'd return an actual Promise
                let promise_code = format!("Promise.resolve({})", serde_json::to_string(&body_text).unwrap_or_else(|_| "\"\"".to_string()));
                context.eval(boa_engine::Source::from_bytes(&promise_code))
            })
        };

        let response_clone2 = self.clone();
        let json_fn = unsafe {
            NativeFunction::from_closure(move |_this: &JsValue, _args: &[JsValue], context: &mut Context| {
                // Get the body text and parse as JSON
                let body_guard = response_clone2.body.lock().unwrap();
                let body_text = body_guard.as_ref().map(|s| s.clone()).unwrap_or_default();
                drop(body_guard);

                // Try to parse the JSON
                let _parse_result = context.eval(boa_engine::Source::from_bytes(&format!("JSON.parse({})", serde_json::to_string(&body_text).unwrap_or_else(|_| "\"{}\"".to_string()))));

                // Return a resolved promise with the parsed JSON
                let promise_code = format!("Promise.resolve(JSON.parse({}))", serde_json::to_string(&body_text).unwrap_or_else(|_| "\"{}\"".to_string()));
                context.eval(boa_engine::Source::from_bytes(&promise_code))
            })
        };

        let _response_clone3 = self.clone();
        let array_buffer_fn = unsafe {
            NativeFunction::from_closure(move |_this: &JsValue, _args: &[JsValue], context: &mut Context| {
                // For now, just return a resolved promise with an empty ArrayBuffer
                println!("[JS] Response.arrayBuffer() called (simplified implementation)");
                context.eval(boa_engine::Source::from_bytes("Promise.resolve(new ArrayBuffer(0))"))
            })
        };

        let _response_clone4 = self.clone();
        let blob_fn = unsafe {
            NativeFunction::from_closure(move |_this: &JsValue, _args: &[JsValue], context: &mut Context| {
                println!("[JS] Response.blob() called (not fully implemented)");
                context.eval(boa_engine::Source::from_bytes("Promise.resolve({})"))
            })
        };

        let obj = ObjectInitializer::new(context)
            .property(
                JsString::from("status"),
                JsValue::from(self.status),
                Attribute::all(),
            )
            .property(
                JsString::from("statusText"),
                JsValue::from(JsString::from(self.status_text.clone())),
                Attribute::all(),
            )
            .property(
                JsString::from("ok"),
                JsValue::from(self.ok),
                Attribute::all(),
            )
            .property(
                JsString::from("url"),
                JsValue::from(JsString::from(self.url.clone())),
                Attribute::all(),
            )
            .property(
                JsString::from("redirected"),
                JsValue::from(false),
                Attribute::all(),
            )
            .property(
                JsString::from("type"),
                JsValue::from(JsString::from("basic")),
                Attribute::all(),
            )
            .function(text_fn, JsString::from("text"), 0)
            .function(json_fn, JsString::from("json"), 0)
            .function(array_buffer_fn, JsString::from("arrayBuffer"), 0)
            .function(blob_fn, JsString::from("blob"), 0)
            .build();

        Ok(obj.into())
    }
}

/// Setup the fetch API in the JavaScript context
pub fn setup_fetch(context: &mut Context) -> Result<(), String> {
    println!("[JS] Setting up fetch API");

    let fetch_fn = NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], context: &mut Context| {
        // Get the URL argument
        let url = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        if url.is_empty() {
            println!("[JS] fetch() called with empty URL");
            let error_code = "Promise.reject(new Error('URL is required'))";
            return context.eval(boa_engine::Source::from_bytes(error_code));
        }

        println!("[JS] fetch('{}') called", url);

        // Get options if provided (second argument)
        let method = if let Some(options) = args.get(1).and_then(|v| v.as_object()) {
            if let Ok(method_val) = options.get(JsString::from("method"), context) {
                method_val.as_string()
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_else(|| "GET".to_string())
            } else {
                "GET".to_string()
            }
        } else {
            "GET".to_string()
        };

        println!("[JS] fetch method: {}", method);

        // Perform the fetch synchronously (blocking)
        // This is a simplification - in a real browser, this would be async
        let url_clone = url.clone();
        let fetch_result = std::thread::spawn(move || {
            // Create a new tokio runtime for this thread
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let client = HttpClient::new();
                client.fetch(&url_clone).await
            })
        }).join();

        match fetch_result {
            Ok(Ok(body)) => {
                println!("[JS] fetch successful, body length: {}", body.len());
                // Create a Response object
                let response = FetchResponse::new(body, 200, url.clone());
                match response.to_js_object(context) {
                    Ok(response_obj) => {
                        // Return a resolved promise with the response
                        // Store the response object in a way we can reference it
                        let global = context.global_object();
                        let _ = global.set(JsString::from("__fetchResponse"), response_obj.clone(), false, context);

                        let promise_code = "Promise.resolve(__fetchResponse)";
                        match context.eval(boa_engine::Source::from_bytes(promise_code)) {
                            Ok(promise) => {
                                // Clean up the temporary global
                                let _ = global.delete_property_or_throw(JsString::from("__fetchResponse"), context);
                                Ok(promise)
                            }
                            Err(e) => {
                                println!("[JS] Failed to create promise: {}", e);
                                let error_code = "Promise.reject(new Error('Failed to create promise'))";
                                context.eval(boa_engine::Source::from_bytes(error_code))
                            }
                        }
                    }
                    Err(e) => {
                        println!("[JS] Failed to create response object: {}", e);
                        let error_code = "Promise.reject(new Error('Failed to create response'))";
                        context.eval(boa_engine::Source::from_bytes(error_code))
                    }
                }
            }
            Ok(Err(e)) => {
                println!("[JS] fetch failed: {}", e);
                let error_msg = format!("Fetch failed: {}", e);
                let error_code = format!("Promise.reject(new Error({}))",
                    serde_json::to_string(&error_msg).unwrap_or_else(|_| "\"Fetch failed\"".to_string()));
                context.eval(boa_engine::Source::from_bytes(&error_code))
            }
            Err(_) => {
                println!("[JS] fetch thread panicked");
                let error_code = "Promise.reject(new Error('Fetch failed'))";
                context.eval(boa_engine::Source::from_bytes(error_code))
            }
        }
    });

    context.register_global_builtin_callable(JsString::from("fetch"), 1, fetch_fn)
        .map_err(|e| format!("Failed to register fetch: {}", e))?;

    println!("[JS] fetch API initialized");
    Ok(())
}
