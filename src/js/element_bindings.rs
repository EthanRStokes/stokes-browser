// Element bindings for JavaScript using V8
use std::rc::Rc;
use std::cell::RefCell;
use crate::dom::{DomNode, NodeType, ElementData};

/// Wrapper for a DOM element that can be used in JavaScript
pub struct ElementWrapper {
    pub node: Rc<RefCell<DomNode>>,
}

impl ElementWrapper {
    pub fn new(node: Rc<RefCell<DomNode>>) -> Self {
        Self { node }
    }

    /// Create a JavaScript object from a DOM element
    pub fn create_js_element(
        element_rc: &Rc<RefCell<DomNode>>,
        scope: &mut v8::HandleScope,
    ) -> Result<v8::Local<v8::Object>, String> {
        let element = element_rc.borrow();
        let element_obj = v8::Object::new(scope);

        if let NodeType::Element(ref data) = element.node_type {
            // tagName
            let key = v8::String::new(scope, "tagName").unwrap();
            let val = v8::String::new(scope, &data.tag_name.to_uppercase()).unwrap();
            element_obj.set(scope, key.into(), val.into());

            // id
            let key = v8::String::new(scope, "id").unwrap();
            let val = v8::String::new(scope, data.id().unwrap_or("")).unwrap();
            element_obj.set(scope, key.into(), val.into());

            // className
            let key = v8::String::new(scope, "className").unwrap();
            let class_val = data.attributes.get("class").map(|s| s.as_str()).unwrap_or("");
            let val = v8::String::new(scope, class_val).unwrap();
            element_obj.set(scope, key.into(), val.into());

            // textContent
            let key = v8::String::new(scope, "textContent").unwrap();
            let text = element.text_content();
            let val = v8::String::new(scope, &text).unwrap();
            element_obj.set(scope, key.into(), val.into());

            // nodeName
            let key = v8::String::new(scope, "nodeName").unwrap();
            let val = v8::String::new(scope, &data.tag_name.to_uppercase()).unwrap();
            element_obj.set(scope, key.into(), val.into());

            // nodeType
            let key = v8::String::new(scope, "nodeType").unwrap();
            let val = v8::Integer::new(scope, 1); // ELEMENT_NODE
            element_obj.set(scope, key.into(), val.into());

            // Add getAttribute method
            let get_attribute_fn = v8::Function::new(
                scope,
                |scope: &mut v8::HandleScope,
                 args: v8::FunctionCallbackArguments,
                 mut retval: v8::ReturnValue| {
                    let attr_name = if args.length() > 0 {
                        args.get(0).to_string(scope)
                            .map(|s| s.to_rust_string_lossy(scope))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };

                    println!("[JS] element.getAttribute('{}') called", attr_name);
                    retval.set(v8::null(scope).into());
                },
            ).unwrap();

            let key = v8::String::new(scope, "getAttribute").unwrap();
            element_obj.set(scope, key.into(), get_attribute_fn.into());

            // Add setAttribute method
            let set_attribute_fn = v8::Function::new(
                scope,
                |scope: &mut v8::HandleScope,
                 args: v8::FunctionCallbackArguments,
                 _retval: v8::ReturnValue| {
                    let attr_name = if args.length() > 0 {
                        args.get(0).to_string(scope)
                            .map(|s| s.to_rust_string_lossy(scope))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };

                    let attr_value = if args.length() > 1 {
                        args.get(1).to_string(scope)
                            .map(|s| s.to_rust_string_lossy(scope))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };

                    println!("[JS] element.setAttribute('{}', '{}') called", attr_name, attr_value);
                },
            ).unwrap();

            let key = v8::String::new(scope, "setAttribute").unwrap();
            element_obj.set(scope, key.into(), set_attribute_fn.into());

            // Add addEventListener method
            let add_event_listener_fn = v8::Function::new(
                scope,
                |scope: &mut v8::HandleScope,
                 args: v8::FunctionCallbackArguments,
                 _retval: v8::ReturnValue| {
                    let event_type = if args.length() > 0 {
                        args.get(0).to_string(scope)
                            .map(|s| s.to_rust_string_lossy(scope))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };

                    println!("[JS] element.addEventListener('{}') called", event_type);
                },
            ).unwrap();

            let key = v8::String::new(scope, "addEventListener").unwrap();
            element_obj.set(scope, key.into(), add_event_listener_fn.into());
        }

        Ok(element_obj)
    }

    /// Create a stub element for createElement
    pub fn create_stub_element(
        tag_name: &str,
        scope: &mut v8::HandleScope,
    ) -> Result<v8::Local<v8::Object>, String> {
        let element_obj = v8::Object::new(scope);

        // tagName
        let key = v8::String::new(scope, "tagName").unwrap();
        let val = v8::String::new(scope, &tag_name.to_uppercase()).unwrap();
        element_obj.set(scope, key.into(), val.into());

        // nodeName
        let key = v8::String::new(scope, "nodeName").unwrap();
        let val = v8::String::new(scope, &tag_name.to_uppercase()).unwrap();
        element_obj.set(scope, key.into(), val.into());

        // nodeType
        let key = v8::String::new(scope, "nodeType").unwrap();
        let val = v8::Integer::new(scope, 1); // ELEMENT_NODE
        element_obj.set(scope, key.into(), val.into());

        Ok(element_obj)
    }
}

