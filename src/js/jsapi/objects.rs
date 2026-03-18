use crate::js::jsapi::error::{get_pending_exception, JsError};
use mozjs::conversions::jsstr_to_string;
use mozjs::glue::{RUST_JSID_IS_STRING, RUST_JSID_TO_STRING};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsval::{JSVal, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::{HandleObject, MutableHandleValue};
use std::ptr::NonNull;
use mozjs::rust::wrappers2::JS_GetProperty;

/// get a single member of a JSObject
#[allow(dead_code)]
pub fn get_obj_prop_val(
    context: &mut SafeJSContext,
    obj: HandleObject,
    prop_name: &str,
    ret_val: MutableHandleValue,
) -> Result<(), JsError> {
    get_obj_prop_val_raw(context, obj, prop_name, ret_val)
}

/// get a single member of a JSObject
#[allow(dead_code)]
pub fn get_obj_prop_val_raw(
    context: &mut SafeJSContext,
    obj: HandleObject,
    prop_name: &str,
    ret_val: MutableHandleValue,
) -> Result<(), JsError> {
    let n = format!("{}\0", prop_name);
    let ok = unsafe {
        JS_GetProperty(
            context,
            obj,
            n.as_ptr() as *const libc::c_char,
            ret_val.into(),
        )
    };

    if !ok {
        if let Some(err) = get_pending_exception(context) {
            return Err(err);
        }
    }

    Ok(())
}

// get a property of a JSObject as String
pub fn get_obj_prop_val_as_string(
    context: &mut SafeJSContext,
    obj: HandleObject,
    prop_name: &str,
) -> Result<String, &'static str> {
    let raw_cx = unsafe { context.raw_cx() };
    rooted!(in (raw_cx) let mut rval = UndefinedValue());
    let res = get_obj_prop_val(context, obj, prop_name, rval.handle_mut().into());
    if res.is_err() {
        panic!("{}", res.err().unwrap().message);
    }

    value_to_str(context, *rval)
}

pub fn get_obj_prop_val_as_i32(
    context: &mut SafeJSContext,
    obj: HandleObject,
    prop_name: &str,
) -> i32 {
    let raw_cx = unsafe { context.raw_cx() };
    rooted!(in (raw_cx) let mut rval = UndefinedValue());
    let res = get_obj_prop_val(context, obj, prop_name, rval.handle_mut().into());
    if res.is_err() {
        panic!("{}", res.err().unwrap().message);
    }

    let val: JSVal = *rval;
    val.to_int32()
}

/// convert a StringValue to a rust string
// todo, refactor to use HandleValue
#[allow(dead_code)]
pub fn value_to_str(
    context: &mut SafeJSContext,
    val: mozjs::jsapi::Value,
) -> Result<String, &'static str> {
    if val.is_string() {
        let jsa: *mut mozjs::jsapi::JSString = val.to_string();
        Ok(jsstring_to_string(context, jsa))
    } else {
        Err("value was not a String")
    }
}

/// convert a JSString to a rust string
pub fn jsstring_to_string(
    context: &mut SafeJSContext,
    js_string: *mut mozjs::jsapi::JSString,
) -> String {
    unsafe { jsstr_to_string(context.raw_cx(), NonNull::new(js_string).unwrap()) }
}

// convert a PropertyKey or JSID to String
pub fn jsid_to_string(context: &mut SafeJSContext, id: mozjs::jsapi::HandleId) -> String {
    assert!(unsafe { RUST_JSID_IS_STRING(id) });
    rooted!(in(unsafe { context.raw_cx() }) let id_str = unsafe{RUST_JSID_TO_STRING(id)});
    jsstring_to_string(context, *id_str)
}