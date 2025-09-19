// CSS stylesheet, rules, and declarations
use super::{PropertyName, CssValue, Selector};
use std::collections::HashMap;

/// A CSS declaration (property: value)
#[derive(Debug, Clone)]
pub struct Declaration {
    pub property: PropertyName,
    pub value: CssValue,
    pub important: bool,
}

impl Declaration {
    pub fn new(property: PropertyName, value: CssValue) -> Self {
        Self {
            property,
            value,
            important: false,
        }
    }

    pub fn with_important(property: PropertyName, value: CssValue, important: bool) -> Self {
        Self {
            property,
            value,
            important,
        }
    }
}

/// A CSS rule (selector + declarations)
#[derive(Debug, Clone)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

impl Rule {
    pub fn new(selectors: Vec<Selector>, declarations: Vec<Declaration>) -> Self {
        Self {
            selectors,
            declarations,
        }
    }

    /// Get the highest specificity of all selectors in this rule
    pub fn specificity(&self) -> u32 {
        self.selectors.iter()
            .map(|s| s.specificity)
            .max()
            .unwrap_or(0)
    }
}

/// A CSS stylesheet containing multiple rules
#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

impl Stylesheet {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
        }
    }

    /// Add a rule to the stylesheet
    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// Get default user agent stylesheet
    pub fn default_styles() -> Self {
        let mut stylesheet = Self::new();
        
        // Default block elements
        let block_elements = vec![
            "html", "body", "div", "section", "article", "aside", "header", "footer",
            "nav", "main", "blockquote", "ul", "ol", "li", "dl", "dt", "dd", "form", "fieldset", "table"
        ];
        
        for element in block_elements {
            let selectors = vec![Selector::parse(element).into_iter().next().unwrap()];
            let declarations = vec![
                Declaration::new(PropertyName::Display, CssValue::Keyword("block".to_string())),
            ];
            stylesheet.add_rule(Rule::new(selectors, declarations));
        }
        
        // Default inline elements
        let inline_elements = vec![
            "span", "a", "em", "strong", "code", "b", "i", "u", "small", "sub", "sup"
        ];
        
        for element in inline_elements {
            let selectors = vec![Selector::parse(element).into_iter().next().unwrap()];
            let declarations = vec![
                Declaration::new(PropertyName::Display, CssValue::Keyword("inline".to_string())),
            ];
            stylesheet.add_rule(Rule::new(selectors, declarations));
        }
        
        // Body default styles
        let selectors = vec![Selector::parse("body").into_iter().next().unwrap()];
        let declarations = vec![
            Declaration::new(PropertyName::Margin, CssValue::Length(super::values::Length::px(0.0))),
            Declaration::new(PropertyName::Padding, CssValue::Length(super::values::Length::px(0.0))),
            Declaration::new(PropertyName::BackgroundColor, CssValue::Color(super::values::Color::Named("white".to_string()))),
        ];
        stylesheet.add_rule(Rule::new(selectors, declarations));
        
        // HTML default styles
        let selectors = vec![Selector::parse("html").into_iter().next().unwrap()];
        let declarations = vec![
            Declaration::new(PropertyName::Margin, CssValue::Length(super::values::Length::px(0.0))),
            Declaration::new(PropertyName::Padding, CssValue::Length(super::values::Length::px(0.0))),
        ];
        stylesheet.add_rule(Rule::new(selectors, declarations));
        
        // Headings default styles
        let heading_styles = vec![
            ("h1", "32px", "bold"),
            ("h2", "24px", "bold"),
            ("h3", "18.72px", "bold"),
            ("h4", "16px", "bold"),
            ("h5", "13.28px", "bold"),
            ("h6", "10.72px", "bold"),
        ];
        
        for (tag, size, weight) in heading_styles {
            let selectors = vec![Selector::parse(tag).into_iter().next().unwrap()];
            let declarations = vec![
                Declaration::new(PropertyName::FontSize, CssValue::parse(size)),
                Declaration::new(PropertyName::FontWeight, CssValue::Keyword(weight.to_string())),
                Declaration::new(PropertyName::MarginTop, CssValue::parse("0.83em")),
                Declaration::new(PropertyName::MarginBottom, CssValue::parse("0.83em")),
                Declaration::new(PropertyName::Display, CssValue::Keyword("block".to_string())),
            ];
            stylesheet.add_rule(Rule::new(selectors, declarations));
        }
        
        // Paragraph default margins
        let selectors = vec![Selector::parse("p").into_iter().next().unwrap()];
        let declarations = vec![
            Declaration::new(PropertyName::MarginTop, CssValue::parse("1em")),
            Declaration::new(PropertyName::MarginBottom, CssValue::parse("1em")),
            Declaration::new(PropertyName::Display, CssValue::Keyword("block".to_string())),
        ];
        stylesheet.add_rule(Rule::new(selectors, declarations));
        
        // Link default styles
        let selectors = vec![Selector::parse("a").into_iter().next().unwrap()];
        let declarations = vec![
            Declaration::new(PropertyName::Color, CssValue::Color(super::values::Color::Named("blue".to_string()))),
        ];
        stylesheet.add_rule(Rule::new(selectors, declarations));
        
        stylesheet
    }

    /// Merge another stylesheet into this one
    pub fn merge(&mut self, other: Stylesheet) {
        self.rules.extend(other.rules);
    }
}
