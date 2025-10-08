// Style resolver that computes final styles for DOM nodes
use super::values::ComputedValues;
use crate::css::{Stylesheet, Selector, Declaration};
use crate::dom::{DomNode, NodeType, ElementData};

/// Style resolver that computes final styles for DOM nodes
pub struct StyleResolver {
    stylesheets: Vec<Stylesheet>,
}

impl StyleResolver {
    pub fn new() -> Self {
        let mut resolver = Self {
            stylesheets: Vec::new(),
        };

        // Add default user agent stylesheet
        resolver.add_stylesheet(Stylesheet::default_styles());

        resolver
    }

    /// Add a stylesheet to consider during style resolution
    pub fn add_stylesheet(&mut self, stylesheet: Stylesheet) {
        self.stylesheets.push(stylesheet);
    }

    /// Resolve styles for a DOM node
    pub fn resolve_styles(&self, node: &DomNode, parent_values: Option<&ComputedValues>) -> ComputedValues {
        let mut computed = match &node.node_type {
            NodeType::Element(element_data) => {
                ComputedValues::default_for_element(&element_data.tag_name)
            }
            _ => ComputedValues::default(),
        };

        // Inherit from parent where appropriate
        if let Some(parent) = parent_values {
            computed.color = computed.color.or_else(|| parent.color.clone());
            computed.font_family = parent.font_family.clone();
            computed.font_size = parent.font_size; // Will be adjusted by relative units
        }

        // Apply matching CSS rules
        if let NodeType::Element(element_data) = &node.node_type {
            let matching_rules = self.find_matching_rules(element_data);

            // Sort by specificity (lower specificity first)
            let mut sorted_rules: Vec<_> = matching_rules.into_iter().collect();
            sorted_rules.sort_by_key(|(_, rule)| rule.specificity());

            // Apply declarations in specificity order
            for (_, rule) in sorted_rules {
                for declaration in &rule.declarations {
                    super::applicator::apply_declaration(&mut computed, declaration, parent_values);
                }
            }

            // Apply inline styles (highest specificity)
            if let Some(style_attr) = element_data.attributes.get("style") {
                let parser = crate::css::parser::CssParser::new();
                let inline_declarations = parser.parse_inline_styles(style_attr);
                for declaration in inline_declarations {
                    super::applicator::apply_declaration(&mut computed, &declaration, parent_values);
                }
            }
        }

        computed
    }

    /// Find all rules that match an element
    fn find_matching_rules<'a>(&'a self, element_data: &ElementData) -> Vec<(&'a Selector, &'a crate::css::stylesheet::Rule)> {
        let mut matching_rules = Vec::new();

        for stylesheet in &self.stylesheets {
            for rule in &stylesheet.rules {
                for selector in &rule.selectors {
                    if selector.matches_element(element_data) {
                        matching_rules.push((selector, rule));
                        break; // Only need one matching selector per rule
                    }
                }
            }
        }

        matching_rules
    }
}

