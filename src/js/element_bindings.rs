// Element bindings for JavaScript
use boa_engine::{Context, JsResult as BoaResult, JsValue, NativeFunction, JsString, object::builtins::JsArray};
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_gc::{Finalize, Trace};
use std::rc::Rc;
use std::cell::RefCell;
use crate::dom::{DomNode, NodeType, ElementData, EventType};

/// Wrapper for a DOM element that can be used in JavaScript
#[derive(Debug, Clone, Trace, Finalize)]
pub struct ElementWrapper {
    #[unsafe_ignore_trace]
    pub node: Rc<RefCell<DomNode>>,
}

impl ElementWrapper {
    pub fn new(node: Rc<RefCell<DomNode>>) -> Self {
        Self { node }
    }

    /// Create a JavaScript object from a DOM element
    pub fn create_js_element(
        element_rc: &Rc<RefCell<DomNode>>,
        context: &mut Context,
    ) -> BoaResult<JsValue> {
        let element = element_rc.borrow();

        if let NodeType::Element(ref data) = element.node_type {
            // Clone the Rc for use in closures
            let node_for_get_attr = Rc::clone(element_rc);
            let node_for_set_attr = Rc::clone(element_rc);
            let node_for_remove_attr = Rc::clone(element_rc);
            let node_for_has_attr = Rc::clone(element_rc);
            let node_for_append_child = Rc::clone(element_rc);
            let node_for_remove_child = Rc::clone(element_rc);
            let node_for_insert_before = Rc::clone(element_rc);
            let node_for_query_selector = Rc::clone(element_rc);
            let node_for_query_selector_all = Rc::clone(element_rc);
            let node_for_add_event_listener = Rc::clone(element_rc);
            let node_for_remove_event_listener = Rc::clone(element_rc);
            let node_for_text_content_get = Rc::clone(element_rc);
            let node_for_text_content_set = Rc::clone(element_rc);
            // Store the node reference for child manipulation
            let node_ref_for_storage = Rc::clone(element_rc);

            // Create accessor functions before ObjectInitializer to avoid borrow conflicts
            let text_content_getter = unsafe {
                NativeFunction::from_closure(move |_this: &JsValue, _args: &[JsValue], _context: &mut Context| {
                    let node = node_for_text_content_get.borrow();
                    let content = node.text_content();
                    Ok(JsValue::from(JsString::from(content)))
                }).to_js_function(context.realm())
            };

            let text_content_setter = unsafe {
                NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                    let text = args.get(0)
                        .and_then(|v| v.as_string())
                        .map(|s| s.to_std_string_escaped())
                        .unwrap_or_default();

                    let mut node = node_for_text_content_set.borrow_mut();
                    node.set_text_content(&text);
                    println!("[JS] element.textContent = '{}'", text);
                    Ok(JsValue::undefined())
                }).to_js_function(context.realm())
            };

            let js_element = ObjectInitializer::new(context)
                .property(
                    JsString::from("tagName"),
                    JsValue::from(JsString::from(data.tag_name.to_uppercase())),
                    boa_engine::property::Attribute::all(),
                )
                .property(
                    JsString::from("id"),
                    JsValue::from(JsString::from(data.id().unwrap_or(""))),
                    boa_engine::property::Attribute::all(),
                )
                .property(
                    JsString::from("className"),
                    JsValue::from(JsString::from(
                        data.attributes.get("class").map(|s| s.as_str()).unwrap_or("")
                    )),
                    boa_engine::property::Attribute::all(),
                )
                .accessor(
                    JsString::from("textContent"),
                    Some(text_content_getter),
                    Some(text_content_setter),
                    boa_engine::property::Attribute::all(),
                )
                .property(
                    JsString::from("nodeName"),
                    JsValue::from(JsString::from(data.tag_name.to_uppercase())),
                    boa_engine::property::Attribute::all(),
                )
                .property(
                    JsString::from("nodeType"),
                    JsValue::from(1), // ELEMENT_NODE
                    boa_engine::property::Attribute::all(),
                )
                // Store a pointer value as a unique identifier (for internal use)
                .property(
                    JsString::from("__nodePtr"),
                    JsValue::from(Rc::as_ptr(&node_ref_for_storage) as i64),
                    Attribute::empty(), // Hidden property
                )
                // getAttribute implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                            let attr_name = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();

                            let node = node_for_get_attr.borrow();
                            if let NodeType::Element(ref element_data) = node.node_type {
                                if let Some(value) = element_data.attributes.get(&attr_name) {
                                    println!("[JS] element.getAttribute('{}') = '{}'", attr_name, value);
                                    Ok(JsValue::from(JsString::from(value.clone())))
                                } else {
                                    println!("[JS] element.getAttribute('{}') = null", attr_name);
                                    Ok(JsValue::null())
                                }
                            } else {
                                Ok(JsValue::null())
                            }
                        })
                    },
                    JsString::from("getAttribute"),
                    1,
                )
                // setAttribute implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                            let attr_name = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();
                            let attr_value = args.get(1)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();

                            let mut node = node_for_set_attr.borrow_mut();
                            if let NodeType::Element(ref mut element_data) = node.node_type {
                                element_data.attributes.insert(attr_name.clone(), attr_value.clone());
                                println!("[JS] element.setAttribute('{}', '{}')", attr_name, attr_value);
                            }
                            Ok(JsValue::undefined())
                        })
                    },
                    JsString::from("setAttribute"),
                    2,
                )
                // removeAttribute implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                            let attr_name = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();

                            let mut node = node_for_remove_attr.borrow_mut();
                            if let NodeType::Element(ref mut element_data) = node.node_type {
                                element_data.attributes.remove(&attr_name);
                                println!("[JS] element.removeAttribute('{}')", attr_name);
                            }
                            Ok(JsValue::undefined())
                        })
                    },
                    JsString::from("removeAttribute"),
                    1,
                )
                // hasAttribute implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                            let attr_name = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();

                            let node = node_for_has_attr.borrow();
                            if let NodeType::Element(ref element_data) = node.node_type {
                                let has = element_data.attributes.contains_key(&attr_name);
                                println!("[JS] element.hasAttribute('{}') = {}", attr_name, has);
                                Ok(JsValue::from(has))
                            } else {
                                Ok(JsValue::from(false))
                            }
                        })
                    },
                    JsString::from("hasAttribute"),
                    1,
                )
                // appendChild implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
                            let child_value = args.get(0).cloned().unwrap_or(JsValue::null());

                            println!("[JS] element.appendChild() called");

                            // Try to extract the child element's node reference
                            if let Some(child_obj) = child_value.as_object() {
                                // Get the __nodePtr property to identify which node this is
                                let node_ptr_key = JsString::from("__nodePtr");
                                if let Ok(ptr_value) = child_obj.get(node_ptr_key, context) {
                                    if let Some(child_ptr) = ptr_value.as_number() {
                                        println!("[JS] Found child node pointer: {}", child_ptr as i64);

                                        // We need to find the child node from our parent's children or create it
                                        // Since we can't easily convert the pointer back safely, we'll use a different approach:
                                        // Check if this child is already in the tree somewhere
                                        let parent_node = node_for_append_child.borrow();

                                        // For now, if the child has a valid pointer, we'll treat it as a valid operation
                                        // In a full implementation, we'd maintain a registry of node pointers
                                        drop(parent_node);

                                        // Try to get the child's tag name to create a new node
                                        let tag_key = JsString::from("tagName");
                                        if let Ok(tag_value) = child_obj.get(tag_key, context) {
                                            if let Some(tag_str) = tag_value.as_string() {
                                                let tag_name = tag_str.to_std_string_escaped().to_lowercase();
                                                println!("[JS] Appending child with tag: {}", tag_name);

                                                // Create a new node and add it
                                                let element_data = ElementData::new(&tag_name);
                                                let new_child = DomNode::new(NodeType::Element(element_data), None);

                                                let mut parent_node = node_for_append_child.borrow_mut();
                                                let child_rc = parent_node.add_child(new_child);

                                                // Update the child's parent reference
                                                child_rc.borrow_mut().parent = Some(Rc::downgrade(&node_for_append_child));

                                                println!("[JS] Successfully appended child. Parent now has {} children",
                                                         parent_node.children.len());
                                            }
                                        }
                                    }
                                }
                            }

                            Ok(child_value)
                        })
                    },
                    JsString::from("appendChild"),
                    1,
                )
                // removeChild implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
                            let child_value = args.get(0).cloned().unwrap_or(JsValue::null());

                            println!("[JS] element.removeChild() called");

                            // Try to extract the child element's node reference
                            if let Some(child_obj) = child_value.as_object() {
                                // Get the __nodePtr property
                                let node_ptr_key = JsString::from("__nodePtr");
                                if let Ok(ptr_value) = child_obj.get(node_ptr_key, context) {
                                    if let Some(child_ptr) = ptr_value.as_number() {
                                        println!("[JS] Found child node pointer to remove: {}", child_ptr as i64);

                                        let mut parent_node = node_for_remove_child.borrow_mut();

                                        // Find and remove the child by pointer comparison
                                        let child_ptr_addr = child_ptr as i64;
                                        let initial_count = parent_node.children.len();

                                        parent_node.children.retain(|child| {
                                            let child_addr = Rc::as_ptr(child) as i64;
                                            child_addr != child_ptr_addr
                                        });

                                        let final_count = parent_node.children.len();
                                        if initial_count > final_count {
                                            println!("[JS] Successfully removed child. Parent now has {} children", final_count);
                                        } else {
                                            println!("[JS] Child not found in parent");
                                        }
                                    }
                                }
                            }

                            Ok(child_value)
                        })
                    },
                    JsString::from("removeChild"),
                    1,
                )
                // insertBefore implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                            println!("[JS] element.insertBefore() called");
                            // Note: This is a simplified implementation
                            Ok(args.get(0).cloned().unwrap_or(JsValue::null()))
                        })
                    },
                    JsString::from("insertBefore"),
                    2,
                )
                // addEventListener implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                            let event_type_str = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();

                            let callback = args.get(1)
                                .and_then(|v| v.as_object());

                            let use_capture = args.get(2)
                                .and_then(|v| v.as_boolean())
                                .unwrap_or(false);

                            println!("[JS] element.addEventListener('{}', callback, {})", event_type_str, use_capture);

                            if let Some(callback_obj) = callback {
                                let event_type = EventType::from_str(&event_type_str);
                                let mut node = node_for_add_event_listener.borrow_mut();
                                let listener_id = node.event_listeners.add_listener(event_type, callback_obj, use_capture);
                                println!("[JS] Event listener added with ID: {}", listener_id);
                            } else {
                                println!("[JS] Warning: addEventListener called without a valid callback function");
                            }

                            Ok(JsValue::undefined())
                        })
                    },
                    JsString::from("addEventListener"),
                    3,
                )
                // removeEventListener implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                            let event_type_str = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();

                            let callback = args.get(1)
                                .and_then(|v| v.as_object());

                            println!("[JS] element.removeEventListener('{}') called", event_type_str);

                            if let Some(callback_obj) = callback {
                                let event_type = EventType::from_str(&event_type_str);
                                let mut node = node_for_remove_event_listener.borrow_mut();
                                let removed = node.event_listeners.remove_listener(&event_type, &callback_obj);
                                if removed {
                                    println!("[JS] Event listener removed successfully");
                                } else {
                                    println!("[JS] Event listener not found");
                                }
                            } else {
                                println!("[JS] Warning: removeEventListener called without a valid callback function");
                            }

                            Ok(JsValue::undefined())
                        })
                    },
                    JsString::from("removeEventListener"),
                    2,
                )
                // querySelector implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
                            let selector = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();

                            println!("[JS] element.querySelector('{}') called", selector);

                            let node = node_for_query_selector.borrow();
                            let results = node.query_selector(&selector);

                            if let Some(result_rc) = results.first() {
                                drop(node); // Release borrow before creating JS element
                                ElementWrapper::create_js_element(result_rc, context)
                            } else {
                                Ok(JsValue::null())
                            }
                        })
                    },
                    JsString::from("querySelector"),
                    1,
                )
                // querySelectorAll implementation
                .function(
                    unsafe {
                        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
                            let selector = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();

                            println!("[JS] element.querySelectorAll('{}') called", selector);

                            let node = node_for_query_selector_all.borrow();
                            let results = node.query_selector(&selector);

                            let array = JsArray::new(context);
                            for (i, result_rc) in results.iter().enumerate() {
                                if let Ok(js_elem) = ElementWrapper::create_js_element(result_rc, context) {
                                    let _ = array.set(i, js_elem, true, context);
                                }
                            }

                            println!("[JS] element.querySelectorAll found {} elements", results.len());
                            Ok(array.into())
                        })
                    },
                    JsString::from("querySelectorAll"),
                    1,
                )
                .build();

            Ok(js_element.into())
        } else {
            Ok(JsValue::null())
        }
    }

    /// Create a simple stub element (for document.createElement)
    pub fn create_stub_element(tag_name: &str, context: &mut Context) -> BoaResult<JsValue> {
        // Create a new element node
        let element_data = ElementData::new(tag_name);
        let new_node = DomNode::new(NodeType::Element(element_data), None);
        let node_rc = Rc::new(RefCell::new(new_node));

        // Use the same create_js_element function for consistency
        Self::create_js_element(&node_rc, context)
    }
}
