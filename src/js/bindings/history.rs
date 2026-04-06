use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::helpers::{define_function, js_value_to_string, set_int_property, set_string_property, ToSafeCx};
use mozjs::jsapi::{CallArgs, CurrentGlobalOrNull, JSContext, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use std::os::raw::c_uint;

pub(crate) unsafe fn setup_history_bindings(
    cx: &mut mozjs::context::JSContext,
    global: *mut JSObject,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let history = JS_NewPlainObject(raw_cx));
    if history.get().is_null() {
        return Err("Failed to create history object".to_string());
    }

    define_function(cx, history.get(), "pushState", Some(history_push_state), 3)?;
    define_function(cx, history.get(), "replaceState", Some(history_replace_state), 3)?;
    define_function(cx, history.get(), "back", Some(history_back), 0)?;
    define_function(cx, history.get(), "forward", Some(history_forward), 0)?;
    define_function(cx, history.get(), "go", Some(history_go), 1)?;

    set_int_property(cx, history.get(), "length", 1)?;
    let state_name = std::ffi::CString::new("state").unwrap();
    rooted!(in(raw_cx) let state_val = NullValue());
    JS_DefineProperty(
        raw_cx,
        history.handle().into(),
        state_name.as_ptr(),
        state_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    rooted!(in(raw_cx) let history_val = ObjectValue(history.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("history").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        history_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

unsafe fn set_history_state_and_length(raw_cx: *mut JSContext, args: &CallArgs, increment_length: bool) {
    let safe_cx = &mut raw_cx.to_safe_cx();

    let history_obj = if args.thisv().is_object() && !args.thisv().is_null() {
        args.thisv().to_object()
    } else {
        rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
        if global.get().is_null() {
            return;
        }
        rooted!(in(raw_cx) let mut history_val = UndefinedValue());
        let history_name = std::ffi::CString::new("history").unwrap();
        if !JS_GetProperty(
            raw_cx,
            global.handle().into(),
            history_name.as_ptr(),
            history_val.handle_mut().into(),
        ) || !history_val.get().is_object() {
            return;
        }
        history_val.get().to_object()
    };

    rooted!(in(raw_cx) let history_rooted = history_obj);

    if args.argc_ >= 1 {
        let state_name = std::ffi::CString::new("state").unwrap();
        rooted!(in(raw_cx) let state_val = *args.get(0));
        JS_DefineProperty(
            raw_cx,
            history_rooted.handle().into(),
            state_name.as_ptr(),
            state_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    if increment_length {
        rooted!(in(raw_cx) let mut length_val = UndefinedValue());
        let length_name = std::ffi::CString::new("length").unwrap();
        if JS_GetProperty(
            raw_cx,
            history_rooted.handle().into(),
            length_name.as_ptr(),
            length_val.handle_mut().into(),
        ) {
            let next = if length_val.get().is_int32() {
                length_val.get().to_int32().saturating_add(1)
            } else if length_val.get().is_double() {
                (length_val.get().to_double() as i32).saturating_add(1)
            } else {
                1
            };
            let _ = set_int_property(safe_cx, history_rooted.get(), "length", next);
        }
    }
}

unsafe fn maybe_update_location_from_history_arg(raw_cx: *mut JSContext, args: &CallArgs, url_arg_index: usize) {
    if (args.argc_ as usize) <= url_arg_index {
        return;
    }

    let safe_cx = &mut raw_cx.to_safe_cx();
    let url_str = js_value_to_string(safe_cx, *args.get(url_arg_index as u32));
    if url_str.is_empty() {
        return;
    }

    let resolved_url = DOM_REF.with(|dom_ref| {
        dom_ref
            .borrow()
            .as_ref()
            .and_then(|dom_ptr| {
                let dom = unsafe { &**dom_ptr };
                dom.url.resolve_relative(&url_str)
            })
    });

    let Some(resolved_url) = resolved_url else {
        return;
    };

    let hostname = resolved_url.host_str().unwrap_or("").to_string();
    let port = resolved_url.port().map(|p| p.to_string()).unwrap_or_default();
    let host = if port.is_empty() {
        hostname.clone()
    } else {
        format!("{}:{}", hostname, port)
    };
    let search = resolved_url.query().map(|query| format!("?{}", query)).unwrap_or_default();
    let hash = resolved_url.fragment().map(|fragment| format!("#{}", fragment)).unwrap_or_default();

    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
    if global.get().is_null() {
        return;
    }

    rooted!(in(raw_cx) let mut location_val = UndefinedValue());
    let location_name = std::ffi::CString::new("location").unwrap();
    if !JS_GetProperty(
        raw_cx,
        global.handle().into(),
        location_name.as_ptr(),
        location_val.handle_mut().into(),
    ) || !location_val.get().is_object() {
        return;
    }

    let location_obj = location_val.get().to_object();
    let _ = set_string_property(safe_cx, location_obj, "href", resolved_url.as_str());
    let _ = set_string_property(safe_cx, location_obj, "protocol", &format!("{}:", resolved_url.scheme()));
    let _ = set_string_property(safe_cx, location_obj, "host", &host);
    let _ = set_string_property(safe_cx, location_obj, "hostname", &hostname);
    let _ = set_string_property(safe_cx, location_obj, "port", &port);
    let _ = set_string_property(safe_cx, location_obj, "pathname", resolved_url.path());
    let _ = set_string_property(safe_cx, location_obj, "search", &search);
    let _ = set_string_property(safe_cx, location_obj, "hash", &hash);
    let _ = set_string_property(safe_cx, location_obj, "origin", &resolved_url.origin().ascii_serialization());
}

pub(crate) unsafe extern "C" fn history_push_state(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    set_history_state_and_length(raw_cx, &args, true);
    maybe_update_location_from_history_arg(raw_cx, &args, 2);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn history_replace_state(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    set_history_state_and_length(raw_cx, &args, false);
    maybe_update_location_from_history_arg(raw_cx, &args, 2);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn history_back(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn history_forward(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn history_go(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(UndefinedValue());
    true
}
