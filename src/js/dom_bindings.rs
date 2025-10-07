// DOM bindings for JavaScript
use boa_engine::{Context, JsResult as BoaResult, JsValue, NativeFunction, object::builtins::JsArray, JsString};
use boa_gc::{Finalize, Trace};
use std::rc::Rc;
use std::cell::RefCell;
use base64::Engine;
use crate::dom::{DomNode, NodeType, ElementData};

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
    fn get_element_by_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        // Get the id argument
        let id = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        if id.is_empty() {
            return Ok(JsValue::null());
        }

        // Get the document root from the 'this' context
        // For now, we'll return null as we need to properly store the document reference
        // TODO: Store document root in a way that's accessible from JavaScript callbacks
        println!("[JS] document.getElementById('{}') called", id);

        // Return null for now - will need to create a proper element object
        Ok(JsValue::null())
    }

    /// document.getElementsByTagName implementation
    fn get_elements_by_tag_name(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let tag_name = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        println!("[JS] document.getElementsByTagName('{}') called", tag_name);

        // Return an empty array for now
        let array = JsArray::new(context);
        Ok(array.into())
    }

    /// document.querySelector implementation
    fn query_selector(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let selector = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        println!("[JS] document.querySelector('{}') called", selector);
        Ok(JsValue::null())
    }

    /// document.querySelectorAll implementation
    fn query_selector_all(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let selector = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        println!("[JS] document.querySelectorAll('{}') called", selector);

        // Return an empty array for now
        let array = JsArray::new(context);
        Ok(array.into())
    }

    /// document.createElement implementation
    fn create_element(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let tag_name = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        println!("[JS] document.createElement('{}') called", tag_name);

        // TODO: Create an actual element and return it
        Ok(JsValue::null())
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

    // Create the document object
    // Clone document_root for use in closures
    let doc_root_for_get_by_id = document_root.clone();
    let doc_root_for_get_by_tag = document_root.clone();
    let doc_root_for_query = document_root.clone();
    let doc_root_for_query_all = document_root.clone();

    // Create closures for document methods that capture the document root
    let get_element_by_id_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            let id = args.get(0)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();

            if id.is_empty() {
                return Ok(JsValue::null());
            }

            println!("[JS] document.getElementById('{}') called", id);

            // Search the DOM tree for the element
            let root = doc_root_for_get_by_id.borrow();
            match root.get_element_by_id(&id) {
                Some(element_rc) => {
                    // Create a JavaScript object representing this element
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
                                JsString::from("className"),
                                JsValue::from(JsString::from(
                                    data.attributes.get("class").map(|s| s.as_str()).unwrap_or("")
                                )),
                                boa_engine::property::Attribute::all(),
                            )
                            .property(
                                JsString::from("textContent"),
                                JsValue::from(JsString::from(element.text_content())),
                                boa_engine::property::Attribute::all(),
                            )
                            .build();

                        println!("[JS] Found element with id '{}': <{}>", id, data.tag_name);
                        Ok(js_element.into())
                    } else {
                        Ok(JsValue::null())
                    }
                }
                None => {
                    println!("[JS] Element with id '{}' not found", id);
                    Ok(JsValue::null())
                }
            }
        })
    };

    let get_elements_by_tag_name_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            let tag_name = args.get(0)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();

            println!("[JS] document.getElementsByTagName('{}') called", tag_name);

            let root = doc_root_for_get_by_tag.borrow();
            let elements = root.query_selector(&tag_name);

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
                        .build();
                    let _ = array.set(i, js_element, true, context);
                }
            }

            println!("[JS] Found {} element(s) with tag '{}'", array.length(context).unwrap_or(0), tag_name);
            Ok(array.into())
        })
    };

    let query_selector_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            let selector = args.get(0)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();

            println!("[JS] document.querySelector('{}') called", selector);

            let root = doc_root_for_query.borrow();
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
        })
    };

    let query_selector_all_fn = unsafe {
        NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], context: &mut Context| {
            let selector = args.get(0)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();

            println!("[JS] document.querySelectorAll('{}') called", selector);

            let root = doc_root_for_query_all.borrow();
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
        })
    };

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
            NativeFunction::from_fn_ptr(DocumentWrapper::create_element),
            JsString::from("createElement"),
            1,
        )
        .build();

    // Create the window object
    let window = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(WindowObject::alert),
            JsString::from("alert"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::set_timeout),
            JsString::from("setTimeout"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::set_interval),
            JsString::from("setInterval"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::clear_timeout),
            JsString::from("clearTimeout"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::clear_interval),
            JsString::from("clearInterval"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::request_animation_frame),
            JsString::from("requestAnimationFrame"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::cancel_animation_frame),
            JsString::from("cancelAnimationFrame"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::get_computed_style),
            JsString::from("getComputedStyle"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::add_event_listener),
            JsString::from("addEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(WindowObject::remove_event_listener),
            JsString::from("removeEventListener"),
            2,
        )
        .build();

    // Create the navigator object
    let navigator = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(NavigatorObject::user_agent),
            JsString::from("userAgent"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(NavigatorObject::language),
            JsString::from("language"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(NavigatorObject::platform),
            JsString::from("platform"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(NavigatorObject::online),
            JsString::from("online"),
            0,
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

    // Register document object in global scope
    context.register_global_property(JsString::from("document"), document, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register document object: {}", e))?;

    // Register window object in global scope
    context.register_global_property(JsString::from("window"), window, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register window object: {}", e))?;

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

    // Create fetch stub (returns a rejected promise for now)
    let fetch_fn = NativeFunction::from_fn_ptr(|_this: &JsValue, args: &[JsValue], _context: &mut Context| {
        let url = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();
        println!("[JS] fetch('{}') called (not implemented)", url);
        Ok(JsValue::undefined())
    });
    context.register_global_builtin_callable(JsString::from("fetch"), 1, fetch_fn)
        .map_err(|e| format!("Failed to register fetch: {}", e))?;

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
