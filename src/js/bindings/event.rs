use crate::js::helpers::{js_value_to_string, set_bool_property, set_string_property, ToSafeCx};
use crate::js::JsRuntime;
use mozjs::jsapi::{
    CallArgs, JSContext, JS_DefineProperty, JSPROP_ENUMERATE,
};
use mozjs::jsval::{JSVal, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::HandleObject;
use mozjs::rust::wrappers2::JS_DefineFunction;
use std::os::raw::c_uint;
use tracing::warn;

unsafe extern "C" fn event_constructor(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let event_type = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    rooted!(in(raw_cx) let event_obj = mozjs::jsapi::JS_NewPlainObject(raw_cx));
    if event_obj.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    set_string_property(safe_cx, event_obj.get(), "type", &event_type).ok();
    set_bool_property(safe_cx, event_obj.get(), "bubbles", false).ok();
    set_bool_property(safe_cx, event_obj.get(), "cancelable", false).ok();
    set_bool_property(safe_cx, event_obj.get(), "composed", false).ok();

    args.rval().set(ObjectValue(event_obj.get()));
    true
}

unsafe extern "C" fn custom_event_constructor(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let event_type = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    let detail = if argc > 1 && args.get(1).is_object() {
        *args.get(1)
    } else {
        UndefinedValue()
    };

    rooted!(in(raw_cx) let event_obj = mozjs::jsapi::JS_NewPlainObject(raw_cx));
    if event_obj.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    set_string_property(safe_cx, event_obj.get(), "type", &event_type).ok();
    set_bool_property(safe_cx, event_obj.get(), "bubbles", false).ok();
    set_bool_property(safe_cx, event_obj.get(), "cancelable", false).ok();
    set_bool_property(safe_cx, event_obj.get(), "composed", false).ok();

    rooted!(in(raw_cx) let detail_val = detail);
    rooted!(in(raw_cx) let event_obj_rooted = event_obj.get());
    let detail_name = std::ffi::CString::new("detail").unwrap();
    JS_DefineProperty(
        raw_cx,
        event_obj_rooted.handle().into(),
        detail_name.as_ptr(),
        detail_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    args.rval().set(ObjectValue(event_obj.get()));
    true
}

pub(crate) unsafe fn setup_event_constructors(raw_cx: *mut JSContext, global: HandleObject) -> Result<(), String> {
    let event_name = std::ffi::CString::new("Event").unwrap();
    if JS_DefineFunction(
        &mut raw_cx.to_safe_cx(),
        global,
        event_name.as_ptr(),
        Some(event_constructor),
        1,
        JSPROP_ENUMERATE as u32,
    )
    .is_null()
    {
        return Err("Failed to define Event constructor".to_string());
    }

    let custom_event_name = std::ffi::CString::new("CustomEvent").unwrap();
    if JS_DefineFunction(
        &mut raw_cx.to_safe_cx(),
        global,
        custom_event_name.as_ptr(),
        Some(custom_event_constructor),
        1,
        JSPROP_ENUMERATE as u32,
    )
    .is_null()
    {
        return Err("Failed to define CustomEvent constructor".to_string());
    }

    Ok(())
}
