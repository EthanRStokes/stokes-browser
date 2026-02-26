use dioxus_core::Template;
use rustc_hash::FxHashMap;

#[derive(Debug)]
pub struct DomState {
    /// Store of templates keyed by unique name
    pub(crate) templates: FxHashMap<Template, Vec<usize>>,
    /// Stack machine state for applying dioxus mutations
    pub(crate) stack: Vec<usize>,
    /// Mapping from vdom ElementId -> rdom NodeId
    pub(crate) node_id_mapping: Vec<Option<usize>>,
    /// Count of each handler type
    pub(crate) event_handler_counts: [u32; 32],
    /// Mounted events queued as elements are mounted
    pub(crate) queued_mounted_events: Vec<usize>,
}

impl DomState {
    /// Initialize the DioxusState in the RealDom
    pub fn create(root_id: usize) -> Self {
        Self {
            templates: FxHashMap::default(),
            stack: vec![root_id],
            node_id_mapping: vec![Some(root_id)],
            event_handler_counts: [0; 32],
            queued_mounted_events: Vec::new(),
        }
    }

    /// Convert an ElementId to a NodeId
    pub fn element_to_node_id(&self, element_id: usize) -> usize {
        self.try_element_to_node_id(element_id).unwrap()
    }

    /// Attempt to convert an ElementId to a NodeId. This will return None if the ElementId is not in the RealDom.
    pub fn try_element_to_node_id(&self, element_id: usize) -> Option<usize> {
        self.node_id_mapping.get(element_id).copied().flatten()
    }

    pub(crate) fn anchor_and_nodes(&mut self, id: usize, m: usize) -> (usize, Vec<usize>) {
        let anchor_node_id = self.element_to_node_id(id);
        let new_nodes = self.m_stack_nodes(m);
        (anchor_node_id, new_nodes)
    }

    pub(crate) fn m_stack_nodes(&mut self, m: usize) -> Vec<usize> {
        self.stack.split_off(self.stack.len() - m)
    }

    pub(crate) fn queue_mount_event(&mut self, id: usize) {
        self.queued_mounted_events.push(id);
    }
}