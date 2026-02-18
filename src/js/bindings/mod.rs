use std::ptr;
use std::rc::Rc;
use mozjs::glue::CreateJobQueue;
use mozjs::jsapi::SetJobQueue;
use crate::dom::Dom;
use crate::js::bindings::timers::TimerManager;
use crate::js::{JsResult, JsRuntime};
use crate::js::jsapi::promise::init_rejection_tracker;
use crate::js::runtime::JOB_QUEUE_TRAPS;

pub(crate) mod cookies;
pub(crate) mod dom_bindings;
pub(crate) mod element_bindings;
pub(crate) mod registry;
pub(crate) mod timers;
pub(crate) mod alert_callback;
pub mod console;
pub mod fetch;
pub mod performance;

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

    // Set up performance API
    performance::setup_performance(runtime)?;

    // Set up fetch API
    fetch::setup_fetch(runtime, user_agent.clone())?;

    // Set up DOM bindings
    dom_bindings::setup_dom_bindings(runtime, document_root, user_agent)?;

    // Set up document.cookie property (must be done after DOM bindings are set up)
    // This uses Object.defineProperty which requires the document object to exist
    dom_bindings::setup_cookie_property_deferred(runtime)?;

    // Set up document.head property (must be done after DOM bindings are set up)
    dom_bindings::setup_head_property_deferred(runtime)?;

    Ok(())
}