use mozjs::gc::HandleObject;
use mozjs::jsapi::{JSContext, JSFunction, JSNative, JS_DefineFunction};

/// define a new native function on an object
// todo refactor to accept MutableHandleValue #26
pub fn define_native_function(
    cx: *mut JSContext,
    obj: HandleObject,
    function_name: &str,
    native_function: JSNative,
) -> *mut JSFunction {
    let n = format!("{}\0", function_name);

    let ret: *mut JSFunction = unsafe {
        JS_DefineFunction(
            cx,
            obj.into(),
            n.as_ptr() as *const libc::c_char,
            native_function,
            1,
            0,
        )
    };

    ret

    //https://developer.mozilla.org/en-US/docs/Mozilla/Projects/SpiderMonkey/JSAPI_reference/JS_DefineFunction
}