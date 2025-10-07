// DOM bindings for JavaScript
use boa_engine::{Context, JsResult as BoaResult, JsValue, NativeFunction, object::builtins::JsArray, JsString};
use boa_gc::{Finalize, Trace};
use std::rc::Rc;
use std::cell::RefCell;
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
    fn get_element_by_id(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        // Get the id argument
        let id = args.get(0)
            .and_then(|v| v.as_string())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default();

        if id.is_empty() {
            return Ok(JsValue::null());
        }

        // TODO: Actually search the DOM for the element with this ID
        // For now, return null
        println!("[JS] document.getElementById('{}') called", id);
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
    fn set_timeout(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] setTimeout called (not fully implemented yet)");
        // Return a dummy timer ID
        Ok(JsValue::from(1))
    }

    /// window.setInterval implementation (basic)
    fn set_interval(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        println!("[JS] setInterval called (not fully implemented yet)");
        // Return a dummy timer ID
        Ok(JsValue::from(1))
    }
}

/// Set up DOM bindings in the JavaScript context
pub fn setup_dom_bindings(context: &mut Context, document_root: Rc<RefCell<DomNode>>) -> Result<(), String> {
    use boa_engine::object::ObjectInitializer;

    // Create the document object
    let document = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(DocumentWrapper::get_element_by_id),
            JsString::from("getElementById"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(DocumentWrapper::get_elements_by_tag_name),
            JsString::from("getElementsByTagName"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(DocumentWrapper::query_selector),
            JsString::from("querySelector"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(DocumentWrapper::query_selector_all),
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
        .build();

    // Register document object in global scope
    context.register_global_property(JsString::from("document"), document, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register document object: {}", e))?;

    // Register window object in global scope
    context.register_global_property(JsString::from("window"), window, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register window object: {}", e))?;

    // Initialize dataLayer as an empty array (for Google Analytics/Tag Manager compatibility)
    let data_layer = JsArray::new(context);
    context.register_global_property(JsString::from("dataLayer"), data_layer, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register dataLayer: {}", e))?;

    println!("[JS] DOM bindings initialized");
    Ok(())
}
