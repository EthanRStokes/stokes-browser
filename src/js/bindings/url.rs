use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::helpers::{create_empty_array, create_js_string, define_function, define_property_accessor, js_value_to_string, set_string_property, ToSafeCx};
use crate::js::JsRuntime;
use mozjs::conversions::jsstr_to_string;
use mozjs::jsapi::{
    CallArgs, HandleObject, JSContext, JSObject, JSPROP_ENUMERATE,
};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use std::ffi::CString;
use std::os::raw::c_uint;
use std::ptr::NonNull;
use mozjs::gc::Handle;
use mozjs::rust::wrappers2::{JS_DefineFunction, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject};
use url::Url;

/// Register global URL-related constructors.
pub fn setup_url(runtime: &mut JsRuntime) -> Result<(), String> {
    runtime.do_with_jsapi(|cx, global| unsafe {
        let url_name = CString::new("URL").unwrap();
        if JS_DefineFunction(
            cx,
            global.into(),
            url_name.as_ptr(),
            Some(url_constructor),
            2,
            JSPROP_ENUMERATE as u32,
        )
        .is_null()
        {
            return Err("Failed to define URL constructor".to_string());
        }

        let params_name = CString::new("URLSearchParams").unwrap();
        if JS_DefineFunction(
            cx,
            global.into(),
            params_name.as_ptr(),
            Some(url_search_params_constructor),
            1,
            JSPROP_ENUMERATE as u32,
        )
        .is_null()
        {
            return Err("Failed to define URLSearchParams constructor".to_string());
        }

        Ok(())
    })?;

    // SpiderMonkey native functions defined from Rust may not be `new`-able.
    // Rebind to JS constructor wrappers so `new URL(...)` and `new URLSearchParams(...)` work.
    runtime.execute(
        r#"
        (function () {
            const __nativeURL = globalThis.URL;
            const __nativeURLSearchParams = globalThis.URLSearchParams;

            globalThis.URL = function URL(input, base) {
                return __nativeURL(input, base);
            };

            globalThis.URLSearchParams = function URLSearchParams(init) {
                return __nativeURLSearchParams(init);
            };
        })();
        "#,
        false,
    )?;

    Ok(())
}

unsafe extern "C" fn url_constructor(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc < 1 {
        args.rval().set(UndefinedValue());
        return false;
    }

    let input = extract_url_argument(safe_cx, *args.get(0));

    // Resolve the base: use the explicit argument when present and not undefined/null,
    // otherwise fall back to the document URL (mirrors browser behaviour).
    let base: Option<String> = if argc > 1 {
        let base_val = *args.get(1);
        if base_val.is_undefined() || base_val.is_null() {
            document_base_url()
        } else {
            let s = extract_url_argument(safe_cx, base_val);
            if s.is_empty() { document_base_url() } else { Some(s) }
        }
    } else {
        document_base_url()
    };

    let resolved = match resolve_url(&input, base.as_deref()) {
        Ok(url) => url,
        Err(err) => {
            eprintln!("[JS] URL parse error: {}", err);
            args.rval().set(UndefinedValue());
            return false;
        }
    };

    rooted!(in(raw_cx) let url_obj = JS_NewPlainObject(safe_cx));
    if url_obj.get().is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    if define_function(safe_cx, url_obj.get(), "toString", Some(url_to_string), 0).is_err() {
        args.rval().set(UndefinedValue());
        return false;
    }
    if define_function(safe_cx, url_obj.get(), "toJSON", Some(url_to_json), 0).is_err() {
        args.rval().set(UndefinedValue());
        return false;
    }

    if set_url_properties(safe_cx, url_obj.get(), &resolved).is_err() {
        args.rval().set(UndefinedValue());
        return false;
    }

    args.rval().set(ObjectValue(url_obj.get()));
    true
}

unsafe extern "C" fn url_to_string(raw_cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, 0);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if let Some(href) = get_this_href(safe_cx, &args) {
        args.rval().set(create_js_string(safe_cx, &href));
        true
    } else {
        args.rval().set(UndefinedValue());
        false
    }
}

unsafe extern "C" fn url_to_json(cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    url_to_string(cx, 0, vp)
}

unsafe extern "C" fn url_search_params_constructor(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let raw = if argc > 0 {
        params_raw_from_value(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    match create_url_search_params_object(safe_cx, &raw) {
        Ok(obj) => {
            args.rval().set(ObjectValue(obj));
            true
        }
        Err(_) => {
            args.rval().set(UndefinedValue());
            false
        }
    }
}

unsafe extern "C" fn url_search_params_append(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(safe_cx, *args.get(1))
    } else {
        String::new()
    };

    let mut pairs = params_pairs_from_this(safe_cx, &args);
    pairs.push((name, value));
    let _ = write_params_pairs_to_this(safe_cx, &args, &pairs);

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn url_search_params_set(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(safe_cx, *args.get(1))
    } else {
        String::new()
    };

    let mut pairs = params_pairs_from_this(safe_cx, &args);
    let first_match_index = pairs.iter().position(|(k, _)| *k == name);
    pairs.retain(|(k, _)| *k != name);

    if let Some(index) = first_match_index {
        let index = index.min(pairs.len());
        pairs.insert(index, (name, value));
    } else {
        pairs.push((name, value));
    }

    let _ = write_params_pairs_to_this(safe_cx, &args, &pairs);

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn url_search_params_get_size(
    cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let safe_cx = &mut cx.to_safe_cx();
    let args = CallArgs::from_vp(vp, argc);
    let size = params_pairs_from_this(safe_cx, &args).len() as i32;
    args.rval().set(Int32Value(size));
    true
}

unsafe extern "C" fn url_search_params_get(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    let value = params_pairs_from_this(safe_cx, &args)
        .into_iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v);

    if let Some(v) = value {
        args.rval().set(create_js_string(safe_cx, &v));
    } else {
        args.rval().set(NullValue());
    }

    true
}

unsafe extern "C" fn url_search_params_get_all(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx  = &mut raw_cx.to_safe_cx();
    let name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    let pairs = params_pairs_from_this(safe_cx, &args);
    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));

    let mut idx = 0_u32;
    for (k, v) in pairs {
        if k == name {
            rooted!(in(raw_cx) let val = create_js_string(safe_cx, &v));
            rooted!(in(raw_cx) let array_obj = array.get());
            mozjs::rust::wrappers::JS_SetElement(
                raw_cx,
                array_obj.handle().into(),
                idx,
                val.handle().into(),
            );
            idx += 1;
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

unsafe extern "C" fn url_search_params_delete(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let safe_cx = &mut raw_cx.to_safe_cx();
    let args = CallArgs::from_vp(vp, argc);
    let name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    let mut pairs = params_pairs_from_this(safe_cx, &args);
    pairs.retain(|(k, _)| *k != name);
    let _ = write_params_pairs_to_this(safe_cx, &args, &pairs);

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn url_search_params_has(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    let has = params_pairs_from_this(safe_cx, &args)
        .into_iter()
        .any(|(k, _)| k == name);

    args.rval().set(BooleanValue(has));
    true
}

unsafe extern "C" fn url_search_params_to_string(
    raw_cx: *mut JSContext,
    _argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, 0);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let encoded = serialize_params_pairs(&params_pairs_from_this(safe_cx, &args));
    args.rval().set(create_js_string(safe_cx, &encoded));
    true
}

fn resolve_url(input: &str, base: Option<&str>) -> Result<Url, String> {
    if let Some(base) = base {
        let parsed_base =
            Url::parse(base).map_err(|e| format!("Invalid base URL '{}': {}", base, e))?;
        parsed_base
            .join(input)
            .map_err(|e| format!("Failed to resolve URL '{}' against '{}': {}", input, base, e))
    } else {
        Url::parse(input).map_err(|e| format!("Invalid URL '{}': {}", input, e))
    }
}

/// Return the current document URL as a string, used as the default base for relative URL resolution.
fn document_base_url() -> Option<String> {
    DOM_REF.with(|dom_ref| {
        dom_ref.borrow().as_ref().map(|dom_ptr| {
            let dom = unsafe { &**dom_ptr };
            let url: Url = (&dom.url).into();
            url.as_str().to_string()
        })
    })
}

unsafe fn extract_url_argument(cx: &mut SafeJSContext, val: JSVal) -> String {
    let raw_cx = cx.raw_cx();
    // undefined and null mean "no value" — fall back to the document URL.
    if val.is_undefined() || val.is_null() {
        return document_base_url().unwrap_or_default();
    }

    if val.is_object() {
        rooted!(in(raw_cx) let obj = val.to_object());
        if let Some(href) = get_string_property(cx, obj.handle().into(), "href") {
            return href;
        }
    }

    let raw = js_value_to_string(cx, val);
    if !raw.is_empty() {
        return raw;
    }

    document_base_url().unwrap_or_default()
}

unsafe fn set_url_properties(cx: &mut SafeJSContext, obj: *mut JSObject, url: &Url) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    let protocol = format!("{}:", url.scheme());
    let hostname = url.host_str().unwrap_or("");
    let port = url.port().map(|p| p.to_string()).unwrap_or_default();
    let host = if port.is_empty() {
        hostname.to_string()
    } else {
        format!("{}:{}", hostname, port)
    };
    let search = url.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let hash = url
        .fragment()
        .map(|fragment| format!("#{}", fragment))
        .unwrap_or_default();

    set_string_property(cx, obj, "href", url.as_str())?;
    set_string_property(cx, obj, "protocol", &protocol)?;
    set_string_property(cx, obj, "username", url.username())?;
    set_string_property(cx, obj, "password", url.password().unwrap_or(""))?;
    set_string_property(cx, obj, "host", &host)?;
    set_string_property(cx, obj, "hostname", hostname)?;
    set_string_property(cx, obj, "port", &port)?;
    set_string_property(cx, obj, "pathname", url.path())?;
    set_string_property(cx, obj, "search", &search)?;
    set_string_property(cx, obj, "hash", &hash)?;
    set_string_property(cx, obj, "origin", &url.origin().ascii_serialization())?;

    let params_obj = create_url_search_params_object(cx, url.query().unwrap_or(""))?;
    rooted!(in(raw_cx) let params_val = ObjectValue(params_obj));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let name = CString::new("searchParams").unwrap();
    JS_DefineProperty(
        cx,
        obj_rooted.handle(),
        name.as_ptr(),
        params_val.handle(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

unsafe fn get_this_href(cx: &mut SafeJSContext, args: &CallArgs) -> Option<String> {
    let raw_cx = cx.raw_cx();
    let this_val = args.thisv();
    if !this_val.get().is_object() || this_val.get().is_null() {
        return None;
    }

    rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
    get_string_property(cx, this_obj.handle().into(), "href")
}

fn normalize_params_raw(raw: &str) -> String {
    raw.strip_prefix('?').unwrap_or(raw).to_string()
}

fn parse_params_pairs(raw: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(normalize_params_raw(raw).as_bytes())
        .into_owned()
        .collect()
}

fn serialize_params_pairs(pairs: &[(String, String)]) -> String {
    let mut out = String::new();
    url::form_urlencoded::Serializer::new(&mut out)
        .extend_pairs(pairs.iter().map(|(k, v)| (&k[..], &v[..])));
    out
}

unsafe fn params_raw_from_value(cx: &mut SafeJSContext, value: JSVal) -> String {
    let raw_cx = cx.raw_cx();
    if value.is_undefined() || value.is_null() {
        return String::new();
    }

    if value.is_object() && !value.is_null() {
        rooted!(in(raw_cx) let obj = value.to_object());
        if let Some(raw) = get_string_property(cx, obj.handle().into(), "__paramsRaw") {
            return normalize_params_raw(&raw);
        }
        if let Some(search) = get_string_property(cx, obj.handle().into(), "search") {
            return normalize_params_raw(&search);
        }
    }

    normalize_params_raw(&js_value_to_string(cx, value))
}

unsafe fn create_url_search_params_object(
    cx: &mut SafeJSContext,
    raw: &str,
) -> Result<*mut JSObject, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let obj = JS_NewPlainObject(cx));
    if obj.get().is_null() {
        return Err("Failed to create URLSearchParams object".to_string());
    }

    set_string_property(cx, obj.get(), "__paramsRaw", &normalize_params_raw(raw))?;
    define_function(cx, obj.get(), "append", Some(url_search_params_append), 2)?;
    define_function(cx, obj.get(), "set", Some(url_search_params_set), 2)?;
    define_function(cx, obj.get(), "get", Some(url_search_params_get), 1)?;
    define_function(cx, obj.get(), "getAll", Some(url_search_params_get_all), 1)?;
    define_function(cx, obj.get(), "delete", Some(url_search_params_delete), 1)?;
    define_function(cx, obj.get(), "has", Some(url_search_params_has), 1)?;
    define_function(cx, obj.get(), "toString", Some(url_search_params_to_string), 0)?;
    define_function(cx, obj.get(), "__getSize", Some(url_search_params_get_size), 0)?;
    define_property_accessor(cx, obj.get(), "size", "__getSize", "__getSize")?;

    Ok(obj.get())
}

unsafe fn params_pairs_from_this(cx: &mut SafeJSContext, args: &CallArgs) -> Vec<(String, String)> {
    let raw_cx = cx.raw_cx();
    let this_val = args.thisv();
    if !this_val.get().is_object() || this_val.get().is_null() {
        return Vec::new();
    }

    rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
    let raw = get_string_property(cx, this_obj.handle().into(), "__paramsRaw").unwrap_or_default();
    parse_params_pairs(&raw)
}

unsafe fn write_params_pairs_to_this(
    cx: &mut SafeJSContext,
    args: &CallArgs,
    pairs: &[(String, String)],
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    let this_val = args.thisv();
    if !this_val.get().is_object() || this_val.get().is_null() {
        return Ok(());
    }

    rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
    set_string_property(cx, this_obj.get(), "__paramsRaw", &serialize_params_pairs(pairs))?;
    Ok(())
}

unsafe fn get_string_property(cx: &mut SafeJSContext, obj: Handle<*mut JSObject>, name: &str) -> Option<String> {
    let raw_cx = cx.raw_cx();
    let name_cstr = CString::new(name).ok()?;
    rooted!(in(raw_cx) let mut val = UndefinedValue());

    if !JS_GetProperty(cx, obj, name_cstr.as_ptr(), val.handle_mut()) {
        return None;
    }

    if val.is_undefined() || val.is_null() {
        return None;
    }

    if !val.is_string() {
        return Some(js_value_to_string(cx, *val));
    }

    let js_str = val.to_string();
    if js_str.is_null() {
        return None;
    }

    Some(jsstr_to_string(raw_cx, NonNull::new(js_str).unwrap()))
}
