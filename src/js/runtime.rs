use super::{JsResult, TimerManager};
// JavaScript runtime management using Mozilla's SpiderMonkey (mozjs)
use mozjs::jsval::{JSVal, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::{JSEngine, Runtime, RealmOptions, SIMPLE_GLOBAL_CLASS};
use std::cell::RefCell;
use std::ffi::CString;
use std::ptr;
use std::rc::Rc;
use std::time::Duration;
use mozjs::context::JSContext;
use mozjs::jsapi::{CurrentGlobalOrNull, JSObject, JS_ClearPendingException, JS_GetPendingException, JS_GetStringLength, JS_IsExceptionPending, MutableHandleValue, ReadOnlyCompileOptions};
use mozjs::rust::wrappers2::{JS_GetTwoByteStringCharsAndLength, JS_ValueToSource, RunJobs};

// Stack size for growing when needed (16MB to handle very large scripts)
const STACK_SIZE: usize = 16 * 1024 * 1024;
// Red zone threshold (32KB)
const _RED_ZONE: usize = 32 * 1024;

/// JavaScript runtime that manages execution context
pub struct JsRuntime {
    engine: JSEngine,
    runtime: Runtime,
    timer_manager: Rc<TimerManager>,
    user_agent: String,
}

impl JsRuntime {
    /// Create a new JavaScript runtime
    pub fn new(user_agent: String) -> JsResult<Self> {
        // Initialize the JS engine
        let engine = JSEngine::init().map_err(|_| "Failed to initialize JS engine".to_string())?;
        //let engine = Rc::new(engine);

        let runtime = Runtime::new(engine.handle());

        // Create and set up timer manager
        let timer_manager = Rc::new(TimerManager::new());

        let mut js_runtime = Self {
            engine,
            runtime,
            timer_manager: timer_manager.clone(),
            user_agent,
        };

        // Set up timers
        super::timers::setup_timers(&mut js_runtime, timer_manager)?;

        Ok(js_runtime)
    }

    /// Get the raw JSContext pointer
    pub fn cx(&mut self) -> &mut JSContext {
        self.runtime.cx()
    }

    /// Get the global object
    pub fn global(&mut self) -> *mut JSObject {
        unsafe {
            let raw_cx = self.cx().raw_cx();
            rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
            global.get()
        }
    }

    /// Execute JavaScript code
    pub fn execute(&mut self, code: &str) -> JsResult<JSVal> {
        let cx = self.runtime.cx();
        let raw_cx = unsafe { cx.raw_cx() };

        unsafe {
            rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
            if global.get().is_null() {
                return Err("No global object".to_string());
            }

            rooted!(in(raw_cx) let mut rval = UndefinedValue());

            let code_cstring = CString::new(code).map_err(|e| format!("Invalid JS code: {}", e))?;
            let filename = CString::new("script").unwrap();

            let opts = CompileOptions::new(cx, filename.as_ptr(), 1);
            let result = mozjs::rust::wrappers::Evaluate(
                raw_cx,
                opts,
                &mut transform_u16_to_source_text(code),
                rval.handle_mut(),
            );

            if result {
                Ok(rval.get())
            } else {
                // Get the exception if there was one
                if JS_IsExceptionPending(raw_cx) {
                    rooted!(in(raw_cx) let mut exception = UndefinedValue());
                    if JS_GetPendingException(raw_cx, MutableHandleValue::from(exception.handle_mut())) {
                        JS_ClearPendingException(raw_cx);
                        // Try to convert exception to string
                        rooted!(in(raw_cx) let exc_str = JS_ValueToSource(cx, exception.handle()));
                        if !exc_str.get().is_null() {
                            let chars = JS_GetTwoByteStringCharsAndLength(cx, *exc_str.handle(), &mut JS_GetStringLength(exc_str.get()));
                            if !chars.is_null() {
                                let len = JS_GetStringLength(exc_str.get());
                                let slice = std::slice::from_raw_parts(chars, len as usize);
                                let msg = String::from_utf16_lossy(slice);
                                return Err(format!("JavaScript error: {}", msg));
                            }
                        }
                    }
                }
                Err("JavaScript execution failed".to_string())
            }
        }
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
    fn run_pending_jobs(&mut self) {
        let cx = self.runtime.cx();
        unsafe {
            // Run microtask checkpoint to process promise jobs
            for _ in 0..100 {
                RunJobs(cx);
            }
        }
    }

    /// Process pending timers and execute callbacks that are ready
    /// Returns true if any timers were executed
    pub fn process_timers(&mut self) -> bool {
        self.timer_manager.process_timers(&mut self)
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

/// Helper struct for compile options
struct CompileOptions {
    filename: *const std::ffi::c_char,
    lineno: u32,
}

impl CompileOptions {
    fn new(_cx: *mut JSContext, filename: *const std::ffi::c_char, lineno: u32) -> Self {
        Self { filename, lineno }
    }
}
