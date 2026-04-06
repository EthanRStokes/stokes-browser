use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::bindings::event_listeners;
use crate::js::helpers::ToSafeCx;
use crate::js::helpers::{define_function, define_js_property_getter, js_value_to_string};
use crate::js::JsRuntime;
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{CallArgs, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use std::os::raw::c_uint;
use tracing::trace;
use tracing::warn;

/// Set up the global `Window` constructor and window-level APIs on `globalThis`.
pub(crate) unsafe fn setup_window_bindings(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    _user_agent: &str,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global_val = ObjectValue(global));
    rooted!(in(raw_cx) let global_rooted = global);

    rooted!(in(raw_cx) let window_constructor = JS_NewPlainObject(raw_cx));
    if window_constructor.get().is_null() {
        return Err("Failed to create Window constructor".to_string());
    }

    rooted!(in(raw_cx) let window_prototype = JS_NewPlainObject(raw_cx));
    if window_prototype.get().is_null() {
        return Err("Failed to create Window prototype".to_string());
    }
    define_function(
        cx,
        window_prototype.get(),
        "addEventListener",
        Some(window_add_event_listener),
        3,
    )?;
    define_function(
        cx,
        window_prototype.get(),
        "removeEventListener",
        Some(window_remove_event_listener),
        3,
    )?;
    rooted!(in(raw_cx) let window_proto_val = ObjectValue(window_prototype.get()));
    let window_proto_name = std::ffi::CString::new("prototype").unwrap();
    JS_DefineProperty(
        raw_cx,
        window_constructor.handle().into(),
        window_proto_name.as_ptr(),
        window_proto_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    rooted!(in(raw_cx) let window_constructor_val = ObjectValue(window_constructor.get()));
    let name = std::ffi::CString::new("Window").unwrap();
    if !JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        window_constructor_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        return Err("Failed to define Window constructor".to_string());
    }

    for name in &["window", "self", "top", "parent", "globalThis", "frames"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        if !JS_DefineProperty(
            raw_cx,
            global_rooted.handle().into(),
            cname.as_ptr(),
            global_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        ) {
            return Err(format!("Failed to define global alias: {}", name));
        }
    }

    rooted!(in(raw_cx) let mut document_val = UndefinedValue());
    let document_name = std::ffi::CString::new("document").unwrap();
    if JS_GetProperty(
        raw_cx,
        global_rooted.handle().into(),
        document_name.as_ptr(),
        document_val.handle_mut().into(),
    ) && document_val.get().is_object()
    {
        rooted!(in(raw_cx) let document_obj = document_val.get().to_object());
        let default_view = std::ffi::CString::new("defaultView").unwrap();
        if !JS_DefineProperty(
            raw_cx,
            document_obj.handle().into(),
            default_view.as_ptr(),
            global_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        ) {
            return Err("Failed to define document.defaultView".to_string());
        }

        let parent_window = std::ffi::CString::new("parentWindow").unwrap();
        if !JS_DefineProperty(
            raw_cx,
            document_obj.handle().into(),
            parent_window.as_ptr(),
            global_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        ) {
            return Err("Failed to define document.parentWindow".to_string());
        }
    }

    define_function(cx, global, "alert", Some(window_alert), 1)?;
    define_function(cx, global, "confirm", Some(window_confirm), 1)?;
    define_function(cx, global, "prompt", Some(window_prompt), 2)?;
    define_function(
        cx,
        global,
        "requestAnimationFrame",
        Some(window_request_animation_frame),
        1,
    )?;
    define_function(
        cx,
        global,
        "cancelAnimationFrame",
        Some(window_cancel_animation_frame),
        1,
    )?;
    define_function(
        cx,
        global,
        "getComputedStyle",
        Some(window_get_computed_style),
        1,
    )?;
    define_function(cx, global, "addEventListener", Some(window_add_event_listener), 3)?;
    define_function(
        cx,
        global,
        "removeEventListener",
        Some(window_remove_event_listener),
        3,
    )?;
    define_function(cx, global, "scrollTo", Some(window_scroll_to), 2)?;
    define_function(cx, global, "scrollBy", Some(window_scroll_by), 2)?;
    define_function(
        cx,
        global,
        "__evaluateMediaQuery",
        Some(window_evaluate_media_query),
        1,
    )?;

    define_function(cx, global, "__getInnerWidth", Some(window_get_inner_width), 0)?;
    define_js_property_getter(cx, global, "innerWidth", "__getInnerWidth")?;
    define_function(cx, global, "__getInnerHeight", Some(window_get_inner_height), 0)?;
    define_js_property_getter(cx, global, "innerHeight", "__getInnerHeight")?;
    define_function(cx, global, "__getOuterWidth", Some(window_get_outer_width), 0)?;
    define_js_property_getter(cx, global, "outerWidth", "__getOuterWidth")?;
    define_function(cx, global, "__getOuterHeight", Some(window_get_outer_height), 0)?;
    define_js_property_getter(cx, global, "outerHeight", "__getOuterHeight")?;
    define_function(cx, global, "__getScreenX", Some(window_get_screen_x), 0)?;
    define_js_property_getter(cx, global, "screenX", "__getScreenX")?;
    define_function(cx, global, "__getScreenY", Some(window_get_screen_y), 0)?;
    define_js_property_getter(cx, global, "screenY", "__getScreenY")?;
    define_function(cx, global, "__getScrollX", Some(window_get_scroll_x), 0)?;
    define_js_property_getter(cx, global, "scrollX", "__getScrollX")?;
    define_js_property_getter(cx, global, "pageXOffset", "__getScrollX")?;
    define_function(cx, global, "__getScrollY", Some(window_get_scroll_y), 0)?;
    define_js_property_getter(cx, global, "scrollY", "__getScrollY")?;
    define_js_property_getter(cx, global, "pageYOffset", "__getScrollY")?;
    define_function(
        cx,
        global,
        "__getDevicePixelRatio",
        Some(window_get_device_pixel_ratio),
        0,
    )?;
    define_js_property_getter(cx, global, "devicePixelRatio", "__getDevicePixelRatio")?;

    Ok(())
}

pub(crate) unsafe extern "C" fn window_alert(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let message = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    super::alert_callback::trigger_alert(message);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn window_confirm(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let message = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    warn!("[JS] window.confirm('{}') called on partial binding (always returns false)", message);
    args.rval().set(BooleanValue(false));
    true
}

pub(crate) unsafe extern "C" fn window_prompt(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let message = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    warn!("[JS] window.prompt('{}') called on partial binding (always returns null)", message);
    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn window_request_animation_frame(_raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn!("[JS] requestAnimationFrame() called on partial binding (callback is not scheduled)");
    args.rval().set(Int32Value(1));
    true
}

pub(crate) unsafe extern "C" fn window_cancel_animation_frame(_raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    trace!("[JS] cancelAnimationFrame called");
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn window_get_computed_style(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    warn!("[JS] getComputedStyle called");

    rooted!(in(raw_cx) let style = JS_NewPlainObject(raw_cx));
    if !style.get().is_null() {
        let _ = define_function(safe_cx, style.get(), "getPropertyValue", Some(style_get_property_value), 1);
        args.rval().set(ObjectValue(style.get()));
    } else {
        args.rval().set(NullValue());
    }
    true
}

pub(crate) unsafe extern "C" fn style_get_property_value(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let property = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] style.getPropertyValue('{}') called", property);
    args.rval().set(crate::js::helpers::create_js_string(safe_cx, ""));
    true
}

pub(crate) unsafe extern "C" fn window_add_event_listener(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let event_type = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    if event_type.is_empty() || argc < 2 {
        args.rval().set(UndefinedValue());
        return true;
    }
    let callback_val = *args.get(1);
    if !callback_val.is_object() {
        args.rval().set(UndefinedValue());
        return true;
    }
    let use_capture = if argc >= 3 { let opt = *args.get(2); opt.is_boolean() && opt.to_boolean() } else { false };
    event_listeners::add_listener(safe_cx, event_listeners::WINDOW_NODE_ID, event_type, callback_val.to_object(), use_capture);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn window_remove_event_listener(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let event_type = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    if event_type.is_empty() || argc < 2 {
        args.rval().set(UndefinedValue());
        return true;
    }
    let callback_val = *args.get(1);
    if !callback_val.is_object() {
        args.rval().set(UndefinedValue());
        return true;
    }
    let use_capture = if argc >= 3 { let opt = *args.get(2); opt.is_boolean() && opt.to_boolean() } else { false };
    event_listeners::remove_listener(event_listeners::WINDOW_NODE_ID, &event_type, callback_val.to_object(), use_capture);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn window_scroll_to(_raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn!("[JS] window.scrollTo() called on partial binding (scroll state is not updated)");
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn window_scroll_by(_raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn!("[JS] window.scrollBy() called on partial binding (scroll state is not updated)");
    args.rval().set(UndefinedValue());
    true
}

pub(crate) fn setup_match_media_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        (function() {
            const root = typeof globalThis !== 'undefined' ? globalThis : window;

            function enrichChangeEvent(event, mql) {
                try {
                    Object.defineProperty(event, 'matches', {
                        value: mql.matches,
                        configurable: true,
                        enumerable: true,
                    });
                } catch (_err) {
                    event.matches = mql.matches;
                }

                try {
                    Object.defineProperty(event, 'media', {
                        value: mql.media,
                        configurable: true,
                        enumerable: true,
                    });
                } catch (_err) {
                    event.media = mql.media;
                }

                return event;
            }

            function createChangeEvent(mql) {
                let event;
                if (typeof root.Event === 'function') {
                    event = new root.Event('change');
                } else {
                    event = { type: 'change' };
                }
                return enrichChangeEvent(event, mql);
            }

            function assertDispatchEventArgument(event) {
                if (event == null || typeof event !== 'object') {
                    throw new TypeError("Failed to execute 'dispatchEvent' on 'MediaQueryList': parameter 1 is not of type 'Event'.");
                }

                if (typeof root.Event === 'function' && !(event instanceof root.Event)) {
                    throw new TypeError("Failed to execute 'dispatchEvent' on 'MediaQueryList': parameter 1 is not of type 'Event'.");
                }

                if (typeof event.type !== 'string') {
                    throw new TypeError("Failed to execute 'dispatchEvent' on 'MediaQueryList': parameter 1 is not of type 'Event'.");
                }
            }

            if (!(root.__matchMediaRegistry instanceof Set)) {
                root.__matchMediaRegistry = new Set();
            }

            root.matchMedia = function(query) {
                const mediaText = String(query == null ? '' : query);
                const listeners = new Set();
                let onchangeHandler = null;

                const mql = {
                    get matches() {
                        return !!root.__evaluateMediaQuery(mediaText);
                    },
                    media: mediaText,
                    get onchange() {
                        return onchangeHandler;
                    },
                    set onchange(handler) {
                        onchangeHandler = (typeof handler === 'function') ? handler : null;
                    },
                    addListener(listener) {
                        if (typeof listener === 'function') {
                            listeners.add(listener);
                        }
                    },
                    removeListener(listener) {
                        listeners.delete(listener);
                    },
                    addEventListener(type, listener) {
                        if (type === 'change' && typeof listener === 'function') {
                            listeners.add(listener);
                        }
                    },
                    removeEventListener(type, listener) {
                        if (type === 'change') {
                            listeners.delete(listener);
                        }
                    },
                    dispatchEvent(event) {
                        assertDispatchEventArgument(event);
                        if (event.type !== 'change') {
                            return true;
                        }

                        const enrichedEvent = enrichChangeEvent(event, mql);

                        for (const listener of Array.from(listeners)) {
                            try {
                                listener.call(mql, enrichedEvent);
                            } catch (_err) {}
                        }

                        if (typeof onchangeHandler === 'function') {
                            try {
                                onchangeHandler.call(mql, enrichedEvent);
                            } catch (_err) {}
                        }

                        return true;
                    },
                };

                mql.__lastMatches = mql.matches;
                root.__matchMediaRegistry.add(mql);
                return mql;
            };

            root.__notifyMatchMediaListeners = function() {
                for (const mql of Array.from(root.__matchMediaRegistry)) {
                    if (!mql) {
                        continue;
                    }

                    const next = !!mql.matches;
                    if (next !== mql.__lastMatches) {
                        mql.__lastMatches = next;
                        mql.dispatchEvent(createChangeEvent(mql));
                    }
                }
            };
        })();
    "#;

    runtime.execute(script, false).map_err(|e| {
        warn!("[JS] Failed to set up window.matchMedia: {}", e);
        e
    })?;

    Ok(())
}

pub(crate) unsafe fn setup_base64_functions(cx: &mut SafeJSContext, global: *mut JSObject) -> Result<(), String> {
    define_function(cx, global, "atob", Some(window_atob), 1)?;
    define_function(cx, global, "btoa", Some(window_btoa), 1)?;
    Ok(())
}

pub(crate) fn normalize_atob_input(input: &str) -> Option<String> {
    let mut normalized: String = input
        .chars()
        .filter(|c| !matches!(c, '\u{0009}' | '\u{000A}' | '\u{000C}' | '\u{000D}' | '\u{0020}'))
        .collect();

    if normalized.len() % 4 == 0 {
        if normalized.ends_with("==") {
            normalized.truncate(normalized.len() - 2);
        } else if normalized.ends_with('=') {
            normalized.truncate(normalized.len() - 1);
        }
    }

    if normalized.len() % 4 == 1 {
        return None;
    }

    if normalized
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '+' || c == '/'))
    {
        return None;
    }

    match normalized.len() % 4 {
        0 => {}
        2 => normalized.push_str("=="),
        3 => normalized.push('='),
        _ => return None,
    }

    Some(normalized)
}

pub(crate) fn decode_atob_binary_string(input: &str) -> Result<String, ()> {
    let normalized = normalize_atob_input(input).ok_or(())?;
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .map_err(|_| ())?;
    Ok(decoded.into_iter().map(char::from).collect())
}

pub(crate) unsafe extern "C" fn window_atob(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let encoded = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    match decode_atob_binary_string(&encoded) {
        Ok(decoded) => {
            args.rval().set(crate::js::helpers::create_js_string(safe_cx, &decoded));
            true
        }
        Err(()) => {
            warn!("[JS] atob() received invalid base64 input");
            args.rval().set(UndefinedValue());
            false
        }
    }
}

pub(crate) unsafe extern "C" fn window_btoa(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let data = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
    args.rval().set(crate::js::helpers::create_js_string(safe_cx, &encoded));
    true
}

fn get_window_width() -> i32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport.window_size.0 as i32;
        }
        1920
    })
}

fn get_window_height() -> i32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport.window_size.1 as i32;
        }
        1080
    })
}

fn get_scroll_x() -> i32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport_scroll.x as i32;
        }
        0
    })
}

fn get_scroll_y() -> i32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport_scroll.y as i32;
        }
        0
    })
}

fn get_device_pixel_ratio() -> f32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport.scale() as f32;
        }
        1.0
    })
}

pub(crate) unsafe extern "C" fn window_get_inner_width(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(Int32Value(get_window_width()));
    true
}

pub(crate) unsafe extern "C" fn window_get_inner_height(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(Int32Value(get_window_height()));
    true
}

pub(crate) unsafe extern "C" fn window_get_outer_width(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(Int32Value(get_window_width()));
    true
}

pub(crate) unsafe extern "C" fn window_get_outer_height(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(Int32Value(get_window_height()));
    true
}

pub(crate) unsafe extern "C" fn window_get_screen_x(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(Int32Value(0));
    true
}

pub(crate) unsafe extern "C" fn window_get_screen_y(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(Int32Value(0));
    true
}

pub(crate) unsafe extern "C" fn window_get_scroll_x(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(Int32Value(get_scroll_x()));
    true
}

pub(crate) unsafe extern "C" fn window_get_scroll_y(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(Int32Value(get_scroll_y()));
    true
}

pub(crate) unsafe extern "C" fn window_get_device_pixel_ratio(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(mozjs::jsval::DoubleValue(get_device_pixel_ratio() as f64));
    true
}

pub(crate) unsafe extern "C" fn window_evaluate_media_query(raw_cx: *mut mozjs::jsapi::JSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let query = if argc > 0 {
        crate::js::helpers::js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    let matches = evaluate_media_query(&query, get_window_width() as f32, get_window_height() as f32, get_device_pixel_ratio());
    args.rval().set(BooleanValue(matches));
    true
}

pub(crate) fn evaluate_media_query(query: &str, width: f32, height: f32, dpr: f32) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return false;
    }

    split_media_query_list(trimmed)
        .into_iter()
        .any(|part| evaluate_single_media_condition(part, width, height, dpr))
}

fn split_media_query_list(query: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;

    for (idx, ch) in query.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = query[start..idx].trim();
                if !part.is_empty() {
                    out.push(part);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }

    let tail = query[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }

    out
}

fn evaluate_single_media_condition(input: &str, width: f32, height: f32, dpr: f32) -> bool {
    let mut remaining = input.trim().to_ascii_lowercase();
    if remaining.is_empty() {
        return false;
    }

    let mut invert = false;
    if let Some(rest) = consume_keyword(&remaining, "not") {
        invert = true;
        remaining = rest.to_string();
    } else if let Some(rest) = consume_keyword(&remaining, "only") {
        remaining = rest.to_string();
    }

    if remaining.is_empty() {
        return false;
    }

    let mut media_type_matches = true;
    if !remaining.starts_with('(') {
        let mut split = remaining.splitn(2, char::is_whitespace);
        let media_type = split.next().unwrap_or_default();
        let rest = split.next().unwrap_or_default().trim_start();

        media_type_matches = match media_type {
            "all" | "screen" => true,
            "print" => false,
            _ => false,
        };

        remaining = rest.to_string();
    }

    let mut all_features_match = true;
    while !remaining.trim_start().is_empty() {
        remaining = remaining.trim_start().to_string();
        if let Some(rest) = consume_keyword(&remaining, "and") {
            remaining = rest.to_string();
        }

        remaining = remaining.trim_start().to_string();
        if !remaining.starts_with('(') {
            return false;
        }

        let closing = match find_matching_paren(&remaining) {
            Some(i) => i,
            None => return false,
        };

        let feature = &remaining[1..closing];
        let matches = evaluate_media_feature(feature.trim(), width, height, dpr);
        all_features_match &= matches;

        remaining = remaining[closing + 1..].to_string();
    }

    let result = media_type_matches && all_features_match;
    if invert { !result } else { result }
}

fn consume_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    if !input.starts_with(keyword) {
        return None;
    }

    let remainder = &input[keyword.len()..];
    let starts_with_ws = remainder.chars().next().map(|c| c.is_whitespace()).unwrap_or(false);
    if remainder.is_empty() || starts_with_ws || remainder.starts_with('(') {
        Some(remainder.trim_start())
    } else {
        None
    }
}

fn find_matching_paren(input: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn evaluate_media_feature(feature: &str, width: f32, height: f32, dpr: f32) -> bool {
    if feature.is_empty() {
        return false;
    }

    let mut parts = feature.splitn(2, ':');
    let name = parts.next().unwrap_or_default().trim();
    let value = parts.next().map(str::trim);

    match (name, value) {
        ("width", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| approx_eq(width, px)),
        ("min-width", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| width >= px),
        ("max-width", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| width <= px),
        ("height", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| approx_eq(height, px)),
        ("min-height", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| height >= px),
        ("max-height", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| height <= px),
        ("orientation", Some(v)) => {
            let orientation = if width >= height { "landscape" } else { "portrait" };
            orientation == v
        }
        ("prefers-color-scheme", Some(v)) => v == "light",
        ("prefers-reduced-motion", Some(v)) => v == "no-preference",
        ("resolution", Some(v)) => parse_resolution_dppx(v).is_some_and(|target| approx_eq(dpr, target)),
        ("min-resolution", Some(v)) => parse_resolution_dppx(v).is_some_and(|target| dpr >= target),
        ("max-resolution", Some(v)) => parse_resolution_dppx(v).is_some_and(|target| dpr <= target),
        ("color", None) | ("monochrome", None) => true,
        _ => false,
    }
}

fn parse_length_px(value: &str, width: f32, height: f32) -> Option<f32> {
    let s = value.trim().to_ascii_lowercase();
    let (num, unit) = split_number_and_unit(&s)?;

    let parsed = num.parse::<f32>().ok()?;
    let px = match unit {
        "" | "px" => parsed,
        "em" | "rem" => parsed * 16.0,
        "vw" => (parsed / 100.0) * width,
        "vh" => (parsed / 100.0) * height,
        _ => return None,
    };

    Some(px)
}

fn parse_resolution_dppx(value: &str) -> Option<f32> {
    let s = value.trim().to_ascii_lowercase();
    let (num, unit) = split_number_and_unit(&s)?;
    let parsed = num.parse::<f32>().ok()?;

    match unit {
        "dppx" | "x" => Some(parsed),
        "dpi" => Some(parsed / 96.0),
        "dpcm" => Some(parsed * 2.54 / 96.0),
        _ => None,
    }
}

fn split_number_and_unit(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut split_idx = 0usize;
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+' {
            split_idx = idx + ch.len_utf8();
            continue;
        }
        split_idx = idx;
        break;
    }

    if split_idx == 0 {
        return None;
    }

    let (num, unit) = trimmed.split_at(split_idx);
    Some((num.trim(), unit.trim()))
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}






