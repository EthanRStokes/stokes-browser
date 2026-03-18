use crate::js::jsapi::error::{get_pending_exception, JsError};
use crate::js::helpers::{js_value_to_string, ToSafeCx};
use mozjs::jsapi::{HandleValueArray, Heap, JSObject, PromiseRejectionHandlingState,};
use mozjs::jsval::{JSVal, ObjectValue, UndefinedValue};
use mozjs::panic::wrap_panic;
use mozjs::rust::{HandleObject, HandleValue, ValueArray};
use mozjs::rust::wrappers::JS_GetPromiseResult;
use mozjs::rust::Runtime;
use mozjs::rooted;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::CString;
use std::os::raw::c_void;
use std::ptr;
use std::rc::Rc;
use mozjs::context::{JSContext, RawJSContext};
use mozjs::realm::AutoRealm;
use mozjs::rust::wrappers2::{AddRawValueRoot, CurrentGlobalOrNull, JS_CallFunctionValue, RemoveRawValueRoot, ResolvePromise, SetPromiseRejectionTrackerCallback};

// Thread-local queue for promise jobs
thread_local! {
    static PROMISE_JOB_QUEUE: RefCell<VecDeque<Rc<PromiseJobCallback>>> = RefCell::new(VecDeque::new());
    static REJECTION_TRANSITIONS: RefCell<VecDeque<PromiseRejectionTransition>> = RefCell::new(VecDeque::new());
    static REPORTED_UNHANDLED_PROMISES: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

#[derive(Clone, Copy)]
enum RejectionTransitionState {
    Unhandled,
    Handled,
}

struct PromiseRejectionTransition {
    promise: PersistentRooted,
    state: RejectionTransitionState,
}

/// resolve a Promise with a given resolution value
pub fn resolve_promise(
    context: &mut JSContext,
    promise: HandleObject,
    resolution_value: HandleValue,
) -> Result<(), JsError> {
    let raw_cx = unsafe { context.raw_cx() };
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

pub fn init_rejection_tracker(cx: &mut JSContext) {
    unsafe {
        SetPromiseRejectionTrackerCallback(cx, Some(promise_rejection_tracker), ptr::null_mut())
    };
}

unsafe extern "C" fn promise_rejection_tracker(
    raw_cx: *mut RawJSContext,
    _muted_errors: bool,
    promise: mozjs::jsapi::HandleObject,
    state: PromiseRejectionHandlingState,
    _data: *mut c_void,
) {
    let safe_cx = &mut raw_cx.to_safe_cx();
    let transition_state = match state {
        PromiseRejectionHandlingState::Unhandled => RejectionTransitionState::Unhandled,
        PromiseRejectionHandlingState::Handled => RejectionTransitionState::Handled,
    };

    let rooted_promise = PersistentRooted::new_from_obj(safe_cx, promise.get());

    REJECTION_TRANSITIONS.with(|queue| {
        queue.borrow_mut().push_back(PromiseRejectionTransition {
            promise: rooted_promise,
            state: transition_state,
        });
    });
}

pub(crate) unsafe extern "C" fn enqueue_promise_job(
    _extra: *const c_void,
    cx: *mut RawJSContext,
    _promise: mozjs::jsapi::HandleObject,
    job: mozjs::jsapi::HandleObject,
    _allocation_site: mozjs::jsapi::HandleObject,
    _incumbent_global: mozjs::jsapi::HandleObject,
) -> bool {
    let safe_cx = &mut cx.to_safe_cx();
    let mut result = false;
    wrap_panic(&mut || unsafe {
        let cb = PromiseJobCallback::new(safe_cx, job.get());

        // Add the job to our promise job queue
        PROMISE_JOB_QUEUE.with(|queue| {
            queue.borrow_mut().push_back(cb);
        });

        result = true
    });
    result
}

/// Run one full microtask checkpoint.
///
/// This drains queued promise jobs and then reports any rejection transitions.
/// Returns the number of promise jobs executed.
pub fn perform_microtask_checkpoint(cx: &mut JSContext) -> usize {
    let raw_cx = unsafe { cx.raw_cx() };
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

    process_rejection_transitions(cx);

    executed
}

// Backward-compatible alias for existing call sites.
pub fn run_promise_jobs(cx: &mut mozjs::context::JSContext) -> usize {
    perform_microtask_checkpoint(cx)
}

/// Check if there are pending promise jobs
pub fn has_pending_promise_jobs() -> bool {
    PROMISE_JOB_QUEUE.with(|queue| !queue.borrow().is_empty())
}

fn process_rejection_transitions(cx: &mut JSContext) {
    let transitions: Vec<PromiseRejectionTransition> = REJECTION_TRANSITIONS.with(|queue| {
        queue.borrow_mut().drain(..).collect()
    });

    if transitions.is_empty() {
        return;
    }

    let mut pending_unhandled: HashMap<usize, PersistentRooted> = HashMap::new();
    let mut handled_to_dispatch: Vec<PersistentRooted> = Vec::new();

    for transition in transitions {
        let promise_id = transition.promise.get() as usize;
        match transition.state {
            RejectionTransitionState::Unhandled => {
                pending_unhandled.insert(promise_id, transition.promise);
            }
            RejectionTransitionState::Handled => {
                if pending_unhandled.remove(&promise_id).is_some() {
                    continue;
                }

                let was_reported = REPORTED_UNHANDLED_PROMISES.with(|set| {
                    set.borrow().contains(&promise_id)
                });

                if was_reported {
                    REPORTED_UNHANDLED_PROMISES.with(|set| {
                        set.borrow_mut().remove(&promise_id);
                    });
                    handled_to_dispatch.push(transition.promise);
                }
            }
        }
    }

    let mut unhandled_reason_counts: HashMap<String, usize> = HashMap::new();

    for (promise_id, promise) in pending_unhandled {
        let already_reported = REPORTED_UNHANDLED_PROMISES.with(|set| {
            set.borrow().contains(&promise_id)
        });
        if already_reported {
            continue;
        }

        let not_canceled = unsafe {
            dispatch_promise_rejection_event(cx, "unhandledrejection", promise.get(), true)
        };

        if not_canceled {
            let reason_text = unsafe { promise_reason_to_string(cx, promise.get()) };
            *unhandled_reason_counts.entry(reason_text).or_insert(0) += 1;
        }

        REPORTED_UNHANDLED_PROMISES.with(|set| {
            set.borrow_mut().insert(promise_id);
        });
    }

    for (reason, count) in unhandled_reason_counts {
        if count == 1 {
            log::error!("Unhandled promise rejection: {reason}");
        } else {
            log::error!("Unhandled promise rejection ({count}x): {reason}");
        }
    }

    for promise in handled_to_dispatch {
        unsafe {
            let _ = dispatch_promise_rejection_event(
                cx,
                "rejectionhandled",
                promise.get(),
                false,
            );
        }
    }
}

unsafe fn dispatch_promise_rejection_event(
    cx: &mut JSContext,
    event_type: &str,
    promise_obj: *mut JSObject,
    cancelable: bool,
) -> bool {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let promise = promise_obj);
    if promise.get().is_null() {
        return true;
    }

    let mut cx = AutoRealm::new_from_handle(cx, promise.handle());
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(&cx));
    if global.get().is_null() {
        return true;
    }

    let reason = promise_rejection_reason(&mut cx, promise_obj);

    crate::js::bindings::event_listeners::dispatch_window_promise_rejection_event(
        &mut cx,
        global.get(),
        event_type,
        promise_obj,
        reason,
        cancelable,
    )
}

fn promise_rejection_reason(cx: &mut JSContext, promise_obj: *mut JSObject) -> JSVal {
    let raw_cx = unsafe { cx.raw_cx() };
    rooted!(in(raw_cx) let promise = promise_obj);
    rooted!(in(raw_cx) let mut reason = UndefinedValue());
    unsafe { JS_GetPromiseResult(promise.handle().into(), reason.handle_mut().into()); }
    *reason
}

unsafe fn promise_reason_to_string(cx: &mut JSContext, promise_obj: *mut JSObject) -> String {
    let reason = promise_rejection_reason(cx, promise_obj);
    js_value_to_string(cx, reason)
}

struct PromiseJobCallback {
    pub parent: CallbackFunction,
}

impl PromiseJobCallback {
    pub unsafe fn new(cx: &mut JSContext, a_callback: *mut JSObject) -> Rc<PromiseJobCallback> {
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

    unsafe fn call(&self, cx: &mut JSContext, a_this_obj: HandleObject) -> Result<(), ()> {
        let raw_cx = cx.raw_cx_no_gc();
        rooted!(in(raw_cx) let callback_obj = self.parent.callback_holder().get());
        if callback_obj.get().is_null() {
            return Err(());
        }

        // Promise jobs must run in the callback's realm so `CurrentGlobalOrNull`
        // and any allocation/call internals use the right compartment.
        let mut cx = &mut AutoRealm::new_from_handle(cx, callback_obj.handle());
        let raw_cx = cx.raw_cx();
        rooted!(in(raw_cx) let mut rval = UndefinedValue());
        rooted!(in(raw_cx) let callable = ObjectValue(self.parent.callback_holder().get()));
        rooted!(in(raw_cx) let this_obj = if a_this_obj.get().is_null() {
            CurrentGlobalOrNull(cx)
        } else {
            a_this_obj.get()
        });
        if this_obj.get().is_null() {
            return Err(());
        }
        rooted!(in(raw_cx) let zero_args = ValueArray::<0usize>::new([]));
        let ok = JS_CallFunctionValue(
            cx,
            this_obj.handle().into(),
            callable.handle().into(),
            &HandleValueArray::from(&zero_args),
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
    pub unsafe fn init(&mut self, cx: &mut JSContext, callback: *mut JSObject) {
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
    pub fn new_from_obj(cx: &mut JSContext, obj: *mut JSObject) -> Self {
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
    pub unsafe fn init(&mut self, cx: &mut JSContext, js_obj: *mut JSObject) {
        self.heap_obj.set(js_obj);
        self.permanent_js_root.set(ObjectValue(js_obj));
        let c_str = CString::new("EsPersistentRooted::root").unwrap();
        #[cfg(target_arch = "x86_64")]
        let c_str = c_str.as_ptr() as *const i8;
        #[cfg(target_arch = "aarch64")]
        let c_str = c_str.as_ptr() as *const u8;
        assert!(AddRawValueRoot(
            cx,
            self.permanent_js_root.get_unsafe(),
            c_str
        ));
    }
}

impl Drop for PersistentRooted {
    fn drop(&mut self) {
        unsafe {
            if let Some(cx) = Runtime::get() {
                RemoveRawValueRoot(&cx.to_safe_cx(), self.permanent_js_root.get_unsafe());
            }
        }
    }
}