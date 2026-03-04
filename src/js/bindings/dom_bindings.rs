use super::super::helpers::{
    create_empty_array, create_js_string, define_function, js_value_to_string,
    set_bool_property, set_int_property, set_string_property, get_node_id_from_value,
};
use super::cookies::{ensure_cookie_jar_initialized, set_document_url, Cookie, COOKIE_JAR, DOCUMENT_URL};
use super::element_bindings;
// DOM bindings for JavaScript using mozjs
use crate::dom::{AttributeMap, Dom};
use crate::js::bindings::element_bindings::element_append_child;
use crate::js::selectors::matches_selector;
use crate::js::JsRuntime;
use html5ever::ns;
use markup5ever::QualName;
use mozjs::jsapi::{
    CallArgs, JSContext, JSObject, JS_DefineProperty, JS_NewPlainObject,
    JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use std::cell::RefCell;
use std::os::raw::c_uint;

// Thread-local storage for DOM reference
thread_local! {
    pub(crate) static DOM_REF: RefCell<Option<*mut Dom>> = RefCell::new(None);
    pub(crate) static USER_AGENT: RefCell<String> = RefCell::new(String::new());
    pub(crate) static LOCAL_STORAGE: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    pub(crate) static SESSION_STORAGE: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
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

    runtime.do_with_jsapi(|_rt, cx, global| unsafe {
        let global_ptr = global.get();

        // Create and set up document object
        setup_document(cx, global_ptr)?;

        // Set up window object (as alias to global)
        setup_window(cx, global_ptr, &user_agent)?;

        // Set up navigator object
        setup_navigator(cx, global_ptr, &user_agent)?;

        // Set up location object
        setup_location(cx, global_ptr)?;

        // Set up localStorage and sessionStorage
        setup_storage(cx, global_ptr)?;

        // Set up Node constructor with constants
        setup_node_constructor(cx, global_ptr)?;

        // Set up Element and HTMLElement constructors
        setup_element_constructors(cx, global_ptr)?;

        // Set up HTMLFormElement constructor
        setup_html_form_element_constructor(cx, global_ptr)?;

        // Set up HTMLIFrameElement constructor
        setup_html_iframe_element_constructor(cx, global_ptr)?;

        // Set up Event and CustomEvent constructors
        setup_event_constructors(cx, global_ptr)?;

        // Set up XMLHttpRequest constructor
        setup_xhr_constructor(cx, global_ptr)?;

        // Set up atob/btoa functions
        setup_base64_functions(cx, global_ptr)?;

        // Set up dataLayer for Google Analytics compatibility
        setup_data_layer(cx, global_ptr)?;

        Ok(())
    })
}

/// Set up the document.cookie property with getter/setter
/// This should be called from the runtime after initialization is complete
pub fn setup_cookie_property_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        Object.defineProperty(document, 'cookie', {
            get: function() {
                return document.__getCookie();
            },
            set: function(value) {
                document.__setCookie(value);
            },
            configurable: true,
            enumerable: true
        });
    "#;

    // Use the runtime's execute method which handles realm entry properly
    runtime.execute(script).map_err(|e| {
        println!("[JS] Warning: Failed to set up document.cookie property: {}", e);
        e
    })?;

    Ok(())
}

/// Set up the document.head property with getter
/// This should be called from the runtime after initialization is complete
pub fn setup_head_property_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        Object.defineProperty(document, 'head', {
            get: function() {
                return document.__getHead();
            },
            configurable: true,
            enumerable: true
        });
    "#;

    // Use the runtime's execute method which handles realm entry properly
    runtime.execute(script).map_err(|e| {
        println!("[JS] Warning: Failed to set up document.head property: {}", e);
        e
    })?;

    Ok(())
}

/// Set up the document.body property with getter/setter
/// This should be called from the runtime after initialization is complete
pub fn setup_body_property_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        Object.defineProperty(document, 'body', {
            get: function() {
                return document.__getBody();
            },
            set: function(value) {
                document.__setBody(value);
            },
            configurable: true,
            enumerable: true
        });
    "#;

    runtime.execute(script).map_err(|e| {
        println!("[JS] Warning: Failed to set up document.body property: {}", e);
        e
    })?;

    Ok(())
}

/// Set up window.matchMedia and MediaQueryList behavior.
/// This must run after the window object exists
pub fn setup_match_media_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        (function() {
            const root = typeof globalThis !== 'undefined' ? globalThis : window;

            function enrichChangeEvent(event, mql) {
                try {
                    Object.defineProperty(event, 'matches', {
                        value: mql.matches,
                        configurable: true,
                        enumerable: true,
                    });
                } catch (_err) {
                    event.matches = mql.matches;
                }

                try {
                    Object.defineProperty(event, 'media', {
                        value: mql.media,
                        configurable: true,
                        enumerable: true,
                    });
                } catch (_err) {
                    event.media = mql.media;
                }

                return event;
            }

            function createChangeEvent(mql) {
                let event;
                if (typeof root.Event === 'function') {
                    event = new root.Event('change');
                } else {
                    event = { type: 'change' };
                }
                return enrichChangeEvent(event, mql);
            }

            function assertDispatchEventArgument(event) {
                if (event == null || typeof event !== 'object') {
                    throw new TypeError("Failed to execute 'dispatchEvent' on 'MediaQueryList': parameter 1 is not of type 'Event'.");
                }

                if (typeof root.Event === 'function' && !(event instanceof root.Event)) {
                    throw new TypeError("Failed to execute 'dispatchEvent' on 'MediaQueryList': parameter 1 is not of type 'Event'.");
                }

                if (typeof event.type !== 'string') {
                    throw new TypeError("Failed to execute 'dispatchEvent' on 'MediaQueryList': parameter 1 is not of type 'Event'.");
                }
            }

            if (!(root.__matchMediaRegistry instanceof Set)) {
                root.__matchMediaRegistry = new Set();
            }

            root.matchMedia = function(query) {
                const mediaText = String(query == null ? '' : query);
                const listeners = new Set();
                let onchangeHandler = null;

                const mql = {
                    get matches() {
                        return !!root.__evaluateMediaQuery(mediaText);
                    },
                    media: mediaText,
                    get onchange() {
                        return onchangeHandler;
                    },
                    set onchange(handler) {
                        onchangeHandler = (typeof handler === 'function') ? handler : null;
                    },
                    addListener(listener) {
                        if (typeof listener === 'function') {
                            listeners.add(listener);
                        }
                    },
                    removeListener(listener) {
                        listeners.delete(listener);
                    },
                    addEventListener(type, listener) {
                        if (type === 'change' && typeof listener === 'function') {
                            listeners.add(listener);
                        }
                    },
                    removeEventListener(type, listener) {
                        if (type === 'change') {
                            listeners.delete(listener);
                        }
                    },
                    dispatchEvent(event) {
                        assertDispatchEventArgument(event);
                        if (event.type !== 'change') {
                            return true;
                        }

                        const enrichedEvent = enrichChangeEvent(event, mql);

                        for (const listener of Array.from(listeners)) {
                            try {
                                listener.call(mql, enrichedEvent);
                            } catch (_err) {}
                        }

                        if (typeof onchangeHandler === 'function') {
                            try {
                                onchangeHandler.call(mql, enrichedEvent);
                            } catch (_err) {}
                        }

                        return true;
                    },
                };

                mql.__lastMatches = mql.matches;
                root.__matchMediaRegistry.add(mql);
                return mql;
            };

            root.__notifyMatchMediaListeners = function() {
                for (const mql of Array.from(root.__matchMediaRegistry)) {
                    if (!mql) {
                        continue;
                    }

                    const next = !!mql.matches;
                    if (next !== mql.__lastMatches) {
                        mql.__lastMatches = next;
                        mql.dispatchEvent(createChangeEvent(mql));
                    }
                }
            };
        })();
    "#;

    runtime.execute(script).map_err(|e| {
        println!("[JS] Warning: Failed to set up window.matchMedia: {}", e);
        e
    })?;

    Ok(())
}

// ============================================================================
// Setup functions
// ============================================================================

/// Set up the document object
unsafe fn setup_document(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let document = JS_NewPlainObject(raw_cx));
    if document.get().is_null() {
        return Err("Failed to create document object".to_string());
    }

    // Create Document constructor
    rooted!(in(raw_cx) let document_constructor = JS_NewPlainObject(raw_cx));
    if document_constructor.get().is_null() {
        return Err("Failed to create Document constructor".to_string());
    }

    rooted!(in(raw_cx) let document_constructor_val = ObjectValue(document_constructor.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("Document").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        document_constructor_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Define document methods
    define_function(raw_cx, document.get(), "getElementById", Some(document_get_element_by_id), 1)?;
    define_function(raw_cx, document.get(), "getElementsByTagName", Some(document_get_elements_by_tag_name), 1)?;
    define_function(raw_cx, document.get(), "getElementsByClassName", Some(document_get_elements_by_class_name), 1)?;
    define_function(raw_cx, document.get(), "querySelector", Some(document_query_selector), 1)?;
    define_function(raw_cx, document.get(), "querySelectorAll", Some(document_query_selector_all), 1)?;
    define_function(raw_cx, document.get(), "createElement", Some(document_create_element), 1)?;
    define_function(raw_cx, document.get(), "createTextNode", Some(document_create_text_node), 1)?;
    define_function(raw_cx, document.get(), "createDocumentFragment", Some(document_create_document_fragment), 0)?;

    // Add cookie getter and setter helper functions
    define_function(raw_cx, document.get(), "__getCookie", Some(document_get_cookie), 0)?;
    define_function(raw_cx, document.get(), "__setCookie", Some(document_set_cookie), 1)?;

    // Add document.head getter function
    define_function(raw_cx, document.get(), "__getHead", Some(document_get_head), 0)?;
    define_function(raw_cx, document.get(), "__getBody", Some(document_get_body), 0)?;
    define_function(raw_cx, document.get(), "__setBody", Some(document_set_body), 1)?;

    // Create documentElement (represents <html>) using a proper element with methods
    let doc_elem_val = element_bindings::create_stub_element(raw_cx, "html")?;
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

    // Set document on global
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

    Ok(())
}

/// Set up the window object (as alias to global)
// FIXME: Window dimensions, scroll positions, and devicePixelRatio are hardcoded - should get actual values from renderer
unsafe fn setup_window(
    raw_cx: *mut JSContext,
    global: *mut JSObject,
    _user_agent: &str,
) -> Result<(), String> {
    rooted!(in(raw_cx) let global_val = ObjectValue(global));
    rooted!(in(raw_cx) let global_rooted = global);

    // Create Window constructor
    rooted!(in(raw_cx) let window_constructor = JS_NewPlainObject(raw_cx));
    if window_constructor.get().is_null() {
        return Err("Failed to create Window constructor".to_string());
    }

    rooted!(in(raw_cx) let window_constructor_val = ObjectValue(window_constructor.get()));
    let name = std::ffi::CString::new("Window").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        window_constructor_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // window, self, top, parent, globalThis, frames all point to global
    // FIXME: `frames` should be a proper WindowProxy collection that allows indexed access to child iframes
    for name in &["window", "self", "top", "parent", "globalThis", "frames"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            raw_cx,
            global_rooted.handle().into(),
            cname.as_ptr(),
            global_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Define window functions on global
    define_function(raw_cx, global, "alert", Some(window_alert), 1)?;
    define_function(raw_cx, global, "confirm", Some(window_confirm), 1)?;
    define_function(raw_cx, global, "prompt", Some(window_prompt), 2)?;
    define_function(raw_cx, global, "requestAnimationFrame", Some(window_request_animation_frame), 1)?;
    define_function(raw_cx, global, "cancelAnimationFrame", Some(window_cancel_animation_frame), 1)?;
    define_function(raw_cx, global, "getComputedStyle", Some(window_get_computed_style), 1)?;
    define_function(raw_cx, global, "addEventListener", Some(window_add_event_listener), 3)?;
    define_function(raw_cx, global, "removeEventListener", Some(window_remove_event_listener), 3)?;
    define_function(raw_cx, global, "scrollTo", Some(window_scroll_to), 2)?;
    define_function(raw_cx, global, "scrollBy", Some(window_scroll_by), 2)?;
    define_function(raw_cx, global, "__evaluateMediaQuery", Some(window_evaluate_media_query), 1)?;

    // Set window dimension properties
    set_int_property(raw_cx, global, "innerWidth", get_window_width())?;
    set_int_property(raw_cx, global, "innerHeight", get_window_height())?;
    set_int_property(raw_cx, global, "outerWidth", 1920)?;
    set_int_property(raw_cx, global, "outerHeight", 1080)?;
    set_int_property(raw_cx, global, "screenX", 0)?;
    set_int_property(raw_cx, global, "screenY", 0)?;
    set_int_property(raw_cx, global, "scrollX", get_scroll_x())?;
    set_int_property(raw_cx, global, "scrollY", get_scroll_y())?;
    set_int_property(raw_cx, global, "pageXOffset", get_scroll_x())?;
    set_int_property(raw_cx, global, "pageYOffset", get_scroll_y())?;
    set_int_property(raw_cx, global, "devicePixelRatio", 1)?;

    Ok(())
}

fn get_window_width() -> i32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport.window_size.0 as i32;
        }
        1920
    })
}

fn get_window_height() -> i32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport.window_size.1 as i32;
        }
        1080
    })
}

fn get_scroll_x() -> i32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport_scroll.x as i32;
        }
        0
    })
}

fn get_scroll_y() -> i32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport_scroll.y as i32;
        }
        0
    })
}

fn get_device_pixel_ratio() -> f32 {
    DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = unsafe { &**dom };
            return dom.viewport.scale() as f32;
        }
        1.0
    })
}

/// Set up the navigator object
// TODO: Many navigator properties are hardcoded (language, platform) - should detect from system
unsafe fn setup_navigator(
    raw_cx: *mut JSContext,
    global: *mut JSObject,
    user_agent: &str,
) -> Result<(), String> {
    rooted!(in(raw_cx) let navigator = JS_NewPlainObject(raw_cx));
    if navigator.get().is_null() {
        return Err("Failed to create navigator object".to_string());
    }

    set_string_property(raw_cx, navigator.get(), "userAgent", user_agent)?;
    set_string_property(raw_cx, navigator.get(), "language", "en-US")?;
    set_string_property(raw_cx, navigator.get(), "platform", std::env::consts::OS)?;
    set_string_property(raw_cx, navigator.get(), "appName", "Stokes Browser")?;
    set_string_property(raw_cx, navigator.get(), "appVersion", "1.0")?;
    set_string_property(raw_cx, navigator.get(), "vendor", "Stokes")?;
    set_bool_property(raw_cx, navigator.get(), "onLine", true)?;
    set_bool_property(raw_cx, navigator.get(), "cookieEnabled", true)?;

    // Set navigator on global
    rooted!(in(raw_cx) let navigator_val = ObjectValue(navigator.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("navigator").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        navigator_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up the location object
// FIXME: Location properties are hardcoded to "about:blank" - should reflect actual page URL
unsafe fn setup_location(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let location = JS_NewPlainObject(raw_cx));
    if location.get().is_null() {
        return Err("Failed to create location object".to_string());
    }

    let (href, protocol, host, hostname, port, pathname, search, hash, origin) = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = dom_ref.borrow().as_ref() {
            let dom = unsafe { &**dom_ptr };
            let url: url::Url = (&dom.url).into();
            let hostname = url.host_str().unwrap_or("").to_string();
            let port = url.port().map(|p| p.to_string()).unwrap_or_default();
            let host = if port.is_empty() {
                hostname.clone()
            } else {
                format!("{}:{}", hostname, port)
            };
            let search = url
                .query()
                .map(|query| format!("?{}", query))
                .unwrap_or_default();
            let hash = url
                .fragment()
                .map(|fragment| format!("#{}", fragment))
                .unwrap_or_default();

            (
                url.as_str().to_string(),
                format!("{}:", url.scheme()),
                host,
                hostname,
                port,
                url.path().to_string(),
                search,
                hash,
                url.origin().ascii_serialization(),
            )
        } else {
            (
                "about:blank".to_string(),
                "about:".to_string(),
                String::new(),
                String::new(),
                String::new(),
                "blank".to_string(),
                String::new(),
                String::new(),
                "null".to_string(),
            )
        }
    });

    set_string_property(raw_cx, location.get(), "href", &href)?;
    set_string_property(raw_cx, location.get(), "protocol", &protocol)?;
    set_string_property(raw_cx, location.get(), "host", &host)?;
    set_string_property(raw_cx, location.get(), "hostname", &hostname)?;
    set_string_property(raw_cx, location.get(), "port", &port)?;
    set_string_property(raw_cx, location.get(), "pathname", &pathname)?;
    set_string_property(raw_cx, location.get(), "search", &search)?;
    set_string_property(raw_cx, location.get(), "hash", &hash)?;
    set_string_property(raw_cx, location.get(), "origin", &origin)?;

    define_function(raw_cx, location.get(), "reload", Some(location_reload), 0)?;
    define_function(raw_cx, location.get(), "assign", Some(location_assign), 1)?;
    define_function(raw_cx, location.get(), "replace", Some(location_replace), 1)?;
    define_function(raw_cx, location.get(), "toString", Some(location_to_string), 0)?;

    // Set location on global
    rooted!(in(raw_cx) let location_val = ObjectValue(location.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("location").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        location_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up localStorage and sessionStorage
// TODO: Storage length property is set to 0 and not dynamically updated when items are added/removed
unsafe fn setup_storage(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let global_rooted = global);

    // Create localStorage object
    rooted!(in(raw_cx) let local_storage = JS_NewPlainObject(raw_cx));
    if local_storage.get().is_null() {
        return Err("Failed to create localStorage object".to_string());
    }

    define_function(raw_cx, local_storage.get(), "getItem", Some(local_storage_get_item), 1)?;
    define_function(raw_cx, local_storage.get(), "setItem", Some(local_storage_set_item), 2)?;
    define_function(raw_cx, local_storage.get(), "removeItem", Some(local_storage_remove_item), 1)?;
    define_function(raw_cx, local_storage.get(), "clear", Some(local_storage_clear), 0)?;
    define_function(raw_cx, local_storage.get(), "key", Some(local_storage_key), 1)?;
    set_int_property(raw_cx, local_storage.get(), "length", 0)?;

    rooted!(in(raw_cx) let local_storage_val = ObjectValue(local_storage.get()));
    let name = std::ffi::CString::new("localStorage").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        local_storage_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Create sessionStorage object
    rooted!(in(raw_cx) let session_storage = JS_NewPlainObject(raw_cx));
    if session_storage.get().is_null() {
        return Err("Failed to create sessionStorage object".to_string());
    }

    define_function(raw_cx, session_storage.get(), "getItem", Some(session_storage_get_item), 1)?;
    define_function(raw_cx, session_storage.get(), "setItem", Some(session_storage_set_item), 2)?;
    define_function(raw_cx, session_storage.get(), "removeItem", Some(session_storage_remove_item), 1)?;
    define_function(raw_cx, session_storage.get(), "clear", Some(session_storage_clear), 0)?;
    define_function(raw_cx, session_storage.get(), "key", Some(session_storage_key), 1)?;
    set_int_property(raw_cx, session_storage.get(), "length", 0)?;

    rooted!(in(raw_cx) let session_storage_val = ObjectValue(session_storage.get()));
    let name = std::ffi::CString::new("sessionStorage").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        session_storage_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up Node constructor with node type constants
unsafe fn setup_node_constructor(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let node = JS_NewPlainObject(raw_cx));
    if node.get().is_null() {
        return Err("Failed to create Node constructor".to_string());
    }

    set_int_property(raw_cx, node.get(), "ELEMENT_NODE", 1)?;
    set_int_property(raw_cx, node.get(), "ATTRIBUTE_NODE", 2)?;
    set_int_property(raw_cx, node.get(), "TEXT_NODE", 3)?;
    set_int_property(raw_cx, node.get(), "CDATA_SECTION_NODE", 4)?;
    set_int_property(raw_cx, node.get(), "ENTITY_REFERENCE_NODE", 5)?;
    set_int_property(raw_cx, node.get(), "ENTITY_NODE", 6)?;
    set_int_property(raw_cx, node.get(), "PROCESSING_INSTRUCTION_NODE", 7)?;
    set_int_property(raw_cx, node.get(), "COMMENT_NODE", 8)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_NODE", 9)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_TYPE_NODE", 10)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_FRAGMENT_NODE", 11)?;
    set_int_property(raw_cx, node.get(), "NOTATION_NODE", 12)?;

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

/// Set up Element and HTMLElement constructors
unsafe fn setup_element_constructors(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let element = JS_NewPlainObject(raw_cx));
    if element.get().is_null() {
        return Err("Failed to create Element constructor".to_string());
    }
    set_int_property(raw_cx, element.get(), "ELEMENT_NODE", 1)?;

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

    // HTMLElement constructor (alias for now)
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

/// Set up HTMLFormElement constructor
unsafe fn setup_html_form_element_constructor(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let html_form_element = JS_NewPlainObject(raw_cx));
    if html_form_element.get().is_null() {
        return Err("Failed to create HTMLFormElement constructor".to_string());
    }

    rooted!(in(raw_cx) let html_form_element_val = ObjectValue(html_form_element.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("HTMLFormElement").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        html_form_element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up HTMLIFrameElement constructor with prototype
unsafe fn setup_html_iframe_element_constructor(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    use crate::js::helpers::define_property_accessor;

    // Create HTMLIFrameElement constructor
    rooted!(in(raw_cx) let html_iframe_element = JS_NewPlainObject(raw_cx));
    if html_iframe_element.get().is_null() {
        return Err("Failed to create HTMLIFrameElement constructor".to_string());
    }

    // Create prototype object
    rooted!(in(raw_cx) let prototype = JS_NewPlainObject(raw_cx));
    if prototype.get().is_null() {
        return Err("Failed to create HTMLIFrameElement prototype".to_string());
    }

    // Define getter/setter functions for contentWindow property
    define_function(raw_cx, prototype.get(), "__getContentWindow", Some(html_iframe_element_get_content_window), 0)?;
    define_function(raw_cx, prototype.get(), "__setContentWindow", Some(html_iframe_element_set_content_window), 1)?;

    // Define getter/setter functions for contentDocument property
    define_function(raw_cx, prototype.get(), "__getContentDocument", Some(html_iframe_element_get_content_document), 0)?;
    define_function(raw_cx, prototype.get(), "__setContentDocument", Some(html_iframe_element_set_content_document), 1)?;

    // Define getter/setter functions for src property
    define_function(raw_cx, prototype.get(), "__getSrc", Some(html_iframe_element_get_src), 0)?;
    define_function(raw_cx, prototype.get(), "__setSrc", Some(html_iframe_element_set_src), 1)?;

    // Define contentWindow as property with getter/setter on prototype
    define_property_accessor(raw_cx, prototype.get(), "contentWindow", "__getContentWindow", "__setContentWindow")?;

    // Define contentDocument as property with getter/setter on prototype
    define_property_accessor(raw_cx, prototype.get(), "contentDocument", "__getContentDocument", "__setContentDocument")?;

    // Define src as property with getter/setter on prototype
    define_property_accessor(raw_cx, prototype.get(), "src", "__getSrc", "__setSrc")?;

    // Set prototype on constructor
    rooted!(in(raw_cx) let prototype_val = ObjectValue(prototype.get()));
    rooted!(in(raw_cx) let html_iframe_element_rooted = html_iframe_element.get());
    let prototype_name = std::ffi::CString::new("prototype").unwrap();
    JS_DefineProperty(
        raw_cx,
        html_iframe_element_rooted.handle().into(),
        prototype_name.as_ptr(),
        prototype_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Add constructor to global
    rooted!(in(raw_cx) let html_iframe_element_val = ObjectValue(html_iframe_element.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("HTMLIFrameElement").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        html_iframe_element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up Event and CustomEvent constructors
unsafe fn setup_event_constructors(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let event = JS_NewPlainObject(raw_cx));
    if event.get().is_null() {
        return Err("Failed to create Event constructor".to_string());
    }

    rooted!(in(raw_cx) let event_val = ObjectValue(event.get()));
    rooted!(in(raw_cx) let global_rooted = global);

    let name = std::ffi::CString::new("Event").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        event_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    let name = std::ffi::CString::new("CustomEvent").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        event_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up XMLHttpRequest constructor
unsafe fn setup_xhr_constructor(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let xhr = JS_NewPlainObject(raw_cx));
    if xhr.get().is_null() {
        return Err("Failed to create XMLHttpRequest constructor".to_string());
    }

    rooted!(in(raw_cx) let xhr_val = ObjectValue(xhr.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("XMLHttpRequest").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        xhr_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up atob/btoa functions
unsafe fn setup_base64_functions(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    define_function(raw_cx, global, "atob", Some(window_atob), 1)?;
    define_function(raw_cx, global, "btoa", Some(window_btoa), 1)?;
    Ok(())
}

/// Set up dataLayer for Google Analytics compatibility
unsafe fn setup_data_layer(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let data_layer = create_empty_array(raw_cx));
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

// ============================================================================
// Document methods
// ============================================================================

/// document.cookie getter implementation
unsafe extern "C" fn document_get_cookie(raw_cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, 0);

    ensure_cookie_jar_initialized();

    let cookie_string = DOCUMENT_URL.with(|doc_url| {
        let url_opt = doc_url.borrow();
        if let Some(ref url) = *url_opt {
            let domain = url.host_str().unwrap_or("localhost");
            let path = url.path();

            COOKIE_JAR.with(|jar| jar.borrow_mut().get_cookie_string(domain, path))
        } else {
            String::new()
        }
    });

    args.rval().set(create_js_string(raw_cx, &cookie_string));
    true
}

/// document.cookie setter implementation
unsafe extern "C" fn document_set_cookie(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let cookie_str = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        args.rval().set(UndefinedValue());
        return true;
    };

    println!("[JS] document.cookie = '{}' (setting cookie)", cookie_str);

    ensure_cookie_jar_initialized();

    DOCUMENT_URL.with(|doc_url| {
        let url_opt = doc_url.borrow();
        if let Some(ref url) = *url_opt {
            let domain = url.host_str().unwrap_or("localhost");
            let path = url.path();

            if let Some(cookie) = Cookie::parse(&cookie_str, domain, path) {
                COOKIE_JAR.with(|jar| {
                    jar.borrow_mut().set_cookie(cookie);
                });
            } else {
                println!("[JS] Warning: Failed to parse cookie: {}", cookie_str);
            }
        }
    });

    args.rval().set(UndefinedValue());
    true
}

/// document.head getter implementation
unsafe extern "C" fn document_get_head(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] document.head called");

    let head_element = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            // Search through all nodes to find the head element
            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    let tag_name = elem_data.name.local.to_string().to_lowercase();
                    if tag_name == "head" {
                        return Some((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                    }
                }
            }
        }
        None
    });

    if let Some((node_id, tag_name, attributes)) = head_element {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, node_id, &tag_name, &attributes) {
            args.rval().set(js_elem);
        } else {
            args.rval().set(mozjs::jsval::NullValue());
        }
    } else {
        println!("[JS] head element not found");
        args.rval().set(mozjs::jsval::NullValue());
    }

    true
}

/// document.body getter implementation
unsafe extern "C" fn document_get_body(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let body_element = DOM_REF.with(|dom_ref| {
        let dom_ptr = (*dom_ref.borrow())?;
        let dom = unsafe { &*dom_ptr };
        let node_id = dom.body_id()?;
        let node = dom.get_node(node_id)?;
        let elem_data = node.element_data()?;
        Some((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()))
    });

    if let Some((node_id, tag_name, attributes)) = body_element {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, node_id, &tag_name, &attributes) {
            args.rval().set(js_elem);
            return true;
        }
    }

    args.rval().set(NullValue());
    true
}

/// document.body setter implementation
unsafe extern "C" fn document_set_body(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    if argc == 0 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let value = *args.get(0);
    if value.is_null() || value.is_undefined() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let Some(new_body_id) = get_node_id_from_value(raw_cx, value) else {
        args.rval().set(UndefinedValue());
        return true;
    };

    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = unsafe { &mut *dom_ptr };
            let _ = dom.set_document_body(new_body_id);
        }
    });

    args.rval().set(UndefinedValue());
    true
}

/// document.getElementById implementation
unsafe extern "C" fn document_get_element_by_id(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let id = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    if id.is_empty() {
        args.rval().set(mozjs::jsval::NullValue());
        return true;
    }

    println!("[JS] document.getElementById('{}') called", id);

    let element_data = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            if let Some(&node_id) = dom.nodes_to_id.get(&id) {
                if let Some(node) = dom.get_node(node_id) {
                    if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                        let tag_name = elem_data.name.local.to_string();
                        let attributes = elem_data.attributes.clone();
                        return Some((node_id, tag_name, attributes));
                    }
                }
            }
        }
        None
    });

    if let Some((node_id, tag_name, attributes)) = element_data {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, node_id, &tag_name, &attributes) {
            args.rval().set(js_elem);
        } else {
            args.rval().set(mozjs::jsval::NullValue());
        }
    } else {
        println!("[JS] Element with id '{}' not found", id);
        args.rval().set(mozjs::jsval::NullValue());
    }

    true
}

/// document.getElementsByTagName implementation
unsafe extern "C" fn document_get_elements_by_tag_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let tag_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.getElementsByTagName('{}') called", tag_name);

    let matching_elements: Vec<(usize, String, AttributeMap)> = DOM_REF.with(|dom_ref| {
        let mut results = Vec::new();
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            let tag_name_lower = tag_name.to_lowercase();

            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    let node_tag = elem_data.name.local.to_string().to_lowercase();
                    if tag_name_lower == "*" || node_tag == tag_name_lower {
                        results.push((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                    }
                }
            }
        }
        results
    });

    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));

    for (index, (node_id, tag, attrs)) in matching_elements.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, *node_id, tag, attrs) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            mozjs::rust::wrappers::JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.getElementsByClassName implementation
unsafe extern "C" fn document_get_elements_by_class_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.getElementsByClassName('{}') called", class_name);

    let search_classes: Vec<&str> = class_name.split_whitespace().collect();

    let matching_elements: Vec<(usize, String, AttributeMap)> = DOM_REF.with(|dom_ref| {
        let mut results = Vec::new();
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;

            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    if let Some(class_attr) = elem_data.attributes.iter().find(|attr| attr.name.local.as_ref() == "class") {
                        let element_classes: Vec<&str> = class_attr.value.split_whitespace().collect();
                        if search_classes.iter().all(|sc| element_classes.contains(sc)) {
                            results.push((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                        }
                    }
                }
            }
        }
        results
    });

    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));

    for (index, (node_id, tag, attrs)) in matching_elements.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, *node_id, tag, attrs) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            mozjs::rust::wrappers::JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.querySelector implementation
unsafe extern "C" fn document_query_selector(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.querySelector('{}') called", selector);

    let element_data = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    if matches_selector(&selector, &elem_data.name.local.to_string(), &elem_data.attributes) {
                        return Some((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                    }
                }
            }
        }
        None
    });

    if let Some((node_id, tag_name, attributes)) = element_data {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, node_id, &tag_name, &attributes) {
            args.rval().set(js_elem);
        } else {
            args.rval().set(mozjs::jsval::NullValue());
        }
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// document.querySelectorAll implementation
unsafe extern "C" fn document_query_selector_all(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.querySelectorAll('{}') called", selector);

    let matching_elements: Vec<(usize, String, AttributeMap)> = DOM_REF.with(|dom_ref| {
        let mut results = Vec::new();
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    if matches_selector(&selector, &elem_data.name.local.to_string(), &elem_data.attributes) {
                        results.push((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                    }
                }
            }
        }
        results
    });

    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));

    for (index, (node_id, tag, attrs)) in matching_elements.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, *node_id, tag, attrs) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            mozjs::rust::wrappers::JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.createElement implementation
unsafe extern "C" fn document_create_element(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let tag_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    if tag_name.is_empty() {
        args.rval().set(mozjs::jsval::NullValue());
        return true;
    }

    println!("[JS] document.createElement('{}') called", tag_name);

    DOM_REF.with(|dom| {
        if let Some(dom_ptr) = *dom.borrow() {
            let dom = unsafe { &mut *dom_ptr };
            // HTML documents should create elements in the HTML namespace.
            let local = markup5ever::LocalName::from(tag_name.to_lowercase());
            let node_id = dom.create_element(QualName::new(None, ns!(html), local), AttributeMap::empty());
            if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, node_id, &tag_name, dom.nodes[node_id].attrs().unwrap()) {
                args.rval().set(js_elem);
                println!("Successfully created element '{}'", tag_name);
                return;
            }
        }
    });
    true
}

/// document.createTextNode implementation
unsafe extern "C" fn document_create_text_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let text = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.createTextNode('{}') called", text);

    rooted!(in(raw_cx) let text_node = JS_NewPlainObject(raw_cx));
    if !text_node.get().is_null() {
        let _ = set_int_property(raw_cx, text_node.get(), "nodeType", 3);
        let _ = set_string_property(raw_cx, text_node.get(), "nodeName", "#text");
        let _ = set_string_property(raw_cx, text_node.get(), "textContent", &text);
        let _ = set_string_property(raw_cx, text_node.get(), "nodeValue", &text);
        args.rval().set(ObjectValue(text_node.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// document.createDocumentFragment implementation
unsafe extern "C" fn document_create_document_fragment(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] document.createDocumentFragment() called");

    rooted!(in(raw_cx) let fragment = JS_NewPlainObject(raw_cx));
    if !fragment.get().is_null() {
        let _ = set_int_property(raw_cx, fragment.get(), "nodeType", 11);
        let _ = set_string_property(raw_cx, fragment.get(), "nodeName", "#document-fragment");
        let _ = define_function(raw_cx, fragment.get(), "appendChild", Some(element_append_child), 1);
        let _ = define_function(raw_cx, fragment.get(), "querySelector", Some(document_query_selector), 1);
        let _ = define_function(raw_cx, fragment.get(), "querySelectorAll", Some(document_query_selector_all), 1);
        args.rval().set(ObjectValue(fragment.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

// ============================================================================
// Window methods
// ============================================================================

/// window.alert implementation
unsafe extern "C" fn window_alert(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    super::alert_callback::trigger_alert(message);
    args.rval().set(UndefinedValue());
    true
}

/// window.confirm implementation
unsafe extern "C" fn window_confirm(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.confirm('{}') called - returning false", message);
    args.rval().set(BooleanValue(false));
    true
}

/// window.prompt implementation
unsafe extern "C" fn window_prompt(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.prompt('{}') called - returning null", message);
    args.rval().set(mozjs::jsval::NullValue());
    true
}

/// window.requestAnimationFrame implementation
unsafe extern "C" fn window_request_animation_frame(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] requestAnimationFrame called");
    args.rval().set(Int32Value(1));
    true
}

/// window.cancelAnimationFrame implementation
unsafe extern "C" fn window_cancel_animation_frame(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] cancelAnimationFrame called");
    args.rval().set(UndefinedValue());
    true
}

/// window.getComputedStyle implementation
unsafe extern "C" fn window_get_computed_style(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] getComputedStyle called");

    rooted!(in(raw_cx) let style = JS_NewPlainObject(raw_cx));
    if !style.get().is_null() {
        let _ = define_function(raw_cx, style.get(), "getPropertyValue", Some(style_get_property_value), 1);
        args.rval().set(ObjectValue(style.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// window.addEventListener implementation
unsafe extern "C" fn window_add_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    if event_type.is_empty() || argc < 2 || !args.get(1).is_object() || args.get(1).is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let use_capture = if argc > 2 {
        let options = *args.get(2);
        options.is_boolean() && options.to_boolean()
    } else {
        false
    };

    let callback = args.get(1).to_object();
    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = unsafe { &mut *dom_ptr };
            let root_id = dom.root_element().id;
            if let Some(node) = dom.get_node_mut(root_id) {
                node.event_listeners
                    .add_listener_by_name(&event_type, callback, use_capture);
            }
        }
    });

    args.rval().set(UndefinedValue());
    true
}

/// window.removeEventListener implementation
unsafe extern "C" fn window_remove_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    if event_type.is_empty() || argc < 2 || !args.get(1).is_object() || args.get(1).is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let use_capture = if argc > 2 {
        let options = *args.get(2);
        options.is_boolean() && options.to_boolean()
    } else {
        false
    };

    let callback = args.get(1).to_object();
    DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = *dom_ref.borrow() {
            let dom = unsafe { &mut *dom_ptr };
            let root_id = dom.root_element().id;
            if let Some(node) = dom.get_node_mut(root_id) {
                node.event_listeners
                    .remove_listener_by_callback(&event_type, callback, use_capture);
            }
        }
    });

    args.rval().set(UndefinedValue());
    true
}

/// window.scrollTo implementation
unsafe extern "C" fn window_scroll_to(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] window.scrollTo called");
    args.rval().set(UndefinedValue());
    true
}

/// window.scrollBy implementation
unsafe extern "C" fn window_scroll_by(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] window.scrollBy called");
    args.rval().set(UndefinedValue());
    true
}

/// window.atob implementation (base64 decode)
unsafe extern "C" fn window_atob(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let encoded = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    use base64::Engine;
    match base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) {
        Ok(decoded) => {
            if let Ok(s) = String::from_utf8(decoded) {
                args.rval().set(create_js_string(raw_cx, &s));
            } else {
                args.rval().set(create_js_string(raw_cx, ""));
            }
        }
        Err(_) => {
            args.rval().set(create_js_string(raw_cx, ""));
        }
    }
    true
}

/// window.btoa implementation (base64 encode)
unsafe extern "C" fn window_btoa(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let data = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
    args.rval().set(create_js_string(raw_cx, &encoded));
    true
}

/// Internal media query evaluator used by window.matchMedia.
unsafe extern "C" fn window_evaluate_media_query(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let query = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    let width = get_window_width() as f32;
    let height = get_window_height() as f32;
    let dpr = get_device_pixel_ratio();

    let matches = evaluate_media_query(&query, width, height, dpr);
    args.rval().set(BooleanValue(matches));
    true
}

fn evaluate_media_query(query: &str, width: f32, height: f32, dpr: f32) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return false;
    }

    split_media_query_list(trimmed)
        .into_iter()
        .any(|part| evaluate_single_media_condition(part, width, height, dpr))
}

fn split_media_query_list(query: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;

    for (idx, ch) in query.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let part = query[start..idx].trim();
                if !part.is_empty() {
                    out.push(part);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }

    let tail = query[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }

    out
}

fn evaluate_single_media_condition(input: &str, width: f32, height: f32, dpr: f32) -> bool {
    let mut remaining = input.trim().to_ascii_lowercase();
    if remaining.is_empty() {
        return false;
    }

    let mut invert = false;
    if let Some(rest) = consume_keyword(&remaining, "not") {
        invert = true;
        remaining = rest.to_string();
    } else if let Some(rest) = consume_keyword(&remaining, "only") {
        remaining = rest.to_string();
    }

    if remaining.is_empty() {
        return false;
    }

    let mut media_type_matches = true;
    if !remaining.starts_with('(') {
        let mut split = remaining.splitn(2, char::is_whitespace);
        let media_type = split.next().unwrap_or_default();
        let rest = split.next().unwrap_or_default().trim_start();

        media_type_matches = match media_type {
            "all" | "screen" => true,
            "print" => false,
            _ => false,
        };

        remaining = rest.to_string();
    }

    let mut all_features_match = true;
    while !remaining.trim_start().is_empty() {
        remaining = remaining.trim_start().to_string();
        if let Some(rest) = consume_keyword(&remaining, "and") {
            remaining = rest.to_string();
        }

        remaining = remaining.trim_start().to_string();
        if !remaining.starts_with('(') {
            return false;
        }

        let closing = match find_matching_paren(&remaining) {
            Some(i) => i,
            None => return false,
        };

        let feature = &remaining[1..closing];
        let matches = evaluate_media_feature(feature.trim(), width, height, dpr);
        all_features_match &= matches;

        remaining = remaining[closing + 1..].to_string();
    }

    let result = media_type_matches && all_features_match;
    if invert { !result } else { result }
}

fn consume_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    if !input.starts_with(keyword) {
        return None;
    }

    let remainder = &input[keyword.len()..];
    let starts_with_ws = remainder.chars().next().map(|c| c.is_whitespace()).unwrap_or(false);
    if remainder.is_empty() || starts_with_ws || remainder.starts_with('(') {
        Some(remainder.trim_start())
    } else {
        None
    }
}

fn find_matching_paren(input: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn evaluate_media_feature(feature: &str, width: f32, height: f32, dpr: f32) -> bool {
    if feature.is_empty() {
        return false;
    }

    let mut parts = feature.splitn(2, ':');
    let name = parts.next().unwrap_or_default().trim();
    let value = parts.next().map(str::trim);

    match (name, value) {
        ("width", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| approx_eq(width, px)),
        ("min-width", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| width >= px),
        ("max-width", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| width <= px),
        ("height", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| approx_eq(height, px)),
        ("min-height", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| height >= px),
        ("max-height", Some(v)) => parse_length_px(v, width, height).is_some_and(|px| height <= px),
        ("orientation", Some(v)) => {
            let orientation = if width >= height { "landscape" } else { "portrait" };
            orientation == v
        }
        ("prefers-color-scheme", Some(v)) => v == "light",
        ("prefers-reduced-motion", Some(v)) => v == "no-preference",
        ("resolution", Some(v)) => parse_resolution_dppx(v).is_some_and(|target| approx_eq(dpr, target)),
        ("min-resolution", Some(v)) => parse_resolution_dppx(v).is_some_and(|target| dpr >= target),
        ("max-resolution", Some(v)) => parse_resolution_dppx(v).is_some_and(|target| dpr <= target),
        ("color", None) | ("monochrome", None) => true,
        _ => false,
    }
}

fn parse_length_px(value: &str, width: f32, height: f32) -> Option<f32> {
    let s = value.trim().to_ascii_lowercase();
    let (num, unit) = split_number_and_unit(&s)?;

    let parsed = num.parse::<f32>().ok()?;
    let px = match unit {
        "" | "px" => parsed,
        "em" | "rem" => parsed * 16.0,
        "vw" => (parsed / 100.0) * width,
        "vh" => (parsed / 100.0) * height,
        _ => return None,
    };

    Some(px)
}

fn parse_resolution_dppx(value: &str) -> Option<f32> {
    let s = value.trim().to_ascii_lowercase();
    let (num, unit) = split_number_and_unit(&s)?;
    let parsed = num.parse::<f32>().ok()?;

    match unit {
        "dppx" | "x" => Some(parsed),
        "dpi" => Some(parsed / 96.0),
        "dpcm" => Some(parsed * 2.54 / 96.0),
        _ => None,
    }
}

fn split_number_and_unit(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut split_idx = 0usize;
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+' {
            split_idx = idx + ch.len_utf8();
            continue;
        }
        split_idx = idx;
        break;
    }

    if split_idx == 0 {
        return None;
    }

    let (num, unit) = trimmed.split_at(split_idx);
    Some((num.trim(), unit.trim()))
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

// ============================================================================
// Location methods
// ============================================================================

/// location.reload implementation
unsafe extern "C" fn location_reload(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] location.reload() called");
    args.rval().set(UndefinedValue());
    true
}

/// location.assign implementation
unsafe extern "C" fn location_assign(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let url = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] location.assign('{}') called", url);
    args.rval().set(UndefinedValue());
    true
}

/// location.replace implementation
unsafe extern "C" fn location_replace(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let url = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] location.replace('{}') called", url);
    args.rval().set(UndefinedValue());
    true
}

/// location.toString implementation
unsafe extern "C" fn location_to_string(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let href = DOM_REF.with(|dom_ref| {
        if let Some(dom_ptr) = dom_ref.borrow().as_ref() {
            let dom = unsafe { &**dom_ptr };
            let url: url::Url = (&dom.url).into();
            url.as_str().to_string()
        } else {
            "about:blank".to_string()
        }
    });

    args.rval().set(create_js_string(raw_cx, &href));
    true
}

// ============================================================================
// Storage methods
// ============================================================================

/// localStorage.getItem implementation
unsafe extern "C" fn local_storage_get_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    let value = LOCAL_STORAGE.with(|storage| storage.borrow().get(&key).cloned());

    if let Some(val) = value {
        args.rval().set(create_js_string(raw_cx, &val));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// localStorage.setItem implementation
unsafe extern "C" fn local_storage_set_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().insert(key, value);
    });

    args.rval().set(UndefinedValue());
    true
}

/// localStorage.removeItem implementation
unsafe extern "C" fn local_storage_remove_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().remove(&key);
    });

    args.rval().set(UndefinedValue());
    true
}

/// localStorage.clear implementation
unsafe extern "C" fn local_storage_clear(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().clear();
    });

    args.rval().set(UndefinedValue());
    true
}

/// localStorage.key implementation
unsafe extern "C" fn local_storage_key(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let index = if argc > 0 {
        let val = *args.get(0);
        if val.is_int32() {
            val.to_int32() as usize
        } else if val.is_double() {
            val.to_double() as usize
        } else {
            0
        }
    } else {
        0
    };

    let key = LOCAL_STORAGE.with(|storage| {
        let storage = storage.borrow();
        storage.keys().nth(index).cloned()
    });

    if let Some(k) = key {
        args.rval().set(create_js_string(raw_cx, &k));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// sessionStorage.getItem implementation
unsafe extern "C" fn session_storage_get_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    let value = SESSION_STORAGE.with(|storage| storage.borrow().get(&key).cloned());

    if let Some(val) = value {
        args.rval().set(create_js_string(raw_cx, &val));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// sessionStorage.setItem implementation
unsafe extern "C" fn session_storage_set_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().insert(key, value);
    });

    args.rval().set(UndefinedValue());
    true
}

/// sessionStorage.removeItem implementation
unsafe extern "C" fn session_storage_remove_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().remove(&key);
    });

    args.rval().set(UndefinedValue());
    true
}

/// sessionStorage.clear implementation
unsafe extern "C" fn session_storage_clear(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().clear();
    });

    args.rval().set(UndefinedValue());
    true
}

/// sessionStorage.key implementation
unsafe extern "C" fn session_storage_key(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let index = if argc > 0 {
        let val = *args.get(0);
        if val.is_int32() {
            val.to_int32() as usize
        } else if val.is_double() {
            val.to_double() as usize
        } else {
            0
        }
    } else {
        0
    };

    let key = SESSION_STORAGE.with(|storage| {
        let storage = storage.borrow();
        storage.keys().nth(index).cloned()
    });

    if let Some(k) = key {
        args.rval().set(create_js_string(raw_cx, &k));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// Element methods (shared)
/// style.getPropertyValue implementation
unsafe extern "C" fn style_get_property_value(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let property = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] style.getPropertyValue('{}') called", property);
    args.rval().set(create_js_string(raw_cx, ""));
    true
}

/// HTMLIFrameElement contentWindow getter
// TODO
unsafe extern "C" fn html_iframe_element_get_content_window(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] HTMLIFrameElement.contentWindow getter called");

    // For now, return null as iframe content window is not implemented
    // In a full implementation, this would return the Window object of the iframe's document
    args.rval().set(mozjs::jsval::NullValue());
    true
}

/// HTMLIFrameElement contentWindow setter
// TODO
unsafe extern "C" fn html_iframe_element_set_content_window(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] HTMLIFrameElement.contentWindow setter called");

    // contentWindow is read-only in the spec, but we provide a setter to avoid errors
    args.rval().set(UndefinedValue());
    true
}

/// HTMLIFrameElement contentDocument getter
// TODO
unsafe extern "C" fn html_iframe_element_get_content_document(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] HTMLIFrameElement.contentDocument getter called");

    // For now, return null as iframe content document is not implemented
    // In a full implementation, this would return the Document object of the iframe
    args.rval().set(mozjs::jsval::NullValue());
    true
}

/// HTMLIFrameElement contentDocument setter
// TODO
unsafe extern "C" fn html_iframe_element_set_content_document(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] HTMLIFrameElement.contentDocument setter called");

    // contentDocument is read-only in the spec, but we provide a setter to avoid errors
    args.rval().set(UndefinedValue());
    true
}

/// HTMLIFrameElement src getter
// TODO
unsafe extern "C" fn html_iframe_element_get_src(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] HTMLIFrameElement.src getter called");

    // For now, return empty string
    // In a full implementation, this would get the src attribute from the element
    args.rval().set(create_js_string(raw_cx, ""));
    true
}

/// HTMLIFrameElement src setter
// TODO
unsafe extern "C" fn html_iframe_element_set_src(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let src = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] HTMLIFrameElement.src setter called with value: {}", src);

    // In a full implementation, this would set the src attribute and potentially load the iframe
    args.rval().set(UndefinedValue());
    true
}

#[cfg(test)]
mod tests {
    use super::evaluate_media_query;

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
}
