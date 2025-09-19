// CSS parser implementation
use super::{PropertyName, CssValue, Selector, Stylesheet, Rule, Declaration};

/// Simple CSS parser
pub struct CssParser;

impl CssParser {
    pub fn new() -> Self {
        Self
    }

    /// Parse a CSS string into a stylesheet
    pub fn parse(&self, css: &str) -> Stylesheet {
        let mut stylesheet = Stylesheet::new();

        // Remove comments
        let css = self.remove_comments(css);

        // Split into rules by finding selector { ... } blocks
        let rules = self.split_into_rules(&css);

        for rule_text in rules {
            if let Some(rule) = self.parse_rule(&rule_text) {
                stylesheet.add_rule(rule);
            }
        }

        stylesheet
    }

    /// Remove CSS comments
    fn remove_comments(&self, css: &str) -> String {
        let mut result = String::new();
        let mut chars = css.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '/' {
                if let Some(&'*') = chars.peek() {
                    chars.next(); // consume '*'
                    // Skip until */
                    let mut found_end = false;
                    while let Some(ch) = chars.next() {
                        if ch == '*' {
                            if let Some(&'/') = chars.peek() {
                                chars.next(); // consume '/'
                                found_end = true;
                                break;
                            }
                        }
                    }
                    if !found_end {
                        // Malformed comment, but continue
                    }
                } else {
                    result.push(ch);
                }
            } else {
                result.push(ch);
            }
        }

        result
    }

    /// Split CSS into individual rules
    fn split_into_rules(&self, css: &str) -> Vec<String> {
        let mut rules = Vec::new();
        let mut current_rule = String::new();
        let mut brace_depth = 0;
        let mut in_string = false;
        let mut string_char = '"';
        let mut in_at_rule = false;

        for ch in css.chars() {
            match ch {
                '"' | '\'' if !in_string => {
                    in_string = true;
                    string_char = ch;
                    current_rule.push(ch);
                }
                ch if in_string && ch == string_char => {
                    in_string = false;
                    current_rule.push(ch);
                }
                '@' if !in_string && brace_depth == 0 => {
                    // Start of at-rule like @media
                    in_at_rule = true;
                    current_rule.push(ch);
                }
                '{' if !in_string => {
                    brace_depth += 1;
                    current_rule.push(ch);
                }
                '}' if !in_string => {
                    current_rule.push(ch);
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        let trimmed = current_rule.trim().to_string();
                        if !trimmed.is_empty() && !in_at_rule {
                            // Only add regular CSS rules, skip @media and other at-rules
                            rules.push(trimmed);
                        }
                        current_rule.clear();
                        in_at_rule = false;
                    }
                }
                _ => {
                    current_rule.push(ch);
                }
            }
        }

        rules
    }

    /// Parse a single CSS rule
    fn parse_rule(&self, rule_text: &str) -> Option<Rule> {
        // Find the opening brace
        if let Some(brace_pos) = rule_text.find('{') {
            let selector_part = rule_text[..brace_pos].trim();
            let declarations_part = rule_text[brace_pos + 1..]
                .trim_end_matches('}')
                .trim();

            // Parse selectors
            let selectors = self.parse_selectors(selector_part);
            if selectors.is_empty() {
                return None;
            }

            // Parse declarations
            let declarations = self.parse_declarations(declarations_part);

            Some(Rule::new(selectors, declarations))
        } else {
            None
        }
    }

    /// Parse CSS selectors (comma-separated)
    fn parse_selectors(&self, selector_text: &str) -> Vec<Selector> {
        Selector::parse(selector_text)
    }

    /// Parse CSS declarations
    fn parse_declarations(&self, declarations_text: &str) -> Vec<Declaration> {
        let mut declarations = Vec::new();

        // Split by semicolons
        for decl_text in declarations_text.split(';') {
            let decl_text = decl_text.trim();
            if decl_text.is_empty() {
                continue;
            }

            if let Some(declaration) = self.parse_declaration(decl_text) {
                declarations.push(declaration);
            }
        }

        declarations
    }

    /// Parse a single CSS declaration
    fn parse_declaration(&self, decl_text: &str) -> Option<Declaration> {
        // Split by colon
        if let Some(colon_pos) = decl_text.find(':') {
            let property_text = decl_text[..colon_pos].trim();
            let mut value_text = decl_text[colon_pos + 1..].trim();

            // Check for !important
            let important = if value_text.ends_with("!important") {
                value_text = value_text[..value_text.len() - 10].trim();
                true
            } else {
                false
            };

            let property = PropertyName::from(property_text);
            let value = CssValue::parse(value_text);

            Some(Declaration::with_important(property, value, important))
        } else {
            None
        }
    }

    /// Parse inline styles from a style attribute
    pub fn parse_inline_styles(&self, style_attr: &str) -> Vec<Declaration> {
        self.parse_declarations(style_attr)
    }
}
