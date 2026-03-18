use super::JsResult;
use crate::dom::Dom;
use crate::js::bindings::timers::TimerManager;
use crate::js::jsapi::define_native_function::define_native_function;
use crate::js::jsapi::objects::get_obj_prop_val_as_string;
use crate::js::jsapi::promise::enqueue_promise_job;
use hirofa_utils::eventloop::EventLoop;
use lazy_static::lazy_static;
use log::trace;
use mozjs::context::{JSContext, RawJSContext};
use mozjs::conversions::jsstr_to_string;
use mozjs::gc::HandleObject;
use mozjs::glue::JobQueueTraps;
use mozjs::jsapi::{CallArgs, JSContext as ApiJSContext, SetModuleMetadataHook, SetModulePrivate, SetScriptPrivate, SourceText};
use mozjs::jsapi::{Heap, JSObject, JSScript, OnNewGlobalHookOption};
// JavaScript runtime management using Mozilla's SpiderMonkey (mozjs)
use mozjs::jsval::{PrivateValue, StringValue, UndefinedValue};
use mozjs::panic::{maybe_resume_unwind};
use mozjs::rooted;
use mozjs::rust::wrappers2::{Compile1, CompileModule1, JS_ClearPendingException, JS_DefineProperty, JS_ExecuteScript, JS_GetPendingException, JS_GetScriptPrivate, JS_IsExceptionPending, JS_NewGlobalObject, JS_NewUCStringCopyN, JS_ValueToSource, ModuleEvaluate, ModuleLink};
use mozjs::rust::{transform_str_to_source_text, CompileOptionsWrapper, JSEngine, MutableHandleValue, RealmOptions, Runtime, SIMPLE_GLOBAL_CLASS};
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_void;
use std::ptr;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Duration;
use mozjs::realm::AutoRealm;
use url::Url;
use crate::js::bindings::initialize_bindings;
use crate::js::helpers::ToSafeCx;

lazy_static! {
    static ref ENGINE_HANDLER_PRODUCER: EventLoop = EventLoop::new();
}

thread_local! {
    static ENGINE: RefCell<JSEngine> = RefCell::new(JSEngine::init().unwrap());
}

pub type GlobalOp = dyn Fn(*mut ApiJSContext, CallArgs) -> bool + Send + 'static;

thread_local! {
    pub(crate) static RUNTIME: RefCell<Option<*mut JsRuntime>> = RefCell::new(None);
    static GLOBAL_OPS: RefCell<HashMap<&'static str, Box<GlobalOp>>> = RefCell::new(HashMap::new());
}

pub(crate) static JOB_QUEUE_TRAPS: JobQueueTraps = JobQueueTraps {
    getHostDefinedData: Some(get_host_defined_data),
    enqueuePromiseJob: Some(enqueue_promise_job),
    runJobs: None,
    empty: Some(empty),
    pushNewInterruptQueue: None,
    popInterruptQueue: None,
    dropInterruptQueues: None,
};

// Stack size for growing when needed (16MB to handle very large scripts)
const STACK_SIZE: usize = 16 * 1024 * 1024;
// Red zone threshold (32KB)
const _RED_ZONE: usize = 32 * 1024;

/// JavaScript runtime that manages execution context
pub struct JsRuntime {
    // IMPORTANT: Field order matters for drop order!
    // runtime must be dropped before engine since runtime holds a handle to engine
    dom: *mut Dom,
    timer_manager: Rc<TimerManager>,
    user_agent: String,
    global: Box<Heap<*mut JSObject>>,
    event_loop: EventLoop,
    runtime: Runtime,
}

impl JsRuntime {
    /// Create a new JavaScript runtime
    pub fn new(dom: *mut Dom, user_agent: String) -> JsResult<Self> {
        let mut runtime = Runtime::new(
            ENGINE_HANDLER_PRODUCER.exe(|| ENGINE.with(|engine| engine.borrow().handle()))
        );

        // Create and set up timer manager
        let timer_manager = Rc::new(TimerManager::new());

        // Create a global object
        let global = Box::new(Heap::default());
        {
            let cx = runtime.cx();
            let options = RealmOptions::default();

            unsafe {
                rooted!(&in(cx) let global_root = JS_NewGlobalObject(
                    cx,
                    &SIMPLE_GLOBAL_CLASS,
                    ptr::null_mut(),
                    OnNewGlobalHookOption::FireOnNewGlobalHook,
                    &*options,
                ));

                if global_root.get().is_null() {
                    return Err("Failed to create global object".to_string());
                }

                global.set(global_root.get());
            }
        }

        let mut js_runtime = Self {
            dom,
            timer_manager: timer_manager.clone(),
            user_agent,
            global,
            event_loop: EventLoop::new(),
            runtime,
        };
        // NOTE: Do NOT set RUNTIME here — js_runtime is a local stack variable that will be
        // moved when this function returns Ok(js_runtime).  The caller must update RUNTIME
        // after placing the returned value in its final, stable memory location.
        let user_agent = js_runtime.user_agent.clone();

        // Enter the realm for the global object before setting up bindings
        js_runtime.enter_realm_and_initialize(dom, user_agent, timer_manager)?;

        Ok(js_runtime)
    }

    /// Enter the realm and initialize bindings
    fn enter_realm_and_initialize(&mut self, dom: *mut Dom, user_agent: String, timer_manager: Rc<TimerManager>) -> JsResult<()> {
        // Get raw pointers before entering the realm to avoid borrow conflicts
        let raw_cx = unsafe { self.runtime.cx().raw_cx() };
        let cx = &mut raw_cx.to_safe_cx();
        let global_ptr = self.global.get();

        unsafe {
            rooted!(in(raw_cx) let global_root = global_ptr);
            let cx = AutoRealm::new_from_handle(cx, global_root.handle());

            // Required for import.meta support in module scripts.
            SetModuleMetadataHook(self.runtime.rt(), Some(module_metadata_hook));

            initialize_bindings(self, dom, user_agent, timer_manager)?;
        }
        Ok(())
    }

    /// Get the raw JSContext pointer
    pub fn cx(&mut self) -> &mut JSContext {
        self.runtime.cx()
    }

    /// Get the global object
    pub fn global(&self) -> *mut JSObject {
        self.global.get()
    }

    /// Execute JavaScript code
    pub fn execute(&mut self, code: &str, print_error: bool) -> JsResult<()> {
        let cx = self.runtime.cx();
        let raw_cx = unsafe { cx.raw_cx() };
        let global_ptr = self.global.get();

        unsafe {
            // Enter the realm of our global object
            rooted!(in(raw_cx) let global = global_ptr);
            if global.get().is_null() {
                return Err("No global object".to_string());
            }

            // Must enter the realm before compiling or executing scripts
            let mut cx = &mut AutoRealm::new_from_handle(cx, global.handle());
            let raw_cx = cx.raw_cx();

            rooted!(in(raw_cx) let mut rval = UndefinedValue());
            let rval = rval.handle_mut();

            let dom_ref = &*self.dom;
            let url = Url::from(&dom_ref.url);

            // Compile the script first
            rooted!(in(raw_cx) let mut compiled_script = ptr::null_mut::<JSScript>());
            compiled_script.set(Self::compile_script(cx, code, "", 1));

            if compiled_script.is_null() {
                // Handle compilation error
                if JS_IsExceptionPending(cx) {
                    rooted!(in(raw_cx) let mut exception = UndefinedValue());
                    if JS_GetPendingException(cx, MutableHandleValue::from(exception.handle_mut())) {
                        JS_ClearPendingException(cx);
                        rooted!(in(raw_cx) let exc_str = JS_ValueToSource(cx, exception.handle()));
                        if !exc_str.get().is_null() {
                            let msg = jsstr_to_string(raw_cx, NonNull::new(exc_str.handle().get()).unwrap());
                            if print_error {
                                return Err(format!("JavaScript COMPILE error: {}\n{}", msg, code));
                            }
                            return Err(format!("JavaScript COMPILE error: {}", msg));
                        }
                    }
                }
                return Err("JavaScript compilation failed".to_string());
            }

            let script = NonNull::new(*compiled_script).expect("Can't be null");

            if !Self::evaluate_script(cx, script, url, MutableHandleValue::from(rval)) {
                // Handle evaluation error
                if JS_IsExceptionPending(cx) {
                    rooted!(in(raw_cx) let mut exception = UndefinedValue());
                    if JS_GetPendingException(cx, MutableHandleValue::from(exception.handle_mut())) {
                        JS_ClearPendingException(cx);
                        rooted!(in(raw_cx) let exc_str = JS_ValueToSource(cx, exception.handle()));
                        if !exc_str.get().is_null() {
                            let msg = jsstr_to_string(raw_cx, NonNull::new(exc_str.handle().get()).unwrap());
                            if print_error {
                                return Err(format!("JavaScript EVAL error: {}\n{}", msg, code));
                            }
                            return Err(format!("JavaScript EVAL error: {}", msg))
                        }
                    }
                }
                return Err("JavaScript evaluation failed".to_string());
            }
            maybe_resume_unwind();
            Ok(())
        }
    }

    fn compile_script(
        context: &mut JSContext,
        text: &str,
        filename: &str,
        line_number: u32,
    ) -> *mut JSScript {
        let options = unsafe { CompileOptionsWrapper::new(&context, filename.parse().unwrap(), line_number) };

        // First try to compile the script as-is
        let result = unsafe { Compile1(context, options.ptr, &mut transform_str_to_source_text(text)) };

        // If compilation failed and the code looks like a JSON object or array,
        // wrap it in parentheses to make it a valid expression.
        // This handles cases where a script is just a large JSON structure.
        if result.is_null() {
            let trimmed = text.trim();
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                // Clear any pending exception from the first compile attempt
                unsafe { JS_ClearPendingException(context) };

                // Wrap as an assignment to window.__INLINE_DATA__ so other scripts can access it
                // This mimics how browsers handle inline JSON configuration scripts
                let wrapped = format!("(window.__INLINE_DATA__ = window.__INLINE_DATA__ || []).push({})", text);
                let options = unsafe { CompileOptionsWrapper::new(&context, filename.parse().unwrap(), line_number) };
                return unsafe { Compile1(context, options.ptr, &mut transform_str_to_source_text(&wrapped)) };
            }
        }

        result
    }

    fn evaluate_script(
        context: &mut JSContext,
        compiled_script: NonNull<JSScript>,
        url: Url,
        rval: MutableHandleValue,
    ) -> bool {
        let raw_cx = unsafe { context.raw_cx() };
        rooted!(in(raw_cx) let record = compiled_script.as_ptr());
        rooted!(in(raw_cx) let mut script_private = UndefinedValue());

        unsafe { JS_GetScriptPrivate(*record, script_private.handle_mut()) };

        if script_private.is_undefined() {
            // Set script private data if needed
            let url = Rc::new(url);
            unsafe { SetScriptPrivate(*record, &PrivateValue(Rc::into_raw(url) as *const _)) };
        }

        unsafe { JS_ExecuteScript(context, record.handle(), rval) }
    }

    /// Execute JavaScript code from a script tag
    pub fn execute_script(&mut self, code: &str, print_eval_error: bool) -> JsResult<()> {
        match self.execute(code, print_eval_error) {
            Ok(_result) => {
                // Process any remaining jobs after script execution
                self.run_pending_jobs();
                Ok(())
            },
            Err(e) => {
                eprintln!("Script execution error: {}", e);
                Err(e)
            }
        }
    }

    fn effective_module_source_url(&self, source_url: Option<&str>) -> String {
        source_url
            .map(str::to_string)
            .unwrap_or_else(|| unsafe { (&*self.dom).url.to_string() })
    }

    fn compile_module_script(
        context: &mut JSContext,
        text: &str,
        filename: &str,
        line_number: u32,
    ) -> *mut JSObject {
        let options = unsafe { CompileOptionsWrapper::new(&context, filename.parse().unwrap(), line_number) };
        unsafe { CompileModule1(context, options.ptr, &mut transform_str_to_source_text(text)) }
    }

    unsafe fn extract_js_exception(
        context: &mut JSContext,
        raw_cx: *mut RawJSContext,
        prefix: &str,
        code: &str,
        print_error: bool,
    ) -> String {
        if JS_IsExceptionPending(context) {
            rooted!(in(raw_cx) let mut exception = UndefinedValue());
            if JS_GetPendingException(context, MutableHandleValue::from(exception.handle_mut())) {
                JS_ClearPendingException(context);
                rooted!(in(raw_cx) let exc_str = JS_ValueToSource(context, exception.handle()));
                if !exc_str.get().is_null() {
                    let msg = jsstr_to_string(raw_cx, NonNull::new(exc_str.handle().get()).unwrap());
                    if print_error {
                        return format!("{prefix}: {msg}\n{code}");
                    }
                    return format!("{prefix}: {msg}");
                }
            }
        }

        prefix.to_string()
    }

    /// Execute JavaScript that originated from `<script type=\"module\">`.
    pub fn execute_module_script(&mut self, code: &str, source_url: Option<&str>, print_eval_error: bool) -> JsResult<()> {
        let source_name = self.effective_module_source_url(source_url);
        let cx = self.runtime.cx();
        let raw_cx = unsafe { cx.raw_cx() };
        let global_ptr = self.global.get();

        unsafe {
            rooted!(in(raw_cx) let global = global_ptr);
            if global.get().is_null() {
                return Err("No global object".to_string());
            }

            let cx = &mut AutoRealm::new_from_handle(cx, global.handle());
            let raw_cx = cx.raw_cx();

            rooted!(in(raw_cx) let mut module = ptr::null_mut::<JSObject>());
            module.set(Self::compile_module_script(cx, code, &source_name, 1));
            if module.get().is_null() {
                let msg = Self::extract_js_exception(cx, raw_cx, "JavaScript MODULE COMPILE error", code, print_eval_error);
                eprintln!("Module script execution error: {}", msg);
                return Err(msg);
            }

            let source_url_utf16: Vec<u16> = source_name.encode_utf16().collect();
            rooted!(in(raw_cx) let source_url_js = JS_NewUCStringCopyN(cx, source_url_utf16.as_ptr(), source_url_utf16.len()));
            rooted!(in(raw_cx) let module_private = StringValue(&*source_url_js.get()));
            let module_private_value = module_private.get();
            SetModulePrivate(module.get(), &module_private_value);

            if !ModuleLink(cx, module.handle().into()) {
                let msg = Self::extract_js_exception(cx, raw_cx, "JavaScript MODULE INSTANTIATE error", code, print_eval_error);
                eprintln!("Module script execution error: {}", msg);
                return Err(msg);
            }

            rooted!(in(raw_cx) let mut eval_result = UndefinedValue());
            if !ModuleEvaluate(cx, module.handle().into(), eval_result.handle_mut().into()) {
                let msg = Self::extract_js_exception(cx, raw_cx, "JavaScript MODULE EVAL error", code, print_eval_error);
                eprintln!("Module script execution error: {}", msg);
                return Err(msg);
            }

            maybe_resume_unwind();
        }

        self.run_pending_jobs();
        Ok(())
    }

    /// Run all pending jobs in the job queue (for Promises)
    /// This handles microtasks like Promise callbacks and needs proper error handling
    pub fn run_pending_jobs(&mut self) {
        use crate::js::jsapi::promise::perform_microtask_checkpoint;

        self.do_with_jsapi(|cx, _global| {
            let executed = perform_microtask_checkpoint(cx);
            if executed > 0 {
                log::trace!("Executed {} promise jobs", executed);
            }
        });
    }

    /// Check if there are pending promise jobs
    pub fn has_pending_promise_jobs(&self) -> bool {
        crate::js::jsapi::promise::has_pending_promise_jobs()
    }

    /// Check if there are any active timers
    pub fn has_active_timers(&self) -> bool {
        self.timer_manager.has_active_timers()
    }

    /// Get the time until the next timer should fire
    pub fn time_until_next_timer(&self) -> Option<Duration> {
        self.timer_manager.time_until_next_timer()
    }

    /// Process pending timers (setTimeout/setInterval callbacks)
    /// Returns true if any timers were executed
    pub fn process_timers(&mut self) -> bool {
        let timer_manager = self.timer_manager.clone();
        let had_timers = timer_manager.process_timers(self);
        let had_fetch_settlements = crate::js::bindings::fetch::process_pending_fetches(self);

        if had_timers || had_fetch_settlements {
            // After timer callbacks or async fetch settlements, run pending promise jobs.
            self.run_pending_jobs();
        }

        had_timers || had_fetch_settlements
    }

    /// Get the runtime reference
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }


    pub fn do_with_jsapi<C, R>(&mut self, consumer: C) -> R
    where
        C: FnOnce(&mut JSContext, HandleObject) -> R,
    {
        let rt = &mut self.runtime;
        let cx = rt.cx();
        let global = unsafe { self.global.handle() };
        let safe_global = unsafe { mozjs::gc::Handle::from_raw(global) };

        let ret;
        {
            let mut cx = AutoRealm::new_from_handle(cx, safe_global);
            ret = consumer(&mut cx, safe_global);
        }
        ret
    }

    pub fn add_global_function<F>(&mut self, name: &'static str, func: F)
    where
        F: Fn(*mut RawJSContext, CallArgs) -> bool + Send + 'static,
    {
        GLOBAL_OPS.with(move |global_ops_rc| {
            let global_ops = &mut *global_ops_rc.borrow_mut();
            global_ops.insert(name, Box::new(func));
        });

        self.do_with_jsapi(|cx, global| unsafe {
            define_native_function(
                cx,
                global,
                name,
                Some(global_op_native_method),
            )
        });
    }

    pub fn do_in_es_event_queue<J>(&self, job: J)
    where
        J: FnOnce(&JsRuntime) + Send + 'static,
    {
        trace!("do_in_spidermonkey_runtime_thread");
        // this is executed in the single thread in the Threadpool, therefore Runtime and global are stored in a thread_local

        let async_job = || {
            RUNTIME.with(|engine| unsafe {
                let mut engine = engine.borrow_mut();
                if let Some(engine) = engine.as_mut() {
                    job(&mut &**engine)
                }
            });
        };

        self.event_loop.add_void(async_job);
    }

    pub fn do_in_es_event_queue_sync<R: Send + 'static, J>(&self, job: J) -> R
    where
        J: FnOnce(&JsRuntime) -> R + Send + 'static,
    {
        trace!("do_in_spidermonkey_runtime_thread_sync");
        // this is executed in the single thread in the Threadpool, therefore Runtime and global are stored in a thread_local

        let job = || {
            RUNTIME.with(|engine| unsafe {
                let mut engine = engine.borrow_mut();
                let engine = engine
                    .as_mut()
                    .expect("JsRuntime event queue called without an active runtime pointer");
                job(&mut &**engine)
            })
        };

        self.event_loop.exe(job)
    }
}

impl Drop for JsRuntime {
    fn drop(&mut self) {
        // Prevent later callbacks from dereferencing a stale thread-local runtime pointer.
        let self_ptr = self as *mut JsRuntime;
        RUNTIME.with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some(ptr) = *slot {
                if ptr == self_ptr {
                    *slot = None;
                }
            }
        });
    }
}

unsafe extern "C" fn empty(_extra: *const c_void) -> bool {
    false
}

/// Callback for getting host-defined data associated with promises.
/// Returns true with null data since we don't use host-defined data.
unsafe extern "C" fn get_host_defined_data(
    _extra: *const c_void,
    _cx: *mut ApiJSContext,
    data: mozjs::jsapi::MutableHandleObject,
) -> bool {
    // Set data to null - we don't have any host-defined data
    data.set(std::ptr::null_mut());
    true
}

unsafe extern "C" fn global_op_native_method(
    cx: *mut ApiJSContext,
    argc: u32,
    vp: *mut mozjs::jsapi::Value
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut cx.to_safe_cx();
    let callee = args.callee();
    let prop_name = get_obj_prop_val_as_string(
        safe_cx,
        HandleObject::from_marked_location(&callee),
        "name",
    );
    if let Ok(prop_name) = prop_name {
        return GLOBAL_OPS.with(|global_ops_ref| {
            let global_ops = &*global_ops_ref.borrow();
            let boxed_op = global_ops.get(prop_name.as_str()).expect("could not find op");
            boxed_op(cx, args)
        });
    }
    false
}

unsafe extern "C" fn module_metadata_hook(
    raw_cx: *mut ApiJSContext,
    private_value: mozjs::jsapi::HandleValue,
    meta_object: mozjs::jsapi::HandleObject,
) -> bool {
    let safe_cx = &mut raw_cx.to_safe_cx();
    rooted!(in(raw_cx) let module_private = private_value.get());
    if !module_private.get().is_string() {
        return true;
    }

    let cname = std::ffi::CString::new("url").unwrap();
    rooted!(in(raw_cx) let meta_object = meta_object.get());
    JS_DefineProperty(
        safe_cx,
        meta_object.handle(),
        cname.as_ptr(),
        module_private.handle().into(),
        mozjs::jsapi::JSPROP_ENUMERATE as u32,
    )
}

/// Helper to convert a Rust string to a SourceText for SpiderMonkey
unsafe fn transform_u16_to_source_text(code: &str) -> SourceText<u16> {
    let utf16: Vec<u16> = code.encode_utf16().collect();
    let mut source = SourceText {
        units_: ptr::null_mut(),
        length_: 0,
        ownsUnits_: false,
        _phantom_0: std::marker::PhantomData,
    };
    // Note: In a real implementation, we'd use SourceText methods properly
    // This is a simplified version
    source.units_ = utf16.as_ptr() as *mut _;
    source.length_ = utf16.len() as u32;
    std::mem::forget(utf16); // Leak to prevent deallocation
    source
}
