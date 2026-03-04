// JavaScript engine module (using Mozilla's SpiderMonkey via mozjs)
mod runtime;
mod helpers;
mod selectors;
pub(crate) mod bindings;
mod jsapi;

pub use runtime::JsRuntime;

pub(crate) fn with_runtime_mut<R>(f: impl FnOnce(&mut JsRuntime) -> R) -> Option<R> {
    runtime::RUNTIME.with(|runtime| {
        let mut runtime_ref = runtime.borrow_mut();
        let runtime_ptr = runtime_ref.as_mut()?;
        Some(unsafe { f(&mut **runtime_ptr) })
    })
}

/// JavaScript execution result
pub type JsResult<T> = Result<T, String>;

const STACK_SIZE: usize = 16 * 1024 * 1024; // 16MB

/// Execute JavaScript code in the runtime
pub fn execute_script(runtime: &mut JsRuntime, code: &str) -> JsResult<()> {
    stacker::grow(STACK_SIZE, || {
        runtime.execute_script(code)
    })
}
