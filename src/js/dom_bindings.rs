use super::element_bindings::ElementWrapper;
use crate::dom::{DomNode, NodeType};
use base64::Engine;
// DOM bindings for JavaScript
use boa_engine::{object::builtins::JsArray, Context, JsResult as BoaResult, JsString, JsValue, NativeFunction};
use boa_gc::{Finalize, Trace};
use std::cell::RefCell;
use std::rc::Rc;

/// Document object wrapper
#[derive(Debug, Clone, Trace, Finalize)]
struct DocumentWrapper {
    #[unsafe_ignore_trace]
    root: Rc<RefCell<DomNode>>,
}

impl DocumentWrapper {
    fn new(root: Rc<RefCell<DomNode>>) -> Self {
        Self { root }
    }

    /// document.getElementById implementation
    fn get_element_by_id(&self, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let id = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        if id.is_empty() {
            return Ok(JsValue::null());
        }

        println!("[JS] document.getElementById('{}') called", id);

        // Search the DOM tree for the element
        let root = self.root.borrow();
        match root.get_element_by_id(&id) {
            Some(element_rc) => {
                drop(root); // Release the borrow before creating JS element
                let element = element_rc.borrow();
                if let NodeType::Element(ref data) = element.node_type {
                    println!("[JS] Found element with id '{}': <{}>", id, data.tag_name);
                    drop(element);
                    ElementWrapper::create_js_element(&element_rc, context)
                } else {
                    Ok(JsValue::null())
                }
            }
            None => {
                println!("[JS] Element with id '{}' not found", id);
                Ok(JsValue::null())
            }
        }
    }

    /// document.getElementsByTagName implementation
    fn get_elements_by_tag_name(&self, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        use boa_engine::object::ObjectInitializer;

        let tag_name = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        println!("[JS] document.getElementsByTagName('{}') called", tag_name);

        let root = self.root.borrow();
        let elements = root.get_elements_by_tag_name(&tag_name);

        let array = JsArray::new(context);
        for (i, element_rc) in elements.iter().enumerate() {
            let element = element_rc.borrow();
            if let NodeType::Element(ref data) = element.node_type {
                // Capture attributes for the closure
                let attributes = data.attributes.clone();

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
                        JsString::from("nodeName"),
                        JsValue::from(JsString::from(data.tag_name.to_uppercase())),
                        boa_engine::property::Attribute::all(),
                    )
                    .function(
                        unsafe {
                            NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                                let attr_name = args.get(0)
                                    .and_then(|v| v.as_string())
                                    .map(|s| s.to_std_string_escaped())
                                    .unwrap_or_default();

                                if let Some(value) = attributes.get(&attr_name) {
                                    Ok(JsValue::from(JsString::from(value.clone())))
                                } else {
                                    Ok(JsValue::null())
                                }
                            })
                        },
                        JsString::from("getAttribute"),
                        1,
                    )
                    .function(
                        NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                            let attr_name = args.get(0)
                                .and_then(|v| v.as_string())
                                .map(|s| s.to_std_string_escaped())
                                .unwrap_or_default();
                            println!("[JS] element.hasAttribute('{}') called", attr_name);
                            Ok(JsValue::from(false))
                        }),
                        JsString::from("hasAttribute"),
                        1,
                    )
                    .build();
                let _ = array.set(i, js_element, true, context);
            }
        }

        println!("[JS] Found {} element(s) with tag '{}'", array.length(context).unwrap_or(0), tag_name);
        Ok(array.into())
    }

    /// document.querySelector implementation
    fn query_selector(&self, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        use boa_engine::object::ObjectInitializer;

        let selector = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        println!("[JS] document.querySelector('{}') called", selector);

        let root = self.root.borrow();
        let elements = root.query_selector(&selector);

        if let Some(element_rc) = elements.first() {
            let element = element_rc.borrow();
            if let NodeType::Element(ref data) = element.node_type {
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
                        JsString::from("textContent"),
                        JsValue::from(JsString::from(element.text_content())),
                        boa_engine::property::Attribute::all(),
                    )
                    .build();

                println!("[JS] Found element matching '{}': <{}>", selector, data.tag_name);
                return Ok(js_element.into());
            }
        }

        println!("[JS] No element found matching '{}'", selector);
        Ok(JsValue::null())
    }

    /// document.querySelectorAll implementation
    fn query_selector_all(&self, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        use boa_engine::object::ObjectInitializer;

        let selector = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        println!("[JS] document.querySelectorAll('{}') called", selector);

        let root = self.root.borrow();
        let elements = root.query_selector(&selector);

        let array = JsArray::new(context);
        for (i, element_rc) in elements.iter().enumerate() {
            let element = element_rc.borrow();
            if let NodeType::Element(ref data) = element.node_type {
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
                        JsString::from("textContent"),
                        JsValue::from(JsString::from(element.text_content())),
                        boa_engine::property::Attribute::all(),
                    )
                    .build();
                let _ = array.set(i, js_element, true, context);
            }
        }

        println!("[JS] Found {} element(s) matching '{}'", array.length(context).unwrap_or(0), selector);
        Ok(array.into())
    }

    /// document.createElement implementation
    fn create_element(&self, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let tag_name = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        if tag_name.is_empty() {
            println!("[JS] document.createElement called with empty tag name");
            return Ok(JsValue::null());
        }

        println!("[JS] document.createElement('{}') called", tag_name);

        // Use ElementWrapper to create a proper element with working getAttribute/setAttribute
        ElementWrapper::create_stub_element(&tag_name, context)
    }
}

/// Window object functions
struct WindowObject;

impl WindowObject {
    /// window.alert implementation
    fn alert(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let message = args.get(0)
            .map(|v| v.to_string(context))
            .transpose()?
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        println!("[JS Alert] {}", message);
        Ok(JsValue::undefined())
    }

    /// window.setTimeout implementation (basic)
    fn set_timeout(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] setTimeout called (not fully implemented yet)");
        // Return a dummy timer ID
        Ok(JsValue::from(1))
    }

    /// window.setInterval implementation (basic)
    fn set_interval(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] setInterval called (not fully implemented yet)");
        // Return a dummy timer ID
        Ok(JsValue::from(1))
    }

    /// window.clearTimeout implementation
    fn clear_timeout(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] clearTimeout called");
        Ok(JsValue::undefined())
    }

    /// window.clearInterval implementation
    fn clear_interval(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] clearInterval called");
        Ok(JsValue::undefined())
    }

    /// window.requestAnimationFrame implementation
    fn request_animation_frame(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] requestAnimationFrame called");
        // Return a dummy request ID
        Ok(JsValue::from(1))
    }

    /// window.cancelAnimationFrame implementation
    fn cancel_animation_frame(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] cancelAnimationFrame called");
        Ok(JsValue::undefined())
    }

    /// window.getComputedStyle implementation
    fn get_computed_style(_this: &JsValue, _args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        use boa_engine::object::ObjectInitializer;
        println!("[JS] getComputedStyle called");
        // Return an empty style object
        let style = ObjectInitializer::new(context).build();
        Ok(style.into())
    }

    /// window.addEventListener implementation
    fn add_event_listener(_this: &JsValue, args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        let event_type = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();
        println!("[JS] window.addEventListener('{}') called", event_type);
        Ok(JsValue::undefined())
    }

    /// window.removeEventListener implementation
    fn remove_event_listener(_this: &JsValue, args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        let event_type = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();
        println!("[JS] window.removeEventListener('{}') called", event_type);
        Ok(JsValue::undefined())
    }
}

/// Navigator object functions
struct NavigatorObject;

impl NavigatorObject {
    fn user_agent(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        Ok(JsValue::from(JsString::from("Stokes Browser/1.0")))
    }

    fn language(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        Ok(JsValue::from(JsString::from("en-US")))
    }

    fn platform(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        Ok(JsValue::from(JsString::from(std::env::consts::OS)))
    }

    fn online(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        Ok(JsValue::from(true))
    }

    fn app_name(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        Ok(JsValue::from(JsString::from("Stokes Browser")))
    }
}

/// Location object functions
struct LocationObject;

impl LocationObject {
    fn href(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        Ok(JsValue::from(JsString::from("about:blank")))
    }

    fn reload(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] location.reload() called");
        Ok(JsValue::undefined())
    }
}

/// Storage object (localStorage/sessionStorage)
struct StorageObject;

impl StorageObject {
    fn get_item(_this: &JsValue, args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        let key = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();
        println!("[JS] Storage.getItem('{}') called", key);
        Ok(JsValue::null())
    }

    fn set_item(_this: &JsValue, args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        let key = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();
        let value = args.get(1)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();
        println!("[JS] Storage.setItem('{}', '{}') called", key, value);
        Ok(JsValue::undefined())
    }

    fn remove_item(_this: &JsValue, args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        let key = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();
        println!("[JS] Storage.removeItem('{}') called", key);
        Ok(JsValue::undefined())
    }

    fn clear(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] Storage.clear() called");
        Ok(JsValue::undefined())
    }
}

/// Set up DOM bindings in the JavaScript context
pub fn setup_dom_bindings(context: &mut Context, document_root: Rc<RefCell<DomNode>>) -> Result<(), String> {
    use boa_engine::object::ObjectInitializer;

    // Create the Node constructor with node type constants
    let node = ObjectInitializer::new(context)
        .property(JsString::from("ELEMENT_NODE"), 1, boa_engine::property::Attribute::all())
        .property(JsString::from("ATTRIBUTE_NODE"), 2, boa_engine::property::Attribute::all())
        .property(JsString::from("TEXT_NODE"), 3, boa_engine::property::Attribute::all())
        .property(JsString::from("CDATA_SECTION_NODE"), 4, boa_engine::property::Attribute::all())
        .property(JsString::from("ENTITY_REFERENCE_NODE"), 5, boa_engine::property::Attribute::all())
        .property(JsString::from("ENTITY_NODE"), 6, boa_engine::property::Attribute::all())
        .property(JsString::from("PROCESSING_INSTRUCTION_NODE"), 7, boa_engine::property::Attribute::all())
        .property(JsString::from("COMMENT_NODE"), 8, boa_engine::property::Attribute::all())
        .property(JsString::from("DOCUMENT_NODE"), 9, boa_engine::property::Attribute::all())
        .property(JsString::from("DOCUMENT_TYPE_NODE"), 10, boa_engine::property::Attribute::all())
        .property(JsString::from("DOCUMENT_FRAGMENT_NODE"), 11, boa_engine::property::Attribute::all())
        .property(JsString::from("NOTATION_NODE"), 12, boa_engine::property::Attribute::all())
        .build();

    // Create the DocumentWrapper instance
    let doc_wrapper = DocumentWrapper::new(document_root);

    // Create closures that use the DocumentWrapper methods
    let doc_wrapper_for_get_by_id = doc_wrapper.clone();
    let get_element_by_id_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            doc_wrapper_for_get_by_id.get_element_by_id(args, context)
        })
    };

    let doc_wrapper_for_get_by_tag = doc_wrapper.clone();
    let get_elements_by_tag_name_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            doc_wrapper_for_get_by_tag.get_elements_by_tag_name(args, context)
        })
    };

    let doc_wrapper_for_query = doc_wrapper.clone();
    let query_selector_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            doc_wrapper_for_query.query_selector(args, context)
        })
    };

    let doc_wrapper_for_query_all = doc_wrapper.clone();
    let query_selector_all_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            doc_wrapper_for_query_all.query_selector_all(args, context)
        })
    };

    let doc_wrapper_for_create = doc_wrapper.clone();
    let create_element_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            doc_wrapper_for_create.create_element(args, context)
        })
    };

    // Create a documentElement object (represents the <html> element)
    let document_element = ObjectInitializer::new(context)
        .property(
            JsString::from("tagName"),
            JsValue::from(JsString::from("HTML")),
            boa_engine::property::Attribute::all(),
        )
        .property(
            JsString::from("nodeName"),
            JsValue::from(JsString::from("HTML")),
            boa_engine::property::Attribute::all(),
        )
        .property(
            JsString::from("nodeType"),
            JsValue::from(1), // ELEMENT_NODE
            boa_engine::property::Attribute::all(),
        )
        .function(
            NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                let attr_name = args.get(0)
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                println!("[JS] documentElement.getAttribute('{}') called", attr_name);
                Ok(JsValue::null())
            }),
            JsString::from("getAttribute"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                let attr_name = args.get(0)
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                let attr_value = args.get(1)
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                println!("[JS] documentElement.setAttribute('{}', '{}') called", attr_name, attr_value);
                Ok(JsValue::undefined())
            }),
            JsString::from("setAttribute"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                let event_type = args.get(0)
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                println!("[JS] documentElement.addEventListener('{}') called", event_type);
                Ok(JsValue::undefined())
            }),
            JsString::from("addEventListener"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                let event_type = args.get(0)
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                println!("[JS] documentElement.removeEventListener('{}') called", event_type);
                Ok(JsValue::undefined())
            }),
            JsString::from("removeEventListener"),
            3,
        )
        .build();

    // Create the document object with the new closures
    let document = ObjectInitializer::new(context)
        .function(
            get_element_by_id_fn,
            JsString::from("getElementById"),
            1,
        )
        .function(
            get_elements_by_tag_name_fn,
            JsString::from("getElementsByTagName"),
            1,
        )
        .function(
            query_selector_fn,
            JsString::from("querySelector"),
            1,
        )
        .function(
            query_selector_all_fn,
            JsString::from("querySelectorAll"),
            1,
        )
        .function(
            create_element_fn,
            JsString::from("createElement"),
            1,
        )
        .property(
            JsString::from("documentElement"),
            document_element,
            boa_engine::property::Attribute::all(),
        )
        .build();

    // Register document object in global scope
    context.register_global_property(JsString::from("document"), document, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register document object: {}", e))?;

    // In browsers, the global object IS the window object
    // So we need to add window properties to the global object itself
    let global_object = context.global_object();

    // Add window functions directly to global object
    global_object.set(
        JsString::from("alert"),
        NativeFunction::from_fn_ptr(WindowObject::alert).to_js_function(context.realm()),
        true,
        context
    ).map_err(|e| format!("Failed to set alert: {}", e))?;

    global_object.set(
        JsString::from("requestAnimationFrame"),
        NativeFunction::from_fn_ptr(WindowObject::request_animation_frame).to_js_function(context.realm()),
        true,
        context
    ).map_err(|e| format!("Failed to set requestAnimationFrame: {}", e))?;

    global_object.set(
        JsString::from("cancelAnimationFrame"),
        NativeFunction::from_fn_ptr(WindowObject::cancel_animation_frame).to_js_function(context.realm()),
        true,
        context
    ).map_err(|e| format!("Failed to set cancelAnimationFrame: {}", e))?;

    global_object.set(
        JsString::from("getComputedStyle"),
        NativeFunction::from_fn_ptr(WindowObject::get_computed_style).to_js_function(context.realm()),
        true,
        context
    ).map_err(|e| format!("Failed to set getComputedStyle: {}", e))?;

    global_object.set(
        JsString::from("addEventListener"),
        NativeFunction::from_fn_ptr(WindowObject::add_event_listener).to_js_function(context.realm()),
        true,
        context
    ).map_err(|e| format!("Failed to set addEventListener: {}", e))?;

    global_object.set(
        JsString::from("removeEventListener"),
        NativeFunction::from_fn_ptr(WindowObject::remove_event_listener).to_js_function(context.realm()),
        true,
        context
    ).map_err(|e| format!("Failed to set removeEventListener: {}", e))?;

    // Register window as the global object itself (circular reference)
    global_object.set(JsString::from("window"), global_object.clone(), true, context)
        .map_err(|e| format!("Failed to set window: {}", e))?;
    global_object.set(JsString::from("self"), global_object.clone(), true, context)
        .map_err(|e| format!("Failed to set self: {}", e))?;
    global_object.set(JsString::from("top"), global_object.clone(), true, context)
        .map_err(|e| format!("Failed to set top: {}", e))?;
    global_object.set(JsString::from("parent"), global_object.clone(), true, context)
        .map_err(|e| format!("Failed to set parent: {}", e))?;
    global_object.set(JsString::from("globalThis"), global_object.clone(), true, context)
        .map_err(|e| format!("Failed to set globalThis: {}", e))?;

    // Create the navigator object with proper properties (not functions)
    let languages_array = JsArray::from_iter([JsValue::from(JsString::from("en-US"))], context);
    let navigator = ObjectInitializer::new(context)
        .property(
            JsString::from("userAgent"),
            JsValue::from(JsString::from("Stokes Browser/1.0")),
            boa_engine::property::Attribute::all(),
        )
        .property(
            JsString::from("language"),
            JsValue::from(JsString::from("en-US")),
            boa_engine::property::Attribute::all(),
        )
        .property(
            JsString::from("languages"),
            languages_array,
            boa_engine::property::Attribute::all(),
        )
        .property(
            JsString::from("platform"),
            JsValue::from(JsString::from(std::env::consts::OS)),
            boa_engine::property::Attribute::all(),
        )
        .property(
            JsString::from("online"),
            JsValue::from(true),
            boa_engine::property::Attribute::all(),
        )
        .property(
            JsString::from("appName"),
            JsValue::from(JsString::from("Stokes Browser")),
            boa_engine::property::Attribute::all(),
        )
        .build();

    // Create the location object
    let location = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(LocationObject::href),
            JsString::from("href"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(LocationObject::reload),
            JsString::from("reload"),
            0,
        )
        .build();

    // Create the storage object (localStorage/sessionStorage)
    let storage = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(StorageObject::get_item),
            JsString::from("getItem"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(StorageObject::set_item),
            JsString::from("setItem"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(StorageObject::remove_item),
            JsString::from("removeItem"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(StorageObject::clear),
            JsString::from("clear"),
            0,
        )
        .build();

    // Register node constructor in global scope
    context.register_global_property(JsString::from("Node"), node, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register Node constructor: {}", e))?;

    // Register navigator object in global scope
    context.register_global_property(JsString::from("navigator"), navigator, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register navigator object: {}", e))?;

    // Register location object in global scope
    context.register_global_property(JsString::from("location"), location, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register location object: {}", e))?;

    // Register storage object in global scope
    context.register_global_property(JsString::from("localStorage"), storage.clone(), boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register localStorage: {}", e))?;
    context.register_global_property(JsString::from("sessionStorage"), storage, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register sessionStorage: {}", e))?;

    // Create a stub Polymer object (for Polymer library compatibility)
    let polymer = ObjectInitializer::new(context).build();
    context.register_global_property(JsString::from("Polymer"), polymer, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register Polymer object: {}", e))?;

    // Create Element constructor with common constants
    let element = ObjectInitializer::new(context).build();
    context.register_global_property(JsString::from("Element"), element, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register Element constructor: {}", e))?;

    // Create HTMLElement constructor
    let html_element = ObjectInitializer::new(context).build();
    context.register_global_property(JsString::from("HTMLElement"), html_element, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register HTMLElement constructor: {}", e))?;

    // Create Event constructor
    let event = ObjectInitializer::new(context).build();
    context.register_global_property(JsString::from("Event"), event, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register Event constructor: {}", e))?;

    // Create CustomEvent constructor
    let custom_event = ObjectInitializer::new(context).build();
    context.register_global_property(JsString::from("CustomEvent"), custom_event, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register CustomEvent constructor: {}", e))?;

    // Create XMLHttpRequest constructor
    let xhr = ObjectInitializer::new(context).build();
    context.register_global_property(JsString::from("XMLHttpRequest"), xhr, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register XMLHttpRequest constructor: {}", e))?;

    // Note: fetch API is now registered in the fetch module

    // Create atob/btoa functions for base64 encoding/decoding
    let atob_fn = NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], _context: &mut Context| {
        let encoded = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        match base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) {
            Ok(decoded) => {
                if let Ok(s) = String::from_utf8(decoded) {
                    Ok(JsValue::from(JsString::from(s)))
                } else {
                    Ok(JsValue::from(JsString::from("")))
                }
            }
            Err(_) => Ok(JsValue::from(JsString::from("")))
        }
    });
    context.register_global_builtin_callable(JsString::from("atob"), 1, atob_fn)
        .map_err(|e| format!("Failed to register atob: {}", e))?;

    let btoa_fn = NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], _context: &mut Context| {
        let data = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
        Ok(JsValue::from(JsString::from(encoded)))
    });
    context.register_global_builtin_callable(JsString::from("btoa"), 1, btoa_fn)
        .map_err(|e| format!("Failed to register btoa: {}", e))?;

    // Initialize dataLayer as an empty array (for Google Analytics/Tag Manager compatibility)
    let data_layer = JsArray::new(context);
    context.register_global_property(JsString::from("dataLayer"), data_layer, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register dataLayer: {}", e))?;

    println!("[JS] DOM bindings initialized");
    Ok(())
}
