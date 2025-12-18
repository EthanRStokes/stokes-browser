use super::{initialize_bindings, JsResult, TimerManager};
// JavaScript runtime management using Mozilla's SpiderMonkey (mozjs)
use mozjs::jsval::{JSVal, PrivateValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::{JSEngine, Runtime, RealmOptions, SIMPLE_GLOBAL_CLASS, CompileOptionsWrapper, transform_str_to_source_text};
use std::ptr;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Duration;
use mozjs::context::JSContext;
use mozjs::conversions::jsstr_to_string;
use mozjs::jsapi::{JSObject, JS_ClearPendingException, JS_GetPendingException, JS_GetStringLength, JS_IsExceptionPending, MutableHandleValue, OnNewGlobalHookOption, Heap, SetScriptPrivate, JSScript, JSContext as RawJSContext, Compile1, JS_ExecuteScript, Handle};
use mozjs::panic::maybe_resume_unwind;
use mozjs::rust::wrappers2::{JS_NewGlobalObject, JS_ValueToSource, JS_GetScriptPrivate};
use url::Url;
use crate::dom::Dom;

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
    runtime: Runtime,
    engine: JSEngine,
}

impl JsRuntime {
    /// Create a new JavaScript runtime
    pub fn new(dom: *mut Dom, user_agent: String) -> JsResult<Self> {
        // Initialize the JS engine
        let engine = JSEngine::init().map_err(|_| "Failed to initialize JS engine".to_string())?;

        let mut runtime = Runtime::new(engine.handle());

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
            runtime,
            engine,
        };
        let user_agent = js_runtime.user_agent.clone();

        // Enter the realm for the global object before setting up bindings
        js_runtime.enter_realm_and_initialize(dom, user_agent, timer_manager)?;

        Ok(js_runtime)
    }

    /// Enter the realm and initialize bindings
    fn enter_realm_and_initialize(&mut self, dom: *mut Dom, user_agent: String, timer_manager: Rc<TimerManager>) -> JsResult<()> {
        // Get raw pointers before entering the realm to avoid borrow conflicts
        let raw_cx = unsafe { self.runtime.cx().raw_cx() };
        let global_ptr = self.global.get();

        unsafe {
            rooted!(in(raw_cx) let global_root = global_ptr);
            let _realm = mozjs::jsapi::JSAutoRealm::new(raw_cx, global_root.get());

            // Set up timers
            super::timers::setup_timers(self, timer_manager)?;

            initialize_bindings(self, dom, user_agent)?;
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
    pub fn execute(&mut self, code: &str) -> JsResult<()> {
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
            let _realm = mozjs::jsapi::JSAutoRealm::new(raw_cx, global.get());

            rooted!(in(raw_cx) let mut rval = UndefinedValue());
            let rval = rval.handle_mut();

            let dom_ref = &*self.dom;
            let url = Url::from(&dom_ref.url);

            // Compile the script first
            rooted!(in(raw_cx) let mut compiled_script = ptr::null_mut::<JSScript>());
            compiled_script.set(Self::compile_script(cx, code, "", 1));

            if compiled_script.is_null() {
                // Handle compilation error
                if JS_IsExceptionPending(raw_cx) {
                    rooted!(in(raw_cx) let mut exception = UndefinedValue());
                    if JS_GetPendingException(raw_cx, MutableHandleValue::from(exception.handle_mut())) {
                        JS_ClearPendingException(raw_cx);
                        rooted!(in(raw_cx) let exc_str = JS_ValueToSource(cx, exception.handle()));
                        if !exc_str.get().is_null() {
                            let msg = jsstr_to_string(raw_cx, NonNull::new(exc_str.handle().get()).unwrap());
                            return Err(format!("JavaScript COMPILE error: {}\n{}", msg, code));
                        }
                    }
                }
                return Err("JavaScript compilation failed".to_string());
            }

            let script = NonNull::new(*compiled_script).expect("Can't be null");

            if !Self::evaluate_script(raw_cx, script, url, MutableHandleValue::from(rval)) {
                // Handle evaluation error
                if JS_IsExceptionPending(raw_cx) {
                    rooted!(in(raw_cx) let mut exception = UndefinedValue());
                    if JS_GetPendingException(raw_cx, MutableHandleValue::from(exception.handle_mut())) {
                        JS_ClearPendingException(raw_cx);
                        rooted!(in(raw_cx) let exc_str = JS_ValueToSource(cx, exception.handle()));
                        if !exc_str.get().is_null() {
                            let msg = jsstr_to_string(raw_cx, NonNull::new(exc_str.handle().get()).unwrap());
                            return Err(format!("JavaScript EVAL error: {}\n{}", msg, code));
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
        let options = unsafe { CompileOptionsWrapper::new(&context, filename, line_number) };

        // First try to compile the script as-is
        let result = unsafe { Compile1(context.raw_cx(), options.ptr, &mut transform_str_to_source_text(text)) };

        // If compilation failed and the code looks like a JSON object or array,
        // wrap it in parentheses to make it a valid expression.
        // This handles cases where a script is just a large JSON structure.
        if result.is_null() {
            let trimmed = text.trim();
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                // Clear any pending exception from the first compile attempt
                unsafe { JS_ClearPendingException(context.raw_cx()) };

                // Wrap as an assignment to window.__INLINE_DATA__ so other scripts can access it
                // This mimics how browsers handle inline JSON configuration scripts
                let wrapped = format!("(window.__INLINE_DATA__ = window.__INLINE_DATA__ || []).push({})", text);
                let options = unsafe { CompileOptionsWrapper::new(&context, filename, line_number) };
                return unsafe { Compile1(context.raw_cx(), options.ptr, &mut transform_str_to_source_text(&wrapped)) };
            }
        }

        result
    }

    fn evaluate_script(
        context: *mut RawJSContext,
        compiled_script: NonNull<JSScript>,
        url: Url,
        rval: MutableHandleValue,
    ) -> bool {
        rooted!(in(context) let record = compiled_script.as_ptr());
        rooted!(in(context) let mut script_private = UndefinedValue());

        unsafe { JS_GetScriptPrivate(*record, script_private.handle_mut()) };

        if script_private.is_undefined() {
            // Set script private data if needed
            let url = Rc::new(url);
            unsafe { SetScriptPrivate(*record, &PrivateValue(Rc::into_raw(url) as *const _)) };
        }

        unsafe { JS_ExecuteScript(context, Handle::from(record.handle()), rval) }
    }

    /// Execute JavaScript code from a script tag
    pub fn execute_script(&mut self, code: &str) -> JsResult<()> {
        match self.execute(code) {
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

    /// Run all pending jobs in the job queue (for Promises)
    /// Note: Currently a no-op because RunJobs requires a properly configured job queue.
    /// To enable promise support, a custom JobQueue needs to be set up via SetJobQueue.
    fn run_pending_jobs(&mut self) {
        // TODO: Implement proper job queue support for Promises
        // RunJobs requires SetJobQueue to be called first with a valid JobQueue implementation.
        // Without it, RunJobs will SIGSEGV.
    }

    /// Check if there are any active timers
    pub fn has_active_timers(&self) -> bool {
        self.timer_manager.has_active_timers()
    }

    /// Get the time until the next timer should fire
    pub fn time_until_next_timer(&self) -> Option<Duration> {
        self.timer_manager.time_until_next_timer()
    }

    /// Get the runtime reference
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }
}

/// Helper to convert a Rust string to a SourceText for SpiderMonkey
unsafe fn transform_u16_to_source_text(code: &str) -> mozjs::jsapi::SourceText<u16> {
    let utf16: Vec<u16> = code.encode_utf16().collect();
    let mut source = mozjs::jsapi::SourceText {
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
