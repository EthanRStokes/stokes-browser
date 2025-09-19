// CSS selector implementation
use crate::dom::{DomNode, NodeType, ElementData};
use std::rc::Rc;
use std::cell::RefCell;

/// CSS selector types
#[derive(Debug, Clone, PartialEq)]
pub enum SelectorType {
    Type(String),         // element
    Class(String),        // .class
    Id(String),           // #id
    Attribute(String, Option<String>), // [attr] or [attr=value]
    Universal,            // *
}

/// A CSS selector
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub selector_type: SelectorType,
    pub specificity: u32,
}

impl Selector {
    /// Create a new selector
    pub fn new(selector_type: SelectorType) -> Self {
        let specificity = Self::calculate_specificity(&selector_type);
        Self {
            selector_type,
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

        if trimmed.starts_with('#') {
            // ID selector
            let id = trimmed[1..].to_string();
            Some(Self::new(SelectorType::Id(id)))
        } else if trimmed.starts_with('.') {
            // Class selector
            let class = trimmed[1..].to_string();
            Some(Self::new(SelectorType::Class(class)))
        } else if trimmed.starts_with('[') && trimmed.ends_with(']') {
            // Attribute selector
            let attr_content = &trimmed[1..trimmed.len()-1];
            if let Some(equals_pos) = attr_content.find('=') {
                let attr_name = attr_content[..equals_pos].trim().to_string();
                let attr_value = attr_content[equals_pos+1..].trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                Some(Self::new(SelectorType::Attribute(attr_name, Some(attr_value))))
            } else {
                Some(Self::new(SelectorType::Attribute(attr_content.to_string(), None)))
            }
        } else if trimmed == "*" {
            // Universal selector
            Some(Self::new(SelectorType::Universal))
        } else {
            // Type selector (element name)
            Some(Self::new(SelectorType::Type(trimmed.to_string())))
        }
    }

    /// Calculate CSS specificity
    fn calculate_specificity(selector_type: &SelectorType) -> u32 {
        match selector_type {
            SelectorType::Id(_) => 100,
            SelectorType::Class(_) => 10,
            SelectorType::Attribute(_, _) => 10,
            SelectorType::Type(_) => 1,
            SelectorType::Universal => 0,
        }
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
    pub(crate) fn matches_element(&self, element: &ElementData) -> bool {
        match &self.selector_type {
            SelectorType::Type(tag_name) => {
                element.tag_name.to_lowercase() == tag_name.to_lowercase()
            }
            SelectorType::Class(class_name) => {
                element.classes().contains(&class_name.as_str())
            }
            SelectorType::Id(id) => {
                element.id() == Some(id.as_str())
            }
            SelectorType::Attribute(attr_name, attr_value) => {
                match element.attributes.get(attr_name) {
                    Some(value) => {
                        match attr_value {
                            Some(expected_value) => value == expected_value,
                            None => true, // Just check if attribute exists
                        }
                    }
                    None => false,
                }
            }
            SelectorType::Universal => true,
        }
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self.selector_type {
            SelectorType::Type(name) => write!(f, "{}", name),
            SelectorType::Class(name) => write!(f, ".{}", name),
            SelectorType::Id(name) => write!(f, "#{}", name),
            SelectorType::Attribute(name, Some(value)) => write!(f, "[{}=\"{}\"]", name, value),
            SelectorType::Attribute(name, None) => write!(f, "[{}]", name),
            SelectorType::Universal => write!(f, "*"),
        }
    }
}
