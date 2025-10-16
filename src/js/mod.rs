// JavaScript engine module
mod runtime;
mod console;
mod dom_bindings;
mod element_bindings;
mod fetch;
mod timers;

pub use runtime::JsRuntime;
pub use timers::TimerManager;

use crate::dom::DomNode;
use boa_engine::{Context, JsValue, Source};
use std::cell::RefCell;
use std::rc::Rc;

/// JavaScript execution result
pub type JsResult<T> = Result<T, String>;

const STACK_SIZE: usize = 16 * 1024 * 1024; // 16MB

/// Execute JavaScript code in a context
pub fn execute_script(context: &mut Context, code: &str) -> JsResult<JsValue> {
    stacker::grow(STACK_SIZE, || {
        context
            .eval(Source::from_bytes(code))
            .map_err(|e| format!("JavaScript error: {}", e))
    })
}

/// Initialize JavaScript bindings for the browser
pub fn initialize_bindings(context: &mut Context, document_root: Rc<RefCell<DomNode>>) -> JsResult<()> {
    // Set up console object
    console::setup_console(context)?;
    
    // Set up DOM bindings
    dom_bindings::setup_dom_bindings(context, document_root)?;
    
    // Set up fetch API
    fetch::setup_fetch(context)?;

    Ok(())
}
