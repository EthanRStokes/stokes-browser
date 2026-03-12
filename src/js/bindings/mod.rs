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
pub mod css;
pub mod event_listeners;
pub mod fetch;
pub mod google;
pub mod performance;
pub mod url;
pub mod xhr;

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

    // Set up URL API
    url::setup_url(runtime)?;

    // Set up CSS namespace object (CSS.supports, CSS.escape, CSS Typed OM, etc.)
    css::setup_css(runtime)?;

    // Set up DOM bindings
    dom_bindings::setup_dom_bindings(runtime, document_root, user_agent)?;

    // Inject google.* polyfill stubs so that Google-hosted scripts that call
    // google.cv, google.rll, google.ml etc. do not throw before they can set
    // up their own real implementations.
    google::setup_google_polyfill(runtime)?;

    // Set up window.matchMedia and MediaQueryList behavior
    dom_bindings::setup_match_media_deferred(runtime)?;

    // Set up document.cookie property (must be done after DOM bindings are set up)
    // This uses Object.defineProperty which requires the document object to exist
    dom_bindings::setup_cookie_property_deferred(runtime)?;

    // Set up document.head property (must be done after DOM bindings are set up)
    dom_bindings::setup_head_property_deferred(runtime)?;

    // Set up document.body property (must be done after DOM bindings are set up)
    dom_bindings::setup_body_property_deferred(runtime)?;

    // Set up document.currentScript property (must be done after DOM bindings are set up)
    dom_bindings::setup_current_script_deferred(runtime)?;

    // Set up the global Image / HTMLImageElement constructor
    dom_bindings::setup_image_constructor_deferred(runtime)?;

    // Set up XMLHttpRequest constructor (full polyfill)
    xhr::setup_xhr(runtime)?;

    Ok(())
}