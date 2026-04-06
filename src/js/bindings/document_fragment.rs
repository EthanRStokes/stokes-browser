use crate::js::helpers::{set_int_property, ToSafeCx};
use mozjs::jsapi::{CallArgs, JS_GetProperty, JSContext};
use mozjs::jsval::{BooleanValue, JSVal, UndefinedValue};
use mozjs::rooted;
use std::os::raw::c_uint;

pub(crate) unsafe extern "C" fn document_fragment_has_child_nodes(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let result = if args.thisv().is_object() && !args.thisv().is_null() {
        rooted!(in(raw_cx) let this_obj = args.thisv().to_object());
        rooted!(in(raw_cx) let mut count_val = UndefinedValue());
        let count_name = std::ffi::CString::new("__childCount").unwrap();
        if JS_GetProperty(
            raw_cx,
            this_obj.handle().into(),
            count_name.as_ptr(),
            count_val.handle_mut().into(),
        ) {
            if count_val.get().is_int32() {
                count_val.get().to_int32() > 0
            } else if count_val.get().is_double() {
                count_val.get().to_double() > 0.0
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    args.rval().set(BooleanValue(result));
    true
}

pub(crate) unsafe extern "C" fn document_fragment_append_child(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if argc == 0 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let child = *args.get(0);
    if child.is_null() || child.is_undefined() {
        args.rval().set(child);
        return true;
    }

    if args.thisv().is_object() && !args.thisv().is_null() {
        rooted!(in(raw_cx) let this_obj = args.thisv().to_object());
        rooted!(in(raw_cx) let mut count_val = UndefinedValue());
        let count_name = std::ffi::CString::new("__childCount").unwrap();
        let mut count = 0;
        if JS_GetProperty(
            raw_cx,
            this_obj.handle().into(),
            count_name.as_ptr(),
            count_val.handle_mut().into(),
        ) {
            if count_val.get().is_int32() {
                count = count_val.get().to_int32();
            } else if count_val.get().is_double() {
                count = count_val.get().to_double() as i32;
            }
        }
        let _ = set_int_property(
            safe_cx,
            this_obj.get(),
            "__childCount",
            count.saturating_add(1),
        );
    }

    args.rval().set(child);
    true
}

