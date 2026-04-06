use crate::js::bindings::dom_bindings::{LOCAL_STORAGE, SESSION_STORAGE};
use crate::js::helpers::{create_js_string, define_function, define_js_property_getter, js_value_to_string, ToSafeCx};
use mozjs::jsapi::{CallArgs, JSContext, JS_DefineProperty, JS_NewPlainObject, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{JSVal, ObjectValue, UInt32Value, UndefinedValue};
use mozjs::rooted;
use std::os::raw::c_uint;

pub(crate) unsafe fn setup_storage_bindings(
    cx: &mut mozjs::context::JSContext,
    global: *mut JSObject,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global_rooted = global);

    rooted!(in(raw_cx) let local_storage = JS_NewPlainObject(raw_cx));
    if local_storage.get().is_null() {
        return Err("Failed to create localStorage object".to_string());
    }

    define_function(cx, local_storage.get(), "getItem", Some(local_storage_get_item), 1)?;
    define_function(cx, local_storage.get(), "setItem", Some(local_storage_set_item), 2)?;
    define_function(cx, local_storage.get(), "removeItem", Some(local_storage_remove_item), 1)?;
    define_function(cx, local_storage.get(), "clear", Some(local_storage_clear), 0)?;
    define_function(cx, local_storage.get(), "key", Some(local_storage_key), 1)?;
    define_function(cx, local_storage.get(), "__getLength", Some(local_storage_length), 0)?;
    define_js_property_getter(cx, local_storage.get(), "length", "__getLength")?;

    rooted!(in(raw_cx) let local_storage_val = ObjectValue(local_storage.get()));
    let name = std::ffi::CString::new("localStorage").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        local_storage_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    rooted!(in(raw_cx) let session_storage = JS_NewPlainObject(raw_cx));
    if session_storage.get().is_null() {
        return Err("Failed to create sessionStorage object".to_string());
    }

    define_function(cx, session_storage.get(), "getItem", Some(session_storage_get_item), 1)?;
    define_function(cx, session_storage.get(), "setItem", Some(session_storage_set_item), 2)?;
    define_function(cx, session_storage.get(), "removeItem", Some(session_storage_remove_item), 1)?;
    define_function(cx, session_storage.get(), "clear", Some(session_storage_clear), 0)?;
    define_function(cx, session_storage.get(), "key", Some(session_storage_key), 1)?;
    define_function(cx, session_storage.get(), "__getLength", Some(session_storage_length), 0)?;
    define_js_property_getter(cx, session_storage.get(), "length", "__getLength")?;

    rooted!(in(raw_cx) let session_storage_val = ObjectValue(session_storage.get()));
    let name = std::ffi::CString::new("sessionStorage").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        session_storage_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

pub(crate) unsafe extern "C" fn local_storage_get_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let key = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    let value = LOCAL_STORAGE.with(|storage| storage.borrow().get(&key).cloned());

    if let Some(val) = value {
        args.rval().set(create_js_string(safe_cx, &val));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

pub(crate) unsafe extern "C" fn local_storage_set_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let key = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(safe_cx, *args.get(1))
    } else {
        String::new()
    };

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().insert(key, value);
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn local_storage_remove_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let key = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().remove(&key);
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn local_storage_clear(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().clear();
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn local_storage_key(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let index = if argc > 0 {
        let val = *args.get(0);
        if val.is_int32() {
            val.to_int32() as usize
        } else if val.is_double() {
            val.to_double() as usize
        } else {
            0
        }
    } else {
        0
    };

    let key = LOCAL_STORAGE.with(|storage| {
        let storage = storage.borrow();
        storage.keys().nth(index).cloned()
    });

    if let Some(k) = key {
        args.rval().set(create_js_string(safe_cx, &k));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

pub(crate) unsafe extern "C" fn local_storage_length(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let length = LOCAL_STORAGE.with(|storage| {
        let storage = storage.borrow();
        storage.len()
    });

    args.rval().set(UInt32Value(length as u32));
    true
}

pub(crate) unsafe extern "C" fn session_storage_get_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let key = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    let value = SESSION_STORAGE.with(|storage| storage.borrow().get(&key).cloned());

    if let Some(val) = value {
        args.rval().set(create_js_string(safe_cx, &val));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

pub(crate) unsafe extern "C" fn session_storage_set_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let key = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(safe_cx, *args.get(1))
    } else {
        String::new()
    };

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().insert(key, value);
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn session_storage_remove_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let key = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().remove(&key);
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn session_storage_clear(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().clear();
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn session_storage_key(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let index = if argc > 0 {
        let val = *args.get(0);
        if val.is_int32() {
            val.to_int32() as usize
        } else if val.is_double() {
            val.to_double() as usize
        } else {
            0
        }
    } else {
        0
    };

    let key = SESSION_STORAGE.with(|storage| {
        let storage = storage.borrow();
        storage.keys().nth(index).cloned()
    });

    if let Some(k) = key {
        args.rval().set(create_js_string(safe_cx, &k));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

pub(crate) unsafe extern "C" fn session_storage_length(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let length = SESSION_STORAGE.with(|storage| {
        let storage = storage.borrow();
        storage.len()
    });

    args.rval().set(UInt32Value(length as u32));
    true
}
