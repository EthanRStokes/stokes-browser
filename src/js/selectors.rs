// CSS selector matching for JavaScript bindings
use crate::dom::AttributeMap;

/// Basic CSS selector matching for single selectors
/// Supports: tag, .class, #id, tag.class, tag#id, [attr], [attr=value], and various attribute operators
// TODO: Doesn't support complex selectors (descendant, child, sibling combinators, :pseudo-classes, ::pseudo-elements)
pub fn matches_selector(selector: &str, tag_name: &str, attributes: &AttributeMap) -> bool {
    // Handle comma-separated selectors (any match)
    if selector.contains(',') {
        return selector
            .split(',')
            .any(|s| matches_selector(s.trim(), tag_name, attributes));
    }

    // Get element's id and class
    let id_attr = attributes
        .iter()
        .find(|attr| attr.name.local.as_ref() == "id")
        .map(|attr| attr.value.as_str())
        .unwrap_or("");
    let class_attr = attributes
        .iter()
        .find(|attr| attr.name.local.as_ref() == "class")
        .map(|attr| attr.value.as_str())
        .unwrap_or("");
    let classes: Vec<&str> = class_attr.split_whitespace().collect();

    let selector = selector.trim();

    // ID selector: #id
    if selector.starts_with('#') {
        let id = &selector[1..];
        // Could be #id.class or #id[attr]
        if let Some(dot_pos) = id.find('.') {
            let (id_part, class_part) = id.split_at(dot_pos);
            return id_attr == id_part && classes.contains(&&class_part[1..]);
        }
        if let Some(bracket_pos) = id.find('[') {
            let id_part = &id[..bracket_pos];
            return id_attr == id_part
                && matches_attribute_selector(&id[bracket_pos..], attributes);
        }
        return id_attr == id;
    }

    // Class selector: .class
    if selector.starts_with('.') {
        let class_selector = &selector[1..];
        // Could be .class1.class2
        if class_selector.contains('.') {
            return class_selector
                .split('.')
                .all(|c| !c.is_empty() && classes.contains(&c));
        }
        // Could be .class[attr]
        if let Some(bracket_pos) = class_selector.find('[') {
            let class_part = &class_selector[..bracket_pos];
            return classes.contains(&class_part)
                && matches_attribute_selector(&class_selector[bracket_pos..], attributes);
        }
        return classes.contains(&class_selector);
    }

    // Attribute selector: [attr] or [attr=value]
    if selector.starts_with('[') {
        return matches_attribute_selector(selector, attributes);
    }

    // Tag selector: tag, tag.class, tag#id, tag[attr]
    let tag_lower = tag_name.to_lowercase();

    // Handle tag.class
    if let Some(dot_pos) = selector.find('.') {
        let tag_part = &selector[..dot_pos];
        let class_part = &selector[dot_pos + 1..];
        if !tag_part.is_empty() && tag_lower != tag_part.to_lowercase() {
            return false;
        }
        // Handle multiple classes: tag.class1.class2
        return class_part
            .split('.')
            .all(|c| !c.is_empty() && classes.contains(&c));
    }

    // Handle tag#id
    if let Some(hash_pos) = selector.find('#') {
        let tag_part = &selector[..hash_pos];
        let id_part = &selector[hash_pos + 1..];
        if !tag_part.is_empty() && tag_lower != tag_part.to_lowercase() {
            return false;
        }
        return id_attr == id_part;
    }

    // Handle tag[attr]
    if let Some(bracket_pos) = selector.find('[') {
        let tag_part = &selector[..bracket_pos];
        if !tag_part.is_empty() && tag_lower != tag_part.to_lowercase() {
            return false;
        }
        return matches_attribute_selector(&selector[bracket_pos..], attributes);
    }

    // Simple tag match
    if selector == "*" {
        return true;
    }
    tag_lower == selector.to_lowercase()
}

/// Match an attribute selector like [attr], [attr=value], [attr^=value], etc.
fn matches_attribute_selector(selector: &str, attributes: &AttributeMap) -> bool {
    if !selector.starts_with('[') || !selector.ends_with(']') {
        return false;
    }
    let inner = &selector[1..selector.len() - 1];

    // [attr=value] or [attr="value"]
    if let Some(eq_pos) = inner.find('=') {
        let operator_start = if eq_pos > 0 {
            match inner.chars().nth(eq_pos - 1) {
                Some('^') | Some('$') | Some('*') | Some('~') | Some('|') => eq_pos - 1,
                _ => eq_pos,
            }
        } else {
            eq_pos
        };

        let attr_name = &inner[..operator_start];
        let operator = &inner[operator_start..eq_pos + 1];
        let mut attr_value = &inner[eq_pos + 1..];

        // Remove quotes if present
        if (attr_value.starts_with('"') && attr_value.ends_with('"'))
            || (attr_value.starts_with('\'') && attr_value.ends_with('\''))
        {
            attr_value = &attr_value[1..attr_value.len() - 1];
        }

        let actual_value = attributes
            .iter()
            .find(|attr| attr.name.local.as_ref() == attr_name)
            .map(|attr| attr.value.as_str());

        match actual_value {
            Some(val) => match operator {
                "=" => val == attr_value,
                "^=" => val.starts_with(attr_value),
                "$=" => val.ends_with(attr_value),
                "*=" => val.contains(attr_value),
                "~=" => val.split_whitespace().any(|v| v == attr_value),
                "|=" => val == attr_value || val.starts_with(&format!("{}-", attr_value)),
                _ => false,
            },
            None => false,
        }
    } else {
        // [attr] - just check if attribute exists
        attributes
            .iter()
            .any(|attr| attr.name.local.as_ref() == inner)
    }
}

#[cfg(test)]
mod tests {

    // Add tests here if needed
}

