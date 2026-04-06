// Element bindings for JavaScript using mozjs
use blitz_traits::net::Request;
use crate::dom::{AttributeMap, NodeData, ShadowRootMode};
use crate::dom::events::focus::generate_focus_events;
use crate::engine::js_provider::ScriptKind;
use crate::engine::script_type::executable_script_kind;
use crate::events::DomEvent;
use crate::js::bindings::custom_elements::custom_elements_upgrade_for_node;
use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::helpers::{create_empty_array, create_js_string, define_function, define_js_property_accessor, define_js_property_getter, get_node_id_from_this, get_node_id_from_value, js_value_to_string, set_int_property, set_string_property, to_css_property_name, ToSafeCx};
use crate::js::selectors::{matches_parsed_selector, parse_selector, selector_seed, SelectorSeed};
use crate::js::bindings::element;
pub(crate) use crate::js::bindings::element::{
    element_add_event_listener, element_dispatch_event, element_remove_event_listener,
};
use html5ever::ns;
use html5ever::local_name;
use markup5ever::QualName;
use mozjs::jsapi::{CallArgs, HandleValueArray, JSContext, JSObject, JSPROP_ENUMERATE};
use mozjs::rust::wrappers2::{AddRawValueRoot, CurrentGlobalOrNull, JS_CallFunctionValue, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject, JS_SetElement, JS_SetProperty, RemoveRawValueRoot};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::Heap;
use mozjs::jsval::{BooleanValue, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::Runtime;
use mozjs::rust::ValueArray;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_uint;
use blitz_traits::shell::ShellProvider;
use tracing::{trace, warn};
use crate::js::bindings::event_listeners;
use crate::js::bindings::warnings::warn_stubbed_binding;

struct PinnedElementWrapper {
    rooted_value: Box<Heap<JSVal>>,
}

impl PinnedElementWrapper {
    unsafe fn new(cx: &mut SafeJSContext, obj: *mut JSObject) -> Self {
        let rooted_value: Box<Heap<JSVal>> = Box::new(Heap::default());
        rooted_value.set(ObjectValue(obj));
        let name = std::ffi::CString::new("PinnedElementWrapper").unwrap();
        AddRawValueRoot(cx, rooted_value.get_unsafe(), name.as_ptr());
        Self { rooted_value }
    }

    #[inline]
    fn get_object(&self) -> Option<*mut JSObject> {
        let val = self.rooted_value.get();
        if val.is_object() {
            Some(val.to_object())
        } else {
            None
        }
    }
}

impl Drop for PinnedElementWrapper {
    fn drop(&mut self) {
        unsafe {
            if let Some(cx) = Runtime::get() {
                RemoveRawValueRoot(&cx.to_safe_cx(), self.rooted_value.get_unsafe());
            }
        }
    }
}

thread_local! {
    static ELEMENT_WRAPPER_CACHE: RefCell<HashMap<usize, PinnedElementWrapper>> = RefCell::new(HashMap::new());
}

pub fn clear_element_wrapper_cache() {
    ELEMENT_WRAPPER_CACHE.with(|cache| cache.borrow_mut().clear());
}

unsafe fn get_cached_element_wrapper(node_id: usize) -> Option<*mut JSObject> {
    ELEMENT_WRAPPER_CACHE.with(|cache| {
        cache
            .borrow()
            .get(&node_id)
            .and_then(PinnedElementWrapper::get_object)
    })
}

unsafe fn cache_element_wrapper(cx: &mut SafeJSContext, node_id: usize, obj: *mut JSObject) {
    ELEMENT_WRAPPER_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(node_id, PinnedElementWrapper::new(cx, obj));
    });
}

fn has_backing_dom_node(node_id: usize) -> bool {
    DOM_REF.with(|dom_ref| {
        dom_ref
            .borrow()
            .as_ref()
            .is_some_and(|dom_ptr| unsafe { (&**dom_ptr).get_node(node_id).is_some() })
    })
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

fn is_descendant_of(dom: &crate::dom::Dom, root_id: usize, candidate_id: usize) -> bool {
    let mut current = Some(candidate_id);
    while let Some(cur_id) = current {
        if cur_id == root_id {
            return true;
        }
        current = dom.get_node(cur_id).and_then(|node| node.parent);
    }
    false
}

unsafe fn maybe_patch_mutation_observer_node(cx: &mut SafeJSContext, node_obj: *mut JSObject) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return;
    }

    rooted!(in(raw_cx) let mut patch_fn = UndefinedValue());
    let patch_name = std::ffi::CString::new("__stokesPatchMutationObserverNode").unwrap();
    if !JS_GetProperty(
        cx,
        global.handle().into(),
        patch_name.as_ptr(),
        patch_fn.handle_mut().into(),
    ) {
        return;
    }

    if !patch_fn.get().is_object() {
        return;
    }

    rooted!(in(raw_cx) let node_rooted = node_obj);
    rooted!(in(raw_cx) let call_args = ValueArray::<0usize>::new([]));
    rooted!(in(raw_cx) let mut rval = UndefinedValue());
    let _ = JS_CallFunctionValue(
        cx,
        node_rooted.handle().into(),
        patch_fn.handle().into(),
        &HandleValueArray::from(&call_args),
        rval.handle_mut().into(),
    );
}

pub(crate) unsafe fn set_object_prototype(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    proto: *mut JSObject,
) -> Result<(), String> {
    if obj.is_null() || proto.is_null() {
        return Err("Cannot set prototype on null object".to_string());
    }
    if obj == proto {
        return Ok(());
    }

    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return Err("No global object for prototype setup".to_string());
    }

    rooted!(in(raw_cx) let mut object_ctor_val = UndefinedValue());
    let object_name = std::ffi::CString::new("Object").unwrap();
    if !JS_GetProperty(
        cx,
        global.handle().into(),
        object_name.as_ptr(),
        object_ctor_val.handle_mut().into(),
    ) || !object_ctor_val.get().is_object() {
        return Err("Failed to resolve global Object constructor".to_string());
    }
    rooted!(in(raw_cx) let object_ctor_obj = object_ctor_val.get().to_object());

    let set_prototype_cache_name = std::ffi::CString::new("__stokesSetPrototypeOfFn").unwrap();
    rooted!(in(raw_cx) let mut set_prototype_fn = UndefinedValue());
    if !JS_GetProperty(
        cx,
        global.handle().into(),
        set_prototype_cache_name.as_ptr(),
        set_prototype_fn.handle_mut().into(),
    ) || !set_prototype_fn.get().is_object() {
        let set_prototype_name = std::ffi::CString::new("setPrototypeOf").unwrap();
        if !JS_GetProperty(
            cx,
            object_ctor_obj.handle().into(),
            set_prototype_name.as_ptr(),
            set_prototype_fn.handle_mut().into(),
        ) || !set_prototype_fn.get().is_object() {
            return Err("Failed to resolve Object.setPrototypeOf".to_string());
        }

        JS_DefineProperty(
            cx,
            global.handle().into(),
            set_prototype_cache_name.as_ptr(),
            set_prototype_fn.handle().into(),
            0,
        );
    }

    rooted!(in(raw_cx) let set_proto_args = ValueArray::<2usize>::new([
        ObjectValue(obj),
        ObjectValue(proto),
    ]));
    rooted!(in(raw_cx) let mut rval = UndefinedValue());
    if !JS_CallFunctionValue(
        cx,
        object_ctor_obj.handle().into(),
        set_prototype_fn.handle().into(),
        &HandleValueArray::from(&set_proto_args),
        rval.handle_mut().into(),
    ) {
        return Err("Object.setPrototypeOf call failed".to_string());
    }

    Ok(())
}

pub(crate) unsafe fn ensure_element_shared_prototype(cx: &mut SafeJSContext) -> Result<*mut JSObject, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return Err("No global object".to_string());
    }

    let key = std::ffi::CString::new("__stokesElementPrototype").unwrap();
    rooted!(in(raw_cx) let mut existing = UndefinedValue());
    if JS_GetProperty(
        cx,
        global.handle().into(),
        key.as_ptr(),
        existing.handle_mut().into(),
    ) && existing.get().is_object() {
        return Ok(existing.get().to_object());
    }

    rooted!(in(raw_cx) let proto = JS_NewPlainObject(cx));
    if proto.get().is_null() {
        return Err("Failed to create element prototype object".to_string());
    }

    // Element prototype only owns Element-specific APIs; Node/EventTarget now flow via parent links.
    element::define_element_bindings(cx, proto.get())?;

    rooted!(in(raw_cx) let proto_val = ObjectValue(proto.get()));
    JS_DefineProperty(
        cx,
        global.handle().into(),
        key.as_ptr(),
        proto_val.handle().into(),
        0,
    );

    Ok(proto.get())
}

fn constructor_name_for_element(is_svg: bool, local_name: &str) -> &'static str {
    if !is_svg {
        if local_name.eq_ignore_ascii_case("input") {
            return "HTMLInputElement";
        }
        if local_name.eq_ignore_ascii_case("form") {
            return "HTMLFormElement";
        }
        if local_name.eq_ignore_ascii_case("iframe") {
            return "HTMLIFrameElement";
        }
        if local_name.eq_ignore_ascii_case("img") {
            return "HTMLImageElement";
        }
        return "HTMLElement";
    }

    if local_name.eq_ignore_ascii_case("svg") {
        "SVGSVGElement"
    } else if local_name.eq_ignore_ascii_case("rect") {
        "SVGRectElement"
    } else {
        "SVGElement"
    }
}

/// Create a JS element wrapper for a DOM node with its real tag name and attributes
pub unsafe fn create_js_element_by_id(
    cx: &mut mozjs::context::JSContext,
    node_id: usize,
    tag_name: &str,
    attributes: &AttributeMap,
) -> Result<JSVal, String> {
    create_js_element_impl(cx, node_id, tag_name, attributes, true, true)
}

/// Create a JS element wrapper from a DOM node id without requiring caller-side
/// tag/attribute extraction and cloning.
pub unsafe fn create_js_element_by_dom_id(
    cx: &mut mozjs::context::JSContext,
    node_id: usize,
) -> Result<JSVal, String> {
    create_js_element_impl(cx, node_id, "", &AttributeMap::empty(), true, true)
}

/// Internal element wrapper builder.
/// `with_parent` controls whether lazy parent lookup is enabled for this wrapper.
/// When false, `parentNode`/`parentElement` resolve to null.
unsafe fn create_js_element_impl(
    cx: &mut SafeJSContext,
    node_id: usize,
    tag_name: &str,
    _attributes: &AttributeMap,
    with_parent: bool,
    _with_tree_relations: bool,
) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();

    let cacheable_node = has_backing_dom_node(node_id);
    if cacheable_node {
        if let Some(cached_obj) = get_cached_element_wrapper(node_id) {
            return Ok(ObjectValue(cached_obj));
        }
    }

    rooted!(in(raw_cx) let element = JS_NewPlainObject(cx));
    if element.get().is_null() {
        return Err("Failed to create element object".to_string());
    }

    let (resolved_local_name, namespace_uri, is_html_element, is_svg_element, constructor_name) =
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        let local_name = elem_data.name.local.to_string();
                        let namespace_uri = if elem_data.name.ns == ns!(html) {
                            Some("http://www.w3.org/1999/xhtml".to_string())
                        } else if elem_data.name.ns == ns!(svg) {
                            Some("http://www.w3.org/2000/svg".to_string())
                        } else if elem_data.name.ns == ns!(mathml) {
                            Some("http://www.w3.org/1998/Math/MathML".to_string())
                        } else {
                            let ns_str = elem_data.name.ns.to_string();
                            if ns_str.is_empty() { None } else { Some(ns_str) }
                        };
                        let is_html = elem_data.name.ns == ns!(html);
                        let is_svg = elem_data.name.ns == ns!(svg);
                        let constructor = constructor_name_for_element(is_svg, &local_name);
                        return (local_name, namespace_uri, is_html, is_svg, constructor.to_string());
                    }
                }
            }

            // Fallback for synthetic/stub elements not backed by a DOM node.
            (
                tag_name.to_string(),
                Some("http://www.w3.org/1999/xhtml".to_string()),
                true,
                false,
                "HTMLElement".to_string(),
            )
        });

    let display_name = if is_html_element {
        resolved_local_name.to_uppercase()
    } else {
        resolved_local_name.clone()
    };

    // Set basic properties
    set_string_property(cx, element.get(), "tagName", &display_name)?;
    set_string_property(cx, element.get(), "nodeName", &display_name)?;
    set_string_property(cx, element.get(), "localName", &resolved_local_name)?;
    set_int_property(cx, element.get(), "nodeType", 1)?; // ELEMENT_NODE
    // id/className are exposed through reflected accessors defined below.
    // FIXME: innerHTML always returns "" instead of serializing the element's child nodes as HTML.
    set_string_property(cx, element.get(), "innerHTML", "")?;
    // FIXME: outerHTML returns a stub "<tag></tag>" instead of the element's full serialized HTML
    // including its attributes and child subtree.
    set_string_property(cx, element.get(), "outerHTML", &format!("<{0}></{0}>", resolved_local_name))?;
    // Note: textContent will be defined as a property accessor below

    // Store the backing DOM node id.
    rooted!(in(raw_cx) let ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let element_rooted = element.get());
    let cname = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        cx,
        element_rooted.handle().into(),
        cname.as_ptr(),
        ptr_val.handle().into(),
        0, // Hidden property
    );

    // Expose namespaceURI as null for no-namespace nodes and as a URI string otherwise.
    let namespace_name = std::ffi::CString::new("namespaceURI").unwrap();
    if let Some(uri) = namespace_uri.as_deref() {
        rooted!(in(raw_cx) let namespace_val = create_js_string(cx, uri));
        JS_DefineProperty(
            cx,
            element_rooted.handle().into(),
            namespace_name.as_ptr(),
            namespace_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    } else {
        rooted!(in(raw_cx) let null_namespace = NullValue());
        JS_DefineProperty(
            cx,
            element_rooted.handle().into(),
            namespace_name.as_ptr(),
            null_namespace.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    rooted!(in(raw_cx) let svg_marker = BooleanValue(is_svg_element));
    let svg_flag_name = std::ffi::CString::new("__isSvgElement").unwrap();
    JS_DefineProperty(
        cx,
        element_rooted.handle().into(),
        svg_flag_name.as_ptr(),
        svg_marker.handle().into(),
        0,
    );

    rooted!(in(raw_cx) let allow_parent_lookup = BooleanValue(with_parent));
    let allow_parent_name = std::ffi::CString::new("__allowParentLookup").unwrap();
    JS_DefineProperty(
        cx,
        element_rooted.handle().into(),
        allow_parent_name.as_ptr(),
        allow_parent_lookup.handle().into(),
        0,
    );

    // Point element.constructor at the best available global constructor object.
    let mut instance_prototype = std::ptr::null_mut::<JSObject>();
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(cx));
    if !global.get().is_null() {
        let ctor_name = std::ffi::CString::new(constructor_name).unwrap();
        rooted!(in(raw_cx) let mut ctor_val = UndefinedValue());
        if JS_GetProperty(cx, global.handle().into(), ctor_name.as_ptr(), ctor_val.handle_mut().into())
            && ctor_val.get().is_object()
        {
            let constructor_prop = std::ffi::CString::new("constructor").unwrap();
            JS_DefineProperty(
                cx,
                element_rooted.handle().into(),
                constructor_prop.as_ptr(),
                ctor_val.handle().into(),
                JSPROP_ENUMERATE as u32,
            );

            let prototype_prop = std::ffi::CString::new("prototype").unwrap();
            rooted!(in(raw_cx) let mut ctor_proto_val = UndefinedValue());
            rooted!(in(raw_cx) let ctor_obj = ctor_val.get().to_object());
            if JS_GetProperty(
                cx,
                ctor_obj.handle().into(),
                prototype_prop.as_ptr(),
                ctor_proto_val.handle_mut().into(),
            ) && ctor_proto_val.get().is_object() {
                instance_prototype = ctor_proto_val.get().to_object();
            }
        }
    }

    let shared_proto = ensure_element_shared_prototype(cx)?;
    if !instance_prototype.is_null() {
        // Ensure element-specific prototypes inherit the shared Element behavior.
        if instance_prototype != shared_proto {
            if let Err(err) = set_object_prototype(cx, instance_prototype, shared_proto) {
                warn!(
                    "[JS] Failed to link constructor prototype for <{}> (node {}): {}",
                    resolved_local_name,
                    node_id,
                    err
                );
            }
        }
    } else {
        instance_prototype = shared_proto;
    }
    if let Err(err) = set_object_prototype(cx, element.get(), instance_prototype) {
        warn!(
            "[JS] Failed to link instance prototype for <{}> (node {}): {}",
            resolved_local_name,
            node_id,
            err
        );
    }

    // Keep one stable JS wrapper per DOM node so framework internals attached
    // to element objects survive across lookups and event dispatch.
    if cacheable_node {
        cache_element_wrapper(cx, node_id, element.get());
    }

    if resolved_local_name.eq_ignore_ascii_case("form") {
        setup_form_element_bindings(cx, element.get())?;
    }

    // style/classList/dataset are lazily created by accessors to reduce wrapper setup cost.

    maybe_patch_mutation_observer_node(cx, element.get());

    Ok(ObjectValue(element.get()))
}

/// Create a stub element
pub unsafe fn create_stub_element(cx: &mut mozjs::context::JSContext, tag_name: &str) -> Result<JSVal, String> {
    // Create element with no attributes
    create_js_element_by_id(cx, 0, tag_name, &AttributeMap::empty())
}

unsafe fn create_js_text_node_by_id(
    cx: &mut SafeJSContext,
    node_id: usize,
    text: &str,
) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let text_node = JS_NewPlainObject(cx));
    if text_node.get().is_null() {
        return Err("Failed to create text node object".to_string());
    }

    set_int_property(cx, text_node.get(), "nodeType", 3)?; // TEXT_NODE
    set_string_property(cx, text_node.get(), "nodeName", "#text")?;
    set_string_property(cx, text_node.get(), "nodeValue", text)?;
    set_string_property(cx, text_node.get(), "textContent", text)?;
    define_function(cx, text_node.get(), "hasChildNodes", Some(element_has_child_nodes), 0)?;

    rooted!(in(raw_cx) let ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let text_rooted = text_node.get());
    let cname = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        cx,
        text_rooted.handle().into(),
        cname.as_ptr(),
        ptr_val.handle().into(),
        0,
    );

    rooted!(in(raw_cx) let null_val = NullValue());
    for name in &["parentNode", "parentElement", "firstChild", "lastChild", "previousSibling", "nextSibling"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            cx,
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
            cx,
            text_rooted.handle().into(),
            cname.as_ptr(),
            child_nodes_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    Ok(ObjectValue(text_node.get()))
}

unsafe fn create_js_comment_node_by_id(cx: &mut SafeJSContext, node_id: usize) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let comment_node = JS_NewPlainObject(cx));
    if comment_node.get().is_null() {
        return Err("Failed to create comment node object".to_string());
    }

    set_int_property(cx, comment_node.get(), "nodeType", 8)?; // COMMENT_NODE
    set_string_property(cx, comment_node.get(), "nodeName", "#comment")?;
    set_string_property(cx, comment_node.get(), "nodeValue", "")?;
    set_string_property(cx, comment_node.get(), "textContent", "")?;
    define_function(cx, comment_node.get(), "hasChildNodes", Some(element_has_child_nodes), 0)?;

    rooted!(in(raw_cx) let ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let comment_rooted = comment_node.get());
    let cname = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        cx,
        comment_rooted.handle().into(),
        cname.as_ptr(),
        ptr_val.handle().into(),
        0,
    );

    rooted!(in(raw_cx) let null_val = NullValue());
    for name in &["parentNode", "parentElement", "firstChild", "lastChild", "previousSibling", "nextSibling"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            cx,
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
            cx,
            comment_rooted.handle().into(),
            cname.as_ptr(),
            child_nodes_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    Ok(ObjectValue(comment_node.get()))
}

unsafe fn create_js_node_wrapper_by_id(cx: &mut SafeJSContext, node_id: usize) -> Option<JSVal> {
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
        Some(NodeWrapperSeed::Element) => create_js_element_impl(cx, node_id, "", &AttributeMap::empty(), false, false).ok(),
        Some(NodeWrapperSeed::Text(text)) => create_js_text_node_by_id(cx, node_id, &text).ok(),
        Some(NodeWrapperSeed::Comment) => create_js_comment_node_by_id(cx, node_id).ok(),
        None => None,
    }
}

// ============================================================================
// Local helper functions
// ============================================================================

/// Convert a hyphen-case string to camelCase (for dataset key conversion).
/// E.g. "foo-bar-baz" → "fooBarBaz"
fn hyphen_to_camel_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            for c in ch.to_uppercase() {
                result.push(c);
            }
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

unsafe fn create_style_object_for_node(cx: &mut SafeJSContext, node_id: usize) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let style = JS_NewPlainObject(cx));
    if style.get().is_null() {
        return Err("Failed to create style object".to_string());
    }

    rooted!(in(raw_cx) let style_ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let style_rooted = style.get());
    let style_id_name = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        cx,
        style_rooted.handle().into(),
        style_id_name.as_ptr(),
        style_ptr_val.handle().into(),
        0,
    );

    define_function(cx, style.get(), "getPropertyValue", Some(style_get_property_value), 1)?;
    define_function(cx, style.get(), "setProperty", Some(style_set_property), 3)?;
    define_function(cx, style.get(), "removeProperty", Some(style_remove_property), 1)?;

    Ok(ObjectValue(style.get()))
}

unsafe fn create_class_list_object_for_node(cx: &mut SafeJSContext, node_id: usize) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let class_list = JS_NewPlainObject(cx));
    if class_list.get().is_null() {
        return Err("Failed to create classList object".to_string());
    }

    rooted!(in(raw_cx) let cl_ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let class_list_rooted = class_list.get());
    let cl_id_name = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        cx,
        class_list_rooted.handle().into(),
        cl_id_name.as_ptr(),
        cl_ptr_val.handle().into(),
        0,
    );

    define_function(cx, class_list.get(), "add", Some(class_list_add), 1)?;
    define_function(cx, class_list.get(), "remove", Some(class_list_remove), 1)?;
    define_function(cx, class_list.get(), "toggle", Some(class_list_toggle), 2)?;
    define_function(cx, class_list.get(), "contains", Some(class_list_contains), 1)?;
    define_function(cx, class_list.get(), "replace", Some(class_list_replace), 2)?;
    set_int_property(cx, class_list.get(), "length", 0)?;

    Ok(ObjectValue(class_list.get()))
}

unsafe fn create_dataset_object_for_node(cx: &mut SafeJSContext, node_id: usize) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let dataset = JS_NewPlainObject(cx));
    if dataset.get().is_null() {
        return Err("Failed to create dataset object".to_string());
    }

    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            if let Some(node) = dom.get_node(node_id) {
                if let NodeData::Element(ref elem_data) = node.data {
                    for attr in elem_data.attributes.iter() {
                        let attr_name = attr.name.local.as_ref();
                        if let Some(data_key) = attr_name.strip_prefix("data-") {
                            let camel_key = hyphen_to_camel_case(data_key);
                            let _ = set_string_property(cx, dataset.get(), &camel_key, attr.value.as_ref());
                        }
                    }
                }
            }
        }
    });

    Ok(ObjectValue(dataset.get()))
}

/// Get the node ID from classList's parent element
unsafe fn get_classlist_parent_node_id(cx: &mut SafeJSContext, args: &CallArgs) -> Option<usize> {
    // First try to get __nodeId directly from this (for when classList is on the element directly)
    if let Some(id) = get_node_id_from_this(cx, args) {
        return Some(id);
    }
    // classList doesn't have __nodeId directly - this is a limitation
    None
}

pub(crate) unsafe fn create_js_shadow_root_by_id(cx: &mut SafeJSContext, node_id: usize) -> Result<JSVal, String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let shadow_root = JS_NewPlainObject(cx));
    if shadow_root.get().is_null() {
        return Err("Failed to create shadow root object".to_string());
    }

    set_int_property(cx, shadow_root.get(), "nodeType", 11)?; // DOCUMENT_FRAGMENT_NODE
    set_string_property(cx, shadow_root.get(), "nodeName", "#document-fragment")?;

    rooted!(in(raw_cx) let node_id_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let shadow_root_rooted = shadow_root.get());
    let cname = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        cx,
        shadow_root_rooted.handle().into(),
        cname.as_ptr(),
        node_id_val.handle().into(),
        0,
    );

    define_function(cx, shadow_root.get(), "appendChild", Some(element_append_child), 1)?;
    define_function(cx, shadow_root.get(), "querySelector", Some(element_query_selector), 1)?;
    define_function(cx, shadow_root.get(), "querySelectorAll", Some(element_query_selector_all), 1)?;

    Ok(ObjectValue(shadow_root.get()))
}

unsafe fn parse_shadow_root_mode(cx: &mut SafeJSContext, args: &CallArgs, argc: c_uint) -> ShadowRootMode {
    let raw_cx = cx.raw_cx();
    if argc == 0 {
        return ShadowRootMode::Open;
    }

    let options = *args.get(0);
    if !options.is_object() || options.is_null() {
        return ShadowRootMode::Open;
    }

    rooted!(in(raw_cx) let options_obj = options.to_object());
    rooted!(in(raw_cx) let mut mode_val = UndefinedValue());
    let mode_name = std::ffi::CString::new("mode").unwrap();
    if !JS_GetProperty(
        cx,
        options_obj.handle().into(),
        mode_name.as_ptr(),
        mode_val.handle_mut().into(),
    ) {
        return ShadowRootMode::Open;
    }

    let mode = js_value_to_string(cx, mode_val.get());
    if mode == "closed" {
        ShadowRootMode::Closed
    } else {
        ShadowRootMode::Open
    }
}

pub(crate) unsafe extern "C" fn element_has_child_nodes(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let result = if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
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

pub(crate) unsafe extern "C" fn element_attach_shadow(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();;

    let Some(host_id) = get_node_id_from_this(safe_cx, &args) else {
        args.rval().set(NullValue());
        return true;
    };

    let mode = parse_shadow_root_mode(safe_cx, &args, argc);

    let shadow_root_id = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &mut *dom_ptr;
            return dom.attach_shadow(host_id, mode).ok();
        }
        None
    });

    if let Some(shadow_root_id) = shadow_root_id {
        if let Ok(shadow_root) = create_js_shadow_root_by_id(safe_cx, shadow_root_id) {
            args.rval().set(shadow_root);
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn element_get_shadow_root(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let open_shadow_root_id = get_node_id_from_this(safe_cx, &args).and_then(|host_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                return (*dom_ptr).open_shadow_root_id(host_id);
            }
            None
        })
    });

    if let Some(shadow_root_id) = open_shadow_root_id {
        if let Ok(shadow_root) = create_js_shadow_root_by_id(safe_cx, shadow_root_id) {
            args.rval().set(shadow_root);
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn element_set_shadow_root_noop(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(UndefinedValue());
    true
}

pub(crate) unsafe extern "C" fn element_get_style_object(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if !args.thisv().is_object() || args.thisv().is_null() {
        args.rval().set(NullValue());
        return true;
    }

    rooted!(in(raw_cx) let this_obj = args.thisv().to_object());
    rooted!(in(raw_cx) let mut cache_val = UndefinedValue());
    let cache_name = std::ffi::CString::new("__styleCache").unwrap();
    if JS_GetProperty(
        safe_cx,
        this_obj.handle().into(),
        cache_name.as_ptr(),
        cache_val.handle_mut().into(),
    ) && cache_val.get().is_object() {
        args.rval().set(cache_val.get());
        return true;
    }

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        if let Ok(style_obj) = create_style_object_for_node(safe_cx, node_id) {
            rooted!(in(raw_cx) let style_val = style_obj);
            JS_SetProperty(
                safe_cx,
                this_obj.handle(),
                cache_name.as_ptr(),
                style_val.handle(),
            );
            args.rval().set(style_val.get());
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn element_get_class_list_object(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if !args.thisv().is_object() || args.thisv().is_null() {
        args.rval().set(NullValue());
        return true;
    }

    rooted!(in(raw_cx) let this_obj = args.thisv().to_object());
    rooted!(in(raw_cx) let mut cache_val = UndefinedValue());
    let cache_name = std::ffi::CString::new("__classListCache").unwrap();
    if JS_GetProperty(
        safe_cx,
        this_obj.handle().into(),
        cache_name.as_ptr(),
        cache_val.handle_mut().into(),
    ) && cache_val.get().is_object() {
        args.rval().set(cache_val.get());
        return true;
    }

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        if let Ok(class_list_obj) = create_class_list_object_for_node(safe_cx, node_id) {
            rooted!(in(raw_cx) let class_list_val = class_list_obj);
            JS_SetProperty(
                safe_cx,
                this_obj.handle(),
                cache_name.as_ptr(),
                class_list_val.handle(),
            );
            args.rval().set(class_list_val.get());
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn element_get_dataset_object(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if !args.thisv().is_object() || args.thisv().is_null() {
        args.rval().set(NullValue());
        return true;
    }

    rooted!(in(raw_cx) let this_obj = args.thisv().to_object());
    rooted!(in(raw_cx) let mut cache_val = UndefinedValue());
    let cache_name = std::ffi::CString::new("__datasetCache").unwrap();
    if JS_GetProperty(
        safe_cx,
        this_obj.handle().into(),
        cache_name.as_ptr(),
        cache_val.handle_mut().into(),
    ) && cache_val.get().is_object() {
        args.rval().set(cache_val.get());
        return true;
    }

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        if let Ok(dataset_obj) = create_dataset_object_for_node(safe_cx, node_id) {
            rooted!(in(raw_cx) let dataset_val = dataset_obj);
            JS_SetProperty(
                safe_cx,
                this_obj.handle(),
                cache_name.as_ptr(),
                dataset_val.handle(),
            );
            args.rval().set(dataset_val.get());
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

pub(crate) unsafe extern "C" fn element_set_object_property_noop(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.__setObjectPropertyNoop",
        "setter is a compatibility no-op",
    );
    args.rval().set(UndefinedValue());
    true
}

unsafe fn should_lookup_parent(cx: &mut SafeJSContext, args: &CallArgs) -> bool {
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

unsafe fn build_parent_wrapper(cx: &mut SafeJSContext, node_id: usize) -> Option<JSVal> {
    let parent_info: Option<(usize, String, AttributeMap)> = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            if let Some(node) = dom.get_node(node_id) {
                if let Some(parent_id) = node.parent {
                    if let Some(parent_node) = dom.get_node(parent_id) {
                        if let NodeData::Element(ref elem_data) = parent_node.data {
                            return Some((
                                parent_id,
                                elem_data.name.local.to_string(),
                                elem_data.attributes.clone(),
                            ));
                        }
                    }
                }
            }
        }
        None
    });

    let (parent_id, parent_tag, parent_attrs) = parent_info?;
    create_js_element_impl(cx, parent_id, &parent_tag, &parent_attrs, false, false).ok()
}

pub(crate) unsafe extern "C" fn element_get_parent_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if !should_lookup_parent(safe_cx, &args) {
        args.rval().set(NullValue());
        return true;
    }

    let parent_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| build_parent_wrapper(safe_cx, node_id));
    args.rval().set(parent_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn element_get_parent_element(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    element_get_parent_node(raw_cx, argc, vp)
}

pub(crate) unsafe extern "C" fn element_get_first_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let first_child_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    return node.children.first().copied().and_then(|id| create_js_node_wrapper_by_id(safe_cx, id));
                }
            }
            None
        })
    });

    args.rval().set(first_child_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn element_get_last_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let last_child_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    return node.children.last().copied().and_then(|id| create_js_node_wrapper_by_id(safe_cx, id));
                }
            }
            None
        })
    });

    args.rval().set(last_child_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn element_get_previous_sibling(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let previous_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                return dom.previous_sibling_id(node_id).and_then(|id| create_js_node_wrapper_by_id(safe_cx, id));
            }
            None
        })
    });

    args.rval().set(previous_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn element_get_next_sibling(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let next_val = get_node_id_from_this(safe_cx, &args).and_then(|node_id| {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                return dom.next_sibling_id(node_id).and_then(|id| create_js_node_wrapper_by_id(safe_cx, id));
            }
            None
        })
    });

    args.rval().set(next_val.unwrap_or(NullValue()));
    true
}

pub(crate) unsafe extern "C" fn element_get_children(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
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
            if let Some(child_val) = create_js_node_wrapper_by_id(safe_cx, *child_id) {
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

pub(crate) unsafe extern "C" fn element_get_child_nodes(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
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
            if let Some(child_val) = create_js_node_wrapper_by_id(safe_cx, *child_id) {
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

// ============================================================================
// Element methods
// ============================================================================

/// element.getAttribute implementation
pub(crate) unsafe extern "C" fn element_get_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let attr_name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] element.getAttribute('{}') called", attr_name);

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let value = DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        return elem_data.attributes.iter()
                            .find(|attr| attr.name.local.as_ref() == attr_name)
                            .map(|attr| attr.value.to_string());
                    }
                }
            }
            None
        });

        if let Some(val) = value {
            trace!("[JS] getAttribute('{}') = '{}'", attr_name, val);
            args.rval().set(create_js_string(safe_cx, &val));
        } else {
            args.rval().set(NullValue());
        }
    } else {
        args.rval().set(NullValue());
    }
    true
}

/// element.setAttribute implementation
pub(crate) unsafe extern "C" fn element_set_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let attr_name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    let attr_value = if argc > 1 {
        js_value_to_string(safe_cx, *args.get(1))
    } else {
        String::new()
    };

    trace!("[JS] element.setAttribute('{}', '{}') called", attr_name, attr_value);

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                // Create QualName for the attribute
                let qname = QualName::new(
                    None,
                    markup5ever::ns!(),
                    markup5ever::LocalName::from(attr_name.as_str()),
                );
                dom.set_attribute(node_id, qname, &attr_value);
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.removeAttribute implementation
pub(crate) unsafe extern "C" fn element_remove_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let attr_name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] element.removeAttribute('{}') called", attr_name);

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                let qname = QualName::new(
                    None,
                    markup5ever::ns!(),
                    markup5ever::LocalName::from(attr_name.as_str()),
                );
                dom.clear_attribute(node_id, qname);
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.hasAttribute implementation
pub(crate) unsafe extern "C" fn element_has_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let attr_name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] element.hasAttribute('{}') called", attr_name);

    let has_attr = if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        return elem_data.attributes.iter()
                            .any(|attr| attr.name.local.as_ref() == attr_name);
                    }
                }
            }
            false
        })
    } else {
        false
    };

    args.rval().set(BooleanValue(has_attr));
    true
}

/// Checks if `child_id` refers to a `<script src="…">` element and, if so, fetches and
/// executes the script via the DOM's net/JS providers.
fn trigger_script_load_if_needed(child_id: usize) {
    let script_load_info = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = unsafe { &*dom_ptr };
            if let Some(node) = dom.get_node(child_id) {
                if let NodeData::Element(ref elem_data) = node.data {
                    if elem_data.name.local.as_ref() == "script" {
                        let script_type = elem_data.attributes.iter()
                            .find(|a| a.name.local.as_ref() == "type")
                            .map(|a| a.value.to_string());

                        let Some(script_kind) = executable_script_kind(script_type.as_deref()) else {
                            // Non-JS types are data blocks in browsers and do not execute.
                            return None;
                        };

                        if let Some(src) = elem_data.attributes.iter()
                            .find(|a| a.name.local.as_ref() == "src")
                            .map(|a| a.value.to_string())
                        {
                            if let Some(url) = dom.url.resolve_relative(&src) {
                                return Some((url, dom.net_provider.clone(), dom.js_provider.clone(), child_id, script_kind));
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
                                    js_provider.execute_module_script_with_node_id(script, script_node_id, module_source_url.clone());
                                } else {
                                    js_provider.execute_script_with_node_id(script, script_node_id);
                                }
                            }
                            Err(e) => eprintln!("[JS] Dynamic script at '{}' is not valid UTF-8: {}", url_str, e),
                        }
                    }
                    Err(e) => eprintln!("[JS] Failed to load dynamic script '{}': {:?}", url_str, e),
                }
            }),
        );
    }
}

/// element.append implementation - appends multiple nodes or DOMStrings as children
pub(crate) unsafe extern "C" fn element_append(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] element.append() called with {} args", argc);

    let parent_id = match get_node_id_from_this(safe_cx, &args) {
        Some(id) => id,
        None => {
            args.rval().set(UndefinedValue());
            return true;
        }
    };

    for i in 0..argc {
        let arg = *args.get(i);
        if arg.is_string() {
            // DOMString argument: create a text node and append it
            let text = js_value_to_string(safe_cx, arg);
            DOM_REF.with(|dom| {
                if let Some(dom_ptr) = *dom.borrow() {
                    let dom = &mut *dom_ptr;
                    let text_node_id = dom.create_text_node(&text);
                    dom.append_children(parent_id, &[text_node_id]);
                }
            });
        } else if let Some(child_id) = get_node_id_from_value(safe_cx, arg) {
            // Node argument: append directly
            DOM_REF.with(|dom| {
                if let Some(dom_ptr) = *dom.borrow() {
                    let dom = &mut *dom_ptr;
                    dom.append_children(parent_id, &[child_id]);
                }
            });
            trigger_script_load_if_needed(child_id);
        }
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.appendChild implementation
pub(crate) unsafe extern "C" fn element_append_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] element.appendChild() called");

    // Extract the child node id from the first argument using helper
    let child_id = if argc > 0 {
        match get_node_id_from_value(safe_cx, *args.get(0)) {
            Some(id) => id,
            None => {
                args.rval().set(UndefinedValue());
                return true;
            }
        }
    } else {
        // No child provided
        args.rval().set(UndefinedValue());
        return true;
    };
    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            // Update parent reference in child node
            if let Some(parent_id) = get_node_id_from_this(safe_cx, &args) {
                dom.append_children(parent_id, &[child_id]);
            }
        }
    });
    custom_elements_upgrade_for_node(safe_cx, child_id);

    // Trigger script loading if a <script> element with a src attribute was appended
    trigger_script_load_if_needed(child_id);

    if argc > 0 {
        // Return the child that was appended
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.removeChild implementation
pub(crate) unsafe extern "C" fn element_remove_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] element.removeChild() called");

    // Extract the child node id from the first argument using helper
    let child_id = if argc > 0 {
        match get_node_id_from_value(safe_cx, *args.get(0)) {
            Some(id) => id,
            None => {
                args.rval().set(UndefinedValue());
                return true;
            }
        }
    } else {
        // No child provided
        args.rval().set(UndefinedValue());
        return true;
    };
    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            // Update parent reference in child node
            if let Some(parent_id) = get_node_id_from_this(safe_cx, &args) {
                dom.remove_node(child_id);
            }
        }
    });
    if argc > 0 {
        // Return the child that was appended
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.insertBefore implementation
pub(crate) unsafe extern "C" fn element_insert_before(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] element.insertBefore() called");

    // insertBefore(newNode, referenceNode)
    // First argument is required: the new node to insert.
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

    // Second argument is the reference node. If null/undefined, fall back to appendChild.
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
                    Some(ref_id) => {
                        // Insert new_child immediately before the reference node.
                        dom.insert_nodes_before(ref_id, &[new_child_id]);
                    }
                    None => {
                        // Reference is null: append to end of parent's children.
                        dom.append_children(parent_id, &[new_child_id]);
                    }
                }
            }
        }
    });
    custom_elements_upgrade_for_node(safe_cx, new_child_id);

    // Trigger script loading if a <script> element with a src attribute was inserted
    trigger_script_load_if_needed(new_child_id);

    // Return the inserted node.
    args.rval().set(*args.get(0));
    true
}

/// element.replaceChild implementation
pub(crate) unsafe extern "C" fn element_replace_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] element.replaceChild() called");

    if argc >= 2 {
        let parent_id = get_node_id_from_this(safe_cx, &args);
        let new_child_id = get_node_id_from_value(safe_cx, *args.get(0));
        let old_child_id = get_node_id_from_value(safe_cx, *args.get(1));

        if let (Some(parent_id), Some(new_child_id), Some(old_child_id)) = (parent_id, new_child_id, old_child_id) {
            DOM_REF.with(|dom| {
                if let Some(dom_ptr) = *dom.borrow() {
                    let dom = &mut *dom_ptr;
                    if dom.parent_id(old_child_id) == Some(parent_id) {
                        dom.replace_node_with(old_child_id, &[new_child_id]);
                    }
                }
            });

            custom_elements_upgrade_for_node(safe_cx, new_child_id);

            trigger_script_load_if_needed(new_child_id);
            args.rval().set(*args.get(1));
            return true;
        }
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.cloneNode implementation
pub(crate) unsafe extern "C" fn element_clone_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let deep = if argc > 0 {
        let val = *args.get(0);
        val.is_boolean() && val.to_boolean()
    } else {
        false
    };

    trace!("[JS] element.cloneNode({}) called", deep);

    // Get the tag name and attributes from the current element
    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let element_data = DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        return Some((elem_data.name.local.to_string(), elem_data.attributes.clone()));
                    }
                }
            }
            None
        });

        if let Some((tag_name, attributes)) = element_data {
            // Create a new element with the same tag and attributes but node_id 0 (not linked to DOM)
            match create_js_element_by_id(safe_cx, 0, &tag_name, &attributes) {
                Ok(elem) => {
                    args.rval().set(elem);
                    return true;
                }
                Err(_) => {}
            }
        }
    }

    // Fallback: try to get tag name from JS properties
    let this_val = args.thisv();
    if this_val.get().is_object() && !this_val.get().is_null() {
        rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
        rooted!(in(raw_cx) let mut tag_val = UndefinedValue());

        let cname = std::ffi::CString::new("tagName").unwrap();
        if JS_GetProperty(safe_cx, this_obj.handle().into(), cname.as_ptr(), tag_val.handle_mut().into()) {
            if tag_val.get().is_string() {
                let tag_name = js_value_to_string(safe_cx, tag_val.get());
                match create_stub_element(safe_cx, &tag_name.to_lowercase()) {
                    Ok(elem) => {
                        args.rval().set(elem);
                        return true;
                    }
                    Err(_) => {}
                }
            }
        }
    }

    // Final fallback
    match create_stub_element(safe_cx, "div") {
        Ok(elem) => args.rval().set(elem),
        Err(_) => args.rval().set(NullValue()),
    }
    true
}

/// element.querySelector implementation
pub(crate) unsafe extern "C" fn element_query_selector(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let selector = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] element.querySelector('{}') called", selector);

    if selector.is_empty() {
        args.rval().set(NullValue());
        return true;
    }

    if let (Some(root_id), Some(id)) = (get_node_id_from_this(safe_cx, &args), simple_id_selector(&selector)) {
        let match_id = DOM_REF.with(|dom_ref| {
            let dom_ptr = (*dom_ref.borrow())?;
            let dom = &*dom_ptr;
            let &candidate_id = dom.nodes_to_id.get(id)?;

            let mut current = Some(candidate_id);
            while let Some(cur_id) = current {
                if cur_id == root_id {
                    return Some(candidate_id);
                }
                current = dom.get_node(cur_id).and_then(|node| node.parent);
            }
            None
        });

        if let Some(match_id) = match_id {
            if let Ok(elem) = create_js_element_by_dom_id(safe_cx, match_id) {
                args.rval().set(elem);
                return true;
            }
        }

        args.rval().set(NullValue());
        return true;
    }

    let parsed_selector = parse_selector(&selector);

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        // Search descendants of this element
        let matching_node_id = DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;

                let seed = selector_seed(&parsed_selector);
                if !matches!(seed, SelectorSeed::None | SelectorSeed::Universal) {
                    let candidate_ids: Vec<usize> = match seed {
                        SelectorSeed::Id(id) => dom
                            .nodes_to_id
                            .get(id)
                            .copied()
                            .into_iter()
                            .collect(),
                        SelectorSeed::Class(class_name) => dom.candidate_nodes_for_class(class_name),
                        SelectorSeed::Tag(tag) => dom.candidate_nodes_for_tag(tag),
                        SelectorSeed::Universal | SelectorSeed::None => Vec::new(),
                    };

                    for candidate_id in candidate_ids {
                        if !is_descendant_of(dom, node_id, candidate_id) || candidate_id == node_id {
                            continue;
                        }
                        if let Some(node) = dom.get_node(candidate_id) {
                            if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                                if matches_parsed_selector(&parsed_selector, elem_data.name.local.as_ref(), &elem_data.attributes) {
                                    return Some(candidate_id);
                                }
                            }
                        }
                    }
                    return None;
                }

                // Traverse the subtree looking for a match
                fn find_in_subtree(
                    dom: &crate::dom::Dom,
                    parent_id: usize,
                    parsed_selector: &crate::js::selectors::ParsedSelector<'_>,
                ) -> Option<usize> {
                    if let Some(parent_node) = dom.get_node(parent_id) {
                        for child_id in &parent_node.children {
                            if let Some(child_node) = dom.get_node(*child_id) {
                                if let crate::dom::NodeData::Element(ref elem_data) = child_node.data {
                                    if matches_parsed_selector(parsed_selector, elem_data.name.local.as_ref(), &elem_data.attributes) {
                                        return Some(*child_id);
                                    }
                                }
                                // Recurse into light-DOM descendants only.
                                if let Some(result) = find_in_subtree(dom, *child_id, parsed_selector) {
                                    return Some(result);
                                }
                            }
                        }
                    }
                    None
                }
                return find_in_subtree(dom, node_id, &parsed_selector);
            }
            None
        });

        if let Some(match_id) = matching_node_id {
            match create_js_element_by_dom_id(safe_cx, match_id) {
                Ok(elem) => {
                    args.rval().set(elem);
                    return true;
                }
                Err(_) => {}
            }
        }
    }

    args.rval().set(NullValue());
    true
}

/// element.querySelectorAll implementation
pub(crate) unsafe extern "C" fn element_query_selector_all(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let selector = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] element.querySelectorAll('{}') called", selector);

    // Create JS array
    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));

    if !selector.is_empty() {
        if let (Some(root_id), Some(id)) = (get_node_id_from_this(safe_cx, &args), simple_id_selector(&selector)) {
            let match_id = DOM_REF.with(|dom_ref| {
                let dom_ptr = (*dom_ref.borrow())?;
                let dom = &*dom_ptr;
                let &candidate_id = dom.nodes_to_id.get(id)?;

                let mut current = Some(candidate_id);
                while let Some(cur_id) = current {
                    if cur_id == root_id {
                        return Some(candidate_id);
                    }
                    current = dom.get_node(cur_id).and_then(|node| node.parent);
                }
                None
            });

            if let Some(match_id) = match_id {
                if let Ok(js_elem) = create_js_element_by_dom_id(safe_cx, match_id) {
                    rooted!(in(raw_cx) let elem_val = js_elem);
                    rooted!(in(raw_cx) let array_obj = array.get());
                    JS_SetElement(safe_cx, array_obj.handle().into(), 0, elem_val.handle().into());
                }
            }

            args.rval().set(ObjectValue(array.get()));
            return true;
        }

        let parsed_selector = parse_selector(&selector);
        if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
            let matching_node_ids: Vec<usize> = DOM_REF.with(|dom_ref| {
                let mut results = Vec::new();
                if let Some(dom_ptr) = *dom_ref.borrow() {
                    let dom = &*dom_ptr;

                    let seed = selector_seed(&parsed_selector);
                    if !matches!(seed, SelectorSeed::None | SelectorSeed::Universal) {
                        let candidate_ids: Vec<usize> = match seed {
                            SelectorSeed::Id(id) => dom
                                .nodes_to_id
                                .get(id)
                                .copied()
                                .into_iter()
                                .collect(),
                            SelectorSeed::Class(class_name) => dom.candidate_nodes_for_class(class_name),
                            SelectorSeed::Tag(tag) => dom.candidate_nodes_for_tag(tag),
                            SelectorSeed::Universal | SelectorSeed::None => Vec::new(),
                        };

                        for candidate_id in candidate_ids {
                            if candidate_id == node_id || !is_descendant_of(dom, node_id, candidate_id) {
                                continue;
                            }
                            if let Some(node) = dom.get_node(candidate_id) {
                                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                                    if matches_parsed_selector(&parsed_selector, elem_data.name.local.as_ref(), &elem_data.attributes) {
                                        results.push(candidate_id);
                                    }
                                }
                            }
                        }

                        return results;
                    }

                    // Collect all matching descendants
                    fn collect_in_subtree(
                        dom: &crate::dom::Dom,
                        parent_id: usize,
                        parsed_selector: &crate::js::selectors::ParsedSelector<'_>,
                        results: &mut Vec<usize>,
                    ) {
                        if let Some(parent_node) = dom.get_node(parent_id) {
                            for child_id in &parent_node.children {
                                if let Some(child_node) = dom.get_node(*child_id) {
                                    if let crate::dom::NodeData::Element(ref elem_data) = child_node.data {
                                        if matches_parsed_selector(parsed_selector, elem_data.name.local.as_ref(), &elem_data.attributes) {
                                            results.push(*child_id);
                                        }
                                    }
                                    // Recurse into light-DOM descendants only.
                                    collect_in_subtree(dom, *child_id, parsed_selector, results);
                                }
                            }
                        }
                    }
                    collect_in_subtree(dom, node_id, &parsed_selector, &mut results);
                }
                results
            });

            for (index, match_id) in matching_node_ids.iter().enumerate() {
                if let Ok(js_elem) = create_js_element_by_dom_id(safe_cx, *match_id) {
                    rooted!(in(raw_cx) let elem_val = js_elem);
                    rooted!(in(raw_cx) let array_obj = array.get());
                    JS_SetElement(safe_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
                }
            }
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

// EventTarget extern callbacks moved to `element.rs`.

/// element.focus implementation
pub(crate) unsafe extern "C" fn element_focus(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let Some(node_id) = get_node_id_from_this(safe_cx, &args) else {
        args.rval().set(UndefinedValue());
        return true;
    };

    let mut generated: Vec<DomEvent> = Vec::new();
    DOM_REF.with(|dom_ref| {
        let Some(dom_ptr) = *dom_ref.borrow() else {
            return;
        };

        let dom = &mut *dom_ptr;
        let can_focus = dom
            .get_node(node_id)
            .is_some_and(|node| node.is_focusable() && node.flags.is_in_document());
        if !can_focus {
            return;
        }

        generate_focus_events(
            dom,
            &mut |doc| {
                doc.set_focus_to(node_id);
            },
            &mut |event| generated.push(event),
        );

        if !generated.is_empty() {
            dom.shell_provider.request_redraw();
        }
    });

    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(safe_cx));
    if !global.get().is_null() {
        for event in generated {
            let chain = DOM_REF.with(|dom_ref| {
                dom_ref
                    .borrow()
                    .as_ref()
                    .and_then(|dom_ptr| {
                        let dom = &**dom_ptr;
                        dom.get_node(event.target).map(|_| dom.node_chain(event.target))
                    })
                    .unwrap_or_else(|| vec![event.target])
            });
            event_listeners::fire_js_event_on_chain(safe_cx, global.get(), &chain, &event);
        }
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.blur implementation
pub(crate) unsafe extern "C" fn element_blur(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let Some(node_id) = get_node_id_from_this(safe_cx, &args) else {
        args.rval().set(UndefinedValue());
        return true;
    };

    let mut generated: Vec<DomEvent> = Vec::new();
    DOM_REF.with(|dom_ref| {
        let Some(dom_ptr) = *dom_ref.borrow() else {
            return;
        };

        let dom = &mut *dom_ptr;
        if dom.focus_node_id != Some(node_id) {
            return;
        }

        generate_focus_events(
            dom,
            &mut |doc| doc.clear_focus(),
            &mut |event| generated.push(event),
        );

        if !generated.is_empty() {
            dom.shell_provider.request_redraw();
        }
    });

    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(safe_cx));
    if !global.get().is_null() {
        for event in generated {
            let chain = DOM_REF.with(|dom_ref| {
                dom_ref
                    .borrow()
                    .as_ref()
                    .and_then(|dom_ptr| {
                        let dom = &**dom_ptr;
                        dom.get_node(event.target).map(|_| dom.node_chain(event.target))
                    })
                    .unwrap_or_else(|| vec![event.target])
            });
            event_listeners::fire_js_event_on_chain(safe_cx, global.get(), &chain, &event);
        }
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.click implementation
pub(crate) unsafe extern "C" fn element_click(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    // FIXME: Does not synthesize a click event, invoke registered click listeners, or simulate
    // the default activation behaviour (e.g. following links, submitting forms).
    warn!("[JS] element.click() called on partial binding (no synthetic click/default action)");
    args.rval().set(UndefinedValue());
    true
}

/// element.getBoundingClientRect implementation
pub(crate) unsafe extern "C" fn element_get_bounding_client_rect(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    warn!("[JS] element.getBoundingClientRect() called on partial binding (returns zero rect)");

    // FIXME: All DOMRect values are hardcoded to 0. Should query the renderer for the element's
    // actual bounding box in viewport coordinates and populate x, y, width, height, top, right,
    // bottom, left accordingly.
    rooted!(in(raw_cx) let rect = JS_NewPlainObject(safe_cx));
    if !rect.get().is_null() {
        let _ = set_int_property(safe_cx, rect.get(), "x", 0);
        let _ = set_int_property(safe_cx, rect.get(), "y", 0);
        let _ = set_int_property(safe_cx, rect.get(), "width", 0);
        let _ = set_int_property(safe_cx, rect.get(), "height", 0);
        let _ = set_int_property(safe_cx, rect.get(), "top", 0);
        let _ = set_int_property(safe_cx, rect.get(), "right", 0);
        let _ = set_int_property(safe_cx, rect.get(), "bottom", 0);
        let _ = set_int_property(safe_cx, rect.get(), "left", 0);
        args.rval().set(ObjectValue(rect.get()));
    } else {
        args.rval().set(NullValue());
    }
    true
}

/// element.getClientRects implementation
pub(crate) unsafe extern "C" fn element_get_client_rects(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    warn!("[JS] element.getClientRects() called on partial binding (returns empty list)");

    // FIXME: Always returns an empty DOMRectList instead of the element's per-line-box rects.
    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));
    args.rval().set(ObjectValue(array.get()));
    true
}

/// element.closest implementation
pub(crate) unsafe extern "C" fn element_closest(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let selector = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] element.closest('{}') called", selector);

    if selector.is_empty() {
        args.rval().set(NullValue());
        return true;
    }

    let parsed_selector = parse_selector(&selector);

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        // Traverse up the parent chain looking for a match
        let matching_node_id = DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                let mut current_id = Some(node_id);

                while let Some(id) = current_id {
                    if let Some(node) = dom.get_node(id) {
                        if let NodeData::Element(ref elem_data) = node.data {
                            // Check if this element matches the selector
                            if matches_parsed_selector(&parsed_selector, elem_data.name.local.as_ref(), &elem_data.attributes) {
                                return Some(id);
                            }
                        }
                        current_id = node.parent;
                    } else {
                        break;
                    }
                }
            }
            None
        });

        if let Some(match_id) = matching_node_id {
            match create_js_element_by_dom_id(safe_cx, match_id) {
                Ok(elem) => {
                    args.rval().set(elem);
                    return true;
                }
                Err(_) => {}
            }
        }
    }

    args.rval().set(NullValue());
    true
}

/// element.matches implementation
pub(crate) unsafe extern "C" fn element_matches(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let selector = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] element.matches('{}') called", selector);

    let mut result = false;

    if !selector.is_empty() {
        let parsed_selector = parse_selector(&selector);
        if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
            DOM_REF.with(|dom_ref| {
                if let Some(dom_ptr) = *dom_ref.borrow() {
                    let dom = &*dom_ptr;
                    if let Some(node) = dom.get_node(node_id) {
                        if let NodeData::Element(ref elem_data) = node.data {
                            result = matches_parsed_selector(&parsed_selector, elem_data.name.local.as_ref(), &elem_data.attributes);
                        }
                    }
                }
            });
        }
    }

    args.rval().set(BooleanValue(result));
    true
}

/// element.contains implementation
pub(crate) unsafe extern "C" fn element_contains(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] element.contains() called");

    // Get the node ID of this element
    let this_node_id = get_node_id_from_this(safe_cx, &args);

    // Get the node ID of the argument element
    let other_node_id = if argc > 0 {
        let other_val = *args.get(0);
        if other_val.is_object() && !other_val.is_null() {
            rooted!(in(raw_cx) let other_obj = other_val.to_object());
            rooted!(in(raw_cx) let mut ptr_val = UndefinedValue());
            let cname = std::ffi::CString::new("__nodeId").unwrap();
            JS_GetProperty(safe_cx, other_obj.handle().into(), cname.as_ptr(), ptr_val.handle_mut().into());
            if ptr_val.get().is_double() {
                Some(ptr_val.get().to_double() as usize)
            } else if ptr_val.get().is_int32() {
                Some(ptr_val.get().to_int32() as usize)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let result = match (this_node_id, other_node_id) {
        (Some(this_id), Some(other_id)) => {
            // A node contains itself
            if this_id == other_id {
                true
            } else {
                // Check if other_id is a descendant of this_id by traversing up from other_id
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

/// element.getRootNode implementation (Node.getRootNode per DOM Living Standard)
pub(crate) unsafe extern "C" fn element_get_root_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] element.getRootNode() called");

    // Parse options - check for {composed: true}
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

    // Represents which kind of root was found
    enum RootKind {
        Document,
        ShadowRoot,
        Element,
    }

    // Walk up the parent chain (honouring shadow boundaries) to find the root node
    let root_info: Option<(usize, RootKind)> = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            let mut current_id = start_id;
            loop {
                let Some(node) = dom.get_node(current_id) else {
                    return None;
                };
                if let Some(parent_id) = node.parent {
                    // Keep climbing
                    current_id = parent_id;
                } else if let Some(host_id) = node.shadow_host {
                    // Reached a shadow root node
                    if composed {
                        // Cross shadow boundary and continue from the host element
                        current_id = host_id;
                    } else {
                        // Stop here – return the shadow root
                        return Some((current_id, RootKind::ShadowRoot));
                    }
                } else {
                    // No parent, no shadow host – this is the top-most root
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
            // Return the global `document` object
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
            // Disconnected subtree – if the root is this very node return `this`
            if root_id == start_id {
                args.rval().set(args.thisv().get());
            } else {
                // Build a JS wrapper for the disconnected root element
                let elem_info = DOM_REF.with(|dom_ref| {
                    if let Some(dom_ptr) = *dom_ref.borrow() {
                        let dom = &*dom_ptr;
                        if let Some(node) = dom.get_node(root_id) {
                            if let NodeData::Element(ref elem_data) = node.data {
                                return Some((elem_data.name.local.to_string(), elem_data.attributes.clone()));
                            }
                        }
                    }
                    None
                });
                if let Some((tag, attrs)) = elem_info {
                    match create_js_element_by_id(safe_cx, root_id, &tag, &attrs) {
                        Ok(val) => args.rval().set(val),
                        Err(_) => args.rval().set(NullValue()),
                    }
                } else {
                    args.rval().set(NullValue());
                }
            }
        }
        None => {
            args.rval().set(NullValue());
        }
    }
    true
}

// ============================================================================
// ChildNode / ParentNode mixin methods
// ============================================================================

/// element.remove() — removes this element from its parent (ChildNode mixin).
pub(crate) unsafe extern "C" fn element_remove(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] element.remove() called");
    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom| {
            if let Some(dom_ptr) = *dom.borrow() {
                let dom = &mut *dom_ptr;
                dom.remove_node(node_id);
            }
        });
    }
    args.rval().set(UndefinedValue());
    true
}

/// Collect DOM node IDs from a varargs call.  String arguments become new text nodes.
unsafe fn collect_nodes_from_varargs(cx: &mut SafeJSContext, args: &CallArgs, argc: c_uint) -> Vec<usize> {
    let mut ids: Vec<usize> = Vec::new();
    for i in 0..argc {
        let arg = *args.get(i);
        if arg.is_string() {
            let text = js_value_to_string(cx, arg);
            DOM_REF.with(|dom| {
                if let Some(dom_ptr) = *dom.borrow() {
                    let dom = &mut *dom_ptr;
                    ids.push(dom.create_text_node(&text));
                }
            });
        } else if let Some(id) = get_node_id_from_value(cx, arg) {
            ids.push(id);
        }
    }
    ids
}

/// element.prepend(...nodes) — inserts nodes at the beginning of this element's children.
pub(crate) unsafe extern "C" fn element_prepend(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] element.prepend() called with {} args", argc);
    if let Some(parent_id) = get_node_id_from_this(safe_cx, &args) {
        let new_ids = collect_nodes_from_varargs(safe_cx, &args, argc);
        if !new_ids.is_empty() {
            DOM_REF.with(|dom| {
                if let Some(dom_ptr) = *dom.borrow() {
                    let dom = &mut *dom_ptr;
                    let first_child = dom.get_node(parent_id).and_then(|n| n.children.first().copied());
                    match first_child {
                        Some(fc) => dom.insert_nodes_before(fc, &new_ids),
                        None => dom.append_children(parent_id, &new_ids),
                    }
                }
            });
        }
    }
    args.rval().set(UndefinedValue());
    true
}

/// element.before(...nodes) — inserts nodes immediately before this element.
pub(crate) unsafe extern "C" fn element_before(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] element.before() called");
    if let Some(self_id) = get_node_id_from_this(safe_cx, &args) {
        let new_ids = collect_nodes_from_varargs(safe_cx, &args, argc);
        if !new_ids.is_empty() {
            DOM_REF.with(|dom| {
                if let Some(dom_ptr) = *dom.borrow() {
                    let dom = &mut *dom_ptr;
                    if dom.get_node(self_id).and_then(|n| n.parent).is_some() {
                        dom.insert_nodes_before(self_id, &new_ids);
                    }
                }
            });
        }
    }
    args.rval().set(UndefinedValue());
    true
}

/// element.after(...nodes) — inserts nodes immediately after this element.
pub(crate) unsafe extern "C" fn element_after(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] element.after() called");
    if let Some(self_id) = get_node_id_from_this(safe_cx, &args) {
        let new_ids = collect_nodes_from_varargs(safe_cx, &args, argc);
        if !new_ids.is_empty() {
            DOM_REF.with(|dom| {
                if let Some(dom_ptr) = *dom.borrow() {
                    let dom = &mut *dom_ptr;
                    if dom.get_node(self_id).and_then(|n| n.parent).is_some() {
                        dom.insert_nodes_after(self_id, &new_ids);
                    }
                }
            });
        }
    }
    args.rval().set(UndefinedValue());
    true
}

/// element.replaceWith(...nodes) — replaces this element with the given nodes.
pub(crate) unsafe extern "C" fn element_replace_with(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] element.replaceWith() called");
    if let Some(self_id) = get_node_id_from_this(safe_cx, &args) {
        let new_ids = collect_nodes_from_varargs(safe_cx, &args, argc);
        DOM_REF.with(|dom| {
            if let Some(dom_ptr) = *dom.borrow() {
                let dom = &mut *dom_ptr;
                if dom.get_node(self_id).and_then(|n| n.parent).is_some() {
                    dom.replace_node_with(self_id, &new_ids);
                }
            }
        });
    }
    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// Layout and geometry getter functions
// ============================================================================

/// element.offsetWidth getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_offset_width(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.offsetWidth",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.offsetHeight getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_offset_height(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.offsetHeight",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.offsetLeft getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_offset_left(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.offsetLeft",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.offsetTop getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_offset_top(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.offsetTop",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.clientWidth getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_client_width(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.clientWidth",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.clientHeight getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_client_height(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.clientHeight",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.scrollWidth getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_scroll_width(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.scrollWidth",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.scrollHeight getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_scroll_height(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.scrollHeight",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.scrollLeft getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_scroll_left(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.scrollLeft",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

/// element.scrollTop getter - warns that this is hardcoded to 0
pub(crate) unsafe extern "C" fn element_get_scroll_top(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Element.scrollTop",
        "hardcoded to 0 (layout not yet interactive)",
    );
    args.rval().set(mozjs::jsval::Int32Value(0));
    true
}

// ============================================================================
// Adjacent-insertion methods
// ============================================================================

/// Shared logic for insertAdjacentElement/Text: resolve a position string to one of the four
/// standard positions ("beforebegin", "afterbegin", "beforeend", "afterend") and perform the
/// DOM insertion of the given new node IDs.
unsafe fn insert_adjacent_nodes(self_id: usize, position: &str, new_ids: &[usize]) {
    if new_ids.is_empty() {
        return;
    }
    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = &mut *dom_ptr;
            match position.to_ascii_lowercase().as_str() {
                "beforebegin" => {
                    if dom.get_node(self_id).and_then(|n| n.parent).is_some() {
                        dom.insert_nodes_before(self_id, new_ids);
                    }
                }
                "afterbegin" => {
                    let first_child = dom.get_node(self_id).and_then(|n| n.children.first().copied());
                    match first_child {
                        Some(fc) => dom.insert_nodes_before(fc, new_ids),
                        None => dom.append_children(self_id, new_ids),
                    }
                }
                "beforeend" => {
                    dom.append_children(self_id, new_ids);
                }
                "afterend" => {
                    if dom.get_node(self_id).and_then(|n| n.parent).is_some() {
                        dom.insert_nodes_after(self_id, new_ids);
                    }
                }
                _ => {}
            }
        }
    });
}

/// element.insertAdjacentHTML(position, html) — parses and inserts HTML relative to the element.
/// FIXME: The html argument is not parsed as a fragment; it is currently a no-op for safety.
pub(crate) unsafe extern "C" fn element_insert_adjacent_html(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let position = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    warn!("[JS] element.insertAdjacentHTML('{}', ...) called on partial binding (HTML is not parsed)", position);
    args.rval().set(UndefinedValue());
    true
}

/// element.insertAdjacentElement(position, element) — inserts element at a position relative to this one.
pub(crate) unsafe extern "C" fn element_insert_adjacent_element(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let position = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    trace!("[JS] element.insertAdjacentElement('{}', ...) called", position);

    let self_id = match get_node_id_from_this(safe_cx, &args) {
        Some(id) => id,
        None => { args.rval().set(NullValue()); return true; }
    };
    let new_id = if argc > 1 { get_node_id_from_value(safe_cx, *args.get(1)) } else { None };
    let Some(new_id) = new_id else {
        args.rval().set(NullValue());
        return true;
    };

    insert_adjacent_nodes(self_id, &position, &[new_id]);

    // Return the inserted element
    args.rval().set(*args.get(1));
    true
}

/// element.insertAdjacentText(position, text) — inserts a text node at a position relative to this element.
pub(crate) unsafe extern "C" fn element_insert_adjacent_text(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let position = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
    let text = if argc > 1 { js_value_to_string(safe_cx, *args.get(1)) } else { String::new() };
    trace!("[JS] element.insertAdjacentText('{}', ...) called", position);

    if let Some(self_id) = get_node_id_from_this(safe_cx, &args) {
        let text_id = DOM_REF.with(|dom| {
            if let Some(dom_ptr) = *dom.borrow() {
                let dom = &mut *dom_ptr;
                Some(dom.create_text_node(&text))
            } else {
                None
            }
        });
        if let Some(text_id) = text_id {
            insert_adjacent_nodes(self_id, &position, &[text_id]);
        }
    }

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// Attribute introspection
// ============================================================================

/// element.getAttributeNames() — returns an array of the element's attribute names.
pub(crate) unsafe extern "C" fn element_get_attribute_names(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    trace!("[JS] element.getAttributeNames() called");

    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let names: Vec<String> = DOM_REF.with(|dom_ref| {
            let mut out = Vec::new();
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        for attr in elem_data.attributes.iter() {
                            out.push(attr.name.local.to_string());
                        }
                    }
                }
            }
            out
        });

        for (i, name) in names.iter().enumerate() {
            let name_val = create_js_string(safe_cx, name);
            rooted!(in(raw_cx) let name_rooted = name_val);
            rooted!(in(raw_cx) let array_obj = array.get());
            JS_SetElement(safe_cx, array_obj.handle().into(), i as u32, name_rooted.handle().into());
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// element.hasAttributes() — returns true if the element has any attributes.
pub(crate) unsafe extern "C" fn element_has_attributes(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let result = if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        return !elem_data.attributes.is_empty();
                    }
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

// ============================================================================
// Scroll stubs
// ============================================================================

/// element.scrollIntoView() — no-op stub (layout is not yet interactive).
pub(crate) unsafe extern "C" fn element_scroll_into_view(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    // FIXME: Should scroll the nearest scrollable ancestor (or the viewport) so that this element
    // is visible, respecting the scrollIntoViewOptions (behavior, block, inline).
    warn!("[JS] element.scrollIntoView() called on partial binding (no scroll performed)");
    args.rval().set(UndefinedValue());
    true
}

/// element.scrollTo() / element.scroll() — no-op stub.
pub(crate) unsafe extern "C" fn element_scroll_to(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    // FIXME: Should update the element's scroll position to the given (x, y) coordinates and fire
    // a scroll event.
    warn!("[JS] element.scrollTo()/scroll() called on partial binding (no scroll performed)");
    args.rval().set(UndefinedValue());
    true
}

/// element.scrollBy() — no-op stub.
pub(crate) unsafe extern "C" fn element_scroll_by(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    // FIXME: Should offset the element's current scroll position by the given (dx, dy) delta and
    // fire a scroll event.
    warn!("[JS] element.scrollBy() called on partial binding (no scroll performed)");
    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// Web Animations API stubs
// ============================================================================

/// Shared no-op callback for stub Animation methods.
unsafe extern "C" fn animation_noop(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    warn_stubbed_binding(
        "Animation.* method",
        "stub Animation object does not implement playback behavior",
    );
    args.rval().set(UndefinedValue());
    true
}

/// element.animate(keyframes, options) — returns a minimal stub Animation object.
// FIXME
pub(crate) unsafe extern "C" fn element_animate(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    warn!("[JS] element.animate() called on partial binding (returns stub Animation)");

    rooted!(in(raw_cx) let anim = JS_NewPlainObject(safe_cx));
    if anim.get().is_null() {
        args.rval().set(NullValue());
        return true;
    }

    // Minimal Animation interface stubs
    let _ = define_function(safe_cx, anim.get(), "cancel", Some(animation_noop), 0);
    let _ = define_function(safe_cx, anim.get(), "finish", Some(animation_noop), 0);
    let _ = define_function(safe_cx, anim.get(), "pause", Some(animation_noop), 0);
    let _ = define_function(safe_cx, anim.get(), "play", Some(animation_noop), 0);
    let _ = define_function(safe_cx, anim.get(), "reverse", Some(animation_noop), 0);
    let _ = define_function(safe_cx, anim.get(), "updatePlaybackRate", Some(animation_noop), 1);
    let _ = define_function(safe_cx, anim.get(), "commitStyles", Some(animation_noop), 0);

    rooted!(in(raw_cx) let null_val = NullValue());
    rooted!(in(raw_cx) let anim_obj = anim.get());
    for prop in &["onfinish", "oncancel", "onremove", "ready", "finished", "effect", "timeline"] {
        let cname = std::ffi::CString::new(*prop).unwrap();
        JS_DefineProperty(safe_cx, anim_obj.handle().into(), cname.as_ptr(), null_val.handle().into(), JSPROP_ENUMERATE as u32);
    }

    let _ = set_string_property(safe_cx, anim.get(), "playState", "finished");
    let _ = set_int_property(safe_cx, anim.get(), "currentTime", 0);
    let _ = set_int_property(safe_cx, anim.get(), "startTime", 0);
    let _ = set_int_property(safe_cx, anim.get(), "playbackRate", 1);

    args.rval().set(ObjectValue(anim.get()));
    true
}

// ============================================================================
// Style methods
// ============================================================================

/// style.getPropertyValue implementation
unsafe extern "C" fn style_get_property_value(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let property = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] style.getPropertyValue('{}') called", property);

    let css_property = to_css_property_name(&property);
    let mut result = String::new();

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        if let Some(style_attr) = elem_data.attributes.iter()
                            .find(|attr| attr.name.local.as_ref() == "style")
                        {
                            // Parse the style string to find the property
                            for declaration in style_attr.value.split(';') {
                                let declaration = declaration.trim();
                                if let Some(colon_pos) = declaration.find(':') {
                                    let prop = declaration[..colon_pos].trim();
                                    if prop == css_property {
                                        result = declaration[colon_pos + 1..].trim().to_string();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    args.rval().set(create_js_string(safe_cx, &result));
    true
}

/// style.setProperty implementation
unsafe extern "C" fn style_set_property(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let property = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(safe_cx, *args.get(1))
    } else {
        String::new()
    };

    trace!("[JS] style.setProperty('{}', '{}') called", property, value);

    // Get the node ID from the style object's __nodeId property
    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                if let Some(node) = dom.get_node_mut(node_id) {
                    if let NodeData::Element(ref mut elem_data) = node.data {
                        // Get existing style attribute
                        let current_style = elem_data.attributes.iter()
                            .find(|attr| attr.name.local.as_ref() == "style")
                            .map(|attr| attr.value.clone())
                            .unwrap_or_default();

                        // Parse current style into a map
                        let mut style_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                        for declaration in current_style.split(';') {
                            let declaration = declaration.trim();
                            if let Some(colon_pos) = declaration.find(':') {
                                let prop = declaration[..colon_pos].trim().to_string();
                                let val = declaration[colon_pos + 1..].trim().to_string();
                                if !prop.is_empty() {
                                    style_map.insert(prop, val);
                                }
                            }
                        }

                        // Convert CSS property name from camelCase to kebab-case
                        let css_property = to_css_property_name(&property);

                        // Set or update the property
                        if value.is_empty() {
                            style_map.remove(&css_property);
                        } else {
                            style_map.insert(css_property, value);
                        }

                        // Reconstruct the style string
                        let new_style: String = style_map.iter()
                            .map(|(k, v)| format!("{}: {}", k, v))
                            .collect::<Vec<_>>()
                            .join("; ");

                        // Update the style attribute
                        let qname = QualName::new(
                            None,
                            markup5ever::ns!(),
                            markup5ever::LocalName::from("style"),
                        );
                        elem_data.attributes.set(qname, &new_style);
                    }
                }
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// style.removeProperty implementation
unsafe extern "C" fn style_remove_property(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let property = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] style.removeProperty('{}') called", property);

    let css_property = to_css_property_name(&property);
    let mut old_value = String::new();

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                if let Some(node) = dom.get_node_mut(node_id) {
                    if let NodeData::Element(ref mut elem_data) = node.data {
                        let current_style = elem_data.attributes.iter()
                            .find(|attr| attr.name.local.as_ref() == "style")
                            .map(|attr| attr.value.clone())
                            .unwrap_or_default();

                        // Parse current style into a map
                        let mut style_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                        for declaration in current_style.split(';') {
                            let declaration = declaration.trim();
                            if let Some(colon_pos) = declaration.find(':') {
                                let prop = declaration[..colon_pos].trim().to_string();
                                let val = declaration[colon_pos + 1..].trim().to_string();
                                if !prop.is_empty() {
                                    style_map.insert(prop, val);
                                }
                            }
                        }

                        // Remove the property and get its old value
                        if let Some(val) = style_map.remove(&css_property) {
                            old_value = val;
                        }

                        // Reconstruct the style string
                        let new_style: String = style_map.iter()
                            .map(|(k, v)| format!("{}: {}", k, v))
                            .collect::<Vec<_>>()
                            .join("; ");

                        // Update the style attribute
                        let qname = QualName::new(
                            None,
                            markup5ever::ns!(),
                            markup5ever::LocalName::from("style"),
                        );
                        elem_data.attributes.set(qname, &new_style);
                    }
                }
            }
        });
    }

    args.rval().set(create_js_string(safe_cx, &old_value));
    true
}

/// classList.add implementation
unsafe extern "C" fn class_list_add(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    // Get the class name(s) to add
    let mut classes_to_add = Vec::new();
    for i in 0..argc {
        let class_name = js_value_to_string(safe_cx, *args.get(i));
        if !class_name.is_empty() {
            classes_to_add.push(class_name);
        }
    }

    trace!("[JS] classList.add({:?}) called", classes_to_add);

    // Get the parent element's node ID from classList's parent
    if let Some(node_id) = get_classlist_parent_node_id(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                if let Some(node) = dom.get_node_mut(node_id) {
                    if let NodeData::Element(ref mut elem_data) = node.data {
                        // Get existing classes
                        let current_classes = elem_data.attributes.iter()
                            .find(|attr| attr.name.local.as_ref() == "class")
                            .map(|attr| attr.value.clone())
                            .unwrap_or_default();

                        let mut class_list: Vec<String> = current_classes
                            .split_whitespace()
                            .map(|s| s.to_string())
                            .collect();

                        // Add new classes if not already present
                        for class in classes_to_add {
                            if !class_list.contains(&class) {
                                class_list.push(class);
                            }
                        }

                        // Update the class attribute
                        let new_classes = class_list.join(" ");
                        let qname = QualName::new(
                            None,
                            markup5ever::ns!(),
                            markup5ever::LocalName::from("class"),
                        );
                        elem_data.attributes.set(qname, &new_classes);
                    }
                }
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// classList.remove implementation
unsafe extern "C" fn class_list_remove(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    // Get the class name(s) to remove
    let mut classes_to_remove = Vec::new();
    for i in 0..argc {
        let class_name = js_value_to_string(safe_cx, *args.get(i));
        if !class_name.is_empty() {
            classes_to_remove.push(class_name);
        }
    }

    trace!("[JS] classList.remove({:?}) called", classes_to_remove);

    if let Some(node_id) = get_classlist_parent_node_id(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                if let Some(node) = dom.get_node_mut(node_id) {
                    if let NodeData::Element(ref mut elem_data) = node.data {
                        let current_classes = elem_data.attributes.iter()
                            .find(|attr| attr.name.local.as_ref() == "class")
                            .map(|attr| attr.value.clone())
                            .unwrap_or_default();

                        let class_list: Vec<String> = current_classes
                            .split_whitespace()
                            .filter(|c| !classes_to_remove.contains(&c.to_string()))
                            .map(|s| s.to_string())
                            .collect();

                        let new_classes = class_list.join(" ");
                        let qname = QualName::new(
                            None,
                            markup5ever::ns!(),
                            markup5ever::LocalName::from("class"),
                        );
                        elem_data.attributes.set(qname, &new_classes);
                    }
                }
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// classList.toggle implementation
unsafe extern "C" fn class_list_toggle(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let class_name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    // Optional force parameter
    let force = if argc > 1 {
        let val = *args.get(1);
        if val.is_boolean() {
            Some(val.to_boolean())
        } else {
            None
        }
    } else {
        None
    };

    trace!("[JS] classList.toggle('{}', {:?}) called", class_name, force);

    let mut result = false;

    if !class_name.is_empty() {
        if let Some(node_id) = get_classlist_parent_node_id(safe_cx, &args) {
            DOM_REF.with(|dom_ref| {
                if let Some(dom_ptr) = *dom_ref.borrow() {
                    let dom = &mut *dom_ptr;
                    if let Some(node) = dom.get_node_mut(node_id) {
                        if let NodeData::Element(ref mut elem_data) = node.data {
                            let current_classes = elem_data.attributes.iter()
                                .find(|attr| attr.name.local.as_ref() == "class")
                                .map(|attr| attr.value.clone())
                                .unwrap_or_default();

                            let mut class_list: Vec<String> = current_classes
                                .split_whitespace()
                                .map(|s| s.to_string())
                                .collect();

                            let has_class = class_list.contains(&class_name);

                            // Determine whether to add or remove based on force parameter
                            let should_add = match force {
                                Some(true) => true,
                                Some(false) => false,
                                None => !has_class,
                            };

                            if should_add && !has_class {
                                class_list.push(class_name);
                                result = true;
                            } else if !should_add && has_class {
                                class_list.retain(|c| c != &class_name);
                                result = false;
                            } else {
                                result = has_class;
                            }

                            let new_classes = class_list.join(" ");
                            let qname = QualName::new(
                                None,
                                markup5ever::ns!(),
                                markup5ever::LocalName::from("class"),
                            );
                            elem_data.attributes.set(qname, &new_classes);
                        }
                    }
                }
            });
        }
    }

    args.rval().set(BooleanValue(result));
    true
}

/// classList.contains implementation
unsafe extern "C" fn class_list_contains(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let class_name = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] classList.contains('{}') called", class_name);

    let mut result = false;

    if !class_name.is_empty() {
        if let Some(node_id) = get_classlist_parent_node_id(safe_cx, &args) {
            DOM_REF.with(|dom_ref| {
                if let Some(dom_ptr) = *dom_ref.borrow() {
                    let dom = &*dom_ptr;
                    if let Some(node) = dom.get_node(node_id) {
                        if let NodeData::Element(ref elem_data) = node.data {
                            let current_classes = elem_data.attributes.iter()
                                .find(|attr| attr.name.local.as_ref() == "class")
                                .map(|attr| attr.value.as_str())
                                .unwrap_or("");

                            result = current_classes
                                .split_whitespace()
                                .any(|c| c == class_name);
                        }
                    }
                }
            });
        }
    }

    args.rval().set(BooleanValue(result));
    true
}

/// classList.replace implementation
unsafe extern "C" fn class_list_replace(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let old_class = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };
    let new_class = if argc > 1 {
        js_value_to_string(safe_cx, *args.get(1))
    } else {
        String::new()
    };

    trace!("[JS] classList.replace('{}', '{}') called", old_class, new_class);

    let mut result = false;

    if !old_class.is_empty() && !new_class.is_empty() {
        if let Some(node_id) = get_classlist_parent_node_id(safe_cx, &args) {
            DOM_REF.with(|dom_ref| {
                if let Some(dom_ptr) = *dom_ref.borrow() {
                    let dom = &mut *dom_ptr;
                    if let Some(node) = dom.get_node_mut(node_id) {
                        if let NodeData::Element(ref mut elem_data) = node.data {
                            let current_classes = elem_data.attributes.iter()
                                .find(|attr| attr.name.local.as_ref() == "class")
                                .map(|attr| attr.value.clone())
                                .unwrap_or_default();

                            let class_list: Vec<String> = current_classes
                                .split_whitespace()
                                .filter(|c| c != &old_class)
                                .map(|s| s.to_string())
                                .collect();

                            if class_list.contains(&old_class) {
                                let new_class_list: Vec<String> = class_list
                                    .into_iter()
                                    .map(|c| if c == old_class { new_class.clone() } else { c })
                                    .collect();

                                let new_classes = new_class_list.join(" ");
                                let qname = QualName::new(
                                    None,
                                    markup5ever::ns!(),
                                    markup5ever::LocalName::from("class"),
                                );
                                elem_data.attributes.set(qname, &new_classes);
                                result = true;
                            }
                        }
                    }
                }
            });
        }
    }

    args.rval().set(BooleanValue(result));
    true
}

/// element.__getTextContent implementation (internal getter for textContent property)
pub(crate) unsafe extern "C" fn element_get_text_content(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    trace!("[JS] element.__getTextContent() called");

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    let text_content = node.text_content();
                    // Mirror into a wrapper-local cache so repeated reads continue to work
                    // even if node lookup fails for subsequent calls.
                    if args.thisv().is_object() && !args.thisv().is_null() {
                        rooted!(in(raw_cx) let this_obj = args.thisv().to_object());
                        rooted!(in(raw_cx) let cache_val = create_js_string(safe_cx, &text_content));
                        let cache_name = std::ffi::CString::new("__textContentCache").unwrap();
                        JS_SetProperty(
                            safe_cx,
                            this_obj.handle(),
                            cache_name.as_ptr(),
                            cache_val.handle(),
                        );
                    }
                    args.rval().set(create_js_string(safe_cx, &text_content));
                    return;
                }
            }
        });
    }

    // Fallback to wrapper-local cache when no DOM node could be resolved.
    if args.thisv().is_object() && !args.thisv().is_null() {
        rooted!(in(raw_cx) let this_obj = args.thisv().to_object());
        rooted!(in(raw_cx) let mut cache_val = UndefinedValue());
        let cache_name = std::ffi::CString::new("__textContentCache").unwrap();
        if JS_GetProperty(
            safe_cx,
            this_obj.handle().into(),
            cache_name.as_ptr(),
            cache_val.handle_mut().into(),
        ) && cache_val.get().is_string() {
            let cached = js_value_to_string(safe_cx, cache_val.get());
            args.rval().set(create_js_string(safe_cx, &cached));
            return true;
        }
    }

    args.rval().set(create_js_string(safe_cx, ""));
    true
}

/// element.__setTextContent implementation (internal setter for textContent property)
pub(crate) unsafe extern "C" fn element_set_text_content(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let text = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    trace!("[JS] element.__setTextContent('{}') called", text);

    // Keep a wrapper-local mirror for robustness during immediate read/modify/write loops.
    if args.thisv().is_object() && !args.thisv().is_null() {
        rooted!(in(raw_cx) let this_obj = args.thisv().to_object());
        rooted!(in(raw_cx) let cache_val = create_js_string(safe_cx, &text));
        let cache_name = std::ffi::CString::new("__textContentCache").unwrap();
        JS_SetProperty(
            safe_cx,
            this_obj.handle(),
            cache_name.as_ptr(),
            cache_val.handle(),
        );
    }

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                dom.set_text_content(node_id, text);
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.__getId implementation (internal getter for id property)
pub(crate) unsafe extern "C" fn element_get_id(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        if let Some(attr) = elem_data.attributes.iter().find(|attr| attr.name.local.as_ref() == "id") {
                            args.rval().set(create_js_string(safe_cx, attr.value.as_ref()));
                            return;
                        }
                    }
                }
            }
        });
    }

    args.rval().set(create_js_string(safe_cx, ""));
    true
}

/// element.__setId implementation (internal setter for id property)
pub(crate) unsafe extern "C" fn element_set_id(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let id_value = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                let qname = QualName::new(
                    None,
                    markup5ever::ns!(),
                    markup5ever::LocalName::from("id"),
                );
                dom.set_attribute(node_id, qname, &id_value);
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.__getClassName implementation (internal getter for className property)
pub(crate) unsafe extern "C" fn element_get_class_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if let Some(node) = dom.get_node(node_id) {
                    if let NodeData::Element(ref elem_data) = node.data {
                        if let Some(attr) = elem_data.attributes.iter().find(|attr| attr.name.local.as_ref() == "class") {
                            args.rval().set(create_js_string(safe_cx, attr.value.as_ref()));
                            return;
                        }
                    }
                }
            }
        });
    }

    args.rval().set(create_js_string(safe_cx, ""));
    true
}

/// element.__setClassName implementation (internal setter for className property)
pub(crate) unsafe extern "C" fn element_set_class_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let class_value = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                let qname = QualName::new(
                    None,
                    markup5ever::ns!(),
                    markup5ever::LocalName::from("class"),
                );
                dom.set_attribute(node_id, qname, &class_value);
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.__getSrc implementation (getter for src IDL-reflected attribute)
pub(crate) unsafe extern "C" fn element_get_src(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let value = get_node_id_from_this(safe_cx, &args)
        .and_then(|id| get_attribute_for_node(id, "src"))
        .unwrap_or_default();
    args.rval().set(create_js_string(safe_cx, &value));
    true
}

/// element.__setSrc implementation (setter for src IDL-reflected attribute)
pub(crate) unsafe extern "C" fn element_set_src(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let value = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
        set_attribute_for_node(node_id, "src", &value);
    }
    args.rval().set(UndefinedValue());
    true
}

/// element.__getType implementation (getter for type IDL-reflected attribute)
pub(crate) unsafe extern "C" fn element_get_type_attr(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let value = get_node_id_from_this(safe_cx, &args)
        .and_then(|id| get_attribute_for_node(id, "type"))
        .unwrap_or_default();
    args.rval().set(create_js_string(safe_cx, &value));
    true
}

/// element.__setType implementation (setter for type IDL-reflected attribute)
pub(crate) unsafe extern "C" fn element_set_type_attr(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let value = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
        set_attribute_for_node(node_id, "type", &value);
    }
    args.rval().set(UndefinedValue());
    true
}

/// element.__getAsync implementation (getter for async IDL-reflected boolean attribute)
pub(crate) unsafe extern "C" fn element_get_async_attr(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let present = get_node_id_from_this(safe_cx, &args)
        .map(|id| get_attribute_for_node(id, "async").is_some())
        .unwrap_or(false);
    args.rval().set(BooleanValue(present));
    true
}

/// element.__setAsync implementation (setter for async IDL-reflected boolean attribute)
pub(crate) unsafe extern "C" fn element_set_async_attr(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let enabled = argc > 0 && {
            let v = *args.get(0);
            if v.is_boolean() {
                v.to_boolean()
            } else if v.is_undefined() || v.is_null() {
                false
            } else {
                true // any other truthy value
            }
        };
        if enabled {
            set_attribute_for_node(node_id, "async", "");
        } else {
            clear_attribute_for_node(node_id, "async");
        }
    }
    args.rval().set(UndefinedValue());
    true
}

/// element.__getValue implementation (getter for value IDL-reflected attribute)
pub(crate) unsafe extern "C" fn element_get_value_attr(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let value = get_node_id_from_this(safe_cx, &args)
        .and_then(|node_id| {
            DOM_REF.with(|dom_ref| {
                let dom_ptr = (*dom_ref.borrow())?;
                let dom = &*dom_ptr;
                let node = dom.get_node(node_id)?;
                let element = node.element_data()?;

                if let Some(input_data) = element.text_input_data() {
                    Some(input_data.editor.raw_text().to_string())
                } else {
                    element
                        .attributes
                        .iter()
                        .find(|a| a.name.local.as_ref() == "value")
                        .map(|a| a.value.to_string())
                }
            })
        })
        .unwrap_or_default();

    args.rval().set(create_js_string(safe_cx, &value));
    true
}

/// element.__setValue implementation (setter for value IDL-reflected attribute)
pub(crate) unsafe extern "C" fn element_set_value_attr(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let value = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
        set_attribute_for_node(node_id, "value", &value);
    }
    args.rval().set(UndefinedValue());
    true
}

/// element.__getChecked implementation (getter for checked IDL-reflected attribute)
pub(crate) unsafe extern "C" fn element_get_checked_attr(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let checked = get_node_id_from_this(safe_cx, &args)
        .and_then(|node_id| {
            DOM_REF.with(|dom_ref| {
                let dom_ptr = (*dom_ref.borrow())?;
                let dom = &*dom_ptr;
                let node = dom.get_node(node_id)?;
                let element = node.element_data()?;
                element.checkbox_input_checked().or_else(|| {
                    Some(
                        element
                            .attributes
                            .iter()
                            .any(|a| a.name.local.as_ref() == "checked"),
                    )
                })
            })
        })
        .unwrap_or(false);

    args.rval().set(BooleanValue(checked));
    true
}

/// element.__setChecked implementation (setter for checked IDL-reflected attribute)
pub(crate) unsafe extern "C" fn element_set_checked_attr(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if let Some(node_id) = get_node_id_from_this(safe_cx, &args) {
        let enabled = argc > 0 && {
            let v = *args.get(0);
            if v.is_boolean() {
                v.to_boolean()
            } else if v.is_undefined() || v.is_null() {
                false
            } else {
                true
            }
        };

        set_checked_for_node(node_id, enabled);
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe fn setup_form_element_bindings(cx: &mut SafeJSContext, element: *mut JSObject) -> Result<(), String> {
    define_function(cx, element, "submit", Some(form_submit), 0)?;
    define_function(cx, element, "requestSubmit", Some(form_request_submit), 1)?;
    define_function(cx, element, "reset", Some(form_reset), 0)?;
    define_function(cx, element, "checkValidity", Some(form_check_validity), 0)?;
    define_function(cx, element, "reportValidity", Some(form_report_validity), 0)?;

    define_function(cx, element, "__getFormAction", Some(form_get_action), 0)?;
    define_function(cx, element, "__setFormAction", Some(form_set_action), 1)?;
    define_function(cx, element, "__getFormMethod", Some(form_get_method), 0)?;
    define_function(cx, element, "__setFormMethod", Some(form_set_method), 1)?;
    define_function(cx, element, "__getFormEnctype", Some(form_get_enctype), 0)?;
    define_function(cx, element, "__setFormEnctype", Some(form_set_enctype), 1)?;
    define_function(cx, element, "__getFormTarget", Some(form_get_target), 0)?;
    define_function(cx, element, "__setFormTarget", Some(form_set_target), 1)?;
    define_function(cx, element, "__getFormName", Some(form_get_name), 0)?;
    define_function(cx, element, "__setFormName", Some(form_set_name), 1)?;
    define_function(cx, element, "__getFormNoValidate", Some(form_get_no_validate), 0)?;
    define_function(cx, element, "__setFormNoValidate", Some(form_set_no_validate), 1)?;
    define_function(cx, element, "__getFormElements", Some(form_get_elements), 0)?;
    define_function(cx, element, "__getFormLength", Some(form_get_length), 0)?;

    define_js_property_accessor(cx, element, "action", "__getFormAction", "__setFormAction")?;
    define_js_property_accessor(cx, element, "method", "__getFormMethod", "__setFormMethod")?;
    define_js_property_accessor(cx, element, "enctype", "__getFormEnctype", "__setFormEnctype")?;
    // encoding is a legacy alias for enctype.
    define_js_property_accessor(cx, element, "encoding", "__getFormEnctype", "__setFormEnctype")?;
    define_js_property_accessor(cx, element, "target", "__getFormTarget", "__setFormTarget")?;
    define_js_property_accessor(cx, element, "name", "__getFormName", "__setFormName")?;
    define_js_property_accessor(cx, element, "noValidate", "__getFormNoValidate", "__setFormNoValidate")?;
    define_js_property_getter(cx, element, "elements", "__getFormElements")?;
    define_js_property_getter(cx, element, "length", "__getFormLength")?;

    Ok(())
}

unsafe fn get_attribute_for_node(node_id: usize, attr: &str) -> Option<String> {
    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            if let Some(node) = dom.get_node(node_id) {
                if let Some(element) = node.element_data() {
                    return element
                        .attributes
                        .iter()
                        .find(|a| a.name.local.as_ref() == attr)
                        .map(|a| a.value.to_string());
                }
            }
        }
        None
    })
}

unsafe fn set_attribute_for_node(node_id: usize, attr: &str, value: &str) {
    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &mut *dom_ptr;
            let qname = QualName::new(None, markup5ever::ns!(), markup5ever::LocalName::from(attr));
            dom.set_attribute(node_id, qname, value);
        }
    });
}

unsafe fn clear_attribute_for_node(node_id: usize, attr: &str) {
    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &mut *dom_ptr;
            let qname = QualName::new(None, markup5ever::ns!(), markup5ever::LocalName::from(attr));
            dom.clear_attribute(node_id, qname);
        }
    });
}

unsafe fn set_checked_for_node(node_id: usize, checked: bool) {
    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &mut *dom_ptr;

            if let Some(node) = dom.get_node_mut(node_id) {
                if let Some(element) = node.element_data_mut() {
                    if let Some(current) = element.checkbox_input_checked_mut() {
                        *current = checked;
                    }
                }
            }

            let qname = QualName::new(None, markup5ever::ns!(), markup5ever::LocalName::from("checked"));
            if checked {
                dom.set_attribute(node_id, qname, "");
            } else {
                dom.clear_attribute(node_id, qname);
            }
        }
    });
}

fn normalize_form_method(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "post" => "post",
        "dialog" => "dialog",
        _ => "get",
    }
}

fn normalize_form_enctype(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "multipart/form-data" => "multipart/form-data",
        "text/plain" => "text/plain",
        _ => "application/x-www-form-urlencoded",
    }
}

unsafe fn form_node_id_from_this(cx: &mut SafeJSContext, args: &CallArgs) -> Option<usize> {
    let node_id = get_node_id_from_this(cx, args)?;
    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            if dom.get_node(node_id).is_some_and(|n| n.data.is_element_with_tag_name(&local_name!("form"))) {
                return Some(node_id);
            }
        }
        None
    })
}

unsafe fn form_control_ids(form_id: usize) -> Vec<usize> {
    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            let mut ids: Vec<usize> = dom
                .controls_to_form
                .iter()
                .filter_map(|(control_id, owner)| if *owner == form_id { Some(*control_id) } else { None })
                .collect();
            ids.sort_unstable();
            return ids;
        }
        Vec::new()
    })
}

unsafe fn form_control_records(form_id: usize) -> Vec<(usize, String, AttributeMap, Option<String>, Option<String>)> {
    DOM_REF.with(|dom_ref| {
        let mut out = Vec::new();
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = &*dom_ptr;
            for control_id in form_control_ids(form_id) {
                let Some(node) = dom.get_node(control_id) else {
                    continue;
                };
                let Some(element) = node.element_data() else {
                    continue;
                };

                let name_attr = element.attr(local_name!("name")).map(ToOwned::to_owned);
                let id_attr = element.attr(local_name!("id")).map(ToOwned::to_owned);
                out.push((
                    control_id,
                    element.name.local.to_string(),
                    element.attributes.clone(),
                    name_attr,
                    id_attr,
                ));
            }
        }
        out
    })
}

unsafe fn form_controls_collection_form_id(cx: &mut SafeJSContext, args: &CallArgs) -> Option<usize> {
    let raw_cx = cx.raw_cx();
    let this_val = args.thisv();
    if !this_val.get().is_object() || this_val.get().is_null() {
        return None;
    }

    rooted!(in(raw_cx) let this_obj = this_val.get().to_object());
    rooted!(in(raw_cx) let mut form_id_val = UndefinedValue());

    let cname = std::ffi::CString::new("__formNodeId").unwrap();
    if !JS_GetProperty(
        cx,
        this_obj.handle().into(),
        cname.as_ptr(),
        form_id_val.handle_mut().into(),
    ) {
        return None;
    }

    if form_id_val.get().is_double() {
        Some(form_id_val.get().to_double() as usize)
    } else if form_id_val.get().is_int32() {
        Some(form_id_val.get().to_int32() as usize)
    } else {
        None
    }
}

unsafe fn form_is_submit_button(dom: &crate::dom::Dom, submitter_id: usize) -> bool {
    dom.get_node(submitter_id)
        .and_then(|node| node.element_data())
        .is_some_and(|element| {
            if element.name.local == local_name!("button") {
                element.is_submit_button()
            } else if element.name.local == local_name!("input") {
                matches!(element.attr(local_name!("type")), Some("submit" | "image"))
            } else {
                false
            }
        })
}

unsafe extern "C" fn form_submit(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                dom.submit_form(form_id, form_id);
            }
        });
    }
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_request_submit(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                if argc > 0 {
                    if let Some(submitter_id) = get_node_id_from_value(safe_cx, *args.get(0)) {
                        let owns_submitter = dom.controls_to_form.get(&submitter_id).is_some_and(|owner| *owner == form_id);
                        if owns_submitter && form_is_submit_button(dom, submitter_id) {
                            dom.submit_form(form_id, submitter_id);
                        }
                    }
                } else {
                    dom.submit_form(form_id, form_id);
                }
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_reset(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        let control_ids = form_control_ids(form_id);
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                for control_id in control_ids {
                    let control_state = dom.get_node(control_id).and_then(|node| {
                        let element = node.element_data()?;
                        let tag = element.name.local.to_string();
                        let input_type = element.attr(local_name!("type")).unwrap_or("text").to_string();
                        let checked_attr = element.attr(local_name!("checked")).is_some();
                        let value_attr = element.attr(local_name!("value")).unwrap_or("").to_string();
                        Some((tag, input_type, checked_attr, value_attr))
                    });

                    let Some((tag, input_type, checked_attr, value_attr)) = control_state else {
                        continue;
                    };

                    if tag == "input" {
                        match input_type.as_str() {
                            "checkbox" | "radio" => {
                                if let Some(node) = dom.get_node_mut(control_id) {
                                    if let Some(element) = node.element_data_mut() {
                                        if let Some(checked) = element.checkbox_input_checked_mut() {
                                            *checked = checked_attr;
                                        }
                                    }
                                }
                            }
                            "file" => {
                                if let Some(node) = dom.get_node_mut(control_id) {
                                    if let Some(element) = node.element_data_mut() {
                                        if let Some(file_data) = element.file_data_mut() {
                                            file_data.clear();
                                        }
                                    }
                                }
                            }
                            _ => {
                                let qname = QualName::new(
                                    None,
                                    markup5ever::ns!(),
                                    markup5ever::LocalName::from("value"),
                                );
                                dom.set_attribute(control_id, qname, &value_attr);
                            }
                        }
                    } else if tag == "textarea" {
                        let qname = QualName::new(
                            None,
                            markup5ever::ns!(),
                            markup5ever::LocalName::from("value"),
                        );
                        dom.set_attribute(control_id, qname, &value_attr);
                    }
                }
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_check_validity(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    // FIXME: Always returns true without running constraint validation on the form's controls.
    warn!("[JS] HTMLFormElement.checkValidity() called on partial binding (always returns true)");
    args.rval().set(BooleanValue(true));
    true
}

unsafe extern "C" fn form_report_validity(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    // FIXME: Always returns true without running constraint validation or highlighting invalid
    // fields to the user via browser UI.
    warn!("[JS] HTMLFormElement.reportValidity() called on partial binding (always returns true)");
    args.rval().set(BooleanValue(true));
    true
}

unsafe extern "C" fn form_get_action(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let mut out = String::new();

    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                let raw_action = dom
                    .get_node(form_id)
                    .and_then(|node| node.element_data())
                    .and_then(|element| element.attr(local_name!("action")))
                    .unwrap_or("");
                out = dom.resolve_url(raw_action).to_string();
            }
        });
    }

    args.rval().set(create_js_string(safe_cx, &out));
    true
}

unsafe extern "C" fn form_set_action(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        let value = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
        set_attribute_for_node(form_id, "action", &value);
    }
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_get_method(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let value = form_node_id_from_this(safe_cx, &args)
        .and_then(|id| get_attribute_for_node(id, "method"))
        .map(|v| normalize_form_method(&v).to_string())
        .unwrap_or_else(|| "get".to_string());
    args.rval().set(create_js_string(safe_cx, &value));
    true
}

unsafe extern "C" fn form_set_method(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        let value = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
        set_attribute_for_node(form_id, "method", normalize_form_method(&value));
    }
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_get_enctype(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let value = form_node_id_from_this(safe_cx, &args)
        .and_then(|id| get_attribute_for_node(id, "enctype"))
        .map(|v| normalize_form_enctype(&v).to_string())
        .unwrap_or_else(|| "application/x-www-form-urlencoded".to_string());
    args.rval().set(create_js_string(safe_cx, &value));
    true
}

unsafe extern "C" fn form_set_enctype(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        let value = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
        set_attribute_for_node(form_id, "enctype", normalize_form_enctype(&value));
    }
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_get_target(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let value = form_node_id_from_this(safe_cx, &args)
        .and_then(|id| get_attribute_for_node(id, "target"))
        .unwrap_or_default();
    args.rval().set(create_js_string(safe_cx, &value));
    true
}

unsafe extern "C" fn form_set_target(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        let value = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
        set_attribute_for_node(form_id, "target", &value);
    }
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_get_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let value = form_node_id_from_this(safe_cx, &args)
        .and_then(|id| get_attribute_for_node(id, "name"))
        .unwrap_or_default();
    args.rval().set(create_js_string(safe_cx, &value));
    true
}

unsafe extern "C" fn form_set_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        let value = if argc > 0 { js_value_to_string(safe_cx, *args.get(0)) } else { String::new() };
        set_attribute_for_node(form_id, "name", &value);
    }
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_get_no_validate(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let no_validate = form_node_id_from_this(safe_cx, &args)
        .and_then(|id| get_attribute_for_node(id, "novalidate"))
        .is_some();
    args.rval().set(BooleanValue(no_validate));
    true
}

unsafe extern "C" fn form_set_no_validate(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        let enabled = argc > 0 && (*args.get(0)).is_boolean() && (*args.get(0)).to_boolean();
        if enabled {
            set_attribute_for_node(form_id, "novalidate", "");
        } else {
            clear_attribute_for_node(form_id, "novalidate");
        }
    }
    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn form_elements_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let Some(form_id) = form_controls_collection_form_id(safe_cx, &args) else {
        args.rval().set(NullValue());
        return true;
    };

    let index = if argc > 0 {
        let idx_val = *args.get(0);
        if idx_val.is_int32() {
            Some(idx_val.to_int32() as i64)
        } else if idx_val.is_double() {
            Some(idx_val.to_double() as i64)
        } else {
            js_value_to_string(safe_cx, idx_val).parse::<i64>().ok()
        }
    } else {
        None
    };

    let Some(index) = index else {
        args.rval().set(NullValue());
        return true;
    };
    if index < 0 {
        args.rval().set(NullValue());
        return true;
    }

    let controls = form_control_records(form_id);
    if let Some((node_id, tag, attrs, _, _)) = controls.get(index as usize) {
        if let Ok(elem) = create_js_element_by_id(safe_cx, *node_id, tag, attrs) {
            args.rval().set(elem);
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

unsafe extern "C" fn form_elements_named_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let Some(form_id) = form_controls_collection_form_id(safe_cx, &args) else {
        args.rval().set(NullValue());
        return true;
    };

    let key = if argc > 0 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        String::new()
    };

    if key.is_empty() {
        args.rval().set(NullValue());
        return true;
    }

    let controls = form_control_records(form_id);
    if let Some((node_id, tag, attrs, _, _)) = controls
        .iter()
        .find(|(_, _, _, name_attr, id_attr)| {
            name_attr.as_ref().is_some_and(|name| name == &key) || id_attr.as_ref().is_some_and(|id| id == &key)
        })
    {
        if let Ok(elem) = create_js_element_by_id(safe_cx, *node_id, tag, attrs) {
            args.rval().set(elem);
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

unsafe extern "C" fn form_get_elements(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    use std::collections::HashSet;

    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    rooted!(in(raw_cx) let collection = create_empty_array(safe_cx));

    if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        rooted!(in(raw_cx) let form_ptr_val = mozjs::jsval::DoubleValue(form_id as f64));
        rooted!(in(raw_cx) let collection_rooted = collection.get());
        let form_id_name = std::ffi::CString::new("__formNodeId").unwrap();
        JS_DefineProperty(
            safe_cx,
            collection_rooted.handle().into(),
            form_id_name.as_ptr(),
            form_ptr_val.handle().into(),
            0,
        );

        let _ = define_function(safe_cx, collection.get(), "item", Some(form_elements_item), 1);
        let _ = define_function(safe_cx, collection.get(), "namedItem", Some(form_elements_named_item), 1);

        let controls = form_control_records(form_id);
        let mut seen_named = HashSet::new();

        for (idx, (id, tag, attrs, name_attr, id_attr)) in controls.iter().enumerate() {
            if let Ok(elem) = create_js_element_by_id(safe_cx, *id, tag, attrs) {
                rooted!(in(raw_cx) let elem_rooted = elem);
                rooted!(in(raw_cx) let collection_obj = collection.get());
                JS_SetElement(safe_cx, collection_obj.handle().into(), idx as u32, elem_rooted.handle().into());

                // Add named property aliases for first matching id/name.
                for key in [name_attr.as_ref(), id_attr.as_ref()].into_iter().flatten() {
                    if key.is_empty() || !seen_named.insert(key.clone()) {
                        continue;
                    }
                    if let Ok(cname) = std::ffi::CString::new(key.as_str()) {
                        JS_DefineProperty(
                            safe_cx,
                            collection_obj.handle().into(),
                            cname.as_ptr(),
                            elem_rooted.handle().into(),
                            JSPROP_ENUMERATE as u32,
                        );
                    }
                }
            }
        }
    }

    args.rval().set(ObjectValue(collection.get()));
    true
}

unsafe extern "C" fn form_get_length(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();
    let len = if let Some(form_id) = form_node_id_from_this(safe_cx, &args) {
        form_control_ids(form_id).len()
    } else {
        0
    };
    args.rval().set(mozjs::jsval::Int32Value(len as i32));
    true
}

