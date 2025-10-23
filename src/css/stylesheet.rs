// CSS stylesheet, rules, and declarations
use super::{CssValue, PropertyName, Selector};

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
            "span", "a", "em", "strong", "code", "b", "i", "u", "small", "sub", "sup", "button"
        ];
        
        for element in inline_elements {
            let selectors = vec![Selector::parse(element).into_iter().next().unwrap()];
            let declarations = vec![
                Declaration::new(PropertyName::Display, CssValue::Keyword("inline".to_string())),
            ];
            stylesheet.add_rule(Rule::new(selectors, declarations));
        }
        
        // Heading styles
        let heading_styles = vec![
            ("h1", 32.0),
            ("h2", 24.0),
            ("h3", 18.72),
            ("h4", 16.0),
            ("h5", 13.28),
            ("h6", 10.72),
        ];
        
        for (tag, size) in heading_styles {
            let selectors = vec![Selector::parse(tag).into_iter().next().unwrap()];
            let declarations = vec![
                Declaration::new(PropertyName::Display, CssValue::Keyword("block".to_string())),
                Declaration::new(PropertyName::FontSize, CssValue::Length(super::values::Length::px(size))),
                Declaration::new(PropertyName::FontWeight, CssValue::Keyword("bold".to_string())),
                Declaration::new(PropertyName::MarginTop, CssValue::Length(super::values::Length::em(0.67))),
                Declaration::new(PropertyName::MarginBottom, CssValue::Length(super::values::Length::em(0.67))),
            ];
            stylesheet.add_rule(Rule::new(selectors, declarations));
        }
        
        // Paragraph styles
        let selectors = vec![Selector::parse("p").into_iter().next().unwrap()];
        let declarations = vec![
            Declaration::new(PropertyName::Display, CssValue::Keyword("block".to_string())),
            Declaration::new(PropertyName::MarginTop, CssValue::Length(super::values::Length::em(1.0))),
            Declaration::new(PropertyName::MarginBottom, CssValue::Length(super::values::Length::em(1.0))),
        ];
        stylesheet.add_rule(Rule::new(selectors, declarations));
        
        // Link styles
        let selectors = vec![Selector::parse("a").into_iter().next().unwrap()];
        let declarations = vec![
            Declaration::new(PropertyName::Color, CssValue::Color(super::values::Color::Named("blue".to_string()))),
        ];
        stylesheet.add_rule(Rule::new(selectors, declarations));

        // Body default styles
        let selectors = vec![Selector::parse("body").into_iter().next().unwrap()];
        let declarations = vec![
            Declaration::new(PropertyName::FontFamily, CssValue::String("Arial, sans-serif".to_string())),
            Declaration::new(PropertyName::FontSize, CssValue::Length(super::values::Length::px(16.0))),
            Declaration::new(PropertyName::Color, CssValue::Color(super::values::Color::Named("black".to_string()))),
            Declaration::new(PropertyName::BackgroundColor, CssValue::Color(super::values::Color::Named("white".to_string()))),
            Declaration::new(PropertyName::Margin, CssValue::Length(super::values::Length::px(8.0))),
        ];
        stylesheet.add_rule(Rule::new(selectors, declarations));

        stylesheet
    }

    /// Merge another stylesheet into this one
    pub fn merge(&mut self, other: Stylesheet) {
        self.rules.extend(other.rules);
    }
}
