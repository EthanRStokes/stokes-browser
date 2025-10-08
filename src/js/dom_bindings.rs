// DOM bindings for JavaScript using V8
use std::rc::Rc;
use std::cell::RefCell;
use crate::dom::{DomNode, NodeType};
use super::element_bindings::ElementWrapper;

/// Set up DOM bindings in the JavaScript context
pub fn setup_dom_bindings(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    document_root: Rc<RefCell<DomNode>>,
) -> Result<(), String> {
    // Create the document object
    let document_obj = v8::Object::new(scope);
    
    // Store document root in an external reference for use in callbacks
    let root_ptr = Box::into_raw(Box::new(document_root.clone())) as *mut std::ffi::c_void;
    
    // getElementById
    let get_element_by_id = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut retval: v8::ReturnValue| {
            if args.length() < 1 {
                retval.set(v8::null(scope).into());
                return;
            }
            
            let id_arg = args.get(0);
            let id_str = id_arg.to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            
            if id_str.is_empty() {
                retval.set(v8::null(scope).into());
                return;
            }
            
            println!("[JS] document.getElementById('{}') called", id_str);
            
            // For now, return null (proper implementation would search the DOM)
            retval.set(v8::null(scope).into());
        },
    ).unwrap();
    
    // getElementsByTagName
    let get_elements_by_tag_name = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut retval: v8::ReturnValue| {
            let tag_name = if args.length() > 0 {
                args.get(0).to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            
            println!("[JS] document.getElementsByTagName('{}') called", tag_name);
            
            // Return empty array
            let array = v8::Array::new(scope, 0);
            retval.set(array.into());
        },
    ).unwrap();
    
    // querySelector
    let query_selector = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut retval: v8::ReturnValue| {
            let selector = if args.length() > 0 {
                args.get(0).to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            
            println!("[JS] document.querySelector('{}') called", selector);
            retval.set(v8::null(scope).into());
        },
    ).unwrap();
    
    // querySelectorAll
    let query_selector_all = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut retval: v8::ReturnValue| {
            let selector = if args.length() > 0 {
                args.get(0).to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            
            println!("[JS] document.querySelectorAll('{}') called", selector);
            let array = v8::Array::new(scope, 0);
            retval.set(array.into());
        },
    ).unwrap();
    
    // createElement
    let create_element = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         mut retval: v8::ReturnValue| {
            let tag_name = if args.length() > 0 {
                args.get(0).to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            
            println!("[JS] document.createElement('{}') called", tag_name);
            
            // Create a simple element object
            let element = v8::Object::new(scope);
            let tag_key = v8::String::new(scope, "tagName").unwrap();
            let tag_val = v8::String::new(scope, &tag_name.to_uppercase()).unwrap();
            element.set(scope, tag_key.into(), tag_val.into());
            
            retval.set(element.into());
        },
    ).unwrap();
    
    // Add methods to document object
    let name = v8::String::new(scope, "getElementById").unwrap();
    document_obj.set(scope, name.into(), get_element_by_id.into());
    
    let name = v8::String::new(scope, "getElementsByTagName").unwrap();
    document_obj.set(scope, name.into(), get_elements_by_tag_name.into());
    
    let name = v8::String::new(scope, "querySelector").unwrap();
    document_obj.set(scope, name.into(), query_selector.into());
    
    let name = v8::String::new(scope, "querySelectorAll").unwrap();
    document_obj.set(scope, name.into(), query_selector_all.into());
    
    let name = v8::String::new(scope, "createElement").unwrap();
    document_obj.set(scope, name.into(), create_element.into());
    
    // Create documentElement
    let document_element = v8::Object::new(scope);
    let tag_key = v8::String::new(scope, "tagName").unwrap();
    let tag_val = v8::String::new(scope, "HTML").unwrap();
    document_element.set(scope, tag_key.into(), tag_val.into());
    
    let name = v8::String::new(scope, "documentElement").unwrap();
    document_obj.set(scope, name.into(), document_element.into());
    
    // Set document on global
    let document_name = v8::String::new(scope, "document").unwrap();
    global.set(scope, document_name.into(), document_obj.into());
    
    // Setup window object functions
    setup_window_bindings(scope, global)?;
    
    // Setup navigator object
    setup_navigator_bindings(scope, global)?;
    
    // Setup location object
    setup_location_bindings(scope, global)?;
    
    Ok(())
}

fn setup_window_bindings(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
) -> Result<(), String> {
    // alert
    let alert_fn = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         args: v8::FunctionCallbackArguments,
         _retval: v8::ReturnValue| {
            let message = if args.length() > 0 {
                args.get(0).to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            println!("[JS Alert] {}", message);
        },
    ).unwrap();
    
    // setTimeout
    let set_timeout_fn = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         _args: v8::FunctionCallbackArguments,
         mut retval: v8::ReturnValue| {
            println!("[JS] setTimeout called (not fully implemented)");
            retval.set(v8::Integer::new(scope, 1).into());
        },
    ).unwrap();
    
    // setInterval
    let set_interval_fn = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         _args: v8::FunctionCallbackArguments,
         mut retval: v8::ReturnValue| {
            println!("[JS] setInterval called (not fully implemented)");
            retval.set(v8::Integer::new(scope, 1).into());
        },
    ).unwrap();
    
    // clearTimeout
    let clear_timeout_fn = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         _args: v8::FunctionCallbackArguments,
         _retval: v8::ReturnValue| {
            println!("[JS] clearTimeout called");
        },
    ).unwrap();
    
    // clearInterval
    let clear_interval_fn = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         _args: v8::FunctionCallbackArguments,
         _retval: v8::ReturnValue| {
            println!("[JS] clearInterval called");
        },
    ).unwrap();
    
    // requestAnimationFrame
    let request_animation_frame_fn = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         _args: v8::FunctionCallbackArguments,
         mut retval: v8::ReturnValue| {
            println!("[JS] requestAnimationFrame called");
            retval.set(v8::Integer::new(scope, 1).into());
        },
    ).unwrap();
    
    // Add to global
    let name = v8::String::new(scope, "alert").unwrap();
    global.set(scope, name.into(), alert_fn.into());
    
    let name = v8::String::new(scope, "setTimeout").unwrap();
    global.set(scope, name.into(), set_timeout_fn.into());
    
    let name = v8::String::new(scope, "setInterval").unwrap();
    global.set(scope, name.into(), set_interval_fn.into());
    
    let name = v8::String::new(scope, "clearTimeout").unwrap();
    global.set(scope, name.into(), clear_timeout_fn.into());
    
    let name = v8::String::new(scope, "clearInterval").unwrap();
    global.set(scope, name.into(), clear_interval_fn.into());
    
    let name = v8::String::new(scope, "requestAnimationFrame").unwrap();
    global.set(scope, name.into(), request_animation_frame_fn.into());
    
    // Set window references (circular)
    let window_name = v8::String::new(scope, "window").unwrap();
    global.set(scope, window_name.into(), global.into());
    
    let self_name = v8::String::new(scope, "self").unwrap();
    global.set(scope, self_name.into(), global.into());
    
    Ok(())
}

fn setup_navigator_bindings(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
) -> Result<(), String> {
    let navigator = v8::Object::new(scope);
    
    // userAgent
    let key = v8::String::new(scope, "userAgent").unwrap();
    let val = v8::String::new(scope, "Stokes Browser/1.0").unwrap();
    navigator.set(scope, key.into(), val.into());
    
    // language
    let key = v8::String::new(scope, "language").unwrap();
    let val = v8::String::new(scope, "en-US").unwrap();
    navigator.set(scope, key.into(), val.into());
    
    // platform
    let key = v8::String::new(scope, "platform").unwrap();
    let val = v8::String::new(scope, std::env::consts::OS).unwrap();
    navigator.set(scope, key.into(), val.into());
    
    // online
    let key = v8::String::new(scope, "online").unwrap();
    let val = v8::Boolean::new(scope, true);
    navigator.set(scope, key.into(), val.into());
    
    // Set on global
    let name = v8::String::new(scope, "navigator").unwrap();
    global.set(scope, name.into(), navigator.into());
    
    Ok(())
}

fn setup_location_bindings(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
) -> Result<(), String> {
    let location = v8::Object::new(scope);
    
    // href
    let key = v8::String::new(scope, "href").unwrap();
    let val = v8::String::new(scope, "about:blank").unwrap();
    location.set(scope, key.into(), val.into());
    
    // reload function
    let reload_fn = v8::Function::new(
        scope,
        |_scope: &mut v8::PinScope,
         _args: v8::FunctionCallbackArguments,
         _retval: v8::ReturnValue| {
            println!("[JS] location.reload() called");
        },
    ).unwrap();
    
    let key = v8::String::new(scope, "reload").unwrap();
    location.set(scope, key.into(), reload_fn.into());
    
    // Set on global
    let name = v8::String::new(scope, "location").unwrap();
    global.set(scope, name.into(), location.into());
    
    Ok(())
}

