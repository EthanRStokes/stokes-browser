use super::super::helpers::create_empty_array;
use super::{custom_elements, event, history, html_form_element, html_iframe_element, interface_registry, location, navigator, storage, window};
use super::cookie::set_document_url;
// DOM bindings for JavaScript using mozjs
use crate::dom::Dom;
use crate::js::JsRuntime;
use mozjs::jsapi::{JSObject, JS_DefineProperty, JSPROP_ENUMERATE};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsval::ObjectValue;
use mozjs::rooted;
use std::cell::RefCell;

// Thread-local storage for DOM reference
thread_local! {
    pub(crate) static DOM_REF: RefCell<Option<*mut Dom>> = RefCell::new(None);
    pub(crate) static USER_AGENT: RefCell<String> = RefCell::new(String::new());
    pub(crate) static LOCAL_STORAGE: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    pub(crate) static SESSION_STORAGE: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    /// Node ID of the currently-executing script element (for document.currentScript)
    pub(crate) static CURRENT_SCRIPT_NODE_ID: RefCell<Option<usize>> = RefCell::new(None);
}

/// Set the node ID of the currently-executing script element.
/// Call with `Some(node_id)` before executing a script and `None` afterwards.
pub fn set_current_script(node_id: Option<usize>) {
    CURRENT_SCRIPT_NODE_ID.with(|id| *id.borrow_mut() = node_id);
}

// ============================================================================
// Public API
// ============================================================================

/// Set up DOM bindings in the JavaScript context
pub fn setup_dom_bindings(
    runtime: &mut JsRuntime,
    document_root: *mut Dom,
    user_agent: String,
) -> Result<(), String> {
    // Store DOM reference in thread-local storage
    DOM_REF.set(Some(document_root));
    USER_AGENT.set(user_agent.clone());

    // Store the document URL for cookie handling
    unsafe {
        let dom = &*document_root;
        let url: url::Url = (&dom.url).into();
        set_document_url(url);
    }

    runtime.do_with_jsapi(|cx, global| unsafe {
        let raw_cx = cx.raw_cx();
        let global_ptr = global.get();


        // Set up window object (as alias to global)
        window::setup_window_bindings(cx, global_ptr, &user_agent)?;

        // Set up navigator object
        navigator::setup_navigator_bindings(cx, global_ptr, &user_agent)?;

        // Set up location object
        location::setup_location_bindings(cx, global_ptr)?;

        // Set up History API object (window.history)
        history::setup_history_bindings(cx, global_ptr)?;

        // Set up localStorage and sessionStorage
        storage::setup_storage_bindings(cx, global_ptr)?;

        // Install core interface constructors (EventTarget/Node/Element).
        interface_registry::install_phase_bindings(
            cx,
            global_ptr,
            interface_registry::InstallPhase::CoreDom,
        )?;

        // Link parent constructors/prototypes via a single registry pass.
        interface_registry::link_phase_inheritance(
            cx,
            global_ptr,
            interface_registry::InstallPhase::CoreDom,
        )?;

        // Set up HTMLFormElement constructor
        html_form_element::setup_html_form_element_constructor_bindings(raw_cx, global_ptr)?;

        // Set up HTMLIFrameElement constructor
        html_iframe_element::setup_html_iframe_element_constructor_bindings(cx, global_ptr)?;

        // Set up Event and CustomEvent constructors
        event::setup_event_constructors(raw_cx, global)?;


        // Set up atob/btoa functions
        window::setup_base64_functions(cx, global_ptr)?;

        // Set up dataLayer for Google Analytics compatibility
        setup_data_layer(cx, global_ptr)?;

        Ok::<(), String>(())
    })?;

    // Install customElements immediately as part of base DOM setup.
    custom_elements::setup_custom_elements(runtime)?;

    Ok(())
}

// Event and CustomEvent constructors
// Set up XMLHttpRequest constructor

/// Set up dataLayer for Google Analytics compatibility
unsafe fn setup_data_layer(cx: &mut SafeJSContext, global: *mut JSObject) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let data_layer = create_empty_array(cx));
    if data_layer.get().is_null() {
        return Err("Failed to create dataLayer array".to_string());
    }

    rooted!(in(raw_cx) let data_layer_val = ObjectValue(data_layer.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("dataLayer").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        data_layer_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

// Window/media-query logic lives in `window.rs`.

#[cfg(test)]
mod tests {
    use crate::js::bindings::window::{decode_atob_binary_string, evaluate_media_query, normalize_atob_input};

    #[test]
    fn width_queries_match_expected_ranges() {
        assert!(evaluate_media_query("(min-width: 600px)", 800.0, 600.0, 1.0));
        assert!(evaluate_media_query("(max-width: 1024px)", 800.0, 600.0, 1.0));
        assert!(!evaluate_media_query("(min-width: 1200px)", 800.0, 600.0, 1.0));
    }

    #[test]
    fn orientation_and_or_list_work() {
        assert!(evaluate_media_query("screen and (orientation: landscape)", 900.0, 700.0, 1.0));
        assert!(evaluate_media_query("(max-width: 500px), (orientation: landscape)", 900.0, 700.0, 1.0));
        assert!(!evaluate_media_query("print and (min-width: 1px)", 900.0, 700.0, 1.0));
    }

    #[test]
    fn resolution_units_are_supported() {
        assert!(evaluate_media_query("(min-resolution: 96dpi)", 800.0, 600.0, 1.0));
        assert!(evaluate_media_query("(resolution: 1dppx)", 800.0, 600.0, 1.0));
        assert!(!evaluate_media_query("(min-resolution: 2dppx)", 800.0, 600.0, 1.0));
    }

    #[test]
    fn atob_accepts_whitespace_and_missing_padding() {
        assert_eq!(decode_atob_binary_string(" SGVs\nbG8= ").unwrap(), "Hello");
        assert_eq!(normalize_atob_input("YQ").unwrap(), "YQ==");
        assert_eq!(normalize_atob_input("YWI").unwrap(), "YWI=");
    }

    #[test]
    fn atob_returns_binary_string_for_non_utf8_bytes() {
        let decoded = decode_atob_binary_string("/wA=").unwrap();
        let code_points: Vec<u32> = decoded.chars().map(|ch| ch as u32).collect();
        assert_eq!(code_points, vec![255, 0]);
    }

    #[test]
    fn atob_rejects_invalid_base64_inputs() {
        assert!(decode_atob_binary_string("A").is_err());
        assert!(decode_atob_binary_string("YW=J").is_err());
        assert!(decode_atob_binary_string("YWJj====").is_err());
        assert!(decode_atob_binary_string("YWJj*").is_err());
    }
}
