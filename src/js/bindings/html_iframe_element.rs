use crate::js::bindings::warnings::{warn_stubbed_binding, warn_unexpected_nullish_return};
use crate::js::helpers::{create_js_string, define_function, define_js_property_accessor, define_js_property_getter, js_value_to_string, ToSafeCx};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{CallArgs, JS_DefineProperty, JS_NewPlainObject, JSContext, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{JSVal, ObjectValue, UndefinedValue};
use mozjs::rooted;
use std::os::raw::c_uint;
use tracing::trace;

pub(crate) unsafe fn setup_html_iframe_element_constructor_bindings(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();

    rooted!(in(raw_cx) let html_iframe_element = JS_NewPlainObject(raw_cx));
    if html_iframe_element.get().is_null() {
        return Err("Failed to create HTMLIFrameElement constructor".to_string());
    }

    rooted!(in(raw_cx) let prototype = JS_NewPlainObject(raw_cx));
    if prototype.get().is_null() {
        return Err("Failed to create HTMLIFrameElement prototype".to_string());
    }

    define_function(
        cx,
        prototype.get(),
        "__getContentWindow",
        Some(html_iframe_element_get_content_window),
        0,
    )?;
    define_function(
        cx,
        prototype.get(),
        "__getContentDocument",
        Some(html_iframe_element_get_content_document),
        0,
    )?;

    define_function(cx, prototype.get(), "__getSrc", Some(html_iframe_element_get_src), 0)?;
    define_function(cx, prototype.get(), "__setSrc", Some(html_iframe_element_set_src), 1)?;

    define_js_property_getter(
        cx,
        prototype.get(),
        "contentWindow",
        "__getContentWindow",
    )?;
    define_js_property_getter(
        cx,
        prototype.get(),
        "contentDocument",
        "__getContentDocument",
    )?;
    define_js_property_accessor(cx, prototype.get(), "src", "__getSrc", "__setSrc")?;

    rooted!(in(raw_cx) let prototype_val = ObjectValue(prototype.get()));
    rooted!(in(raw_cx) let html_iframe_element_rooted = html_iframe_element.get());
    let prototype_name = std::ffi::CString::new("prototype").unwrap();
    JS_DefineProperty(
        raw_cx,
        html_iframe_element_rooted.handle().into(),
        prototype_name.as_ptr(),
        prototype_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    rooted!(in(raw_cx) let html_iframe_element_val = ObjectValue(html_iframe_element.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("HTMLIFrameElement").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        html_iframe_element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

unsafe extern "C" fn html_iframe_element_get_content_window(
    _raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    trace!("[JS] HTMLIFrameElement.contentWindow getter called");
    warn_stubbed_binding(
        "HTMLIFrameElement.contentWindow getter",
        "iframe browsing contexts are not implemented yet",
    );
    warn_unexpected_nullish_return(
        "HTMLIFrameElement.contentWindow getter",
        "null",
        "Window object",
        "no child browsing-context wiring exists yet",
    );

    args.rval().set(mozjs::jsval::NullValue());
    true
}

unsafe extern "C" fn html_iframe_element_get_content_document(
    _raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    trace!("[JS] HTMLIFrameElement.contentDocument getter called");
    warn_stubbed_binding(
        "HTMLIFrameElement.contentDocument getter",
        "iframe browsing contexts are not implemented yet",
    );
    warn_unexpected_nullish_return(
        "HTMLIFrameElement.contentDocument getter",
        "null",
        "Document object",
        "no child browsing-context wiring exists yet",
    );

    args.rval().set(mozjs::jsval::NullValue());
    true
}

unsafe extern "C" fn html_iframe_element_get_src(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] HTMLIFrameElement.src getter called");

    args.rval().set(create_js_string(safe_cx, ""));
    true
}

unsafe extern "C" fn html_iframe_element_set_src(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let src = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] HTMLIFrameElement.src setter called with value: {}", src);

    args.rval().set(UndefinedValue());
    true
}

