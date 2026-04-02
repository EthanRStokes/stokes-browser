use std::ptr;
use std::rc::Rc;
use mozjs::glue::CreateJobQueue;
use mozjs::rust::wrappers2::SetJobQueue;
use crate::dom::Dom;
use crate::js::bindings::timers::TimerManager;
use crate::js::{JsResult, JsRuntime};
use crate::js::jsapi::promise::init_rejection_tracker;
use crate::js::runtime::JOB_QUEUE_TRAPS;

pub(crate) mod cookies;
pub(crate) mod dom_bindings;
pub(crate) mod element_bindings;
pub(crate) mod mutation_observer;
pub(crate) mod registry;
pub(crate) mod timers;
pub(crate) mod alert_callback;
pub(crate) mod warnings;
pub mod abort_signal;
pub mod console;
pub mod css;
pub mod crypto;
pub mod event_listeners;
pub mod event_target;
pub mod fetch;
pub mod performance;
pub mod text_encoding;
pub mod url;
pub mod xhr;

/// Initialize JavaScript bindings for the browser
pub fn initialize_bindings(runtime: &mut JsRuntime, document_root: *mut Dom, user_agent: String, timer_manager: Rc<TimerManager>) -> JsResult<()> {
    let job_queue = unsafe { CreateJobQueue(&JOB_QUEUE_TRAPS, ptr::null_mut(), ptr::null_mut()) };
    runtime.do_with_jsapi(|cx, global| unsafe {
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

    // Set up Web Crypto API (crypto.getRandomValues/randomUUID/subtle.digest)
    crypto::setup_crypto(runtime)?;

    // Set up TextEncoder (UTF-8 encoding support)
    text_encoding::setup_text_encoder(runtime)?;

    // Set up DOM bindings
    dom_bindings::setup_dom_bindings(runtime, document_root, user_agent)?;

    // Set up EventTarget bindings (EventTarget/Event/CustomEvent constructors)
    event_target::setup_event_target(runtime)?;

    // Set up callable SVGElement/SVGSVGElement constructors
    dom_bindings::setup_svg_constructors_deferred(runtime)?;

    // Set up Event and CustomEvent constructors
    dom_bindings::setup_event_constructors_deferred(runtime)?;

    // Set up MutationObserver / MutationRecord native bindings
    mutation_observer::setup_mutation_observer(runtime)?;

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

    // Set up document.implementation and DOMImplementation methods
    dom_bindings::setup_document_implementation_deferred(runtime)?;

    // Set up the global Image / HTMLImageElement constructor
    dom_bindings::setup_image_constructor_deferred(runtime)?;

    // Set up HTMLInputElement constructor/prototype wiring
    dom_bindings::setup_html_input_element_constructor_deferred(runtime)?;

    // Set up XMLHttpRequest constructor (full polyfill)
    xhr::setup_xhr(runtime)?;

    // Set up AbortSignal and AbortController
    abort_signal::setup_abort_signal(runtime)?;


    Ok(())
}