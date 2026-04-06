use crate::js::bindings::element_bindings::{
    element_after, element_animate, element_append,
    element_attach_shadow, element_before, element_blur, element_click,
    element_closest, element_focus,
    element_get_async_attr, element_get_attribute, element_get_attribute_names,
    element_get_bounding_client_rect, element_get_checked_attr, element_get_class_list_object, element_get_class_name,
    element_get_client_height, element_get_client_rects, element_get_client_width,
    element_get_dataset_object, element_get_id,
    element_get_offset_height, element_get_offset_left,
    element_get_offset_top, element_get_offset_width,
    element_get_scroll_height, element_get_scroll_left, element_get_scroll_top,
    element_get_scroll_width, element_get_shadow_root, element_get_src, element_get_style_object,
    element_get_text_content, element_get_type_attr, element_get_value_attr, element_has_attribute,
    element_has_attributes, element_insert_adjacent_element,
    element_insert_adjacent_html, element_insert_adjacent_text,
    element_matches, element_prepend, element_query_selector, element_query_selector_all,
    element_remove, element_remove_attribute,
    element_replace_with, element_scroll_by, element_scroll_into_view,
    element_scroll_to, element_set_async_attr, element_set_attribute, element_set_checked_attr,
    element_set_class_name, element_set_id, element_set_object_property_noop,
    element_set_shadow_root_noop, element_set_src, element_set_text_content, element_set_type_attr,
    element_set_value_attr, ensure_element_shared_prototype,
};
use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::bindings::event_listeners;
use crate::js::helpers::{define_function, define_js_property_accessor, set_int_property};
use crate::js::helpers::{get_node_id_from_this, js_value_to_string, ToSafeCx};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{CallArgs, JSContext, JSNative, JS_DefineProperty, JS_NewPlainObject, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{BooleanValue, JSVal, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers2::{CurrentGlobalOrNull, JS_GetProperty};
use std::os::raw::c_uint;

type ElementIntConstant = (&'static str, i32);
type ElementMethodBinding = (&'static str, JSNative, u32);
type ElementAccessorBinding = (&'static str, &'static str, &'static str);

const ELEMENT_CONSTRUCTOR_CONSTANTS: &[ElementIntConstant] = &[("ELEMENT_NODE", 1)];

const EVENT_TARGET_METHODS: &[ElementMethodBinding] = &[
    ("addEventListener", Some(element_add_event_listener), 3),
    ("removeEventListener", Some(element_remove_event_listener), 3),
    ("dispatchEvent", Some(element_dispatch_event), 1),
];

const ELEMENT_METHODS: &[ElementMethodBinding] = &[
    ("getAttribute", Some(element_get_attribute), 1),
    ("setAttribute", Some(element_set_attribute), 2),
    ("removeAttribute", Some(element_remove_attribute), 1),
    ("hasAttribute", Some(element_has_attribute), 1),
    ("append", Some(element_append), 0),
    ("querySelector", Some(element_query_selector), 1),
    ("querySelectorAll", Some(element_query_selector_all), 1),
    ("focus", Some(element_focus), 0),
    ("blur", Some(element_blur), 0),
    ("click", Some(element_click), 0),
    ("getBoundingClientRect", Some(element_get_bounding_client_rect), 0),
    ("getClientRects", Some(element_get_client_rects), 0),
    ("closest", Some(element_closest), 1),
    ("matches", Some(element_matches), 1),
    ("attachShadow", Some(element_attach_shadow), 1),
    ("remove", Some(element_remove), 0),
    ("prepend", Some(element_prepend), 0),
    ("before", Some(element_before), 0),
    ("after", Some(element_after), 0),
    ("replaceWith", Some(element_replace_with), 0),
    ("insertAdjacentHTML", Some(element_insert_adjacent_html), 2),
    ("insertAdjacentElement", Some(element_insert_adjacent_element), 2),
    ("insertAdjacentText", Some(element_insert_adjacent_text), 2),
    ("getAttributeNames", Some(element_get_attribute_names), 0),
    ("hasAttributes", Some(element_has_attributes), 0),
    ("scrollIntoView", Some(element_scroll_into_view), 0),
    ("scrollTo", Some(element_scroll_to), 0),
    ("scroll", Some(element_scroll_to), 0),
    ("scrollBy", Some(element_scroll_by), 0),
    ("animate", Some(element_animate), 2),
];

const ELEMENT_INTERNAL_METHODS: &[ElementMethodBinding] = &[
    ("__getTextContent", Some(element_get_text_content), 0),
    ("__setTextContent", Some(element_set_text_content), 1),
    ("__getId", Some(element_get_id), 0),
    ("__setId", Some(element_set_id), 1),
    ("__getClassName", Some(element_get_class_name), 0),
    ("__setClassName", Some(element_set_class_name), 1),
    ("__getShadowRoot", Some(element_get_shadow_root), 0),
    ("__setShadowRoot", Some(element_set_shadow_root_noop), 1),
    ("__getStyleObject", Some(element_get_style_object), 0),
    ("__getClassListObject", Some(element_get_class_list_object), 0),
    ("__getDatasetObject", Some(element_get_dataset_object), 0),
    ("__setObjectPropertyNoop", Some(element_set_object_property_noop), 1),
    ("__getSrc", Some(element_get_src), 0),
    ("__setSrc", Some(element_set_src), 1),
    ("__getType", Some(element_get_type_attr), 0),
    ("__setType", Some(element_set_type_attr), 1),
    ("__getAsync", Some(element_get_async_attr), 0),
    ("__setAsync", Some(element_set_async_attr), 1),
    ("__getValue", Some(element_get_value_attr), 0),
    ("__setValue", Some(element_set_value_attr), 1),
    ("__getChecked", Some(element_get_checked_attr), 0),
    ("__setChecked", Some(element_set_checked_attr), 1),
    ("__getOffsetWidth", Some(element_get_offset_width), 0),
    ("__getOffsetHeight", Some(element_get_offset_height), 0),
    ("__getOffsetLeft", Some(element_get_offset_left), 0),
    ("__getOffsetTop", Some(element_get_offset_top), 0),
    ("__getClientWidth", Some(element_get_client_width), 0),
    ("__getClientHeight", Some(element_get_client_height), 0),
    ("__getScrollWidth", Some(element_get_scroll_width), 0),
    ("__getScrollHeight", Some(element_get_scroll_height), 0),
    ("__getScrollLeft", Some(element_get_scroll_left), 0),
    ("__getScrollTop", Some(element_get_scroll_top), 0),
];

const ELEMENT_ACCESSORS: &[ElementAccessorBinding] = &[
    ("textContent", "__getTextContent", "__setTextContent"),
    ("id", "__getId", "__setId"),
    ("className", "__getClassName", "__setClassName"),
    ("shadowRoot", "__getShadowRoot", "__setShadowRoot"),
    ("style", "__getStyleObject", "__setObjectPropertyNoop"),
    ("classList", "__getClassListObject", "__setObjectPropertyNoop"),
    ("dataset", "__getDatasetObject", "__setObjectPropertyNoop"),
    ("src", "__getSrc", "__setSrc"),
    ("type", "__getType", "__setType"),
    ("async", "__getAsync", "__setAsync"),
    ("value", "__getValue", "__setValue"),
    ("checked", "__getChecked", "__setChecked"),
    ("offsetWidth", "__getOffsetWidth", "__setObjectPropertyNoop"),
    ("offsetHeight", "__getOffsetHeight", "__setObjectPropertyNoop"),
    ("offsetLeft", "__getOffsetLeft", "__setObjectPropertyNoop"),
    ("offsetTop", "__getOffsetTop", "__setObjectPropertyNoop"),
    ("clientWidth", "__getClientWidth", "__setObjectPropertyNoop"),
    ("clientHeight", "__getClientHeight", "__setObjectPropertyNoop"),
    ("scrollWidth", "__getScrollWidth", "__setObjectPropertyNoop"),
    ("scrollHeight", "__getScrollHeight", "__setObjectPropertyNoop"),
    ("scrollLeft", "__getScrollLeft", "__setObjectPropertyNoop"),
    ("scrollTop", "__getScrollTop", "__setObjectPropertyNoop"),
];

unsafe fn define_int_constants(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    constants: &[ElementIntConstant],
) -> Result<(), String> {
    for (name, value) in constants {
        set_int_property(cx, obj, name, *value)?;
    }
    Ok(())
}

unsafe fn define_methods(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    methods: &[ElementMethodBinding],
) -> Result<(), String> {
    for (name, func, arity) in methods {
        define_function(cx, obj, name, *func, *arity)?;
    }
    Ok(())
}

unsafe fn define_accessors(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    accessors: &[ElementAccessorBinding],
) -> Result<(), String> {
    for (property, getter, setter) in accessors {
        define_js_property_accessor(cx, obj, property, getter, setter)?;
    }
    Ok(())
}

/// Set up `Element` and `HTMLElement` constructors.
pub(crate) unsafe fn setup_element_constructors_bindings(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let element = JS_NewPlainObject(raw_cx));
    if element.get().is_null() {
        return Err("Failed to create Element constructor".to_string());
    }
    define_int_constants(cx, element.get(), ELEMENT_CONSTRUCTOR_CONSTANTS)?;

    let element_prototype = ensure_element_shared_prototype(cx)?;
    rooted!(in(raw_cx) let element_proto_val = ObjectValue(element_prototype));
    rooted!(in(raw_cx) let element_ctor_rooted = element.get());
    let prototype_name = std::ffi::CString::new("prototype").unwrap();
    JS_DefineProperty(
        raw_cx,
        element_ctor_rooted.handle().into(),
        prototype_name.as_ptr(),
        element_proto_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );


    rooted!(in(raw_cx) let element_val = ObjectValue(element.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("Element").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    let name = std::ffi::CString::new("HTMLElement").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

pub(crate) unsafe fn define_event_target_bindings(
    cx: &mut SafeJSContext,
    proto: *mut JSObject,
) -> Result<(), String> {
    define_methods(cx, proto, EVENT_TARGET_METHODS)
}

pub(crate) unsafe fn define_element_bindings(
    cx: &mut SafeJSContext,
    proto: *mut JSObject,
) -> Result<(), String> {
    define_methods(cx, proto, ELEMENT_METHODS)?;
    define_methods(cx, proto, ELEMENT_INTERNAL_METHODS)?;
    define_accessors(cx, proto, ELEMENT_ACCESSORS)?;

    Ok(())
}

pub(crate) unsafe extern "C" fn element_add_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let event_type = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        args.rval().set(UndefinedValue());
        return true;
    };

    if argc < 2 {
        args.rval().set(UndefinedValue());
        return true;
    }
    let callback_val = *args.get(1);
    if !callback_val.is_object() {
        args.rval().set(UndefinedValue());
        return true;
    }
    let callback_obj = callback_val.to_object();

    let use_capture = if argc >= 3 {
        let opt = *args.get(2);
        if opt.is_boolean() {
            opt.to_boolean()
        } else if opt.is_object() {
            let opt_obj = opt.to_object();
            rooted!(in(raw_cx) let opt_r = opt_obj);
            rooted!(in(raw_cx) let mut cap_val = UndefinedValue());
            let cname = std::ffi::CString::new("capture").unwrap();
            JS_GetProperty(safe_cx, opt_r.handle().into(), cname.as_ptr(), cap_val.handle_mut().into());
            cap_val.get().is_boolean() && cap_val.get().to_boolean()
        } else {
            false
        }
    } else {
        false
    };

    let node_id = match get_node_id_from_this(safe_cx, &args) {
        Some(id) => id,
        None => {
            args.rval().set(UndefinedValue());
            return true;
        }
    };

    event_listeners::add_listener(safe_cx, node_id, event_type, callback_obj, use_capture);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn element_remove_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let event_type = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        args.rval().set(UndefinedValue());
        return true;
    };

    if argc < 2 {
        args.rval().set(UndefinedValue());
        return true;
    }
    let callback_val = *args.get(1);
    if !callback_val.is_object() {
        args.rval().set(UndefinedValue());
        return true;
    }
    let callback_obj = callback_val.to_object();

    let use_capture = if argc >= 3 {
        let opt = *args.get(2);
        if opt.is_boolean() { opt.to_boolean() } else { false }
    } else {
        false
    };

    let node_id = match get_node_id_from_this(safe_cx, &args) {
        Some(id) => id,
        None => {
            args.rval().set(UndefinedValue());
            return true;
        }
    };

    event_listeners::remove_listener(node_id, &event_type, callback_obj, use_capture);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn element_dispatch_event(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc < 1 {
        args.rval().set(BooleanValue(true));
        return true;
    }

    let event_val = *args.get(0);
    if !event_val.is_object() {
        args.rval().set(BooleanValue(true));
        return true;
    }

    let node_id = match get_node_id_from_this(safe_cx, &args) {
        Some(id) => id,
        None => {
            args.rval().set(BooleanValue(true));
            return true;
        }
    };

    let event_obj = event_val.to_object();
    rooted!(in(raw_cx) let event_r = event_obj);

    rooted!(in(raw_cx) let mut type_val = UndefinedValue());
    let type_cname = std::ffi::CString::new("type").unwrap();
    JS_GetProperty(safe_cx, event_r.handle().into(), type_cname.as_ptr(), type_val.handle_mut().into());
    let event_type = if type_val.get().is_string() {
        js_value_to_string(safe_cx, *type_val)
    } else {
        args.rval().set(BooleanValue(true));
        return true;
    };

    rooted!(in(raw_cx) let mut bubbles_val = UndefinedValue());
    let bubbles_cname = std::ffi::CString::new("bubbles").unwrap();
    JS_GetProperty(safe_cx, event_r.handle().into(), bubbles_cname.as_ptr(), bubbles_val.handle_mut().into());
    let bubbles = bubbles_val.get().is_boolean() && bubbles_val.get().to_boolean();

    let chain = DOM_REF.with(|d| {
        d.borrow().as_ref().map(|dom_ptr| {
            let dom = &**dom_ptr;
            dom.node_chain(node_id)
        }).unwrap_or_else(|| vec![node_id])
    });

    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(safe_cx));
    event_listeners::dispatch_event_obj(safe_cx, global.get(), &chain, &event_type, bubbles, event_obj);

    let not_cancelled = !event_listeners::EVENT_DEFAULT_PREVENTED.with(|f| f.get());
    args.rval().set(BooleanValue(not_cancelled));
    true
}

