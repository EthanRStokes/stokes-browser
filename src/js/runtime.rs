// JavaScript runtime management using V8
use std::rc::Rc;
use std::cell::RefCell;
use v8::ContextOptions;
use crate::dom::DomNode;
use super::JsResult;

/// JavaScript runtime that manages V8 execution context
pub struct JsRuntime {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    document_root: Rc<RefCell<DomNode>>,
}

impl JsRuntime {
    /// Create a new JavaScript runtime with V8
    pub fn new(document_root: Rc<RefCell<DomNode>>) -> JsResult<Self> {
        // Initialize V8 platform (only once per process)
        static V8_INIT: std::sync::Once = std::sync::Once::new();
        V8_INIT.call_once(|| {
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });

        // Create a new isolate
        let mut isolate = v8::Isolate::new(Default::default());

        // Create a context and store it as a global handle
        let context = {
            let scope = &mut v8::PinScope::from(v8::HandleScope::new(&mut isolate));
            let context = v8::Context::new(scope, ContextOptions::default());
            let scope = &mut v8::ContextScope::new(scope, context);

            // Get the global object
            let global = context.global(scope);

            // Initialize browser bindings
            super::initialize_bindings(scope, global, document_root.clone())?;

            v8::Global::new(scope, context)
        };

        Ok(Self {
            isolate,
            context,
            document_root,
        })
    }

    /// Execute JavaScript code
    pub fn execute(&mut self, code: &str) -> JsResult<String> {
        let scope = &mut v8::PinScope::from(v8::HandleScope::new(&mut self.isolate));
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);

        // Compile the script
        let code_str = v8::String::new(scope, code)
            .ok_or_else(|| "Failed to create V8 string".to_string())?;

        let script = v8::Script::compile(scope, code_str, None)
            .ok_or_else(|| "Failed to compile script".to_string())?;

        // Execute the script
        let result = script.run(scope)
            .ok_or_else(|| "Script execution failed".to_string())?;

        // Convert result to string
        let result_str = result.to_string(scope)
            .ok_or_else(|| "Failed to convert result to string".to_string())?;

        Ok(result_str.to_rust_string_lossy(scope))
    }

    /// Execute JavaScript code from a script tag
    pub fn execute_script(&mut self, code: &str) -> JsResult<()> {
        match self.execute(code) {
            Ok(result) => {
                if !result.is_empty() && result != "undefined" {
                    println!("[JS Result] {}", result);
                }
                Ok(())
            }
            Err(e) => {
                eprintln!("Script execution error: {}", e);
                Err(e)
            }
        }
    }

    /// Update the document root
    pub fn update_document(&mut self, document_root: Rc<RefCell<DomNode>>) -> JsResult<()> {
        self.document_root = document_root.clone();

        // Re-initialize DOM bindings with new document
        let scope = &mut v8::PinScope::from(v8::HandleScope::new(&mut self.isolate));
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        super::dom_bindings::setup_dom_bindings(scope, global, document_root)?;
        Ok(())
    }
}
