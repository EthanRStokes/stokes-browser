use blitz_traits::net::Request;
use crate::dom::NodeData;
use crate::engine::js_provider::ScriptKind;
use crate::engine::script_type::executable_script_kind;
use crate::js::bindings::custom_elements::custom_elements_upgrade_for_node;
use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::bindings::element_bindings::{
    create_js_element_by_dom_id, create_js_element_by_id, create_js_shadow_root_by_id,
    create_stub_element,
};
use crate::js::helpers::{
    create_empty_array, define_function, define_js_property_getter, get_node_id_from_this,
    get_node_id_from_value, js_value_to_string, set_int_property, set_string_property, ToSafeCx,
};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::{
    CallArgs, JS_DefineProperty, JS_NewPlainObject, JSContext, JSNative, JSObject,
    JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers2::{CurrentGlobalOrNull, JS_GetProperty, JS_SetElement};
use std::os::raw::c_uint;
use tracing::{trace, warn};

type NodeIntConstant = (&'static str, i32);
type NodeMethodBinding = (&'static str, JSNative, u32);
type NodeGetterBinding = (&'static str, &'static str);

const NODE_CONSTANTS: &[NodeIntConstant] = &[
    ("ELEMENT_NODE", 1),
    ("ATTRIBUTE_NODE", 2),
    ("TEXT_NODE", 3),
    ("CDATA_SECTION_NODE", 4),
    ("ENTITY_REFERENCE_NODE", 5),
    ("ENTITY_NODE", 6),
    ("PROCESSING_INSTRUCTION_NODE", 7),
    ("COMMENT_NODE", 8),
    ("DOCUMENT_NODE", 9),
    ("DOCUMENT_TYPE_NODE", 10),
    ("DOCUMENT_FRAGMENT_NODE", 11),
    ("NOTATION_NODE", 12),
    ("DOCUMENT_POSITION_DISCONNECTED", 1),
    ("DOCUMENT_POSITION_PRECEDING", 2),
    ("DOCUMENT_POSITION_FOLLOWING", 4),
    ("DOCUMENT_POSITION_CONTAINS", 8),
    ("DOCUMENT_POSITION_CONTAINED_BY", 16),
    ("DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32),
];

const NODE_SHARED_METHODS: &[NodeMethodBinding] = &[
    ("appendChild", Some(node_append_child), 1),
    ("removeChild", Some(node_remove_child), 1),
    ("insertBefore", Some(node_insert_before), 2),
    ("replaceChild", Some(node_replace_child), 2),
    ("cloneNode", Some(node_clone_node), 1),
    ("contains", Some(node_contains), 1),
    ("hasChildNodes", Some(node_has_child_nodes), 0),
    ("getRootNode", Some(node_get_root_node), 1),
];

const NODE_CONSTRUCTOR_PROTO_METHODS: &[NodeMethodBinding] = &[
    ("isEqualNode", Some(node_is_equal_node), 1),
    ("isSameNode", Some(node_is_same_node), 1),
    ("compareDocumentPosition", Some(node_compare_document_position), 1),
    ("normalize", Some(node_normalize), 0),
    ("lookupPrefix", Some(node_lookup_prefix), 1),
    ("lookupNamespaceURI", Some(node_lookup_namespace_uri), 1),
    ("isDefaultNamespace", Some(node_is_default_namespace), 1),
];

const NODE_INTERNAL_GETTERS: &[NodeMethodBinding] = &[
    ("__getFirstChild", Some(node_get_first_child), 0),
    ("__getLastChild", Some(node_get_last_child), 0),
    ("__getPreviousSibling", Some(node_get_previous_sibling), 0),
    ("__getNextSibling", Some(node_get_next_sibling), 0),
    ("__getChildren", Some(node_get_children), 0),
    ("__getChildNodes", Some(node_get_child_nodes), 0),
    ("__getParentNode", Some(node_get_parent_node), 0),
    ("__getParentElement", Some(node_get_parent_element), 0),
];

const NODE_GETTERS: &[NodeGetterBinding] = &[
    ("firstChild", "__getFirstChild"),
    ("lastChild", "__getLastChild"),
    ("previousSibling", "__getPreviousSibling"),
    ("nextSibling", "__getNextSibling"),
    ("children", "__getChildren"),
    ("childNodes", "__getChildNodes"),
    ("parentNode", "__getParentNode"),
    ("parentElement", "__getParentElement"),
];

unsafe fn define_int_constants(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    constants: &[NodeIntConstant],
) -> Result<(), String> {
    for (name, value) in constants {
        set_int_property(cx, obj, name, *value)?;
    }
    Ok(())
}

unsafe fn define_methods(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    methods: &[NodeMethodBinding],
) -> Result<(), String> {
    for (name, func, arity) in methods {
        define_function(cx, obj, name, *func, *arity)?;
    }
    Ok(())
}

unsafe fn define_getters(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    getters: &[NodeGetterBinding],
) -> Result<(), String> {
    for (property, getter) in getters {
        define_js_property_getter(cx, obj, property, getter)?;
    }
    Ok(())
}

unsafe fn node_matches_other_arg(
    safe_cx: &mut SafeJSContext,
    this_val: JSVal,
    other: Option<JSVal>,
) -> bool {
    let this_id = get_node_id_from_value(safe_cx, this_val);
    let other_id = other.and_then(|value| get_node_id_from_value(safe_cx, value));
    matches!((this_id, other_id), (Some(a), Some(b)) if a == b)
}

unsafe fn node_should_lookup_parent(cx: &mut SafeJSContext, args: &CallArgs) -> bool {
    let raw_cx = cx.raw_cx();
    let this_val = args.thisv();
    if !this_val.get().is_object() || this_val.get().is_null() {
        return false;
    }

    rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
    rooted!(in(raw_cx) let mut allow_parent = UndefinedValue());
    let allow_name = std::ffi::CString::new("__allowParentLookup").unwrap();
    if !JS_GetProperty(
        cx,
        this_obj.handle().into(),
        allow_name.as_ptr(),
        allow_parent.handle_mut().into(),
    ) {
        return false;
    }

    allow_parent.get().is_boolean() && allow_parent.get().to_boolean()
}

unsafe fn node_create_js_text_node_by_id(
    cx: &mut SafeJSContext,
    node_id: usize,
    text: &str,
) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let text_node = JS_NewPlainObject(raw_cx));
    if text_node.get().is_null() {
        return Err("Failed to create text node object".to_string());
    }

    set_int_property(cx, text_node.get(), "nodeType", 3)?;
    set_string_property(cx, text_node.get(), "nodeName", "#text")?;
    set_string_property(cx, text_node.get(), "nodeValue", text)?;
    set_string_property(cx, text_node.get(), "textContent", text)?;

    rooted!(in(raw_cx) let ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let text_rooted = text_node.get());
    let cname = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        raw_cx,
        text_rooted.handle().into(),
        cname.as_ptr(),
        ptr_val.handle().into(),
        0,
    );

    rooted!(in(raw_cx) let null_val = NullValue());
    for name in &["parentNode", "parentElement", "firstChild", "lastChild", "previousSibling", "nextSibling"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            raw_cx,
            text_rooted.handle().into(),
            cname.as_ptr(),
            null_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    rooted!(in(raw_cx) let child_nodes_array = create_empty_array(cx));
    if !child_nodes_array.get().is_null() {
        rooted!(in(raw_cx) let child_nodes_val = ObjectValue(child_nodes_array.get()));
        let cname = std::ffi::CString::new("childNodes").unwrap();
        JS_DefineProperty(
            raw_cx,
            text_rooted.handle().into(),
            cname.as_ptr(),
            child_nodes_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    Ok(ObjectValue(text_node.get()))
}

unsafe fn node_create_js_comment_node_by_id(
    cx: &mut SafeJSContext,
    node_id: usize,
) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let comment_node = JS_NewPlainObject(raw_cx));
    if comment_node.get().is_null() {
        return Err("Failed to create comment node object".to_string());
    }

    set_int_property(cx, comment_node.get(), "nodeType", 8)?;
    set_string_property(cx, comment_node.get(), "nodeName", "#comment")?;
    set_string_property(cx, comment_node.get(), "nodeValue", "")?;
    set_string_property(cx, comment_node.get(), "textContent", "")?;

    rooted!(in(raw_cx) let ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let comment_rooted = comment_node.get());
    let cname = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        raw_cx,
        comment_rooted.handle().into(),
        cname.as_ptr(),
        ptr_val.handle().into(),
        0,
    );

    rooted!(in(raw_cx) let null_val = NullValue());
    for name in &["parentNode", "parentElement", "firstChild", "lastChild", "previousSibling", "nextSibling"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            raw_cx,
            comment_rooted.handle().into(),
            cname.as_ptr(),
            null_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    rooted!(in(raw_cx) let child_nodes_array = create_empty_array(cx));
    if !child_nodes_array.get().is_null() {
        rooted!(in(raw_cx) let child_nodes_val = ObjectValue(child_nodes_array.get()));
        let cname = std::ffi::CString::new("childNodes").unwrap();
        JS_DefineProperty(
            raw_cx,
            comment_rooted.handle().into(),
            cname.as_ptr(),
            child_nodes_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    Ok(ObjectValue(comment_node.get()))
}

unsafe fn node_create_js_node_wrapper_by_id(
    cx: &mut SafeJSContext,
    node_id: usize,
) -> Option<JSVal> {
    enum NodeWrapperSeed {
        Element,
        Text(String),
        Comment,
    }

    let seed = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            if let Some(node) = dom.get_node(node_id) {
                return match &node.data {
                    NodeData::Element(_) => Some(NodeWrapperSeed::Element),
                    NodeData::Text(text_data) => Some(NodeWrapperSeed::Text(text_data.content.clone())),
                    NodeData::Comment => Some(NodeWrapperSeed::Comment),
                    _ => None,
                };
            }
        }
        None
    });

    match seed {
        Some(NodeWrapperSeed::Element) => create_js_element_by_dom_id(cx, node_id).ok(),
        Some(NodeWrapperSeed::Text(text)) => node_create_js_text_node_by_id(cx, node_id, &text).ok(),
        Some(NodeWrapperSeed::Comment) => node_create_js_comment_node_by_id(cx, node_id).ok(),
        None => None,
    }
}

unsafe fn node_build_parent_wrapper(cx: &mut SafeJSContext, node_id: usize) -> Option<JSVal> {
    let parent_id = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            return dom.get_node(node_id).and_then(|node| node.parent);
        }
        None
    })?;

    node_create_js_node_wrapper_by_id(cx, parent_id)
}

fn node_trigger_script_load_if_needed(child_id: usize) {
    let script_load_info = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = unsafe { &*dom_ptr };
            if let Some(node) = dom.get_node(child_id) {
                if let NodeData::Element(ref elem_data) = node.data {
                    if elem_data.name.local.as_ref() == "script" {
                        let script_type = elem_data
                            .attributes
                            .iter()
                            .find(|a| a.name.local.as_ref() == "type")
                            .map(|a| a.value.to_string());

                        let Some(script_kind) = executable_script_kind(script_type.as_deref()) else {
                            return None;
                        };

                        if let Some(src) = elem_data
                            .attributes
                            .iter()
                            .find(|a| a.name.local.as_ref() == "src")
                            .map(|a| a.value.to_string())
                        {
                            if let Some(url) = dom.url.resolve_relative(&src) {
                                return Some((
                                    url,
                                    dom.net_provider.clone(),
                                    dom.js_provider.clone(),
                                    child_id,
                                    script_kind,
                                ));
                            }
                        }
                    }
                }
            }
        }
        None
    });

    if let Some((url, net_provider, js_provider, script_node_id, script_kind)) = script_load_info {
        println!("[JS] Dynamically loading script: {}", url);
        let url_str = url.to_string();
        let module_source_url = (script_kind == ScriptKind::Module).then(|| url_str.clone());
        net_provider.fetch_with_callback(
            Request::get(url),
            Box::new(move |result| {
                match result {
                    Ok((_, bytes)) => {
                        match String::from_utf8(bytes.to_vec()) {
                            Ok(script) => {
                                if script_kind == ScriptKind::Module {
                                    js_provider.execute_module_script_with_node_id(
                                        script,
                                        script_node_id,
                                        module_source_url.clone(),
                                    );
                                } else {
                                    js_provider.execute_script_with_node_id(script, script_node_id);
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "[JS] Dynamic script at '{}' is not valid UTF-8: {}",
                                    url_str, e
                                )
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[JS] Failed to load dynamic script '{}': {:?}", url_str, e)
                    }
                }
            }),
        );
    }
}

/// Set up the global `Node` constructor and `Node.prototype` methods/constants.
pub(crate) unsafe fn setup_node_constructor_bindings(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let node = JS_NewPlainObject(raw_cx));
    if node.get().is_null() {
        return Err("Failed to create Node constructor".to_string());
    }

    define_int_constants(cx, node.get(), NODE_CONSTANTS)?;

    rooted!(in(raw_cx) let node_prototype = JS_NewPlainObject(raw_cx));
    if !node_prototype.get().is_null() {
        define_int_constants(cx, node_prototype.get(), NODE_CONSTANTS)?;
        define_node_bindings(cx, node_prototype.get())?;
        define_methods(cx, node_prototype.get(), NODE_CONSTRUCTOR_PROTO_METHODS)?;

        rooted!(in(raw_cx) let proto_val = ObjectValue(node_prototype.get()));
        rooted!(in(raw_cx) let node_rooted = node.get());
        let proto_name = std::ffi::CString::new("prototype").unwrap();
        JS_DefineProperty(
            raw_cx,
            node_rooted.handle().into(),
            proto_name.as_ptr(),
            proto_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    rooted!(in(raw_cx) let node_val = ObjectValue(node.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("Node").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        node_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

pub(crate) unsafe extern "C" fn node_append_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] node.appendChild() called");

    let child_id = if argc > 0 {
        match get_node_id_from_value(safe_cx, *args.get(0)) {
            Some(id) => id,
            None => {
                args.rval().set(UndefinedValue());
                return true;
            }
        }
    } else {
        args.rval().set(UndefinedValue());
        return true;
    };

    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            if let Some(parent_id) = get_node_id_from_this(safe_cx, &args) {
                dom.append_children(parent_id, &[child_id]);
            }
        }
    });
    custom_elements_upgrade_for_node(safe_cx, child_id);
    node_trigger_script_load_if_needed(child_id);

    args.rval().set(*args.get(0));
    true
}

pub(crate) unsafe extern "C" fn node_remove_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] node.removeChild() called");

    let child_id = if argc > 0 {
        match get_node_id_from_value(safe_cx, *args.get(0)) {
            Some(id) => id,
            None => {
                args.rval().set(UndefinedValue());
                return true;
            }
        }
    } else {
        args.rval().set(UndefinedValue());
        return true;
    };

    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            if get_node_id_from_this(safe_cx, &args).is_some() {
                dom.remove_node(child_id);
            }
        }
    });

    args.rval().set(*args.get(0));
    true
}

pub(crate) unsafe extern "C" fn node_insert_before(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] node.insertBefore() called");

    let new_child_id = if argc > 0 {
        match get_node_id_from_value(safe_cx, *args.get(0)) {
            Some(id) => id,
            None => {
                args.rval().set(UndefinedValue());
                return true;
            }
        }
    } else {
        args.rval().set(UndefinedValue());
        return true;
    };

    let reference_id = if argc > 1 {
        get_node_id_from_value(safe_cx, *args.get(1))
    } else {
        None
    };

    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            if let Some(parent_id) = get_node_id_from_this(safe_cx, &args) {
                match reference_id {
                    Some(ref_id) => dom.insert_nodes_before(ref_id, &[new_child_id]),
                    None => dom.append_children(parent_id, &[new_child_id]),
                }
            }
        }
    });
    custom_elements_upgrade_for_node(safe_cx, new_child_id);
    node_trigger_script_load_if_needed(new_child_id);

    args.rval().set(*args.get(0));
    true
}

pub(crate) unsafe extern "C" fn node_replace_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] node.replaceChild() called");

    if argc >= 2 {
        let parent_id = get_node_id_from_this(safe_cx, &args);
        let new_child_id = get_node_id_from_value(safe_cx, *args.get(0));
        let old_child_id = get_node_id_from_value(safe_cx, *args.get(1));

        if let (Some(parent_id), Some(new_child_id), Some(old_child_id)) =
            (parent_id, new_child_id, old_child_id)
        {
            DOM_REF.with(|dom| {
                if let Some(dom_ptr) = *dom.borrow() {
                    let dom = &mut *dom_ptr;
                    if dom.parent_id(old_child_id) == Some(parent_id) {
                        dom.replace_node_with(old_child_id, &[new_child_id]);
                    }
                }
            });

            custom_elements_upgrade_for_node(safe_cx, new_child_id);
            node_trigger_script_load_if_needed(new_child_id);
            args.rval().set(*args.get(1));
            return true;
        }
    }

    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn node_clone_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let deep = if argc > 0 {
        let val = *args.get(0);
        val.is_boolean() && val.to_boolean()
    } else {
        false
    };

    trace!("[JS] node.cloneNode({}) called", deep);

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let element_data = DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        return Some((
                            elem_data.name.local.to_string(),
                            elem_data.attributes.clone(),
                        ));
                    }
                }
            }
            None
        });

        if let Some((tag_name, attributes)) = element_data {
            if let Ok(elem) = create_js_element_by_id(safe_cx, 0, &tag_name, &attributes) {
                args.rval().set(elem);
                return true;
            }
        }
    }

    let this_val = args.thisv();
    if this_val.get().is_object() && !this_val.get().is_null() {
        rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
        rooted!(in(raw_cx) let mut tag_val = UndefinedValue());

        let cname = std::ffi::CString::new("tagName").unwrap();
        if JS_GetProperty(
            safe_cx,
            this_obj.handle().into(),
            cname.as_ptr(),
            tag_val.handle_mut().into(),
        ) {
            if tag_val.get().is_string() {
                let tag_name = js_value_to_string(safe_cx, tag_val.get());
                if let Ok(elem) = create_stub_element(safe_cx, &tag_name.to_lowercase()) {
                    args.rval().set(elem);
                    return true;
                }
            }
        }
    }

    match create_stub_element(safe_cx, "div") {
        Ok(elem) => args.rval().set(elem),
        Err(_) => args.rval().set(NullValue()),
    }
    true
}

pub(crate) unsafe extern "C" fn node_contains(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] node.contains() called");

    let this_node_id = get_node_id_from_this(safe_cx, &args);
    let other_node_id = if argc > 0 {
        get_node_id_from_value(safe_cx, *args.get(0))
    } else {
        None
    };

    let result = match (this_node_id, other_node_id) {
        (Some(this_id), Some(other_id)) => {
            if this_id == other_id {
                true
            } else {
                DOM_REF.with(|dom_ref| {
                    if let Some(dom_ptr) = *dom_ref.borrow() {
                        let dom = &*dom_ptr;
                        let mut current_id = Some(other_id);

                        while let Some(id) = current_id {
                            if let Some(node) = dom.get_node(id) {
                                if node.parent == Some(this_id) {
                                    return true;
                                }
                                current_id = node.parent;
                            } else {
                                break;
                            }
                        }
                    }
                    false
                })
            }
        }
        _ => false,
    };

    args.rval().set(BooleanValue(result));
    true
}

pub(crate) unsafe extern "C" fn node_has_child_nodes(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let result = if let Some(node_id) = get_node_id_from_value(safe_cx, args.thisv().get()) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    return !node.children.is_empty();
                }
            }
            false
        })
    } else {
        false
    };
    args.rval().set(BooleanValue(result));
    true
}

pub(crate) unsafe extern "C" fn node_get_parent_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if !node_should_lookup_parent(safe_cx, &args) {
        args.rval().set(NullValue());
        return true;
    }

    let parent_val =
        get_node_id_from_this(safe_cx, &args).and_then(|node_id| node_build_parent_wrapper(safe_cx, node_id));
    args.rval().set(parent_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn node_get_parent_element(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    node_get_parent_node(raw_cx, argc, vp)
}

pub(crate) unsafe extern "C" fn node_get_first_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let first_child_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    return node
                        .children
                        .first()
                        .copied()
                        .and_then(|id| node_create_js_node_wrapper_by_id(safe_cx, id));
                }
            }
            None
        })
    });

    args.rval().set(first_child_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn node_get_last_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let last_child_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    return node
                        .children
                        .last()
                        .copied()
                        .and_then(|id| node_create_js_node_wrapper_by_id(safe_cx, id));
                }
            }
            None
        })
    });

    args.rval().set(last_child_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn node_get_previous_sibling(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let previous_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                return dom
                    .previous_sibling_id(node_id)
                    .and_then(|id| node_create_js_node_wrapper_by_id(safe_cx, id));
            }
            None
        })
    });

    args.rval().set(previous_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn node_get_next_sibling(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let next_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                return dom
                    .next_sibling_id(node_id)
                    .and_then(|id| node_create_js_node_wrapper_by_id(safe_cx, id));
            }
            None
        })
    });

    args.rval().set(next_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn node_get_children(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    rooted!(in(raw_cx) let children_array = create_empty_array(safe_cx));
    if !children_array.get().is_null() {
        let child_ids = get_node_id_from_this(safe_cx, &args)
            .map(|node_id| {
                DOM_REF.with(|dom_ref| {
                    if let Some(dom_ptr) = *dom_ref.borrow() {
                        let dom = &*dom_ptr;
                        if let Some(node) = dom.get_node(node_id) {
                            return node
                                .children
                                .iter()
                                .copied()
                                .filter(|child_id| {
                                    dom.get_node(*child_id)
                                        .is_some_and(|child| matches!(child.data, NodeData::Element(_)))
                                })
                                .collect::<Vec<usize>>();
                        }
                    }
                    Vec::new()
                })
            })
            .unwrap_or_default();

        for (idx, child_id) in child_ids.iter().enumerate() {
            if let Some(child_val) = node_create_js_node_wrapper_by_id(safe_cx, *child_id) {
                rooted!(in(raw_cx) let child_rooted = child_val);
                rooted!(in(raw_cx) let children_obj = children_array.get());
                JS_SetElement(
                    safe_cx,
                    children_obj.handle().into(),
                    idx as u32,
                    child_rooted.handle().into(),
                );
            }
        }
    }

    args.rval().set(ObjectValue(children_array.get()));
    true
}

pub(crate) unsafe extern "C" fn node_get_child_nodes(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    rooted!(in(raw_cx) let child_nodes_array = create_empty_array(safe_cx));
    if !child_nodes_array.get().is_null() {
        let child_ids = get_node_id_from_this(safe_cx, &args)
            .map(|node_id| {
                DOM_REF.with(|dom_ref| {
                    if let Some(dom_ptr) = *dom_ref.borrow() {
                        let dom = &*dom_ptr;
                        if let Some(node) = dom.get_node(node_id) {
                            return node.children.clone();
                        }
                    }
                    Vec::new()
                })
            })
            .unwrap_or_default();

        for (idx, child_id) in child_ids.iter().enumerate() {
            if let Some(child_val) = node_create_js_node_wrapper_by_id(safe_cx, *child_id) {
                rooted!(in(raw_cx) let child_rooted = child_val);
                rooted!(in(raw_cx) let child_nodes_obj = child_nodes_array.get());
                JS_SetElement(
                    safe_cx,
                    child_nodes_obj.handle().into(),
                    idx as u32,
                    child_rooted.handle().into(),
                );
            }
        }
    }

    args.rval().set(ObjectValue(child_nodes_array.get()));
    true
}

pub(crate) unsafe extern "C" fn node_get_root_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] node.getRootNode() called");

    let composed = if argc > 0 {
        let options = *args.get(0);
        if options.is_object() && !options.is_null() {
            rooted!(in(raw_cx) let options_obj = options.to_object());
            rooted!(in(raw_cx) let mut composed_val = UndefinedValue());
            let composed_name = std::ffi::CString::new("composed").unwrap();
            JS_GetProperty(
                safe_cx,
                options_obj.handle().into(),
                composed_name.as_ptr(),
                composed_val.handle_mut().into(),
            ) && composed_val.get().is_boolean()
        } else {
            false
        }
    } else {
        false
    };

    let Some(start_id) = get_node_id_from_this(safe_cx, &args) else {
        args.rval().set(NullValue());
        return true;
    };

    enum RootKind {
        Document,
        ShadowRoot,
        Element,
    }

    let root_info: Option<(usize, RootKind)> = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            let mut current_id = start_id;
            loop {
                let Some(node) = dom.get_node(current_id) else {
                    return None;
                };
                if let Some(parent_id) = node.parent {
                    current_id = parent_id;
                } else if let Some(host_id) = node.shadow_host {
                    if composed {
                        current_id = host_id;
                    } else {
                        return Some((current_id, RootKind::ShadowRoot));
                    }
                } else {
                    let kind = match &node.data {
                        NodeData::Document => RootKind::Document,
                        _ => RootKind::Element,
                    };
                    return Some((current_id, kind));
                }
            }
        }
        None
    });

    match root_info {
        Some((_, RootKind::Document)) => {
            rooted!(in(raw_cx) let global = CurrentGlobalOrNull(safe_cx));
            if !global.is_null() {
                rooted!(in(raw_cx) let mut doc_val = UndefinedValue());
                let doc_name = std::ffi::CString::new("document").unwrap();

                if JS_GetProperty(
                    safe_cx,
                    global.handle().into(),
                    doc_name.as_ptr(),
                    doc_val.handle_mut().into(),
                ) {
                    args.rval().set(doc_val.get());
                    return true;
                }
            }
            args.rval().set(NullValue());
        }
        Some((shadow_root_id, RootKind::ShadowRoot)) => {
            match create_js_shadow_root_by_id(safe_cx, shadow_root_id) {
                Ok(val) => args.rval().set(val),
                Err(_) => args.rval().set(NullValue()),
            }
        }
        Some((root_id, RootKind::Element)) => {
            if root_id == start_id {
                args.rval().set(args.thisv().get());
            } else if let Ok(val) = create_js_element_by_dom_id(safe_cx, root_id) {
                args.rval().set(val);
            } else {
                args.rval().set(NullValue());
            }
        }
        None => {
            args.rval().set(NullValue());
        }
    }
    true
}

pub(crate) unsafe extern "C" fn node_normalize(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn!("[JS] node.normalize() called (stub)");
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn node_is_equal_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let result = node_matches_other_arg(
        safe_cx,
        args.thisv().get(),
        if argc > 0 { Some(*args.get(0)) } else { None },
    );
    args.rval().set(BooleanValue(result));
    true
}

pub(crate) unsafe extern "C" fn node_is_same_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let result = node_matches_other_arg(
        safe_cx,
        args.thisv().get(),
        if argc > 0 { Some(*args.get(0)) } else { None },
    );
    args.rval().set(BooleanValue(result));
    true
}

pub(crate) unsafe extern "C" fn node_compare_document_position(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let this_id = get_node_id_from_value(safe_cx, args.thisv().get());
    let other_id = if argc > 0 {
        get_node_id_from_value(safe_cx, *args.get(0))
    } else {
        None
    };

    let bits: i32 = match (this_id, other_id) {
        (Some(a), Some(b)) if a == b => 0,
        (Some(this_id), Some(other_id)) => {
            DOM_REF.with(|dom_ref| {
                if let Some(dom_ptr) = *dom_ref.borrow() {
                    let dom = &*dom_ptr;

                    let ancestors_of = |mut id: usize| -> Vec<usize> {
                        let mut chain = vec![id];
                        while let Some(node) = dom.get_node(id) {
                            if let Some(p) = node.parent {
                                chain.push(p);
                                id = p;
                            } else {
                                break;
                            }
                        }
                        chain
                    };

                    let this_chain = ancestors_of(this_id);
                    let other_chain = ancestors_of(other_id);

                    if this_chain.contains(&other_id) {
                        return 8 | 2;
                    }
                    if other_chain.contains(&this_id) {
                        return 16 | 4;
                    }

                    for &t_anc in &this_chain {
                        if other_chain.contains(&t_anc) {
                            let child_index = |parent_id: usize, child_id: usize| -> Option<usize> {
                                if let Some(parent_node) = dom.get_node(parent_id) {
                                    return parent_node.children.iter().position(|&c| c == child_id);
                                }
                                None
                            };
                            let this_branch = this_chain
                                .iter()
                                .rev()
                                .find(|&&n| n != t_anc && dom.get_node(n).is_some_and(|nd| nd.parent == Some(t_anc)))
                                .copied()
                                .unwrap_or(this_id);
                            let other_branch = other_chain
                                .iter()
                                .rev()
                                .find(|&&n| n != t_anc && dom.get_node(n).is_some_and(|nd| nd.parent == Some(t_anc)))
                                .copied()
                                .unwrap_or(other_id);
                            let this_idx = child_index(t_anc, this_branch).unwrap_or(0);
                            let other_idx = child_index(t_anc, other_branch).unwrap_or(0);
                            return if other_idx < this_idx { 2 } else { 4 };
                        }
                    }
                    1
                } else {
                    1
                }
            })
        }
        _ => 1,
    };

    args.rval().set(Int32Value(bits));
    true
}

pub(crate) unsafe fn define_node_bindings(
    cx: &mut SafeJSContext,
    proto: *mut JSObject,
) -> Result<(), String> {
    define_methods(cx, proto, NODE_SHARED_METHODS)?;
    define_methods(cx, proto, NODE_INTERNAL_GETTERS)?;
    define_getters(cx, proto, NODE_GETTERS)?;

    Ok(())
}

pub(crate) unsafe extern "C" fn node_lookup_prefix(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn!("[JS] node.lookupPrefix() called (stub)");
    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn node_lookup_namespace_uri(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn!("[JS] node.lookupNamespaceURI() called (stub)");
    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn node_is_default_namespace(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn!("[JS] node.isDefaultNamespace() called (stub)");
    args.rval().set(BooleanValue(false));
    true
}

