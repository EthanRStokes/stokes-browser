use crate::js::helpers::{set_bool_property, set_string_property};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{JS_DefineProperty, JS_NewPlainObject, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::ObjectValue;
use mozjs::rooted;

/// Set up the navigator object.
pub(crate) unsafe fn setup_navigator_bindings(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    user_agent: &str,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let navigator = JS_NewPlainObject(raw_cx));
    if navigator.get().is_null() {
        return Err("Failed to create navigator object".to_string());
    }

    set_string_property(cx, navigator.get(), "userAgent", user_agent)?;
    set_string_property(cx, navigator.get(), "language", "en-US")?;
    set_string_property(cx, navigator.get(), "platform", std::env::consts::OS)?;
    set_string_property(cx, navigator.get(), "appName", "Stokes Browser")?;
    set_string_property(cx, navigator.get(), "appVersion", "1.0")?;
    set_string_property(cx, navigator.get(), "vendor", "Stokes")?;
    set_bool_property(cx, navigator.get(), "onLine", true)?;
    set_bool_property(cx, navigator.get(), "cookieEnabled", true)?;

    rooted!(in(raw_cx) let navigator_val = ObjectValue(navigator.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("navigator").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        navigator_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}
