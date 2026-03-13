//! # script_bindings
//!
//! WebIDL-driven binding metadata for the Stokes browser engine.
//!
//! At build time, `build.rs` parses every `.webidl` file under `webidls/` using
//! the **weedle** crate and emits Rust source containing **phf** perfect-hash
//! maps that describe the interfaces, namespaces, methods, and attributes
//! declared in those files.
//!
//! Consumer crates (e.g. the main `stokes-browser` crate) can query these maps
//! at runtime to drive generic SpiderMonkey binding dispatch.

// ── Public data types consumed by the generated code and downstream crates ──

/// Metadata for a single WebIDL operation / method.
#[derive(Debug, Clone, Copy)]
pub struct MethodInfo {
    /// The method name as it appears in the IDL.
    pub name: &'static str,
    /// Number of declared arguments (including optional ones).
    pub arity: usize,
    /// `true` when the last argument uses the `...` variadic syntax.
    pub is_variadic: bool,
    /// The stringified return type (e.g. `"undefined"`, `"DOMString"`).
    pub return_type: &'static str,
}

/// Metadata for a single WebIDL attribute.
#[derive(Debug, Clone, Copy)]
pub struct AttributeInfo {
    /// The attribute name as it appears in the IDL.
    pub name: &'static str,
    /// `true` when the attribute is declared `readonly`.
    pub readonly: bool,
    /// The stringified type (e.g. `"boolean"`, `"DOMString"`).
    pub type_name: &'static str,
}

/// Metadata for a WebIDL `namespace` (e.g. `console`).
#[derive(Debug)]
pub struct NamespaceInfo {
    pub name: &'static str,
    pub methods: &'static phf::Map<&'static str, MethodInfo>,
}

/// Metadata for a WebIDL `interface` (e.g. `Element`, `Document`).
#[derive(Debug)]
pub struct InterfaceInfo {
    pub name: &'static str,
    /// Name of the parent interface, if any (e.g. `"Node"` for `Element`).
    pub parent: Option<&'static str>,
    pub methods: &'static phf::Map<&'static str, MethodInfo>,
    pub attributes: &'static phf::Map<&'static str, AttributeInfo>,
}

// ── Include the build-script-generated maps ──
include!("../compute/bindings.rs");

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_namespace_exists() {
        let ns = NAMESPACES.get("console").expect("console namespace should exist");
        assert_eq!(ns.name, "console");
    }

    #[test]
    fn console_has_log() {
        let ns = NAMESPACES.get("console").unwrap();
        let log = ns.methods.get("log").expect("console should have log method");
        assert_eq!(log.name, "log");
        assert!(log.is_variadic);
        assert_eq!(log.return_type, "undefined");
    }

    #[test]
    fn console_has_warn_and_error() {
        let ns = NAMESPACES.get("console").unwrap();
        assert!(ns.methods.get("warn").is_some());
        assert!(ns.methods.get("error").is_some());
    }

    #[test]
    fn console_has_clear() {
        let ns = NAMESPACES.get("console").unwrap();
        let clear = ns.methods.get("clear").expect("console should have clear method");
        assert_eq!(clear.arity, 0);
        assert!(!clear.is_variadic);
    }

    #[test]
    fn console_method_count() {
        let ns = NAMESPACES.get("console").unwrap();
        // Console.webidl declares 19 methods
        assert_eq!(ns.methods.len(), 19);
    }
}
