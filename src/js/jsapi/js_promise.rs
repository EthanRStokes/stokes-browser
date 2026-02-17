// Rust native representation of JavaScript Promise values
// Provides type-safe Promise operations with proper event loop integration

use crate::js::jsapi::error::{get_pending_exception, JsError};
use crate::js::runtime::{JsRuntime, RUNTIME};
use mozjs::jsapi::{AddRawValueRoot, GetPromiseState, HandleObject, HandleValue, Heap, JSContext, JSObject, JS_NewUCStringCopyN, NewPromiseObject, PromiseState, RejectPromise, RemoveRawValueRoot, ResolvePromise};
use mozjs::jsval::{JSVal, ObjectValue, StringValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers::JS_GetPromiseResult;
use mozjs::rust::Runtime;
use std::ffi::CString;
use std::ptr;
use std::sync::{Arc, Mutex};

/// Represents the state of a JavaScript Promise
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsPromiseState {
    /// Promise is pending and has not yet resolved or rejected
    Pending,
    /// Promise has been fulfilled with a value
    Fulfilled,
    /// Promise has been rejected with a reason
    Rejected,
}

impl From<PromiseState> for JsPromiseState {
    fn from(state: PromiseState) -> Self {
        match state {
            PromiseState::Pending => JsPromiseState::Pending,
            PromiseState::Fulfilled => JsPromiseState::Fulfilled,
            PromiseState::Rejected => JsPromiseState::Rejected,
        }
    }
}

/// A Rust representation of a JavaScript Promise
///
/// This struct holds a persistent reference to a JS Promise object
/// and provides type-safe methods for interacting with it.
///
/// # Thread Safety
///
/// The underlying JSObject is NOT thread-safe. All operations must be
/// performed on the JavaScript runtime thread using the event loop methods.
pub struct JsPromise {
    /// The underlying Promise JSObject, kept rooted to prevent GC
    heap_obj: Box<Heap<*mut JSObject>>,
    /// Permanent root to prevent garbage collection
    permanent_js_root: Box<Heap<JSVal>>,
}

impl JsPromise {
    /// Create a new pending Promise
    ///
    /// This creates a new JavaScript Promise object that can later be
    /// resolved or rejected using the appropriate methods.
    ///
    /// # Safety
    /// Must be called within a JS runtime context (inside do_with_jsapi callback)
    pub unsafe fn new(cx: *mut JSContext) -> Result<Self, JsError> {
        rooted!(in(cx) let null_executor = ptr::null_mut::<JSObject>());
        rooted!(in(cx) let promise = NewPromiseObject(cx, HandleObject::from(null_executor.handle())));

        if promise.get().is_null() {
            return Err(JsError {
                message: "Failed to create Promise object".to_string(),
                filename: String::new(),
                lineno: 0,
                column: 0,
            });
        }

        let mut js_promise = Self {
            heap_obj: Box::new(Heap::default()),
            permanent_js_root: Box::new(Heap::default()),
        };

        js_promise.init_root(cx, promise.get());
        Ok(js_promise)
    }

    /// Create a JsPromise from an existing Promise JSObject
    ///
    /// # Safety
    /// - Must be called within a JS runtime context
    /// - The provided object must be a valid Promise object
    pub unsafe fn from_object(cx: *mut JSContext, promise_obj: *mut JSObject) -> Result<Self, JsError> {
        if promise_obj.is_null() {
            return Err(JsError {
                message: "Cannot create JsPromise from null object".to_string(),
                filename: String::new(),
                lineno: 0,
                column: 0,
            });
        }

        let mut js_promise = Self {
            heap_obj: Box::new(Heap::default()),
            permanent_js_root: Box::new(Heap::default()),
        };

        js_promise.init_root(cx, promise_obj);
        Ok(js_promise)
    }

    /// Initialize the root for the Promise object
    unsafe fn init_root(&mut self, cx: *mut JSContext, obj: *mut JSObject) {
        self.heap_obj.set(obj);
        self.permanent_js_root.set(ObjectValue(obj));
        let c_str = CString::new("JsPromise::root").unwrap();
        assert!(AddRawValueRoot(
            cx,
            self.permanent_js_root.get_unsafe(),
            c_str.as_ptr() as *const i8
        ));
    }

    /// Get the underlying Promise JSObject
    pub fn get(&self) -> *mut JSObject {
        self.heap_obj.get()
    }

    /// Get the current state of the Promise
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn state(&self, _cx: *mut JSContext) -> JsPromiseState {
        rooted!(in(_cx) let promise = self.heap_obj.get());
        let state = GetPromiseState(promise.handle().into());
        JsPromiseState::from(state)
    }

    /// Resolve the Promise with a value
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn resolve(&self, cx: *mut JSContext, value: HandleValue) -> Result<(), JsError> {
        rooted!(in(cx) let promise = self.heap_obj.get());
        let ok = ResolvePromise(cx, promise.handle().into(), value);

        if ok {
            Ok(())
        } else if let Some(err) = get_pending_exception(cx) {
            Err(err)
        } else {
            Err(JsError {
                message: "Unknown error resolving promise".to_string(),
                filename: String::new(),
                lineno: 0,
                column: 0,
            })
        }
    }

    /// Resolve the Promise with undefined value
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn resolve_undefined(&self, cx: *mut JSContext) -> Result<(), JsError> {
        rooted!(in(cx) let value = UndefinedValue());
        self.resolve(cx, value.handle().into())
    }

    /// Resolve the Promise with a string value
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn resolve_string(&self, cx: *mut JSContext, value: &str) -> Result<(), JsError> {
        let utf16: Vec<u16> = value.encode_utf16().collect();
        rooted!(in(cx) let str_obj = JS_NewUCStringCopyN(cx, utf16.as_ptr(), utf16.len()));
        rooted!(in(cx) let str_val = StringValue(&*str_obj.get()));
        self.resolve(cx, str_val.handle().into())
    }

    /// Reject the Promise with a reason
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn reject(&self, cx: *mut JSContext, reason: HandleValue) -> Result<(), JsError> {
        rooted!(in(cx) let promise = self.heap_obj.get());
        let ok = RejectPromise(cx, promise.handle().into(), reason);

        if ok {
            Ok(())
        } else if let Some(err) = get_pending_exception(cx) {
            Err(err)
        } else {
            Err(JsError {
                message: "Unknown error rejecting promise".to_string(),
                filename: String::new(),
                lineno: 0,
                column: 0,
            })
        }
    }

    /// Reject the Promise with a string error message
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn reject_string(&self, cx: *mut JSContext, message: &str) -> Result<(), JsError> {
        let utf16: Vec<u16> = message.encode_utf16().collect();
        rooted!(in(cx) let str_obj = JS_NewUCStringCopyN(cx, utf16.as_ptr(), utf16.len()));
        rooted!(in(cx) let str_val = StringValue(&*str_obj.get()));
        self.reject(cx, str_val.handle().into())
    }

    /// Get the result value of a fulfilled or rejected Promise
    ///
    /// Returns None if the Promise is still pending.
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn get_result(&self, cx: *mut JSContext) -> Option<JSVal> {
        let state = self.state(cx);
        if state == JsPromiseState::Pending {
            return None;
        }

        rooted!(in(cx) let promise = self.heap_obj.get());
        rooted!(in(cx) let mut result = UndefinedValue());
        JS_GetPromiseResult(promise.handle().into(), result.handle_mut().into());
        Some(*result)
    }

    /// Convert to JSVal for passing to JS functions
    pub fn to_jsval(&self) -> JSVal {
        ObjectValue(self.heap_obj.get())
    }
}

impl Drop for JsPromise {
    fn drop(&mut self) {
        unsafe {
            if let Some(cx) = Runtime::get() {
                RemoveRawValueRoot(cx.as_ptr(), self.permanent_js_root.get_unsafe());
            }
        }
    }
}

/// A thread-safe handle to a JsPromise that can be used across threads
///
/// This struct allows Promise resolution/rejection from background threads
/// by queueing the operation on the JS event loop.
pub struct JsPromiseHandle {
    /// Inner promise pointer stored as usize for Send safety
    /// (only valid on JS thread)
    promise_ptr: Arc<Mutex<Option<usize>>>,
}

impl JsPromiseHandle {
    /// Create a new Promise and return both the Promise pointer (as usize) and its handle
    ///
    /// The Promise is created on the JS thread and can be resolved/rejected
    /// from any thread using the handle.
    ///
    /// # Returns
    /// Returns the raw pointer to the JSObject as a usize (for use on the JS thread)
    /// and a handle that can be used from any thread.
    ///
    /// # Example
    /// ```ignore
    /// let (promise_ptr, handle) = JsPromiseHandle::create(&runtime)?;
    ///
    /// // Spawn background work
    /// std::thread::spawn(move || {
    ///     // Do some async work...
    ///     handle.resolve_string(&runtime, "result");
    /// });
    ///
    /// // Use promise_ptr on the JS thread
    /// rval.set(ObjectValue(promise_ptr as *mut JSObject));
    /// ```
    pub fn create(runtime: &JsRuntime) -> Result<(usize, Self), JsError> {
        let promise_ptr: Arc<Mutex<Option<usize>>> = Arc::new(Mutex::new(None));
        let ptr_clone = promise_ptr.clone();

        let result: Result<usize, JsError> = runtime.do_in_es_event_queue_sync(move |_rt| {
            RUNTIME.with(|rc| unsafe {
                let sm_rt = &mut *rc.borrow_mut().unwrap();
                sm_rt.do_with_jsapi(|_rt, cx, _global| {
                    let promise = JsPromise::new(cx)?;
                    let ptr = promise.get() as usize;
                    *ptr_clone.lock().unwrap() = Some(ptr);
                    // Don't drop the promise - it's now managed externally
                    std::mem::forget(promise);
                    Ok(ptr)
                })
            })
        });

        match result {
            Ok(ptr) => Ok((ptr, Self { promise_ptr })),
            Err(e) => Err(e),
        }
    }

    /// Create a new Promise directly from JSContext (must be called on JS thread)
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn create_direct(cx: *mut JSContext) -> Result<(*mut JSObject, Self), JsError> {
        let promise = JsPromise::new(cx)?;
        let ptr = promise.get();
        let ptr_usize = ptr as usize;

        // Don't drop the promise - it's now managed externally
        std::mem::forget(promise);

        let handle = Self {
            promise_ptr: Arc::new(Mutex::new(Some(ptr_usize))),
        };

        Ok((ptr, handle))
    }

    /// Get the raw promise pointer (only valid on JS thread)
    pub fn get_ptr(&self) -> Option<*mut JSObject> {
        self.promise_ptr.lock().unwrap().map(|p| p as *mut JSObject)
    }

    /// Resolve the Promise with undefined on the JS thread
    pub fn resolve_undefined(&self, runtime: &JsRuntime) -> Result<(), JsError> {
        let ptr = *self.promise_ptr.lock().unwrap();
        let ptr = match ptr {
            Some(p) => p,
            None => return Err(JsError {
                message: "Promise handle is invalid".to_string(),
                filename: String::new(),
                lineno: 0,
                column: 0,
            }),
        };

        runtime.do_in_es_event_queue_sync(move |_rt| {
            RUNTIME.with(|rc| unsafe {
                let sm_rt = &mut *rc.borrow_mut().unwrap();
                sm_rt.do_with_jsapi(|_rt, cx, _global| {
                    let promise = JsPromise::from_object(cx, ptr as *mut JSObject)?;
                    let result = promise.resolve_undefined(cx);
                    std::mem::forget(promise); // Don't drop - still managed externally
                    result
                })
            })
        })
    }

    /// Resolve the Promise with a string value on the JS thread
    pub fn resolve_string(&self, runtime: &JsRuntime, value: String) -> Result<(), JsError> {
        let ptr = *self.promise_ptr.lock().unwrap();
        let ptr = match ptr {
            Some(p) => p,
            None => return Err(JsError {
                message: "Promise handle is invalid".to_string(),
                filename: String::new(),
                lineno: 0,
                column: 0,
            }),
        };

        runtime.do_in_es_event_queue_sync(move |_rt| {
            RUNTIME.with(|rc| unsafe {
                let sm_rt = &mut *rc.borrow_mut().unwrap();
                sm_rt.do_with_jsapi(|_rt, cx, _global| {
                    let promise = JsPromise::from_object(cx, ptr as *mut JSObject)?;
                    let result = promise.resolve_string(cx, &value);
                    std::mem::forget(promise); // Don't drop - still managed externally
                    result
                })
            })
        })
    }

    /// Reject the Promise with a string error on the JS thread
    pub fn reject_string(&self, runtime: &JsRuntime, message: String) -> Result<(), JsError> {
        let ptr = *self.promise_ptr.lock().unwrap();
        let ptr = match ptr {
            Some(p) => p,
            None => return Err(JsError {
                message: "Promise handle is invalid".to_string(),
                filename: String::new(),
                lineno: 0,
                column: 0,
            }),
        };

        runtime.do_in_es_event_queue_sync(move |_rt| {
            RUNTIME.with(|rc| unsafe {
                let sm_rt = &mut *rc.borrow_mut().unwrap();
                sm_rt.do_with_jsapi(|_rt, cx, _global| {
                    let promise = JsPromise::from_object(cx, ptr as *mut JSObject)?;
                    let result = promise.reject_string(cx, &message);
                    std::mem::forget(promise); // Don't drop - still managed externally
                    result
                })
            })
        })
    }

    /// Get the current state of the Promise
    pub fn state(&self, runtime: &JsRuntime) -> Result<JsPromiseState, JsError> {
        let ptr = *self.promise_ptr.lock().unwrap();
        let ptr = match ptr {
            Some(p) => p,
            None => return Err(JsError {
                message: "Promise handle is invalid".to_string(),
                filename: String::new(),
                lineno: 0,
                column: 0,
            }),
        };

        runtime.do_in_es_event_queue_sync(move |_rt| {
            RUNTIME.with(|rc| unsafe {
                let sm_rt = &mut *rc.borrow_mut().unwrap();
                sm_rt.do_with_jsapi(|_rt, cx, _global| {
                    let promise = JsPromise::from_object(cx, ptr as *mut JSObject)?;
                    let state = promise.state(cx);
                    std::mem::forget(promise); // Don't drop - still managed externally
                    Ok(state)
                })
            })
        })
    }
}

impl Clone for JsPromiseHandle {
    fn clone(&self) -> Self {
        Self {
            promise_ptr: self.promise_ptr.clone(),
        }
    }
}

/// Builder for creating resolved or rejected Promises
pub struct JsPromiseBuilder;

impl JsPromiseBuilder {
    /// Create a Promise that is already resolved with a value
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn resolved(cx: *mut JSContext, value: HandleValue) -> Result<JsPromise, JsError> {
        let promise = JsPromise::new(cx)?;
        promise.resolve(cx, value)?;
        Ok(promise)
    }

    /// Create a Promise that is already resolved with undefined
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn resolved_undefined(cx: *mut JSContext) -> Result<JsPromise, JsError> {
        let promise = JsPromise::new(cx)?;
        promise.resolve_undefined(cx)?;
        Ok(promise)
    }

    /// Create a Promise that is already resolved with a string
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn resolved_string(cx: *mut JSContext, value: &str) -> Result<JsPromise, JsError> {
        let promise = JsPromise::new(cx)?;
        promise.resolve_string(cx, value)?;
        Ok(promise)
    }

    /// Create a Promise that is already rejected with a reason
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn rejected(cx: *mut JSContext, reason: HandleValue) -> Result<JsPromise, JsError> {
        let promise = JsPromise::new(cx)?;
        promise.reject(cx, reason)?;
        Ok(promise)
    }

    /// Create a Promise that is already rejected with a string message
    ///
    /// # Safety
    /// Must be called within a JS runtime context
    pub unsafe fn rejected_string(cx: *mut JSContext, message: &str) -> Result<JsPromise, JsError> {
        let promise = JsPromise::new(cx)?;
        promise.reject_string(cx, message)?;
        Ok(promise)
    }
}

/// Extension trait for JsRuntime to provide convenient Promise creation
pub trait JsRuntimePromiseExt {
    /// Create a new pending Promise using the event loop
    /// Returns the raw pointer as usize and a handle
    fn create_promise(&self) -> Result<(usize, JsPromiseHandle), JsError>;

    /// Create a resolved Promise with a string value
    /// Returns the raw pointer as usize
    fn create_resolved_promise_string(&self, value: &str) -> Result<usize, JsError>;

    /// Create a rejected Promise with a string message
    /// Returns the raw pointer as usize
    fn create_rejected_promise_string(&self, message: &str) -> Result<usize, JsError>;
}

impl JsRuntimePromiseExt for JsRuntime {
    fn create_promise(&self) -> Result<(usize, JsPromiseHandle), JsError> {
        JsPromiseHandle::create(self)
    }

    fn create_resolved_promise_string(&self, value: &str) -> Result<usize, JsError> {
        let value = value.to_string();
        self.do_in_es_event_queue_sync(move |_rt| {
            RUNTIME.with(|rc| unsafe {
                let sm_rt = &mut *rc.borrow_mut().unwrap();
                sm_rt.do_with_jsapi(|_rt, cx, _global| {
                    let promise = JsPromiseBuilder::resolved_string(cx, &value)?;
                    let ptr = promise.get() as usize;
                    std::mem::forget(promise);
                    Ok(ptr)
                })
            })
        })
    }

    fn create_rejected_promise_string(&self, message: &str) -> Result<usize, JsError> {
        let message = message.to_string();
        self.do_in_es_event_queue_sync(move |_rt| {
            RUNTIME.with(|rc| unsafe {
                let sm_rt = &mut *rc.borrow_mut().unwrap();
                sm_rt.do_with_jsapi(|_rt, cx, _global| {
                    let promise = JsPromiseBuilder::rejected_string(cx, &message)?;
                    let ptr = promise.get() as usize;
                    std::mem::forget(promise);
                    Ok(ptr)
                })
            })
        })
    }
}

/// Utility function to convert a Result into a Promise
///
/// If the Result is Ok, returns a resolved Promise with the value.
/// If the Result is Err, returns a rejected Promise with the error message.
///
/// # Safety
/// Must be called within a JS runtime context
pub unsafe fn result_to_promise<T, E>(
    cx: *mut JSContext,
    result: Result<T, E>,
    value_converter: impl FnOnce(*mut JSContext, T) -> Result<JSVal, JsError>,
) -> Result<JsPromise, JsError>
where
    E: std::fmt::Display,
{
    match result {
        Ok(value) => {
            let js_val = value_converter(cx, value)?;
            rooted!(in(cx) let val = js_val);
            JsPromiseBuilder::resolved(cx, val.handle().into())
        }
        Err(e) => {
            JsPromiseBuilder::rejected_string(cx, &e.to_string())
        }
    }
}

/// Utility function to wrap an async operation in a Promise
///
/// This creates a Promise and spawns a task on the event loop to
/// execute the operation asynchronously.
///
/// # Returns
/// Returns the raw pointer to the Promise as usize
///
/// # Example
/// ```ignore
/// let promise_ptr = async_promise(runtime, |handle| {
///     // This runs asynchronously
///     std::thread::spawn(move || {
///         // Do some work...
///         handle.resolve_string(&runtime, "done");
///     });
/// })?;
///
/// // Use on JS thread
/// rval.set(ObjectValue(promise_ptr as *mut JSObject));
/// ```
pub fn async_promise<F>(runtime: &JsRuntime, async_work: F) -> Result<usize, JsError>
where
    F: FnOnce(JsPromiseHandle) + Send + 'static,
{
    let (promise_ptr, handle) = JsPromiseHandle::create(runtime)?;

    // Execute the async work
    async_work(handle);

    Ok(promise_ptr)
}


