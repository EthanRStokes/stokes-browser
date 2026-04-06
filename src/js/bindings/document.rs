use crate::js::bindings::custom_elements::custom_elements_upgrade_for_node;
use crate::js::bindings::dom_bindings::{CURRENT_SCRIPT_NODE_ID, DOM_REF};
use crate::js::bindings::cookie::{
    ensure_cookie_jar_initialized, COOKIE_JAR, DOCUMENT_URL,
};
use crate::js::bindings::document_fragment::{
    document_fragment_append_child, document_fragment_has_child_nodes,
};
use crate::js::bindings::event_listeners;
use crate::js::bindings::element_bindings;
use crate::js::bindings::node::node_has_child_nodes;
use crate::js::helpers::{
    create_empty_array, create_js_string, define_function, define_js_property_accessor,
    define_js_property_getter, get_node_id_from_value,
    js_value_to_string, set_int_property, set_string_property, ToSafeCx,
};
use crate::js::selectors::{matches_parsed_selector, parse_selector, selector_seed, SelectorSeed};
use crate::dom::AttributeMap;
use html5ever::ns;
use markup5ever::{LocalName, Namespace, QualName};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{
    CallArgs, HandleValueArray, JSContext, JSNative, JS_DefineProperty, JS_GetProperty,
    JS_NewPlainObject, JSObject, JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::ValueArray;
use mozjs::rust::wrappers::JS_SetElement;
use mozjs::rust::wrappers2::{CurrentGlobalOrNull, JS_CallFunctionValue};
use std::os::raw::c_uint;
use tracing::trace;
use tracing::warn;

type DocumentMethodBinding = (&'static str, JSNative, u32);

const DOCUMENT_PROTO_METHODS: &[DocumentMethodBinding] = &[
    ("hasChildNodes", Some(node_has_child_nodes), 0),
    ("getElementById", Some(document_get_element_by_id), 1),
    ("getElementsByTagName", Some(document_get_elements_by_tag_name), 1),
    ("getElementsByClassName", Some(document_get_elements_by_class_name), 1),
    ("querySelector", Some(document_query_selector), 1),
    ("querySelectorAll", Some(document_query_selector_all), 1),
    ("createElement", Some(document_create_element), 1),
    ("createElementNS", Some(document_create_element_ns), 2),
    ("createTextNode", Some(document_create_text_node), 1),
    ("createComment", Some(document_create_comment), 1),
    (
        "createDocumentFragment",
        Some(document_create_document_fragment),
        0,
    ),
    ("addEventListener", Some(document_add_event_listener), 3),
    ("removeEventListener", Some(document_remove_event_listener), 3),
    ("dispatchEvent", Some(document_dispatch_event), 1),
];

const DOCUMENT_INTERNAL_METHODS: &[DocumentMethodBinding] = &[
    ("__getCookie", Some(document_get_cookie), 0),
    ("__setCookie", Some(document_set_cookie), 1),
    ("__getHead", Some(document_get_head), 0),
    ("__getBody", Some(document_get_body), 0),
    ("__setBody", Some(document_set_body), 1),
    ("__getCurrentScript", Some(document_get_current_script), 0),
];

unsafe fn define_methods(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    methods: &[DocumentMethodBinding],
) -> Result<(), String> {
    for (name, func, arity) in methods {
        define_function(cx, obj, name, *func, *arity)?;
    }
    Ok(())
}

unsafe fn get_object_property(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    property: &str,
) -> Option<*mut JSObject> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let rooted_obj = obj);
    rooted!(in(raw_cx) let mut value = UndefinedValue());
    let property_name = std::ffi::CString::new(property).ok()?;
    if JS_GetProperty(
        raw_cx,
        rooted_obj.handle().into(),
        property_name.as_ptr(),
        value.handle_mut().into(),
    ) && value.get().is_object()
    {
        Some(value.get().to_object())
    } else {
        None
    }
}

unsafe fn set_object_prototype(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    proto: *mut JSObject,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return Err("No global object for prototype setup".to_string());
    }

    let object_ctor = get_object_property(cx, global.get(), "Object")
        .ok_or_else(|| "Failed to resolve Object constructor".to_string())?;
    rooted!(in(raw_cx) let object_ctor_rooted = object_ctor);
    rooted!(in(raw_cx) let mut set_proto_fn = UndefinedValue());
    let set_proto_name = std::ffi::CString::new("setPrototypeOf").unwrap();
    if !JS_GetProperty(
        raw_cx,
        object_ctor_rooted.handle().into(),
        set_proto_name.as_ptr(),
        set_proto_fn.handle_mut().into(),
    ) || !set_proto_fn.get().is_object() {
        return Err("Failed to resolve Object.setPrototypeOf".to_string());
    }

    rooted!(in(raw_cx) let args = ValueArray::<2usize>::new([ObjectValue(obj), ObjectValue(proto)]));
    rooted!(in(raw_cx) let mut rval = UndefinedValue());
    if !JS_CallFunctionValue(
        cx,
        object_ctor_rooted.handle().into(),
        set_proto_fn.handle().into(),
        &HandleValueArray::from(&args),
        rval.handle_mut().into(),
    ) {
        return Err("Object.setPrototypeOf call failed".to_string());
    }

    Ok(())
}

unsafe fn setup_document_property_accessors(
    cx: &mut SafeJSContext,
    document_obj: *mut JSObject,
) -> Result<(), String> {
    define_js_property_accessor(cx, document_obj, "cookie", "__getCookie", "__setCookie")?;
    define_js_property_getter(cx, document_obj, "head", "__getHead")?;
    define_js_property_accessor(cx, document_obj, "body", "__getBody", "__setBody")?;
    define_js_property_getter(cx, document_obj, "currentScript", "__getCurrentScript")?;
    Ok(())
}

/// Set up the document object and the `Document` constructor.
pub(crate) unsafe fn setup_document_bindings(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let document = JS_NewPlainObject(raw_cx));
    if document.get().is_null() {
        return Err("Failed to create document object".to_string());
    }

    rooted!(in(raw_cx) let document_constructor = JS_NewPlainObject(raw_cx));
    if document_constructor.get().is_null() {
        return Err("Failed to create Document constructor".to_string());
    }

    rooted!(in(raw_cx) let document_prototype = JS_NewPlainObject(raw_cx));
    if document_prototype.get().is_null() {
        return Err("Failed to create Document prototype".to_string());
    }

    define_methods(cx, document_prototype.get(), DOCUMENT_PROTO_METHODS)?;
    define_methods(cx, document_prototype.get(), DOCUMENT_INTERNAL_METHODS)?;

    // Internal accessors are resolved directly off the instance object.
    define_methods(cx, document.get(), DOCUMENT_INTERNAL_METHODS)?;

    rooted!(in(raw_cx) let document_proto_val = ObjectValue(document_prototype.get()));
    rooted!(in(raw_cx) let document_ctor_rooted = document_constructor.get());
    let prototype_name = std::ffi::CString::new("prototype").unwrap();
    JS_DefineProperty(
        raw_cx,
        document_ctor_rooted.handle().into(),
        prototype_name.as_ptr(),
        document_proto_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    let constructor_name = std::ffi::CString::new("constructor").unwrap();
    rooted!(in(raw_cx) let document_constructor_val = ObjectValue(document_constructor.get()));
    rooted!(in(raw_cx) let document_prototype_rooted = document_prototype.get());
    JS_DefineProperty(
        raw_cx,
        document_prototype_rooted.handle().into(),
        constructor_name.as_ptr(),
        document_constructor_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    let prototype_linked = match set_object_prototype(cx, document.get(), document_prototype.get()) {
        Ok(()) => true,
        Err(err) => {
            warn!("[JS] Failed to link document prototype: {}", err);
            false
        }
    };

    if !prototype_linked {
        // Keep document usable even if prototype linkage fails in this runtime.
        define_methods(cx, document.get(), DOCUMENT_PROTO_METHODS)?;
    }

    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("Document").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        document_constructor_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );


    rooted!(in(raw_cx) let null_cs = NullValue());
    rooted!(in(raw_cx) let document_rooted_cs = document.get());
    let cs_name = std::ffi::CString::new("currentScript").unwrap();
    JS_DefineProperty(
        raw_cx,
        document_rooted_cs.handle().into(),
        cs_name.as_ptr(),
        null_cs.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    rooted!(in(raw_cx) let doc_node_id = mozjs::jsval::DoubleValue(0.0));
    let node_id_name = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        raw_cx,
        document_rooted_cs.handle().into(),
        node_id_name.as_ptr(),
        doc_node_id.handle().into(),
        0,
    );

    let doc_elem_val = element_bindings::create_stub_element(cx, "html")?;
    rooted!(in(raw_cx) let doc_elem_val_rooted = doc_elem_val);
    let name = std::ffi::CString::new("documentElement").unwrap();
    rooted!(in(raw_cx) let document_rooted = document.get());
    JS_DefineProperty(
        raw_cx,
        document_rooted.handle().into(),
        name.as_ptr(),
        doc_elem_val_rooted.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    let base_url_str = DOM_REF.with(|dom_ref| {
        dom_ref
            .borrow()
            .as_ref()
            .map(|dom_ptr| {
                let dom = &**dom_ptr;
                let url: url::Url = (&dom.url).into();
                url.as_str().to_string()
            })
            .unwrap_or_default()
    });
    set_string_property(cx, document.get(), "baseURI", &base_url_str)?;
    set_string_property(cx, document.get(), "URL", &base_url_str)?;
    set_string_property(cx, document.get(), "documentURI", &base_url_str)?;
    set_int_property(cx, document.get(), "nodeType", 9)?;
    set_string_property(cx, document.get(), "nodeName", "#document")?;
    set_string_property(cx, document.get(), "readyState", "complete")?;
    set_string_property(cx, document.get(), "compatMode", "CSS1Compat")?;
    set_string_property(cx, document.get(), "characterSet", "UTF-8")?;
    set_string_property(cx, document.get(), "charset", "UTF-8")?;
    set_string_property(cx, document.get(), "inputEncoding", "UTF-8")?;

    rooted!(in(raw_cx) let document_val = ObjectValue(document.get()));
    let name = std::ffi::CString::new("document").unwrap();
    if !JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        document_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        return Err("Failed to define document property".to_string());
    }

    setup_document_property_accessors(cx, document.get())?;

    Ok(())
}


fn simple_id_selector(selector: &str) -> Option<&str> {
    let trimmed = selector.trim();
    let id = trimmed.strip_prefix('#')?;
    if id.is_empty() {
        return None;
    }
    if id
        .chars()
        .any(|c| matches!(c, '.' | '#' | '[' | ']' | ' ' | '\t' | '\n' | '\r' | '>' | '+' | '~' | ',' | ':'))
    {
        return None;
    }
    Some(id)
}

fn namespace_from_uri(ns_uri: &str) -> Namespace {
    match ns_uri {
        "http://www.w3.org/1999/xhtml" => ns!(html),
        "http://www.w3.org/2000/svg" => ns!(svg),
        "http://www.w3.org/1998/Math/MathML" => ns!(mathml),
        _ => Namespace::from(ns_uri),
    }
}

fn split_qualified_name(qualified_name: &str) -> (Option<String>, String) {
    if let Some((prefix, local)) = qualified_name.split_once(':') {
        if !prefix.is_empty() && !local.is_empty() {
            return (Some(prefix.to_string()), local.to_string());
        }
    }
    (None, qualified_name.to_string())
}

pub(crate) unsafe extern "C" fn document_get_cookie(raw_cx: *mut mozjs::jsapi::JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, 0);
    let safe_cx = &mut raw_cx.to_safe_cx();

    ensure_cookie_jar_initialized();

    let cookie_string = DOCUMENT_URL.with(|doc_url| {
        let url_opt = doc_url.borrow();
        if let Some(ref url) = *url_opt {
            let domain = url.host_str().unwrap_or("localhost");
            let path = url.path();
            let is_secure = url.scheme() == "https";

            COOKIE_JAR.with(|jar| {
                jar.borrow_mut()
                    .get_document_cookie_string(domain, path, is_secure)
            })
        } else {
            String::new()
        }
    });

    args.rval().set(create_js_string(safe_cx, &cookie_string));
    true
}

pub(crate) unsafe extern "C" fn document_set_cookie(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let cookie_str = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        args.rval().set(UndefinedValue());
        return true;
    };

    trace!("[JS] document.cookie = '{}' (setting cookie)", cookie_str);

    ensure_cookie_jar_initialized();

    DOCUMENT_URL.with(|doc_url| {
        let url_opt = doc_url.borrow();
        if let Some(ref url) = *url_opt {
            let domain = url.host_str().unwrap_or("localhost");
            let path = url.path();
            let is_secure = url.scheme() == "https";

            let set_ok = COOKIE_JAR.with(|jar| {
                jar.borrow_mut()
                    .set_from_document_cookie(&cookie_str, domain, path, is_secure)
            });

            if !set_ok {
                warn!("[JS] Failed to parse cookie: {}", cookie_str);
            }
        }
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn document_get_head(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] document.head called");

    let head_node_id = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    if elem_data.name.local.as_ref().eq_ignore_ascii_case("head") {
                        return Some(node_id);
                    }
                }
            }
        }
        None
    });

    if let Some(node_id) = head_node_id {
        if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
            args.rval().set(js_elem);
        } else {
            args.rval().set(NullValue());
        }
    } else {
        trace!("[JS] head element not found");
        args.rval().set(NullValue());
    }

    true
}

pub(crate) unsafe extern "C" fn document_get_body(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let body_node_id = DOM_REF.with(|dom_ref| {
        let dom_ptr = (*dom_ref.borrow())?;
        let dom = unsafe { &*dom_ptr };
        let node_id = dom.body_id()?;
        let node = dom.get_node(node_id)?;
        node.element_data()?;
        Some(node_id)
    });

    if let Some(node_id) = body_node_id {
        if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
            args.rval().set(js_elem);
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn document_set_body(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc == 0 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let value = *args.get(0);
    if value.is_null() || value.is_undefined() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let Some(new_body_id) = get_node_id_from_value(safe_cx, value) else {
        args.rval().set(UndefinedValue());
        return true;
    };

    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let _dom = unsafe { &mut *dom_ptr };
            // TODO: wire Document.body assignment to actual DOM tree replacement.
            let _ = new_body_id;
        }
    });

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn document_get_current_script(raw_cx: *mut mozjs::jsapi::JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, 0);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let element_node_id = CURRENT_SCRIPT_NODE_ID.with(|id| {
        let node_id = (*id.borrow())?;
        DOM_REF.with(|dom_ref| {
            let dom_ptr = (*dom_ref.borrow())?;
            let dom = &*dom_ptr;
            let node = dom.get_node(node_id)?;
            if matches!(node.data, crate::dom::NodeData::Element(_)) {
                Some(node_id)
            } else {
                None
            }
        })
    });

    if let Some(node_id) = element_node_id {
        if let Ok(val) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
            args.rval().set(val);
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn document_get_element_by_id(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let id = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    if id.is_empty() {
        args.rval().set(NullValue());
        return true;
    }

    let node_id = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            if let Some(&node_id) = dom.nodes_to_id.get(&id) {
                if matches!(dom.get_node(node_id).map(|node| &node.data), Some(crate::dom::NodeData::Element(_))) {
                    return Some(node_id);
                }
            }
        }
        None
    });

    if let Some(node_id) = node_id {
        if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
            args.rval().set(js_elem);
            return true;
        }
    }
    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn document_get_elements_by_tag_name(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let tag_name = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };

    let matching_node_ids: Vec<usize> = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            if tag_name == "*" {
                return dom.nodes.iter().filter_map(|(node_id, node)| matches!(node.data, crate::dom::NodeData::Element(_)).then_some(node_id)).collect();
            }
            return dom.candidate_nodes_for_tag(&tag_name);
        }
        Vec::new()
    });

    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));
    for (index, node_id) in matching_node_ids.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, *node_id) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }
    args.rval().set(ObjectValue(array.get()));
    true
}

pub(crate) unsafe extern "C" fn document_get_elements_by_class_name(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let class_name = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    let search_classes: Vec<&str> = class_name.split_whitespace().collect();

    let matching_node_ids: Vec<usize> = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            let Some(first_class) = search_classes.first().copied() else { return Vec::new(); };
            let mut results = Vec::new();
            for node_id in dom.candidate_nodes_for_class(first_class) {
                if let Some(node) = dom.get_node(node_id) {
                    if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                        if let Some(class_attr) = elem_data.attributes.iter().find(|attr| attr.name.local.as_ref() == "class") {
                            if search_classes.iter().all(|sc| class_attr.value.split_whitespace().any(|c| c == *sc)) {
                                results.push(node_id);
                            }
                        }
                    }
                }
            }
            return results;
        }
        Vec::new()
    });

    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));
    for (index, node_id) in matching_node_ids.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, *node_id) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }
    args.rval().set(ObjectValue(array.get()));
    true
}

pub(crate) unsafe extern "C" fn document_query_selector(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let selector = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };

    if let Some(id) = simple_id_selector(&selector) {
        let node_id = DOM_REF.with(|dom_ref| {
            let dom_ptr = (*dom_ref.borrow())?;
            let dom = unsafe { &*dom_ptr };
            let &node_id = dom.nodes_to_id.get(id)?;
            matches!(dom.get_node(node_id).map(|node| &node.data), Some(crate::dom::NodeData::Element(_))).then_some(node_id)
        });
        if let Some(node_id) = node_id {
            if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
                args.rval().set(js_elem);
                return true;
            }
        }
        args.rval().set(NullValue());
        return true;
    }

    let parsed_selector = parse_selector(&selector);
    let node_id = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            let candidate_ids: Vec<usize> = match selector_seed(&parsed_selector) {
                SelectorSeed::Id(id) => dom.nodes_to_id.get(id).copied().into_iter().collect(),
                SelectorSeed::Class(class_name) => dom.candidate_nodes_for_class(class_name),
                SelectorSeed::Tag(tag) => dom.candidate_nodes_for_tag(tag),
                SelectorSeed::Universal | SelectorSeed::None => dom
                    .nodes
                    .iter()
                    .filter_map(|(node_id, node)| matches!(node.data, crate::dom::NodeData::Element(_)).then_some(node_id))
                    .collect(),
            };
            for node_id in candidate_ids {
                if let Some(node) = dom.get_node(node_id) {
                    if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                        if matches_parsed_selector(&parsed_selector, elem_data.name.local.as_ref(), &elem_data.attributes) {
                            return Some(node_id);
                        }
                    }
                }
            }
        }
        None
    });

    if let Some(node_id) = node_id {
        if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
            args.rval().set(js_elem);
            return true;
        }
    }
    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn document_query_selector_all(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let selector = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));

    if let Some(id) = simple_id_selector(&selector) {
        let node_id = DOM_REF.with(|dom_ref| {
            let dom_ptr = (*dom_ref.borrow())?;
            let dom = unsafe { &*dom_ptr };
            let &node_id = dom.nodes_to_id.get(id)?;
            matches!(dom.get_node(node_id).map(|node| &node.data), Some(crate::dom::NodeData::Element(_))).then_some(node_id)
        });
        if let Some(node_id) = node_id {
            if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
                rooted!(in(raw_cx) let elem_val = js_elem);
                rooted!(in(raw_cx) let array_obj = array.get());
                JS_SetElement(raw_cx, array_obj.handle().into(), 0, elem_val.handle().into());
            }
        }
        args.rval().set(ObjectValue(array.get()));
        return true;
    }

    let parsed_selector = parse_selector(&selector);
    let matching_node_ids: Vec<usize> = DOM_REF.with(|dom_ref| {
        let mut results = Vec::new();
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            let candidate_ids: Vec<usize> = match selector_seed(&parsed_selector) {
                SelectorSeed::Id(id) => dom.nodes_to_id.get(id).copied().into_iter().collect(),
                SelectorSeed::Class(class_name) => dom.candidate_nodes_for_class(class_name),
                SelectorSeed::Tag(tag) => dom.candidate_nodes_for_tag(tag),
                SelectorSeed::Universal | SelectorSeed::None => dom
                    .nodes
                    .iter()
                    .filter_map(|(node_id, node)| matches!(node.data, crate::dom::NodeData::Element(_)).then_some(node_id))
                    .collect(),
            };
            for node_id in candidate_ids {
                if let Some(node) = dom.get_node(node_id) {
                    if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                        if matches_parsed_selector(&parsed_selector, elem_data.name.local.as_ref(), &elem_data.attributes) {
                            results.push(node_id);
                        }
                    }
                }
            }
        }
        results
    });

    for (index, node_id) in matching_node_ids.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, *node_id) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }
    args.rval().set(ObjectValue(array.get()));
    true
}

pub(crate) unsafe extern "C" fn document_create_element(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let tag_name = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    if tag_name.is_empty() {
        args.rval().set(NullValue());
        return true;
    }

    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = unsafe { &mut *dom_ptr };
            let local = markup5ever::LocalName::from(tag_name.to_lowercase());
            let node_id = dom.create_element(QualName::new(None, ns!(html), local), AttributeMap::empty());
            custom_elements_upgrade_for_node(safe_cx, node_id);
            if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
                args.rval().set(js_elem);
                return;
            }
        }
        args.rval().set(NullValue());
    });
    true
}

pub(crate) unsafe extern "C" fn document_create_element_ns(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if argc < 2 {
        args.rval().set(NullValue());
        return true;
    }
    let ns_arg = *args.get(0);
    let qualified_name = js_value_to_string(safe_cx, *args.get(1));
    if qualified_name.is_empty() {
        args.rval().set(NullValue());
        return true;
    }
    let namespace_uri = if ns_arg.is_null() || ns_arg.is_undefined() {
        None
    } else {
        let uri = js_value_to_string(safe_cx, ns_arg);
        if uri.is_empty() { None } else { Some(uri) }
    };
    let (prefix, local_name) = split_qualified_name(&qualified_name);
    if local_name.is_empty() {
        args.rval().set(NullValue());
        return true;
    }

    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            let ns = namespace_uri.as_deref().map(namespace_from_uri).unwrap_or_else(|| ns!());
            let qname = QualName::new(prefix.map(|p| p.into()), ns, LocalName::from(local_name.clone()));
            let node_id = dom.create_element(qname, AttributeMap::empty());
            custom_elements_upgrade_for_node(safe_cx, node_id);
            if let Ok(js_elem) = element_bindings::create_js_element_by_dom_id(safe_cx, node_id) {
                args.rval().set(js_elem);
                return;
            }
        }
        args.rval().set(NullValue());
    });
    true
}

pub(crate) unsafe extern "C" fn document_create_text_node(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let text = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] document.createTextNode('{}') called", text);

    let text_node_id = DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            let node_id = dom.create_text_node(&text);
            return dom.get_node(node_id).map(|node| node.id);
        }
        None
    });

    rooted!(in(raw_cx) let text_node = JS_NewPlainObject(raw_cx));
    if !text_node.get().is_null() {
        let _ = define_function(safe_cx, text_node.get(), "hasChildNodes", Some(node_has_child_nodes), 0);
        let _ = set_int_property(safe_cx, text_node.get(), "nodeType", 3);
        let _ = set_string_property(safe_cx, text_node.get(), "nodeName", "#text");
        let _ = set_string_property(safe_cx, text_node.get(), "textContent", &text);
        let _ = set_string_property(safe_cx, text_node.get(), "nodeValue", &text);

        if let Some(node_id) = text_node_id {
            rooted!(in(raw_cx) let node_id_val = mozjs::jsval::DoubleValue(node_id as f64));
            rooted!(in(raw_cx) let text_rooted = text_node.get());
            let node_id_name = std::ffi::CString::new("__nodeId").unwrap();
            JS_DefineProperty(
                raw_cx,
                text_rooted.handle().into(),
                node_id_name.as_ptr(),
                node_id_val.handle().into(),
                0,
            );
        }

        rooted!(in(raw_cx) let null_val = NullValue());
        rooted!(in(raw_cx) let text_rooted = text_node.get());
        for prop in &["parentNode", "parentElement", "firstChild", "lastChild", "previousSibling", "nextSibling"] {
            if let Ok(cname) = std::ffi::CString::new(*prop) {
                JS_DefineProperty(
                    raw_cx,
                    text_rooted.handle().into(),
                    cname.as_ptr(),
                    null_val.handle().into(),
                    JSPROP_ENUMERATE as u32,
                );
            }
        }

        rooted!(in(raw_cx) let child_nodes = create_empty_array(safe_cx));
        if !child_nodes.get().is_null() {
            rooted!(in(raw_cx) let child_nodes_val = ObjectValue(child_nodes.get()));
            let child_nodes_name = std::ffi::CString::new("childNodes").unwrap();
            JS_DefineProperty(
                raw_cx,
                text_rooted.handle().into(),
                child_nodes_name.as_ptr(),
                child_nodes_val.handle().into(),
                JSPROP_ENUMERATE as u32,
            );
        }

        args.rval().set(ObjectValue(text_node.get()));
    } else {
        args.rval().set(NullValue());
    }
    true
}

pub(crate) unsafe extern "C" fn document_create_comment(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let comment_text = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] document.createComment('{}') called", comment_text);

    let comment_node_id = DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            let node_id = dom.create_comment_node();
            return dom.get_node(node_id).map(|node| node.id);
        }
        None
    });

    rooted!(in(raw_cx) let comment_node = JS_NewPlainObject(raw_cx));
    if comment_node.get().is_null() {
        args.rval().set(NullValue());
        return true;
    }

    let _ = define_function(safe_cx, comment_node.get(), "hasChildNodes", Some(node_has_child_nodes), 0);
    let _ = set_int_property(safe_cx, comment_node.get(), "nodeType", 8);
    let _ = set_string_property(safe_cx, comment_node.get(), "nodeName", "#comment");
    // The DOM backend does not yet persist comment text, so keep it on the wrapper object.
    let _ = set_string_property(safe_cx, comment_node.get(), "nodeValue", &comment_text);
    let _ = set_string_property(safe_cx, comment_node.get(), "textContent", &comment_text);

    if let Some(node_id) = comment_node_id {
        rooted!(in(raw_cx) let node_id_val = mozjs::jsval::DoubleValue(node_id as f64));
        rooted!(in(raw_cx) let comment_rooted = comment_node.get());
        let node_id_name = std::ffi::CString::new("__nodeId").unwrap();
        JS_DefineProperty(
            raw_cx,
            comment_rooted.handle().into(),
            node_id_name.as_ptr(),
            node_id_val.handle().into(),
            0,
        );
    }

    rooted!(in(raw_cx) let null_val = NullValue());
    rooted!(in(raw_cx) let comment_rooted = comment_node.get());
    for prop in &["parentNode", "parentElement", "firstChild", "lastChild", "previousSibling", "nextSibling"] {
        if let Ok(cname) = std::ffi::CString::new(*prop) {
            JS_DefineProperty(
                raw_cx,
                comment_rooted.handle().into(),
                cname.as_ptr(),
                null_val.handle().into(),
                JSPROP_ENUMERATE as u32,
            );
        }
    }

    rooted!(in(raw_cx) let child_nodes = create_empty_array(safe_cx));
    if !child_nodes.get().is_null() {
        rooted!(in(raw_cx) let child_nodes_val = ObjectValue(child_nodes.get()));
        let child_nodes_name = std::ffi::CString::new("childNodes").unwrap();
        JS_DefineProperty(
            raw_cx,
            comment_rooted.handle().into(),
            child_nodes_name.as_ptr(),
            child_nodes_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    args.rval().set(ObjectValue(comment_node.get()));
    true
}

pub(crate) unsafe extern "C" fn document_create_document_fragment(raw_cx: *mut mozjs::jsapi::JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    rooted!(in(raw_cx) let fragment = JS_NewPlainObject(raw_cx));
    if !fragment.get().is_null() {
        let _ = define_function(safe_cx, fragment.get(), "hasChildNodes", Some(document_fragment_has_child_nodes), 0);
        let _ = set_int_property(safe_cx, fragment.get(), "__childCount", 0);
        let _ = set_int_property(safe_cx, fragment.get(), "nodeType", 11);
        let _ = set_string_property(safe_cx, fragment.get(), "nodeName", "#document-fragment");
        let _ = define_function(safe_cx, fragment.get(), "appendChild", Some(document_fragment_append_child), 1);
        let _ = define_function(safe_cx, fragment.get(), "querySelector", Some(document_query_selector), 1);
        let _ = define_function(safe_cx, fragment.get(), "querySelectorAll", Some(document_query_selector_all), 1);
        args.rval().set(ObjectValue(fragment.get()));
    } else {
        args.rval().set(NullValue());
    }
    true
}

pub(crate) unsafe extern "C" fn document_add_event_listener(
    raw_cx: *mut mozjs::jsapi::JSContext,
    argc: u32,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let event_type = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    if event_type.is_empty() || argc < 2 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let callback_val = *args.get(1);
    if !callback_val.is_object() || callback_val.is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let use_capture = if argc >= 3 {
        let v = *args.get(2);
        v.is_boolean() && v.to_boolean()
    } else {
        false
    };

    event_listeners::add_listener(
        safe_cx,
        event_listeners::DOCUMENT_NODE_ID,
        event_type,
        callback_val.to_object(),
        use_capture,
    );

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn document_remove_event_listener(
    raw_cx: *mut mozjs::jsapi::JSContext,
    argc: u32,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let event_type = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    if event_type.is_empty() || argc < 2 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let callback_val = *args.get(1);
    if !callback_val.is_object() || callback_val.is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let use_capture = if argc >= 3 {
        let v = *args.get(2);
        v.is_boolean() && v.to_boolean()
    } else {
        false
    };

    event_listeners::remove_listener(
        event_listeners::DOCUMENT_NODE_ID,
        &event_type,
        callback_val.to_object(),
        use_capture,
    );

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn document_dispatch_event(
    raw_cx: *mut mozjs::jsapi::JSContext,
    argc: u32,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc == 0 {
        args.rval().set(BooleanValue(false));
        return true;
    }

    let event_val = *args.get(0);
    if !event_val.is_object() || event_val.is_null() {
        args.rval().set(BooleanValue(false));
        return true;
    }

    rooted!(in(raw_cx) let event_obj = event_val.to_object());
    rooted!(in(raw_cx) let mut type_val = UndefinedValue());
    let type_name = std::ffi::CString::new("type").unwrap();
    let event_type = if JS_GetProperty(
        raw_cx,
        event_obj.handle().into(),
        type_name.as_ptr(),
        type_val.handle_mut().into(),
    ) {
        js_value_to_string(safe_cx, *type_val)
    } else {
        String::new()
    };

    let dispatched = if !event_type.is_empty() {
        rooted!(in(raw_cx) let global = CurrentGlobalOrNull(safe_cx));
        if global.get().is_null() {
            false
        } else {
            event_listeners::dispatch_event_obj(
                safe_cx,
                global.get(),
                &[0],
                &event_type,
                true,
                event_obj.get(),
            );
            true
        }
    } else {
        false
    };

    args.rval().set(BooleanValue(dispatched));
    true
}



