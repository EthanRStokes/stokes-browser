use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::helpers::{create_js_string, define_function, js_value_to_string, set_string_property, ToSafeCx};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use mozjs::jsapi::{CallArgs, JS_DefineProperty, JS_NewPlainObject, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{JSVal, ObjectValue, UndefinedValue};
use mozjs::rooted;
use std::os::raw::c_uint;
use tracing::trace;

/// Set up the location object.
pub(crate) unsafe fn setup_location_bindings(
    cx: &mut mozjs::context::JSContext,
    global: *mut JSObject,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let location = JS_NewPlainObject(raw_cx));
    if location.get().is_null() {
        return Err("Failed to create location object".to_string());
    }

    let (href, protocol, host, hostname, port, pathname, search, hash, origin) = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = dom_ref.borrow().as_ref() {
            let dom = unsafe { &**dom_ptr };
            let url: url::Url = (&dom.url).into();
            let hostname = url.host_str().unwrap_or("").to_string();
            let port = url.port().map(|p| p.to_string()).unwrap_or_default();
            let host = if port.is_empty() {
                hostname.clone()
            } else {
                format!("{}:{}", hostname, port)
            };
            let search = url.query().map(|query| format!("?{}", query)).unwrap_or_default();
            let hash = url.fragment().map(|fragment| format!("#{}", fragment)).unwrap_or_default();

            (
                url.as_str().to_string(),
                format!("{}:", url.scheme()),
                host,
                hostname,
                port,
                url.path().to_string(),
                search,
                hash,
                url.origin().ascii_serialization(),
            )
        } else {
            (
                "about:blank".to_string(),
                "about:".to_string(),
                String::new(),
                String::new(),
                String::new(),
                "blank".to_string(),
                String::new(),
                String::new(),
                "null".to_string(),
            )
        }
    });

    set_string_property(cx, location.get(), "href", &href)?;
    set_string_property(cx, location.get(), "protocol", &protocol)?;
    set_string_property(cx, location.get(), "host", &host)?;
    set_string_property(cx, location.get(), "hostname", &hostname)?;
    set_string_property(cx, location.get(), "port", &port)?;
    set_string_property(cx, location.get(), "pathname", &pathname)?;
    set_string_property(cx, location.get(), "search", &search)?;
    set_string_property(cx, location.get(), "hash", &hash)?;
    set_string_property(cx, location.get(), "origin", &origin)?;

    define_function(cx, location.get(), "reload", Some(location_reload), 0)?;
    define_function(cx, location.get(), "assign", Some(location_assign), 1)?;
    define_function(cx, location.get(), "replace", Some(location_replace), 1)?;
    define_function(cx, location.get(), "toString", Some(location_to_string), 0)?;

    rooted!(in(raw_cx) let location_val = ObjectValue(location.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("location").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        location_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

pub(crate) unsafe extern "C" fn location_reload(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    trace!("[JS] location.reload() called");

    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = dom_ref.borrow().as_ref() {
            let dom = unsafe { &**dom_ptr };
            dom.nav_provider.reload();
        }
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn location_assign(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let url = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] location.assign('{}') called", url);

    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = dom_ref.borrow().as_ref() {
            let dom = unsafe { &**dom_ptr };
            if let Some(resolved) = dom.url.resolve_relative(&url) {
                dom.nav_provider.navigate_to(NavigationOptions::new(
                    resolved,
                    String::from("text/plain"),
                    dom.id(),
                ));
            }
        }
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn location_replace(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let url = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] location.replace('{}') called", url);

    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = dom_ref.borrow().as_ref() {
            let dom = unsafe { &**dom_ptr };
            if let Some(resolved) = dom.url.resolve_relative(&url) {
                dom.nav_provider.navigate_replace(NavigationOptions::new(
                    resolved,
                    String::from("text/plain"),
                    dom.id(),
                ));
            }
        }
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn location_to_string(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let href = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = dom_ref.borrow().as_ref() {
            let dom = unsafe { &**dom_ptr };
            let url: url::Url = (&dom.url).into();
            url.as_str().to_string()
        } else {
            "about:blank".to_string()
        }
    });

    args.rval().set(create_js_string(safe_cx, &href));
    true
}
