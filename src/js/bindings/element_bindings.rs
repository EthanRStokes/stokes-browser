// Element bindings for JavaScript using mozjs
use crate::dom::{AttributeMap, Dom, NodeData};
use markup5ever::QualName;
use mozjs::jsapi::{
    CallArgs, JSContext, JSObject, JS_DefineProperty, JS_NewPlainObject, JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_uint;
use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::helpers::{create_empty_array, create_js_string, define_function, get_node_id_from_this, js_value_to_string, set_int_property, set_string_property, to_css_property_name};
use crate::js::selectors::matches_selector;

// Thread-local storage for DOM reference (shared with dom_bindings)
thread_local! {
    static ELEMENT_CHILDREN: RefCell<HashMap<usize, Vec<usize>>> = RefCell::new(HashMap::new());
}


/// Create a JS element wrapper for a DOM node with its real tag name and attributes
pub unsafe fn create_js_element_by_id(
    raw_cx: *mut JSContext,
    node_id: usize,
    tag_name: &str,
    attributes: &AttributeMap,
) -> Result<JSVal, String> {
    rooted!(in(raw_cx) let element = JS_NewPlainObject(raw_cx));
    if element.get().is_null() {
        return Err("Failed to create element object".to_string());
    }

    // Initialize children storage for this element
    ELEMENT_CHILDREN.with(|children| {
        children.borrow_mut().insert(node_id, Vec::new());
    });

    // Get id and className from attributes
    let id_attr = attributes.iter()
        .find(|attr| attr.name.local.as_ref() == "id")
        .map(|attr| attr.value.as_ref())
        .unwrap_or("");
    let class_attr = attributes.iter()
        .find(|attr| attr.name.local.as_ref() == "class")
        .map(|attr| attr.value.as_ref())
        .unwrap_or("");

    // Set basic properties
    set_string_property(raw_cx, element.get(), "tagName", &tag_name.to_uppercase())?;
    set_string_property(raw_cx, element.get(), "nodeName", &tag_name.to_uppercase())?;
    set_int_property(raw_cx, element.get(), "nodeType", 1)?; // ELEMENT_NODE
    set_string_property(raw_cx, element.get(), "id", id_attr)?;
    set_string_property(raw_cx, element.get(), "className", class_attr)?;
    // TODO: innerHTML, outerHTML, textContent are stub values - should serialize/deserialize actual DOM content
    set_string_property(raw_cx, element.get(), "innerHTML", "")?;
    set_string_property(raw_cx, element.get(), "outerHTML", &format!("<{0}></{0}>", tag_name.to_lowercase()))?;
    set_string_property(raw_cx, element.get(), "textContent", "")?;

    // Store the node_id for reference to the actual DOM node
    rooted!(in(raw_cx) let ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
    rooted!(in(raw_cx) let element_rooted = element.get());
    let cname = std::ffi::CString::new("__nodeId").unwrap();
    JS_DefineProperty(
        raw_cx,
        element_rooted.handle().into(),
        cname.as_ptr(),
        ptr_val.handle().into(),
        0, // Hidden property
    );

    // Define element methods
    define_function(raw_cx, element.get(), "getAttribute", Some(element_get_attribute), 1)?;
    define_function(raw_cx, element.get(), "setAttribute", Some(element_set_attribute), 2)?;
    define_function(raw_cx, element.get(), "removeAttribute", Some(element_remove_attribute), 1)?;
    define_function(raw_cx, element.get(), "hasAttribute", Some(element_has_attribute), 1)?;
    define_function(raw_cx, element.get(), "appendChild", Some(element_append_child), 1)?;
    define_function(raw_cx, element.get(), "removeChild", Some(element_remove_child), 1)?;
    define_function(raw_cx, element.get(), "insertBefore", Some(element_insert_before), 2)?;
    define_function(raw_cx, element.get(), "replaceChild", Some(element_replace_child), 2)?;
    define_function(raw_cx, element.get(), "cloneNode", Some(element_clone_node), 1)?;
    define_function(raw_cx, element.get(), "querySelector", Some(element_query_selector), 1)?;
    define_function(raw_cx, element.get(), "querySelectorAll", Some(element_query_selector_all), 1)?;
    define_function(raw_cx, element.get(), "addEventListener", Some(element_add_event_listener), 3)?;
    define_function(raw_cx, element.get(), "removeEventListener", Some(element_remove_event_listener), 3)?;
    define_function(raw_cx, element.get(), "dispatchEvent", Some(element_dispatch_event), 1)?;
    define_function(raw_cx, element.get(), "focus", Some(element_focus), 0)?;
    define_function(raw_cx, element.get(), "blur", Some(element_blur), 0)?;
    define_function(raw_cx, element.get(), "click", Some(element_click), 0)?;
    define_function(raw_cx, element.get(), "getBoundingClientRect", Some(element_get_bounding_client_rect), 0)?;
    define_function(raw_cx, element.get(), "getClientRects", Some(element_get_client_rects), 0)?;
    define_function(raw_cx, element.get(), "closest", Some(element_closest), 1)?;
    define_function(raw_cx, element.get(), "matches", Some(element_matches), 1)?;
    define_function(raw_cx, element.get(), "contains", Some(element_contains), 1)?;

    // Create style object
    rooted!(in(raw_cx) let style = JS_NewPlainObject(raw_cx));
    if !style.get().is_null() {
        // Store the node_id so style methods can access the parent element
        rooted!(in(raw_cx) let style_ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
        rooted!(in(raw_cx) let style_rooted = style.get());
        let style_id_name = std::ffi::CString::new("__nodeId").unwrap();
        JS_DefineProperty(
            raw_cx,
            style_rooted.handle().into(),
            style_id_name.as_ptr(),
            style_ptr_val.handle().into(),
            0,
        );

        define_function(raw_cx, style.get(), "getPropertyValue", Some(style_get_property_value), 1)?;
        define_function(raw_cx, style.get(), "setProperty", Some(style_set_property), 3)?;
        define_function(raw_cx, style.get(), "removeProperty", Some(style_remove_property), 1)?;

        rooted!(in(raw_cx) let style_val = ObjectValue(style.get()));
        let cname = std::ffi::CString::new("style").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            style_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Create classList object
    rooted!(in(raw_cx) let class_list = JS_NewPlainObject(raw_cx));
    if !class_list.get().is_null() {
        // Store the node_id so classList methods can access the parent element
        rooted!(in(raw_cx) let cl_ptr_val = mozjs::jsval::DoubleValue(node_id as f64));
        rooted!(in(raw_cx) let class_list_rooted = class_list.get());
        let cl_id_name = std::ffi::CString::new("__nodeId").unwrap();
        JS_DefineProperty(
            raw_cx,
            class_list_rooted.handle().into(),
            cl_id_name.as_ptr(),
            cl_ptr_val.handle().into(),
            0,
        );

        define_function(raw_cx, class_list.get(), "add", Some(class_list_add), 1)?;
        define_function(raw_cx, class_list.get(), "remove", Some(class_list_remove), 1)?;
        define_function(raw_cx, class_list.get(), "toggle", Some(class_list_toggle), 2)?;
        define_function(raw_cx, class_list.get(), "contains", Some(class_list_contains), 1)?;
        define_function(raw_cx, class_list.get(), "replace", Some(class_list_replace), 2)?;
        // FIXME: classList.length is hardcoded to 0 - should reflect actual number of classes and update dynamically
        set_int_property(raw_cx, class_list.get(), "length", 0)?;

        rooted!(in(raw_cx) let class_list_val = ObjectValue(class_list.get()));
        let cname = std::ffi::CString::new("classList").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            class_list_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Create dataset object
    rooted!(in(raw_cx) let dataset = JS_NewPlainObject(raw_cx));
    if !dataset.get().is_null() {
        rooted!(in(raw_cx) let dataset_val = ObjectValue(dataset.get()));
        let cname = std::ffi::CString::new("dataset").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            dataset_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Look up the parent from the DOM
    let parent_info: Option<(usize, String, AttributeMap)> = if node_id != 0 {
        DOM_REF.with(|dom_ref| {
            if let Some(ref dom) = *dom_ref.borrow() {
                let dom = &**dom;
                if let Some(node) = dom.get_node(node_id) {
                    if let Some(parent_id) = node.parent {
                        if let Some(parent_node) = dom.get_node(parent_id) {
                            if let NodeData::Element(ref elem_data) = parent_node.data {
                                return Some((parent_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                            }
                        }
                    }
                }
            }
            None
        })
    } else {
        None
    };

    if let Some((parent_id, parent_tag, _parent_attrs)) = parent_info {
        // Create a parent element wrapper with insertBefore method
        rooted!(in(raw_cx) let parent_elem = JS_NewPlainObject(raw_cx));
        if !parent_elem.get().is_null() {
            set_string_property(raw_cx, parent_elem.get(), "tagName", &parent_tag.to_uppercase())?;
            set_string_property(raw_cx, parent_elem.get(), "nodeName", &parent_tag.to_uppercase())?;
            set_int_property(raw_cx, parent_elem.get(), "nodeType", 1)?;

            // Store the parent node_id
            rooted!(in(raw_cx) let parent_ptr_val = mozjs::jsval::DoubleValue(parent_id as f64));
            rooted!(in(raw_cx) let parent_elem_rooted = parent_elem.get());
            let parent_id_name = std::ffi::CString::new("__nodeId").unwrap();
            JS_DefineProperty(
                raw_cx,
                parent_elem_rooted.handle().into(),
                parent_id_name.as_ptr(),
                parent_ptr_val.handle().into(),
                0,
            );

            // Add insertBefore method to parent
            define_function(raw_cx, parent_elem.get(), "insertBefore", Some(element_insert_before), 2)?;
            define_function(raw_cx, parent_elem.get(), "appendChild", Some(element_append_child), 1)?;
            define_function(raw_cx, parent_elem.get(), "removeChild", Some(element_remove_child), 1)?;

            rooted!(in(raw_cx) let parent_val = ObjectValue(parent_elem.get()));
            let cname = std::ffi::CString::new("parentNode").unwrap();
            JS_DefineProperty(
                raw_cx,
                element_rooted.handle().into(),
                cname.as_ptr(),
                parent_val.handle().into(),
                JSPROP_ENUMERATE as u32,
            );
            let cname = std::ffi::CString::new("parentElement").unwrap();
            JS_DefineProperty(
                raw_cx,
                element_rooted.handle().into(),
                cname.as_ptr(),
                parent_val.handle().into(),
                JSPROP_ENUMERATE as u32,
            );
        }
    } else {
        rooted!(in(raw_cx) let null_val = NullValue());
        for name in &["parentNode", "parentElement"] {
            let cname = std::ffi::CString::new(*name).unwrap();
            JS_DefineProperty(
                raw_cx,
                element_rooted.handle().into(),
                cname.as_ptr(),
                null_val.handle().into(),
                JSPROP_ENUMERATE as u32,
            );
        }
    }

    // Set sibling properties to null initially
    rooted!(in(raw_cx) let null_val = NullValue());
    for name in &["firstChild", "lastChild", "previousSibling", "nextSibling"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            null_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Create empty children and childNodes arrays
    rooted!(in(raw_cx) let children_array = create_empty_array(raw_cx));
    if !children_array.get().is_null() {
        rooted!(in(raw_cx) let children_val = ObjectValue(children_array.get()));
        let cname = std::ffi::CString::new("children").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            children_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
        let cname = std::ffi::CString::new("childNodes").unwrap();
        JS_DefineProperty(
            raw_cx,
            element_rooted.handle().into(),
            cname.as_ptr(),
            children_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Set dimension properties
    set_int_property(raw_cx, element.get(), "offsetWidth", 0)?;
    set_int_property(raw_cx, element.get(), "offsetHeight", 0)?;
    set_int_property(raw_cx, element.get(), "offsetLeft", 0)?;
    set_int_property(raw_cx, element.get(), "offsetTop", 0)?;
    set_int_property(raw_cx, element.get(), "clientWidth", 0)?;
    set_int_property(raw_cx, element.get(), "clientHeight", 0)?;
    set_int_property(raw_cx, element.get(), "scrollWidth", 0)?;
    set_int_property(raw_cx, element.get(), "scrollHeight", 0)?;
    set_int_property(raw_cx, element.get(), "scrollLeft", 0)?;
    set_int_property(raw_cx, element.get(), "scrollTop", 0)?;

    Ok(ObjectValue(element.get()))
}

/// Create a stub element (for document.createElement and similar)
pub unsafe fn create_stub_element(raw_cx: *mut JSContext, tag_name: &str) -> Result<JSVal, String> {
    // Create element with no attributes
    create_js_element_by_id(raw_cx, 0, tag_name, &AttributeMap::empty())
}

// ============================================================================
// Local helper functions
// ============================================================================

/// Get the node ID from classList's parent element
unsafe fn get_classlist_parent_node_id(raw_cx: *mut JSContext, args: &CallArgs) -> Option<usize> {
    // First try to get __nodeId directly from this (for when classList is on the element directly)
    if let Some(id) = get_node_id_from_this(raw_cx, args) {
        return Some(id);
    }
    // classList doesn't have __nodeId directly - this is a limitation
    None
}

// ============================================================================
// Element methods
// ============================================================================

/// element.getAttribute implementation
unsafe extern "C" fn element_get_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let attr_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.getAttribute('{}') called", attr_name);

    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
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
            println!("[JS] getAttribute('{}') = '{}'", attr_name, val);
            args.rval().set(create_js_string(raw_cx, &val));
        } else {
            args.rval().set(NullValue());
        }
    } else {
        args.rval().set(NullValue());
    }
    true
}

/// element.setAttribute implementation
unsafe extern "C" fn element_set_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let attr_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let attr_value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    println!("[JS] element.setAttribute('{}', '{}') called", attr_name, attr_value);

    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                if let Some(node) = dom.get_node_mut(node_id) {
                    if let NodeData::Element(ref mut elem_data) = node.data {
                        // Create QualName for the attribute
                        let qname = QualName::new(
                            None,
                            markup5ever::ns!(),
                            markup5ever::LocalName::from(attr_name.as_str()),
                        );
                        elem_data.attributes.set(qname, &attr_value);
                    }
                }
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.removeAttribute implementation
unsafe extern "C" fn element_remove_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let attr_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.removeAttribute('{}') called", attr_name);

    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
        DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &mut *dom_ptr;
                if let Some(node) = dom.get_node_mut(node_id) {
                    if let NodeData::Element(ref mut elem_data) = node.data {
                        // Create QualName for the attribute to remove
                        let qname = QualName::new(
                            None,
                            markup5ever::ns!(),
                            markup5ever::LocalName::from(attr_name.as_str()),
                        );
                        elem_data.attributes.remove(&qname);
                    }
                }
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// element.hasAttribute implementation
unsafe extern "C" fn element_has_attribute(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let attr_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.hasAttribute('{}') called", attr_name);

    let has_attr = if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
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

/// element.appendChild implementation
// FIXME: Just returns child - doesn't actually add child to DOM tree or update parent/child relationships
unsafe extern "C" fn element_append_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.appendChild() called");

    DOM_REF.with(|dom| {
        println!("SKIBIDI TOILET")
    });
    if argc > 0 {
        // Return the child that was appended
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.removeChild implementation
unsafe extern "C" fn element_remove_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.removeChild() called");

    if argc > 0 {
        // Return the child that was removed
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.insertBefore implementation
unsafe extern "C" fn element_insert_before(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.insertBefore() called");

    if argc > 0 {
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.replaceChild implementation
unsafe extern "C" fn element_replace_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.replaceChild() called");

    if argc > 1 {
        // Return the old child that was replaced
        args.rval().set(*args.get(1));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// element.cloneNode implementation
unsafe extern "C" fn element_clone_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let deep = if argc > 0 {
        let val = *args.get(0);
        val.is_boolean() && val.to_boolean()
    } else {
        false
    };

    println!("[JS] element.cloneNode({}) called", deep);

    // Get the tag name and attributes from the current element
    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
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
            match create_js_element_by_id(raw_cx, 0, &tag_name, &attributes) {
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
        if mozjs::jsapi::JS_GetProperty(raw_cx, this_obj.handle().into(), cname.as_ptr(), tag_val.handle_mut().into()) {
            if tag_val.get().is_string() {
                let tag_name = js_value_to_string(raw_cx, tag_val.get());
                match create_stub_element(raw_cx, &tag_name.to_lowercase()) {
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
    match create_stub_element(raw_cx, "div") {
        Ok(elem) => args.rval().set(elem),
        Err(_) => args.rval().set(NullValue()),
    }
    true
}

/// element.querySelector implementation
unsafe extern "C" fn element_query_selector(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.querySelector('{}') called", selector);

    if selector.is_empty() {
        args.rval().set(NullValue());
        return true;
    }

    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
        // Search descendants of this element
        let matching_element = DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                // Traverse the subtree looking for a match
                fn find_in_subtree(dom: &crate::dom::Dom, parent_id: usize, selector: &str) -> Option<(usize, String, crate::dom::AttributeMap)> {
                    if let Some(parent_node) = dom.get_node(parent_id) {
                        for child_id in &parent_node.children {
                            if let Some(child_node) = dom.get_node(*child_id) {
                                if let crate::dom::NodeData::Element(ref elem_data) = child_node.data {
                                    if matches_selector(selector, &elem_data.name.local.to_string(), &elem_data.attributes) {
                                        return Some((*child_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                                    }
                                }
                                // Recurse into children
                                if let Some(result) = find_in_subtree(dom, *child_id, selector) {
                                    return Some(result);
                                }
                            }
                        }
                    }
                    None
                }
                return find_in_subtree(dom, node_id, &selector);
            }
            None
        });

        if let Some((match_id, tag_name, attributes)) = matching_element {
            match create_js_element_by_id(raw_cx, match_id, &tag_name, &attributes) {
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
unsafe extern "C" fn element_query_selector_all(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.querySelectorAll('{}') called", selector);

    // Create JS array
    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));

    if !selector.is_empty() {
        if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
            let matching_elements: Vec<(usize, String, crate::dom::AttributeMap)> = DOM_REF.with(|dom_ref| {
                let mut results = Vec::new();
                if let Some(dom_ptr) = *dom_ref.borrow() {
                    let dom = &*dom_ptr;
                    // Collect all matching descendants
                    fn collect_in_subtree(dom: &crate::dom::Dom, parent_id: usize, selector: &str, results: &mut Vec<(usize, String, crate::dom::AttributeMap)>) {
                        if let Some(parent_node) = dom.get_node(parent_id) {
                            for child_id in &parent_node.children {
                                if let Some(child_node) = dom.get_node(*child_id) {
                                    if let crate::dom::NodeData::Element(ref elem_data) = child_node.data {
                                        if matches_selector(selector, &elem_data.name.local.to_string(), &elem_data.attributes) {
                                            results.push((*child_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                                        }
                                    }
                                    // Recurse into children
                                    collect_in_subtree(dom, *child_id, selector, results);
                                }
                            }
                        }
                    }
                    collect_in_subtree(dom, node_id, &selector, &mut results);
                }
                results
            });

            for (index, (match_id, tag, attrs)) in matching_elements.iter().enumerate() {
                if let Ok(js_elem) = create_js_element_by_id(raw_cx, *match_id, tag, attrs) {
                    rooted!(in(raw_cx) let elem_val = js_elem);
                    rooted!(in(raw_cx) let array_obj = array.get());
                    mozjs::rust::wrappers::JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
                }
            }
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// element.addEventListener implementation
unsafe extern "C" fn element_add_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.addEventListener('{}') called", event_type);
    args.rval().set(UndefinedValue());
    true
}

/// element.removeEventListener implementation
unsafe extern "C" fn element_remove_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.removeEventListener('{}') called", event_type);
    args.rval().set(UndefinedValue());
    true
}

/// element.dispatchEvent implementation
unsafe extern "C" fn element_dispatch_event(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.dispatchEvent() called");
    args.rval().set(BooleanValue(true));
    true
}

/// element.focus implementation
unsafe extern "C" fn element_focus(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.focus() called");
    args.rval().set(UndefinedValue());
    true
}

/// element.blur implementation
unsafe extern "C" fn element_blur(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.blur() called");
    args.rval().set(UndefinedValue());
    true
}

/// element.click implementation
unsafe extern "C" fn element_click(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.click() called");
    args.rval().set(UndefinedValue());
    true
}

/// element.getBoundingClientRect implementation
unsafe extern "C" fn element_get_bounding_client_rect(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.getBoundingClientRect() called");

    // Return a DOMRect-like object
    rooted!(in(raw_cx) let rect = JS_NewPlainObject(raw_cx));
    if !rect.get().is_null() {
        let _ = set_int_property(raw_cx, rect.get(), "x", 0);
        let _ = set_int_property(raw_cx, rect.get(), "y", 0);
        let _ = set_int_property(raw_cx, rect.get(), "width", 0);
        let _ = set_int_property(raw_cx, rect.get(), "height", 0);
        let _ = set_int_property(raw_cx, rect.get(), "top", 0);
        let _ = set_int_property(raw_cx, rect.get(), "right", 0);
        let _ = set_int_property(raw_cx, rect.get(), "bottom", 0);
        let _ = set_int_property(raw_cx, rect.get(), "left", 0);
        args.rval().set(ObjectValue(rect.get()));
    } else {
        args.rval().set(NullValue());
    }
    true
}

/// element.getClientRects implementation
unsafe extern "C" fn element_get_client_rects(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.getClientRects() called");

    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));
    args.rval().set(ObjectValue(array.get()));
    true
}

/// element.closest implementation
unsafe extern "C" fn element_closest(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.closest('{}') called", selector);

    if selector.is_empty() {
        args.rval().set(NullValue());
        return true;
    }

    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
        // Traverse up the parent chain looking for a match
        let matching_element = DOM_REF.with(|dom_ref| {
            if let Some(dom_ptr) = *dom_ref.borrow() {
                let dom = &*dom_ptr;
                let mut current_id = Some(node_id);

                while let Some(id) = current_id {
                    if let Some(node) = dom.get_node(id) {
                        if let NodeData::Element(ref elem_data) = node.data {
                            // Check if this element matches the selector
                            if matches_selector(&selector, &elem_data.name.local.to_string(), &elem_data.attributes) {
                                return Some((id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
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

        if let Some((match_id, tag_name, attributes)) = matching_element {
            match create_js_element_by_id(raw_cx, match_id, &tag_name, &attributes) {
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
unsafe extern "C" fn element_matches(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] element.matches('{}') called", selector);

    let mut result = false;

    if !selector.is_empty() {
        if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
            DOM_REF.with(|dom_ref| {
                if let Some(dom_ptr) = *dom_ref.borrow() {
                    let dom = &*dom_ptr;
                    if let Some(node) = dom.get_node(node_id) {
                        if let NodeData::Element(ref elem_data) = node.data {
                            result = matches_selector(&selector, &elem_data.name.local.to_string(), &elem_data.attributes);
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
unsafe extern "C" fn element_contains(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] element.contains() called");

    // Get the node ID of this element
    let this_node_id = get_node_id_from_this(raw_cx, &args);

    // Get the node ID of the argument element
    let other_node_id = if argc > 0 {
        let other_val = *args.get(0);
        if other_val.is_object() && !other_val.is_null() {
            rooted!(in(raw_cx) let other_obj = other_val.to_object());
            rooted!(in(raw_cx) let mut ptr_val = UndefinedValue());
            let cname = std::ffi::CString::new("__nodeId").unwrap();
            if mozjs::jsapi::JS_GetProperty(raw_cx, other_obj.handle().into(), cname.as_ptr(), ptr_val.handle_mut().into()) {
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

// ============================================================================
// Style methods
// ============================================================================

/// style.getPropertyValue implementation
unsafe extern "C" fn style_get_property_value(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let property = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] style.getPropertyValue('{}') called", property);

    let css_property = to_css_property_name(&property);
    let mut result = String::new();

    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
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

    args.rval().set(create_js_string(raw_cx, &result));
    true
}

/// style.setProperty implementation
unsafe extern "C" fn style_set_property(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let property = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    println!("[JS] style.setProperty('{}', '{}') called", property, value);

    // Get the node ID from the style object's __nodeId property
    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
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

    let property = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] style.removeProperty('{}') called", property);

    let css_property = to_css_property_name(&property);
    let mut old_value = String::new();

    if let Some(node_id) = get_node_id_from_this(raw_cx, &args) {
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

    args.rval().set(create_js_string(raw_cx, &old_value));
    true
}

/// classList.add implementation
unsafe extern "C" fn class_list_add(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    // Get the class name(s) to add
    let mut classes_to_add = Vec::new();
    for i in 0..argc {
        let class_name = js_value_to_string(raw_cx, *args.get(i));
        if !class_name.is_empty() {
            classes_to_add.push(class_name);
        }
    }

    println!("[JS] classList.add({:?}) called", classes_to_add);

    // Get the parent element's node ID from classList's parent
    if let Some(node_id) = get_classlist_parent_node_id(raw_cx, &args) {
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

    // Get the class name(s) to remove
    let mut classes_to_remove = Vec::new();
    for i in 0..argc {
        let class_name = js_value_to_string(raw_cx, *args.get(i));
        if !class_name.is_empty() {
            classes_to_remove.push(class_name);
        }
    }

    println!("[JS] classList.remove({:?}) called", classes_to_remove);

    if let Some(node_id) = get_classlist_parent_node_id(raw_cx, &args) {
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

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
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

    println!("[JS] classList.toggle('{}', {:?}) called", class_name, force);

    let mut result = false;

    if !class_name.is_empty() {
        if let Some(node_id) = get_classlist_parent_node_id(raw_cx, &args) {
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

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] classList.contains('{}') called", class_name);

    let mut result = false;

    if !class_name.is_empty() {
        if let Some(node_id) = get_classlist_parent_node_id(raw_cx, &args) {
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

    let old_class = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let new_class = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    println!("[JS] classList.replace('{}', '{}') called", old_class, new_class);

    let mut result = false;

    if !old_class.is_empty() && !new_class.is_empty() {
        if let Some(node_id) = get_classlist_parent_node_id(raw_cx, &args) {
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




