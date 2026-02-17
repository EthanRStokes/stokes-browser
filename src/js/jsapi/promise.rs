use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::CString;
use std::os::raw::c_void;
use std::ptr;
use std::rc::Rc;
use mozjs::jsapi::{AddRawValueRoot, HandleObject, HandleValue, HandleValueArray, Heap, JSContext, JSObject, JS_CallFunctionValue, PromiseRejectionHandlingState, RemoveRawValueRoot, ResolvePromise, SetPromiseRejectionTrackerCallback, StackFormat};
use mozjs::jsval::{JSVal, ObjectValue, UndefinedValue};
use mozjs::panic::wrap_panic;
use mozjs::{capture_stack, rooted};
use mozjs::rust::Runtime;
use crate::js::jsapi::error::{get_pending_exception, JsError};

// Thread-local queue for promise jobs
thread_local! {
    static PROMISE_JOB_QUEUE: RefCell<VecDeque<Rc<PromiseJobCallback>>> = RefCell::new(VecDeque::new());
}

/// resolve a Promise with a given resolution value
pub fn resolve_promise(
    context: *mut JSContext,
    promise: HandleObject,
    resolution_value: HandleValue,
) -> Result<(), JsError> {
    let ok = unsafe { ResolvePromise(context, promise, resolution_value) };
    if ok {
        Ok(())
    } else if let Some(err) = get_pending_exception(context) {
        Err(err)
    } else {
        Err(JsError {
            message: "unknown error resolving promise".to_string(),
            filename: "".to_string(),
            lineno: 0,
            column: 0,
        })
    }
}

pub fn init_rejection_tracker(cx: *mut JSContext) {
    unsafe {
        SetPromiseRejectionTrackerCallback(cx, Some(promise_rejection_tracker), ptr::null_mut())
    };
}

unsafe extern "C" fn promise_rejection_tracker(
    cx: *mut JSContext,
    _muted_errors: bool,
    _promise: HandleObject,
    _state: PromiseRejectionHandlingState,
    _data: *mut c_void,
) {
    capture_stack!(in (cx) let stack);
    let str_stack = stack.unwrap().as_string(None, StackFormat::SpiderMonkey).unwrap();

    log::error!("promise without rejection handler rejected from {str_stack}")
}

pub(crate) unsafe extern "C" fn enqueue_promise_job(
    _extra: *const c_void,
    cx: *mut JSContext,
    _promise: mozjs::jsapi::HandleObject,
    job: mozjs::jsapi::HandleObject,
    _allocation_site: mozjs::jsapi::HandleObject,
    _incumbent_global: mozjs::jsapi::HandleObject,
) -> bool {
    let mut result = false;
    wrap_panic(&mut || unsafe {
        let cb = PromiseJobCallback::new(cx, job.get());

        // Add the job to our promise job queue
        PROMISE_JOB_QUEUE.with(|queue| {
            queue.borrow_mut().push_back(cb);
        });

        result = true
    });
    result
}

/// Run all pending promise jobs in the queue
/// This should be called after script execution to process microtasks
/// Returns the number of jobs that were executed
pub fn run_promise_jobs(cx: *mut JSContext) -> usize {
    let mut executed = 0;

    // Process jobs until the queue is empty
    // Note: Jobs may enqueue more jobs, so we keep processing until empty
    loop {
        let job = PROMISE_JOB_QUEUE.with(|queue| {
            queue.borrow_mut().pop_front()
        });

        match job {
            Some(cb) => {
                unsafe {
                    let call_res = cb.call(cx, HandleObject::null());
                    if call_res.is_err() {
                        if let Some(err) = get_pending_exception(cx) {
                            log::error!(
                                "Promise job failed {}:{}:{} -> {}",
                                err.filename, err.lineno, err.column, err.message
                            );
                        }
                    }
                }
                executed += 1;
            }
            None => break,
        }
    }

    executed
}

/// Check if there are pending promise jobs
pub fn has_pending_promise_jobs() -> bool {
    PROMISE_JOB_QUEUE.with(|queue| !queue.borrow().is_empty())
}

struct PromiseJobCallback {
    pub parent: CallbackFunction,
}

impl PromiseJobCallback {
    pub unsafe fn new(cx: *mut JSContext, a_callback: *mut JSObject) -> Rc<PromiseJobCallback> {
        let mut ret = Rc::new(PromiseJobCallback {
            parent: CallbackFunction::new(),
        });
        // Note: callback cannot be moved after calling init.
        match Rc::get_mut(&mut ret) {
            Some(ref mut callback) => callback.parent.init(cx, a_callback),
            None => unreachable!(),
        };
        ret
    }

    unsafe fn call(&self, cx: *mut JSContext, a_this_obj: HandleObject) -> Result<(), ()> {
        rooted!(in(cx) let mut rval = UndefinedValue());
        rooted!(in(cx) let callable = ObjectValue(self.parent.callback_holder().get()));
        //rooted!(in(cx) let rooted_this = a_this_obj.get());
        let ok = JS_CallFunctionValue(
            cx,
            a_this_obj,
            callable.handle().into(),
            &HandleValueArray::empty(),
            rval.handle_mut().into(),
        );
        if !ok {
            return Err(());
        }

        Ok(())
    }
}

struct CallbackFunction {
    object: PersistentRooted,
}

impl CallbackFunction {
    /// Create a new `CallbackFunction` for this object.

    pub fn new() -> CallbackFunction {
        CallbackFunction {
            object: PersistentRooted::new(),
        }
    }

    /// Returns the underlying `CallbackObject`.
    pub fn callback_holder(&self) -> &PersistentRooted {
        &self.object
    }

    /// Initialize the callback function with a value.
    /// Should be called once this object is done moving.
    pub unsafe fn init(&mut self, cx: *mut JSContext, callback: *mut JSObject) {
        self.object.init(cx, callback);
    }
}

/// the EsPersistentRooted struct is used to keep an Object rooted while there are no references to it in the script Runtime
/// the root will be released when this struct is dropped
pub struct PersistentRooted {
    /// The underlying `JSObject`.
    heap_obj: Box<Heap<*mut JSObject>>,
    permanent_js_root: Box<Heap<JSVal>>,
}

impl Default for PersistentRooted {
    fn default() -> PersistentRooted {
        PersistentRooted::new()
    }
}

impl PersistentRooted {
    pub fn new() -> PersistentRooted {
        PersistentRooted {
            heap_obj: Box::new(Heap::default()),
            permanent_js_root: Box::new(Heap::default()),
        }
    }

    /// create a new instance of EsPersistentRooted with a given JSObject
    /// this will init the EsPersistentRooted and thus the object will be rooted after calling this method
    pub fn new_from_obj(cx: *mut JSContext, obj: *mut JSObject) -> Self {
        let mut ret = Self::new();
        unsafe { ret.init(cx, obj) };
        ret
    }

    /// get the JSObject rooted by this instance of EsPersistentRooted
    pub fn get(&self) -> *mut JSObject {
        self.heap_obj.get()
    }

    /// # Safety
    /// be safe :)
    pub unsafe fn init(&mut self, cx: *mut JSContext, js_obj: *mut JSObject) {
        self.heap_obj.set(js_obj);
        self.permanent_js_root.set(ObjectValue(js_obj));
        let c_str = CString::new("EsPersistentRooted::root").unwrap();
        assert!(AddRawValueRoot(
            cx,
            self.permanent_js_root.get_unsafe(),
            c_str.as_ptr() as *const i8
        ));
    }
}

impl Drop for PersistentRooted {
    fn drop(&mut self) {
        unsafe {
            let cx = Runtime::get();
            RemoveRawValueRoot(cx.unwrap().as_ptr(), self.permanent_js_root.get_unsafe());
        }
    }
}