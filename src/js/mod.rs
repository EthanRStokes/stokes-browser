// JavaScript engine module
mod runtime;
mod console;
mod dom_bindings;
mod element_bindings;
mod fetch;

pub use runtime::JsRuntime;
pub use console::Console;
pub use element_bindings::ElementWrapper;

use std::rc::Rc;
use std::cell::RefCell;
use crate::dom::DomNode;

/// JavaScript execution result
pub type JsResult<T> = Result<T, String>;

/// Initialize JavaScript bindings for the browser
pub fn initialize_bindings(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    document_root: Rc<RefCell<DomNode>>
) -> JsResult<()> {
    // Set up console object
    console::setup_console(scope, global)?;

    // Set up DOM bindings
    dom_bindings::setup_dom_bindings(scope, global, document_root)?;

    // Set up fetch API
    fetch::setup_fetch(scope, global)?;

    Ok(())
}
