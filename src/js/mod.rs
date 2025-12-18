// JavaScript engine module (using Mozilla's SpiderMonkey via mozjs)
mod runtime;
mod console;
mod cookies;
mod dom_bindings;
mod element_bindings;
mod fetch;
mod helpers;
mod selectors;
mod timers;
mod alert_callback;
mod registry;

pub use runtime::JsRuntime;
pub use timers::TimerManager;
pub use alert_callback::{set_alert_callback, clear_alert_callback};
pub use registry::{get_node, register_node, unregister_node};
pub use cookies::{Cookie, CookieJar, get_cookies_for_request, set_cookie_from_response, clear_all_cookies};

use crate::dom::Dom;
use std::cell::RefCell;
use std::rc::Rc;

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
pub fn initialize_bindings(runtime: &mut JsRuntime, document_root: *mut Dom, user_agent: String) -> JsResult<()> {
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
