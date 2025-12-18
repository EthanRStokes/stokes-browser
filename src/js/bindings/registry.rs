use crate::dom::DomNode;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    // Map from pointer address to Rc<RefCell<DomNode>>
    static REGISTRY: RefCell<HashMap<i64, Rc<RefCell<DomNode>>>> = RefCell::new(HashMap::new());
}

/// Register a node with the given pointer value
pub fn register_node(ptr: i64, node: &Rc<RefCell<DomNode>>) {
    REGISTRY.with(|map| {
        map.borrow_mut().insert(ptr, Rc::clone(node));
    });
}

/// Unregister a node by pointer
pub fn unregister_node(ptr: i64) {
    REGISTRY.with(|map| {
        map.borrow_mut().remove(&ptr);
    });
}

/// Attempt to return an Rc to the DomNode for a given pointer
pub fn get_node(ptr: i64) -> Option<Rc<RefCell<DomNode>>> {
    REGISTRY.with(|map| map.borrow().get(&ptr).cloned())
}
