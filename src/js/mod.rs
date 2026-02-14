// JavaScript engine module (using Mozilla's SpiderMonkey via mozjs)
mod runtime;
mod console;
mod helpers;
mod selectors;
mod bindings;
mod jsapi;

use std::ptr;
use std::rc::Rc;
use mozjs::glue::CreateJobQueue;
use mozjs::jsapi::SetJobQueue;
use crate::dom::Dom;
use crate::js::bindings::{dom_bindings, fetch, timers};
pub use bindings::alert_callback::set_alert_callback;
pub use runtime::JsRuntime;
use crate::js::bindings::timers::TimerManager;
use crate::js::jsapi::promise::init_rejection_tracker;
use crate::js::runtime::JOB_QUEUE_TRAPS;

/// JavaScript execution result
pub type JsResult<T> = Result<T, String>;

const STACK_SIZE: usize = 16 * 1024 * 1024; // 16MB

/// Execute JavaScript code in the runtime
pub fn execute_script(runtime: &mut JsRuntime, code: &str) -> JsResult<()> {
    stacker::grow(STACK_SIZE, || {
        runtime.execute_script(code)
    })
}

/// Initialize JavaScript bindings for the browser
pub fn initialize_bindings(runtime: &mut JsRuntime, document_root: *mut Dom, user_agent: String, timer_manager: Rc<TimerManager>) -> JsResult<()> {
    let job_queue = unsafe { CreateJobQueue(&JOB_QUEUE_TRAPS, ptr::null_mut(), ptr::null_mut()) };
    runtime.do_with_jsapi(|rt, cx, global| unsafe {
        SetJobQueue(cx, job_queue);
        init_rejection_tracker(cx);
    });
    // Set up timers
    timers::setup_timers(runtime, timer_manager)?;

    // Set up console object
    console::setup_console(runtime)?;

    // Set up DOM bindings
    dom_bindings::setup_dom_bindings(runtime, document_root, user_agent)?;

    // Set up fetch API
    fetch::setup_fetch(runtime)?;

    // Set up document.cookie property (must be done after DOM bindings are set up)
    // This uses Object.defineProperty which requires the document object to exist
    dom_bindings::setup_cookie_property_deferred(runtime)?;

    Ok(())
}
