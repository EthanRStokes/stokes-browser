// CSS selector matching for JavaScript bindings
use crate::dom::AttributeMap;

#[derive(Clone, Copy)]
enum SimpleSelector<'a> {
    Universal,
    Id { id: &'a str, class: Option<&'a str>, attr: Option<&'a str> },
    Class { classes: &'a str, attr: Option<&'a str> },
    Attr(&'a str),
    Tag { tag: &'a str, class_list: Option<&'a str>, id: Option<&'a str>, attr: Option<&'a str> },
}

pub struct ParsedSelector<'a> {
    parts: Vec<SimpleSelector<'a>>,
}

pub enum SelectorSeed<'a> {
    Universal,
    Id(&'a str),
    Class(&'a str),
    Tag(&'a str),
    None,
}

pub fn parse_selector(selector: &str) -> ParsedSelector<'_> {
    let mut parts = Vec::new();
    for part in selector.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        parts.push(parse_simple_selector(trimmed));
    }
    ParsedSelector { parts }
}

pub fn matches_parsed_selector(parsed: &ParsedSelector<'_>, tag_name: &str, attributes: &AttributeMap) -> bool {
    parsed
        .parts
        .iter()
        .copied()
        .any(|selector| matches_simple_selector(selector, tag_name, attributes))
}

pub fn selector_seed<'a>(parsed: &'a ParsedSelector<'a>) -> SelectorSeed<'a> {
    if parsed.parts.len() != 1 {
        return SelectorSeed::None;
    }

    match parsed.parts[0] {
        SimpleSelector::Universal => SelectorSeed::Universal,
        SimpleSelector::Id { id, class, attr } if class.is_none() && attr.is_none() => SelectorSeed::Id(id),
        SimpleSelector::Class { classes, attr } if attr.is_none() && !classes.is_empty() && !classes.contains('.') => {
            SelectorSeed::Class(classes)
        }
        SimpleSelector::Tag { tag, class_list, id, attr }
            if class_list.is_none() && id.is_none() && attr.is_none() && !tag.is_empty() => {
            SelectorSeed::Tag(tag)
        }
        _ => SelectorSeed::None,
    }
}

fn parse_simple_selector(selector: &str) -> SimpleSelector<'_> {
    if selector == "*" {
        return SimpleSelector::Universal;
    }

    if let Some(id_selector) = selector.strip_prefix('#') {
        if let Some(dot_pos) = id_selector.find('.') {
            let (id_part, class_part) = id_selector.split_at(dot_pos);
            return SimpleSelector::Id {
                id: id_part,
                class: Some(&class_part[1..]),
                attr: None,
            };
        }
        if let Some(bracket_pos) = id_selector.find('[') {
            return SimpleSelector::Id {
                id: &id_selector[..bracket_pos],
                class: None,
                attr: Some(&id_selector[bracket_pos..]),
            };
        }
        return SimpleSelector::Id {
            id: id_selector,
            class: None,
            attr: None,
        };
    }

    if let Some(class_selector) = selector.strip_prefix('.') {
        if let Some(bracket_pos) = class_selector.find('[') {
            return SimpleSelector::Class {
                classes: &class_selector[..bracket_pos],
                attr: Some(&class_selector[bracket_pos..]),
            };
        }
        return SimpleSelector::Class {
            classes: class_selector,
            attr: None,
        };
    }

    if selector.starts_with('[') {
        return SimpleSelector::Attr(selector);
    }

    if let Some(dot_pos) = selector.find('.') {
        return SimpleSelector::Tag {
            tag: &selector[..dot_pos],
            class_list: Some(&selector[dot_pos + 1..]),
            id: None,
            attr: None,
        };
    }

    if let Some(hash_pos) = selector.find('#') {
        return SimpleSelector::Tag {
            tag: &selector[..hash_pos],
            class_list: None,
            id: Some(&selector[hash_pos + 1..]),
            attr: None,
        };
    }

    if let Some(bracket_pos) = selector.find('[') {
        return SimpleSelector::Tag {
            tag: &selector[..bracket_pos],
            class_list: None,
            id: None,
            attr: Some(&selector[bracket_pos..]),
        };
    }

    SimpleSelector::Tag {
        tag: selector,
        class_list: None,
        id: None,
        attr: None,
    }
}

fn get_attribute<'a>(attributes: &'a AttributeMap, name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attr| attr.name.local.as_ref() == name)
        .map(|attr| attr.value.as_str())
}

fn has_class(class_attr: &str, class_name: &str) -> bool {
    class_attr.split_whitespace().any(|c| c == class_name)
}

fn has_all_classes(class_attr: &str, class_list: &str) -> bool {
    class_list
        .split('.')
        .all(|c| !c.is_empty() && has_class(class_attr, c))
}

fn matches_simple_selector(selector: SimpleSelector<'_>, tag_name: &str, attributes: &AttributeMap) -> bool {
    let id_attr = get_attribute(attributes, "id").unwrap_or("");
    let class_attr = get_attribute(attributes, "class").unwrap_or("");

    match selector {
        SimpleSelector::Universal => true,
        SimpleSelector::Id { id, class, attr } => {
            if id_attr != id {
                return false;
            }
            if let Some(class_name) = class {
                if !has_class(class_attr, class_name) {
                    return false;
                }
            }
            if let Some(attr_selector) = attr {
                return matches_attribute_selector(attr_selector, attributes);
            }
            true
        }
        SimpleSelector::Class { classes, attr } => {
            if classes.contains('.') {
                if !has_all_classes(class_attr, classes) {
                    return false;
                }
            } else if !has_class(class_attr, classes) {
                return false;
            }

            if let Some(attr_selector) = attr {
                return matches_attribute_selector(attr_selector, attributes);
            }
            true
        }
        SimpleSelector::Attr(attr_selector) => matches_attribute_selector(attr_selector, attributes),
        SimpleSelector::Tag { tag, class_list, id, attr } => {
            if !tag.is_empty() && !tag_name.eq_ignore_ascii_case(tag) {
                return false;
            }

            if let Some(classes) = class_list {
                return has_all_classes(class_attr, classes);
            }

            if let Some(expected_id) = id {
                return id_attr == expected_id;
            }

            if let Some(attr_selector) = attr {
                return matches_attribute_selector(attr_selector, attributes);
            }

            true
        }
    }
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
                "|=" => {
                    val == attr_value
                        || (val.starts_with(attr_value)
                            && val.as_bytes().get(attr_value.len()) == Some(&b'-'))
                }
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

