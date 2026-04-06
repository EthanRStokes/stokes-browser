use crate::dom::Dom;
use crate::engine::net_provider::StokesNetProvider;
use crate::js::runtime::RUNTIME;
use std::sync::Arc;

/// Document-scoped runtime state that survives across runtime internals.
pub(crate) struct RuntimeContext {
    dom: *mut Dom,
    user_agent: String,
    current_script_node_id: Option<usize>,
}

impl RuntimeContext {
    pub(crate) fn new(dom: *mut Dom, user_agent: String) -> Self {
        Self {
            dom,
            user_agent,
            current_script_node_id: None,
        }
    }

    pub(crate) fn dom_ptr(&self) -> *mut Dom {
        self.dom
    }

    pub(crate) fn user_agent(&self) -> &str {
        &self.user_agent
    }

    pub(crate) fn update_for_navigation(&mut self, dom: *mut Dom, user_agent: String) {
        self.dom = dom;
        self.user_agent = user_agent;
        self.current_script_node_id = None;
    }

    pub(crate) fn set_current_script_node_id(&mut self, node_id: Option<usize>) {
        self.current_script_node_id = node_id;
    }

    pub(crate) fn current_script_node_id(&self) -> Option<usize> {
        self.current_script_node_id
    }
}

pub(crate) fn with_current_context<R>(consumer: impl FnOnce(&RuntimeContext) -> R) -> Option<R> {
    RUNTIME.with(|cell| {
        let slot = cell.borrow();
        let runtime_ptr = (*slot)?;
        let runtime = unsafe { &*runtime_ptr };
        Some(consumer(runtime.context()))
    })
}

pub(crate) fn with_current_context_mut<R>(consumer: impl FnOnce(&mut RuntimeContext) -> R) -> Option<R> {
    RUNTIME.with(|cell| {
        let mut slot = cell.borrow_mut();
        let runtime_ptr = (*slot)?;
        let runtime = unsafe { &mut *runtime_ptr };
        Some(consumer(runtime.context_mut()))
    })
}

pub(crate) fn current_document_base_url() -> Option<String> {
    with_current_context(|context| {
        let dom_ptr = context.dom_ptr();
        if dom_ptr.is_null() {
            return None;
        }
        let dom = unsafe { &*dom_ptr };
        Some(dom.url.to_string())
    })?
}

pub(crate) fn current_net_provider_and_source_url() -> Option<(Arc<StokesNetProvider>, String)> {
    with_current_context(|context| {
        let dom_ptr = context.dom_ptr();
        if dom_ptr.is_null() {
            return None;
        }

        let dom = unsafe { &*dom_ptr };
        Some((dom.net_provider.clone(), dom.url.to_string()))
    })?
}

pub(crate) fn current_user_agent() -> Option<String> {
    with_current_context(|context| context.user_agent().to_string())
}

pub(crate) fn set_current_script_node_id(node_id: Option<usize>) {
    let _ = with_current_context_mut(|context| {
        context.set_current_script_node_id(node_id);
    });
}

pub(crate) fn current_script_node_id() -> Option<usize> {
    with_current_context(|context| context.current_script_node_id())?
}



