// CSS parser implementation
use super::{CssValue, Declaration, PropertyName, Rule, Selector, Stylesheet};

/// Simple CSS parser
pub struct CssParser;

impl CssParser {
    #[inline]
    pub fn new() -> Self {
        Self
    }

    /// Parse a CSS string into a stylesheet
    pub fn parse(&self, css: &str) -> Stylesheet {
        let mut stylesheet = Stylesheet::new();

        // Parse rules directly without creating intermediate strings
        let rules = self.split_into_rules(css.as_bytes());

        for (start, end) in rules {
            // SAFETY: We know these are valid UTF-8 byte ranges from the original string
            let rule_text = unsafe { std::str::from_utf8_unchecked(&css.as_bytes()[start..end]) };
            if let Some(rule) = self.parse_rule(rule_text) {
                stylesheet.add_rule(rule);
            }
        }

        stylesheet
    }

    /// Split CSS into individual rules while skipping comments
    fn split_into_rules(&self, css: &[u8]) -> Vec<(usize, usize)> {
        let mut rules = Vec::with_capacity(32); // Pre-allocate for typical stylesheets
        let mut rule_start = 0;
        let mut current_pos = 0;
        let mut brace_depth = 0;
        let mut in_string = false;
        let mut string_char = b'"';
        let mut in_at_rule = false;
        let mut in_comment = false;

        while current_pos < css.len() {
            let byte = css[current_pos];

            // Handle comments
            if !in_string && !in_comment && byte == b'/' && current_pos + 1 < css.len() && css[current_pos + 1] == b'*' {
                in_comment = true;
                current_pos += 2;
                continue;
            }

            if in_comment {
                if byte == b'*' && current_pos + 1 < css.len() && css[current_pos + 1] == b'/' {
                    in_comment = false;
                    current_pos += 2;
                } else {
                    current_pos += 1;
                }
                continue;
            }

            // Handle strings
            if !in_string && (byte == b'"' || byte == b'\'') {
                in_string = true;
                string_char = byte;
            } else if in_string && byte == string_char {
                // Check for escape
                if current_pos > 0 && css[current_pos - 1] != b'\\' {
                    in_string = false;
                }
            } else if !in_string {
                match byte {
                    b'@' if brace_depth == 0 => {
                        in_at_rule = true;
                    }
                    b'{' => {
                        brace_depth += 1;
                    }
                    b'}' => {
                        brace_depth -= 1;
                        if brace_depth == 0 && !in_at_rule {
                            // Find actual content bounds (skip whitespace)
                            let mut start = rule_start;
                            let end = current_pos + 1;

                            while start < end && css[start].is_ascii_whitespace() {
                                start += 1;
                            }

                            if start < end {
                                rules.push((start, end));
                            }

                            rule_start = current_pos + 1;
                            in_at_rule = false;
                        } else if brace_depth == 0 {
                            rule_start = current_pos + 1;
                            in_at_rule = false;
                        }
                    }
                    _ => {}
                }
            }

            current_pos += 1;
        }

        rules
    }

    /// Parse a single CSS rule
    #[inline]
    fn parse_rule(&self, rule_text: &str) -> Option<Rule> {
        // Find the opening brace using memchr-like search
        let brace_pos = rule_text.as_bytes().iter().position(|&b| b == b'{')?;

        let selector_part = rule_text[..brace_pos].trim();

        // Find the closing brace from the end
        let close_brace = rule_text.len() - 1;
        if rule_text.as_bytes()[close_brace] != b'}' {
            return None;
        }

        let declarations_part = rule_text[brace_pos + 1..close_brace].trim();

        // Parse selectors
        let selectors = self.parse_selectors(selector_part);
        if selectors.is_empty() {
            return None;
        }

        // Parse declarations
        let declarations = self.parse_declarations(declarations_part);

        Some(Rule::new(selectors, declarations))
    }

    /// Parse CSS selectors (comma-separated)
    #[inline]
    fn parse_selectors(&self, selector_text: &str) -> Vec<Selector> {
        Selector::parse(selector_text)
    }

    /// Parse CSS declarations (optimized)
    fn parse_declarations(&self, declarations_text: &str) -> Vec<Declaration> {
        if declarations_text.is_empty() {
            return Vec::new();
        }

        let mut declarations = Vec::with_capacity(8); // Pre-allocate for common case
        let bytes = declarations_text.as_bytes();
        let mut start = 0;
        let mut in_string = false;
        let mut string_char = b'"';

        for i in 0..bytes.len() {
            let byte = bytes[i];

            // Handle strings
            if !in_string && (byte == b'"' || byte == b'\'') {
                in_string = true;
                string_char = byte;
            } else if in_string && byte == string_char && (i == 0 || bytes[i - 1] != b'\\') {
                in_string = false;
            } else if !in_string && byte == b';' {
                // SAFETY: We know this is valid UTF-8 from the original string
                let decl_text = unsafe { std::str::from_utf8_unchecked(&bytes[start..i]) };
                
                if let Some(declaration) = self.parse_declaration_fast(decl_text) {
                    declarations.push(declaration);
                }

                start = i + 1;
            }
        }

        // Handle last declaration (no trailing semicolon)
        if start < bytes.len() {
            let decl_text = unsafe { std::str::from_utf8_unchecked(&bytes[start..]) };
            
            if let Some(declaration) = self.parse_declaration_fast(decl_text) {
                declarations.push(declaration);
            }
        }

        declarations
    }

    /// Parse a single CSS declaration with fast path
    #[inline]
    fn parse_declaration_fast(&self, decl_text: &str) -> Option<Declaration> {
        // Skip leading whitespace manually to avoid trim allocation
        let bytes = decl_text.as_bytes();
        let mut start = 0;
        let mut end = bytes.len();
        
        // Trim start
        while start < end && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        
        // Trim end
        while end > start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        
        if start >= end {
            return None;
        }
        
        // Find colon using byte search in trimmed range
        let colon_pos = bytes[start..end].iter().position(|&b| b == b':')?;
        let absolute_colon = start + colon_pos;
        
        let property_text = decl_text[start..absolute_colon].trim();
        let value_start = absolute_colon + 1;
        
        // Manually trim value part
        let mut val_start = value_start;
        let mut val_end = end;
        
        while val_start < val_end && bytes[val_start].is_ascii_whitespace() {
            val_start += 1;
        }
        
        while val_end > val_start && bytes[val_end - 1].is_ascii_whitespace() {
            val_end -= 1;
        }
        
        let value_part = &decl_text[val_start..val_end];

        // Check for !important (safe for Unicode)
        let (value_text, important) = if value_part.ends_with("!important") {
            let len = value_part.len() - "!important".len();
            (value_part[..len].trim(), true)
        } else {
            (value_part, false)
        };

        let property = PropertyName::from(property_text);
        let value = CssValue::parse(value_text);

        Some(Declaration::with_important(property, value, important))
    }

    /// Parse inline styles from a style attribute
    #[inline]
    pub fn parse_inline_styles(&self, style_attr: &str) -> Vec<Declaration> {
        self.parse_declarations(style_attr)
    }
}
