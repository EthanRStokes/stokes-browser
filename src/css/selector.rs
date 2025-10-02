// CSS selector implementation
use crate::dom::{DomNode, NodeType, ElementData};
use std::rc::Rc;
use std::cell::RefCell;

/// CSS pseudo-class types
#[derive(Debug, Clone, PartialEq)]
pub enum PseudoClass {
    Link,
    Visited,
    Hover,
    Active,
    Focus,
}

/// CSS selector types
#[derive(Debug, Clone, PartialEq)]
pub enum SelectorType {
    Type(String),         // element
    Class(String),        // .class
    Id(String),           // #id
    Attribute(String, Option<String>), // [attr] or [attr=value]
    Universal,            // *
}

/// A CSS selector with optional pseudo-class
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub selector_type: SelectorType,
    pub pseudo_class: Option<PseudoClass>,
    pub specificity: u32,
}

impl Selector {
    /// Create a new selector
    pub fn new(selector_type: SelectorType) -> Self {
        let specificity = Self::calculate_specificity(&selector_type, &None);
        Self {
            selector_type,
            pseudo_class: None,
            specificity,
        }
    }

    /// Create a new selector with pseudo-class
    pub fn new_with_pseudo(selector_type: SelectorType, pseudo_class: PseudoClass) -> Self {
        let specificity = Self::calculate_specificity(&selector_type, &Some(pseudo_class.clone()));
        Self {
            selector_type,
            pseudo_class: Some(pseudo_class),
            specificity,
        }
    }

    /// Parse a simple CSS selector string
    pub fn parse(selector_str: &str) -> Vec<Self> {
        let mut selectors = Vec::new();
        let trimmed = selector_str.trim();

        if trimmed.is_empty() {
            return selectors;
        }

        // Handle multiple selectors separated by commas
        for part in trimmed.split(',') {
            let part = part.trim();
            if let Some(selector) = Self::parse_single_selector(part) {
                selectors.push(selector);
            }
        }

        selectors
    }

    /// Parse a single selector (no comma separation)
    fn parse_single_selector(selector_str: &str) -> Option<Self> {
        let trimmed = selector_str.trim();

        // Check for pseudo-class
        let (base_selector, pseudo_class) = if let Some(colon_pos) = trimmed.find(':') {
            let base = &trimmed[..colon_pos];
            let pseudo_str = &trimmed[colon_pos + 1..];
            let pseudo = match pseudo_str {
                "link" => Some(PseudoClass::Link),
                "visited" => Some(PseudoClass::Visited),
                "hover" => Some(PseudoClass::Hover),
                "active" => Some(PseudoClass::Active),
                "focus" => Some(PseudoClass::Focus),
                _ => None,
            };
            (base, pseudo)
        } else {
            (trimmed, None)
        };

        let selector_type = if base_selector.starts_with('#') {
            // ID selector
            let id = base_selector[1..].to_string();
            SelectorType::Id(id)
        } else if base_selector.starts_with('.') {
            // Class selector
            let class = base_selector[1..].to_string();
            SelectorType::Class(class)
        } else if base_selector.starts_with('[') && base_selector.ends_with(']') {
            // Attribute selector
            let attr_content = &base_selector[1..base_selector.len()-1];
            if let Some(equals_pos) = attr_content.find('=') {
                let attr_name = attr_content[..equals_pos].trim().to_string();
                let attr_value = attr_content[equals_pos+1..].trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                SelectorType::Attribute(attr_name, Some(attr_value))
            } else {
                SelectorType::Attribute(attr_content.to_string(), None)
            }
        } else if base_selector == "*" {
            // Universal selector
            SelectorType::Universal
        } else {
            // Type selector (element name)
            SelectorType::Type(base_selector.to_string())
        };

        let specificity = Self::calculate_specificity(&selector_type, &pseudo_class);

        Some(Self {
            selector_type,
            pseudo_class,
            specificity,
        })
    }

    /// Calculate CSS specificity
    fn calculate_specificity(selector_type: &SelectorType, pseudo_class: &Option<PseudoClass>) -> u32 {
        let base_specificity = match selector_type {
            SelectorType::Id(_) => 100,
            SelectorType::Class(_) => 10,
            SelectorType::Attribute(_, _) => 10,
            SelectorType::Type(_) => 1,
            SelectorType::Universal => 0,
        };

        let pseudo_specificity = match pseudo_class {
            Some(_) => 10, // Pseudo-classes have same specificity as classes
            None => 0,
        };

        base_specificity + pseudo_specificity
    }

    /// Check if this selector matches a DOM node
    pub fn matches(&self, node: &DomNode) -> bool {
        if let NodeType::Element(element_data) = &node.node_type {
            self.matches_element(element_data)
        } else {
            false
        }
    }

    /// Check if this selector matches an element
    pub fn matches_element(&self, element_data: &ElementData) -> bool {
        // First check if the base selector matches
        let base_matches = match &self.selector_type {
            SelectorType::Type(tag_name) => {
                element_data.tag_name.to_lowercase() == tag_name.to_lowercase()
            }
            SelectorType::Class(class_name) => {
                if let Some(class_attr) = element_data.attributes.get("class") {
                    class_attr.split_whitespace().any(|c| c == class_name)
                } else {
                    false
                }
            }
            SelectorType::Id(id_name) => {
                element_data.attributes.get("id")
                    .map(|id| id == id_name)
                    .unwrap_or(false)
            }
            SelectorType::Attribute(attr_name, attr_value) => {
                if let Some(element_value) = element_data.attributes.get(attr_name) {
                    match attr_value {
                        Some(expected_value) => element_value == expected_value,
                        None => true, // Just check for attribute presence
                    }
                } else {
                    false
                }
            }
            SelectorType::Universal => true,
        };

        if !base_matches {
            return false;
        }

        // Check pseudo-class if present
        if let Some(pseudo) = &self.pseudo_class {
            self.matches_pseudo_class(element_data, pseudo)
        } else {
            true
        }
    }

    /// Check if the element matches the pseudo-class
    fn matches_pseudo_class(&self, element_data: &ElementData, pseudo_class: &PseudoClass) -> bool {
        match pseudo_class {
            PseudoClass::Link => {
                // Link pseudo-class applies to unvisited links
                element_data.tag_name.to_lowercase() == "a" &&
                element_data.attributes.contains_key("href")
            }
            PseudoClass::Visited => {
                // For security reasons, we'll treat all links as unvisited in this simple implementation
                // In a real browser, this would check browser history
                false
            }
            PseudoClass::Hover | PseudoClass::Active | PseudoClass::Focus => {
                // These would require tracking mouse/keyboard state
                // For now, we'll return false for these dynamic pseudo-classes
                false
            }
        }
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let base = match &self.selector_type {
            SelectorType::Type(name) => name.clone(),
            SelectorType::Class(name) => format!(".{}", name),
            SelectorType::Id(name) => format!("#{}", name),
            SelectorType::Attribute(name, Some(value)) => format!("[{}=\"{}\"]", name, value),
            SelectorType::Attribute(name, None) => format!("[{}]", name),
            SelectorType::Universal => "*".to_string(),
        };

        if let Some(pseudo) = &self.pseudo_class {
            let pseudo_str = match pseudo {
                PseudoClass::Link => "link",
                PseudoClass::Visited => "visited",
                PseudoClass::Hover => "hover",
                PseudoClass::Active => "active",
                PseudoClass::Focus => "focus",
            };
            write!(f, "{}:{}", base, pseudo_str)
        } else {
            write!(f, "{}", base)
        }
    }
}
