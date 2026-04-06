use std::collections::{HashMap, HashSet};

use crate::js::bindings::{element, element_bindings, event_target, node};
use crate::js::{JsResult, JsRuntime};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsapi::JSObject;
use mozjs::jsval::UndefinedValue;
use mozjs::rooted;
use mozjs::rust::wrappers2::JS_GetProperty;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InstallPhase {
    Bootstrap,
    CoreDom,
}

type InterfaceInstaller = unsafe fn(&mut SafeJSContext, *mut JSObject) -> Result<(), String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InterfaceDescriptor {
    pub name: &'static str,
    pub parent: Option<&'static str>,
    pub phase: InstallPhase,
    pub install_ctor: Option<InterfaceInstaller>,
    pub install_proto: Option<InterfaceInstaller>,
    pub install_instance: Option<InterfaceInstaller>,
}

const DEFAULT_INTERFACES: &[InterfaceDescriptor] = &[
    InterfaceDescriptor {
        name: "EventTarget",
        parent: None,
        phase: InstallPhase::CoreDom,
        install_ctor: Some(event_target::setup_event_target_constructor_bindings),
        install_proto: None,
        install_instance: None,
    },
    InterfaceDescriptor {
        name: "Node",
        parent: Some("EventTarget"),
        phase: InstallPhase::CoreDom,
        install_ctor: Some(node::setup_node_constructor_bindings),
        install_proto: None,
        install_instance: None,
    },
    InterfaceDescriptor {
        name: "Element",
        parent: Some("Node"),
        phase: InstallPhase::CoreDom,
        install_ctor: Some(element::setup_element_constructors_bindings),
        install_proto: None,
        install_instance: None,
    },
    InterfaceDescriptor {
        name: "Document",
        parent: Some("Node"),
        phase: InstallPhase::CoreDom,
        install_ctor: None,
        install_proto: None,
        install_instance: Some(crate::js::bindings::document::setup_document_bindings),
    },
    InterfaceDescriptor {
        name: "Promise",
        parent: None,
        phase: InstallPhase::Bootstrap,
        install_ctor: None,
        install_proto: None,
        install_instance: None,
    },
];

pub(crate) fn setup_interface_registry(_runtime: &mut JsRuntime) -> JsResult<()> {
    validate_descriptor_graph(DEFAULT_INTERFACES)
}

pub(crate) unsafe fn install_phase_bindings(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    phase: InstallPhase,
) -> JsResult<()> {
    for descriptor in DEFAULT_INTERFACES {
        if descriptor.phase == phase {
            if let Some(installer) = descriptor.install_ctor {
                installer(cx, global)?;
            }
            if let Some(installer) = descriptor.install_proto {
                installer(cx, global)?;
            }
            if let Some(installer) = descriptor.install_instance {
                installer(cx, global)?;
            }

            if descriptor.name != "Promise" {
                assert_constructor_shape(cx, global, descriptor.name)?;
            }
        }
    }

    Ok(())
}

pub(crate) unsafe fn link_phase_inheritance(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    phase: InstallPhase,
) -> JsResult<()> {
    for descriptor in DEFAULT_INTERFACES {
        if descriptor.phase != phase {
            continue;
        }

        let Some(parent_name) = descriptor.parent else {
            continue;
        };

        let child_ctor = get_object_property(cx, global, descriptor.name)
            .ok_or_else(|| format!("{} constructor not found", descriptor.name))?;
        let parent_ctor = get_object_property(cx, global, parent_name)
            .ok_or_else(|| format!("{} constructor not found", parent_name))?;

        let child_proto = get_object_property(cx, child_ctor, "prototype")
            .ok_or_else(|| format!("{}.prototype not found", descriptor.name))?;
        let parent_proto = get_object_property(cx, parent_ctor, "prototype")
            .ok_or_else(|| format!("{}.prototype not found", parent_name))?;

        element_bindings::set_object_prototype(cx, child_proto, parent_proto)?;
        element_bindings::set_object_prototype(cx, child_ctor, parent_ctor)?;

        if descriptor.name == "Document" {
            if let Some(document_obj) = get_object_property(cx, global, "document") {
                element_bindings::set_object_prototype(cx, document_obj, child_proto)?;
            }
        }

        // Confirm linkage did not drop expected prototype properties.
        assert_constructor_shape(cx, global, descriptor.name)?;
        assert_constructor_shape(cx, global, parent_name)?;
    }

    Ok(())
}

unsafe fn assert_constructor_shape(
    cx: &mut SafeJSContext,
    global: *mut JSObject,
    constructor_name: &str,
) -> JsResult<()> {
    let ctor = get_object_property(cx, global, constructor_name)
        .ok_or_else(|| format!("{} constructor not found", constructor_name))?;
    let _proto = get_object_property(cx, ctor, "prototype")
        .ok_or_else(|| format!("{}.prototype not found", constructor_name))?;
    Ok(())
}

unsafe fn get_object_property(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    property: &str,
) -> Option<*mut JSObject> {
    let raw_cx = cx.raw_cx();
    let name = std::ffi::CString::new(property).ok()?;

    rooted!(in(raw_cx) let rooted_obj = obj);
    rooted!(in(raw_cx) let mut value = UndefinedValue());
    if JS_GetProperty(
        cx,
        rooted_obj.handle().into(),
        name.as_ptr(),
        value.handle_mut().into(),
    ) && value.get().is_object()
    {
        Some(value.get().to_object())
    } else {
        None
    }
}

fn validate_descriptor_graph(descriptors: &[InterfaceDescriptor]) -> JsResult<()> {
    let mut by_name = HashMap::new();
    for descriptor in descriptors {
        if by_name.insert(descriptor.name, *descriptor).is_some() {
            return Err(format!(
                "Interface registry contains duplicate descriptor: {}",
                descriptor.name
            ));
        }
    }

    for descriptor in descriptors {
        if let Some(parent) = descriptor.parent {
            if !by_name.contains_key(parent) {
                return Err(format!(
                    "Interface '{}' references unknown parent '{}'",
                    descriptor.name, parent
                ));
            }
        }
    }

    for descriptor in descriptors {
        let mut seen = HashSet::new();
        let mut cursor = descriptor;

        while let Some(parent_name) = cursor.parent {
            if !seen.insert(parent_name) {
                return Err(format!(
                    "Interface '{}' has cyclic inheritance at '{}'",
                    descriptor.name, parent_name
                ));
            }

            cursor = by_name
                .get(parent_name)
                .ok_or_else(|| {
                    format!(
                        "Interface '{}' references unknown parent '{}'",
                        descriptor.name, parent_name
                    )
                })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_descriptor_graph, InstallPhase, InterfaceDescriptor};

    #[test]
    fn validates_default_shape() {
        let descriptors = [
            InterfaceDescriptor {
                name: "EventTarget",
                parent: None,
                phase: InstallPhase::Bootstrap,
                install_ctor: None,
                install_proto: None,
                install_instance: None,
            },
            InterfaceDescriptor {
                name: "Node",
                parent: Some("EventTarget"),
                phase: InstallPhase::CoreDom,
                install_ctor: None,
                install_proto: None,
                install_instance: None,
            },
        ];

        assert!(validate_descriptor_graph(&descriptors).is_ok());
    }

    #[test]
    fn rejects_unknown_parent() {
        let descriptors = [InterfaceDescriptor {
            name: "Node",
            parent: Some("Missing"),
            phase: InstallPhase::CoreDom,
            install_ctor: None,
            install_proto: None,
            install_instance: None,
        }];

        let err = validate_descriptor_graph(&descriptors).unwrap_err();
        assert!(err.contains("unknown parent"));
    }

    #[test]
    fn rejects_cycles() {
        let descriptors = [
            InterfaceDescriptor {
                name: "Node",
                parent: Some("Element"),
                phase: InstallPhase::CoreDom,
                install_ctor: None,
                install_proto: None,
                install_instance: None,
            },
            InterfaceDescriptor {
                name: "Element",
                parent: Some("Node"),
                phase: InstallPhase::CoreDom,
                install_ctor: None,
                install_proto: None,
                install_instance: None,
            },
        ];

        let err = validate_descriptor_graph(&descriptors).unwrap_err();
        assert!(err.contains("cyclic inheritance"));
    }
}


