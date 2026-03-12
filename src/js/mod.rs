// JavaScript engine module (using Mozilla's SpiderMonkey via mozjs)
mod runtime;
mod helpers;
mod selectors;
pub(crate) mod bindings;
mod jsapi;

pub use bindings::alert_callback::set_alert_callback;
pub use runtime::JsRuntime;
/// JavaScript execution result
pub type JsResult<T> = Result<T, String>;

const STACK_SIZE: usize = 16 * 1024 * 1024; // 16MB

/// Execute JavaScript code in the runtime
pub fn execute_script(runtime: &mut JsRuntime, code: &str, debug_js: bool) -> JsResult<()> {
    stacker::grow(STACK_SIZE, || {
        runtime.execute_script(code, debug_js)
    })
}

