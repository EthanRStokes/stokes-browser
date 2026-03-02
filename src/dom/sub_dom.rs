use std::cell::RefCell;
use std::rc::Rc;
use crate::dom::{Dom, PlainDom};

pub struct SubDocumentAttr {
    id: usize,
    dom: Rc<RefCell<Option<Box<PlainDom>>>>,
}

impl SubDocumentAttr {
    pub fn new(dom: Dom) -> Self {
        let id = dom.id();
        let wrapped = Rc::new(RefCell::new(Some(Box::new(PlainDom(dom)))));
        Self { id, dom: wrapped }
    }
    pub fn take_document(&self) -> Option<Box<PlainDom>> {
        self.dom.borrow_mut().take()
    }
}

impl PartialEq for SubDocumentAttr {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
