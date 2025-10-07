// JavaScript runtime management
use boa_engine::{Context, JsValue, Source};
use std::rc::Rc;
use std::cell::RefCell;
use crate::dom::DomNode;
use super::{JsResult, initialize_bindings};

/// JavaScript runtime that manages execution context
pub struct JsRuntime {
    context: Context,
    document_root: Rc<RefCell<DomNode>>,
}

impl JsRuntime {
    /// Create a new JavaScript runtime
    pub fn new(document_root: Rc<RefCell<DomNode>>) -> JsResult<Self> {
        let mut context = Context::default();

        // Initialize browser bindings
        initialize_bindings(&mut context, document_root.clone())?;

        Ok(Self {
            context,
            document_root,
        })
    }

    /// Execute JavaScript code
    pub fn execute(&mut self, code: &str) -> JsResult<JsValue> {
        self.context
            .eval(Source::from_bytes(code))
            .map_err(|e| format!("JavaScript error: {}", e))
    }

    /// Execute JavaScript code from a script tag
    pub fn execute_script(&mut self, code: &str) -> JsResult<()> {
        match self.execute(code) {
            Ok(_) => Ok(()),
            Err(e) => {
                eprintln!("Script execution error: {}", e);
                Err(e)
            }
        }
    }

    /// Get a reference to the context
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    /// Update the document root
    pub fn update_document(&mut self, document_root: Rc<RefCell<DomNode>>) -> JsResult<()> {
        self.document_root = document_root.clone();
        // Re-initialize DOM bindings with new document
        super::dom_bindings::setup_dom_bindings(&mut self.context, document_root)?;
        Ok(())
    }
}

