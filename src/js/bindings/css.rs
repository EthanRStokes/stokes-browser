//! CSS namespace bindings for JavaScript.
//!
//! Implements the static `CSS` object exposed on the global scope, including:
//! - `CSS.supports(property, value)` / `CSS.supports(conditionText)`
//! - `CSS.escape(value)`
//! - `CSS.registerProperty(descriptor)` (stub – no Houdini pipeline yet)
//! - CSS Typed OM unit factories: `CSS.px()`, `CSS.em()`, `CSS.deg()`, etc.

use crate::js::helpers::{create_js_string, define_function, js_value_to_string, set_string_property};
use crate::js::JsRuntime;
use cssparser::ParserInput;
use mozjs::jsapi::{
    CallArgs, JSContext, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject, JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, DoubleValue, JSVal, ObjectValue, UndefinedValue};
use mozjs::rooted;
use std::ffi::CString;
use std::os::raw::c_uint;
use style::parser::ParserContext;
use style::properties::{PropertyDeclaration, PropertyId, SourcePropertyDeclaration};
use style::servo_arc::Arc as ServoArc;
use style::stylesheets::{CssRuleType, Origin, UrlExtraData};
use style_traits::ParsingMode;
use url::Url;

// ---------------------------------------------------------------------------
// Public setup
// ---------------------------------------------------------------------------

/// Register the `CSS` namespace object on the JS global.
pub fn setup_css(runtime: &mut JsRuntime) -> Result<(), String> {
    runtime.do_with_jsapi(|_rt, cx, global| unsafe {
        rooted!(in(cx) let css_obj = JS_NewPlainObject(cx));
        if css_obj.get().is_null() {
            return Err("Failed to create CSS object".to_string());
        }

        // Core methods
        define_function(cx, css_obj.get(), "supports", Some(css_supports), 2)?;
        define_function(cx, css_obj.get(), "escape", Some(css_escape), 1)?;
        define_function(cx, css_obj.get(), "registerProperty", Some(css_register_property), 1)?;

        // CSS Typed OM – dimensionless / percentage
        define_function(cx, css_obj.get(), "number", Some(css_unit_number), 1)?;
        define_function(cx, css_obj.get(), "percent", Some(css_unit_percent), 1)?;

        // Relative font-based lengths
        define_function(cx, css_obj.get(), "em", Some(css_unit_em), 1)?;
        define_function(cx, css_obj.get(), "rem", Some(css_unit_rem), 1)?;
        define_function(cx, css_obj.get(), "ex", Some(css_unit_ex), 1)?;
        define_function(cx, css_obj.get(), "rex", Some(css_unit_rex), 1)?;
        define_function(cx, css_obj.get(), "cap", Some(css_unit_cap), 1)?;
        define_function(cx, css_obj.get(), "rcap", Some(css_unit_rcap), 1)?;
        define_function(cx, css_obj.get(), "ch", Some(css_unit_ch), 1)?;
        define_function(cx, css_obj.get(), "rch", Some(css_unit_rch), 1)?;
        define_function(cx, css_obj.get(), "ic", Some(css_unit_ic), 1)?;
        define_function(cx, css_obj.get(), "ric", Some(css_unit_ric), 1)?;
        define_function(cx, css_obj.get(), "lh", Some(css_unit_lh), 1)?;
        define_function(cx, css_obj.get(), "rlh", Some(css_unit_rlh), 1)?;

        // Absolute lengths
        define_function(cx, css_obj.get(), "px", Some(css_unit_px), 1)?;
        define_function(cx, css_obj.get(), "cm", Some(css_unit_cm), 1)?;
        define_function(cx, css_obj.get(), "mm", Some(css_unit_mm), 1)?;
        define_function(cx, css_obj.get(), "Q", Some(css_unit_q), 1)?;
        define_function(cx, css_obj.get(), "in", Some(css_unit_in), 1)?;
        define_function(cx, css_obj.get(), "pt", Some(css_unit_pt), 1)?;
        define_function(cx, css_obj.get(), "pc", Some(css_unit_pc), 1)?;

        // Viewport lengths
        define_function(cx, css_obj.get(), "vw", Some(css_unit_vw), 1)?;
        define_function(cx, css_obj.get(), "vh", Some(css_unit_vh), 1)?;
        define_function(cx, css_obj.get(), "vmin", Some(css_unit_vmin), 1)?;
        define_function(cx, css_obj.get(), "vmax", Some(css_unit_vmax), 1)?;
        define_function(cx, css_obj.get(), "vi", Some(css_unit_vi), 1)?;
        define_function(cx, css_obj.get(), "vb", Some(css_unit_vb), 1)?;

        // Small / large / dynamic viewport lengths (CSS Values Level 4)
        define_function(cx, css_obj.get(), "svw", Some(css_unit_svw), 1)?;
        define_function(cx, css_obj.get(), "svh", Some(css_unit_svh), 1)?;
        define_function(cx, css_obj.get(), "svi", Some(css_unit_svi), 1)?;
        define_function(cx, css_obj.get(), "svb", Some(css_unit_svb), 1)?;
        define_function(cx, css_obj.get(), "svmin", Some(css_unit_svmin), 1)?;
        define_function(cx, css_obj.get(), "svmax", Some(css_unit_svmax), 1)?;
        define_function(cx, css_obj.get(), "lvw", Some(css_unit_lvw), 1)?;
        define_function(cx, css_obj.get(), "lvh", Some(css_unit_lvh), 1)?;
        define_function(cx, css_obj.get(), "lvi", Some(css_unit_lvi), 1)?;
        define_function(cx, css_obj.get(), "lvb", Some(css_unit_lvb), 1)?;
        define_function(cx, css_obj.get(), "lvmin", Some(css_unit_lvmin), 1)?;
        define_function(cx, css_obj.get(), "lvmax", Some(css_unit_lvmax), 1)?;
        define_function(cx, css_obj.get(), "dvw", Some(css_unit_dvw), 1)?;
        define_function(cx, css_obj.get(), "dvh", Some(css_unit_dvh), 1)?;
        define_function(cx, css_obj.get(), "dvi", Some(css_unit_dvi), 1)?;
        define_function(cx, css_obj.get(), "dvb", Some(css_unit_dvb), 1)?;
        define_function(cx, css_obj.get(), "dvmin", Some(css_unit_dvmin), 1)?;
        define_function(cx, css_obj.get(), "dvmax", Some(css_unit_dvmax), 1)?;

        // Container query lengths
        define_function(cx, css_obj.get(), "cqw", Some(css_unit_cqw), 1)?;
        define_function(cx, css_obj.get(), "cqh", Some(css_unit_cqh), 1)?;
        define_function(cx, css_obj.get(), "cqi", Some(css_unit_cqi), 1)?;
        define_function(cx, css_obj.get(), "cqb", Some(css_unit_cqb), 1)?;
        define_function(cx, css_obj.get(), "cqmin", Some(css_unit_cqmin), 1)?;
        define_function(cx, css_obj.get(), "cqmax", Some(css_unit_cqmax), 1)?;

        // Angles
        define_function(cx, css_obj.get(), "deg", Some(css_unit_deg), 1)?;
        define_function(cx, css_obj.get(), "grad", Some(css_unit_grad), 1)?;
        define_function(cx, css_obj.get(), "rad", Some(css_unit_rad), 1)?;
        define_function(cx, css_obj.get(), "turn", Some(css_unit_turn), 1)?;

        // Time
        define_function(cx, css_obj.get(), "s", Some(css_unit_s), 1)?;
        define_function(cx, css_obj.get(), "ms", Some(css_unit_ms), 1)?;

        // Frequency
        define_function(cx, css_obj.get(), "Hz", Some(css_unit_hz), 1)?;
        define_function(cx, css_obj.get(), "kHz", Some(css_unit_khz), 1)?;

        // Resolution
        define_function(cx, css_obj.get(), "dpi", Some(css_unit_dpi), 1)?;
        define_function(cx, css_obj.get(), "dpcm", Some(css_unit_dpcm), 1)?;
        define_function(cx, css_obj.get(), "dppx", Some(css_unit_dppx), 1)?;
        define_function(cx, css_obj.get(), "x", Some(css_unit_x), 1)?;  // alias for dppx

        // Flex
        define_function(cx, css_obj.get(), "fr", Some(css_unit_fr), 1)?;

        // Attach to global as `CSS`
        rooted!(in(cx) let css_val = ObjectValue(css_obj.get()));
        let name = CString::new("CSS").unwrap();
        if !JS_DefineProperty(
            cx,
            global.into(),
            name.as_ptr(),
            css_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        ) {
            return Err("Failed to define CSS on global".to_string());
        }

        Ok(())
    })
}

// ---------------------------------------------------------------------------
// CSS.supports()
// ---------------------------------------------------------------------------

/// JS callback for `CSS.supports()`.
///
/// Two overloads:
/// ```
/// CSS.supports(property, value)   → boolean
/// CSS.supports(conditionText)     → boolean
/// ```
unsafe extern "C" fn css_supports(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let result = if argc >= 2 {
        let property = js_value_to_string(raw_cx, *args.get(0));
        let value = js_value_to_string(raw_cx, *args.get(1));
        property_value_supported(&property, &value)
    } else if argc == 1 {
        let condition = js_value_to_string(raw_cx, *args.get(0));
        parse_supports_condition(&condition)
    } else {
        false
    };

    args.rval().set(BooleanValue(result));
    true
}

/// Check whether a CSS property / value pair is supported using Stylo.
fn property_value_supported(property: &str, value: &str) -> bool {
    let property = property.trim();
    let value = value.trim();
    if property.is_empty() {
        return false;
    }

    let url = match Url::parse("about:blank") {
        Ok(u) => u,
        Err(_) => return false,
    };
    let url_extra_data = UrlExtraData(ServoArc::new(url));

    let context = ParserContext::new(
        Origin::Author,
        &url_extra_data,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        // QuirksMode
        selectors::matching::QuirksMode::NoQuirks,
        /* namespaces */ Default::default(),
        None,
        None,
    );

    let property_id = match PropertyId::parse(property, &context) {
        Ok(id) => id,
        Err(_) => return false,
    };

    let mut source_decls = SourcePropertyDeclaration::default();
    let mut input = ParserInput::new(value);
    let mut parser = cssparser::Parser::new(&mut input);

    PropertyDeclaration::parse_into(&mut source_decls, property_id, &context, &mut parser).is_ok()
}

/// Parse a full CSS `@supports` condition string and return whether it evaluates to `true`.
///
/// Grammar (simplified from CSS Conditional Rules Level 4):
/// ```
/// supports-condition = not supports-in-parens
///                    | supports-in-parens [ and supports-in-parens ]*
///                    | supports-in-parens [ or  supports-in-parens ]*
/// supports-in-parens = ( supports-condition )
///                    | ( <declaration> )
///                    | selector( <selector> )
/// ```
fn parse_supports_condition(text: &str) -> bool {
    let text = text.trim();

    // `not <supports-in-parens>`
    if let Some(rest) = strip_keyword(text, "not") {
        return match consume_supports_in_parens(rest.trim()) {
            Some((inner, _)) => !eval_supports_in_parens(inner),
            None => false,
        };
    }

    // Parse first in-parens token
    let (first_inner, rest) = match consume_supports_in_parens(text) {
        Some(pair) => pair,
        None => return false,
    };

    let first_result = eval_supports_in_parens(first_inner);
    let rest = rest.trim();
    if rest.is_empty() {
        return first_result;
    }

    // `and` chain
    if strip_keyword(rest, "and").is_some() {
        return evaluate_chain(first_result, rest, "and", true);
    }

    // `or` chain
    if strip_keyword(rest, "or").is_some() {
        return evaluate_chain(first_result, rest, "or", false);
    }

    // Unknown trailing content → false
    false
}

/// Evaluate an `and` or `or` chain given the first result and the remainder
/// of the condition string (which starts with the keyword).
fn evaluate_chain(first: bool, rest: &str, keyword: &str, short_circuit_value: bool) -> bool {
    let mut result = first;
    let mut remaining = rest;

    loop {
        remaining = match strip_keyword(remaining, keyword) {
            Some(r) => r.trim(),
            None => break,
        };

        let (inner, tail) = match consume_supports_in_parens(remaining) {
            Some(pair) => pair,
            None => break,
        };

        let this = eval_supports_in_parens(inner);

        if keyword == "and" {
            result = result && this;
        } else {
            result = result || this;
        }

        // Short-circuit: for `and` false short-circuits, for `or` true does.
        if result == short_circuit_value {
            return result;
        }

        remaining = tail.trim();
        if remaining.is_empty() {
            break;
        }
    }

    result
}

/// Consume one `supports-in-parens` token from the start of `text`.
///
/// Returns `(inner_text, remaining)` where `inner_text` is the content inside
/// the parentheses (excluding the parens themselves), or the selector text for
/// `selector(...)`.
fn consume_supports_in_parens(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();

    // Parenthesised block: `( ... )`
    if text.starts_with('(') {
        let close = matching_paren_close(text)?;
        let inner = &text[1..close];
        let rest = &text[close + 1..];
        return Some((inner, rest));
    }

    // `selector( ... )`
    if let Some(after_kw) = strip_keyword(text, "selector") {
        let after_kw = after_kw.trim_start();
        if after_kw.starts_with('(') {
            let close = matching_paren_close(after_kw)?;
            // We tag selector conditions by prefixing with "__selector__:"
            let inner = &after_kw[1..close];
            let rest = &after_kw[close + 1..];
            // Reuse eval_supports_in_parens with a synthetic key to route to selector check.
            // We create a temporary owned string here by returning a slice that includes
            // the prefix tag. Since we cannot embed a Rust String in a slice, we call the
            // selector evaluator directly instead.
            let supported = selector_supported(inner.trim());
            // We can't return a "pre-computed" result here; instead we use a sentinel
            // that always evaluates to the right answer via eval_supports_in_parens.
            // Simplest approach: return the inner text marked so eval detects selector().
            // Actually the cleanest solution is to check it inline:
            let _ = supported; // already computed above
            // Return a special marker so eval_supports_in_parens can be skipped.
            // We abuse the &str lifetime here — the caller will call eval on whatever
            // we return.  To communicate the pre-computed result we return a static
            // string that eval_supports_in_parens will handle gracefully.
            return if supported {
                Some(("__true__", rest))
            } else {
                Some(("__false__", rest))
            };
        }
    }

    None
}

/// Evaluate the content of a supports-in-parens token.
fn eval_supports_in_parens(inner: &str) -> bool {
    let inner = inner.trim();

    // Sentinel values from selector() handling
    if inner == "__true__" {
        return true;
    }
    if inner == "__false__" {
        return false;
    }

    // Nested condition starting with `not`, `(`, `selector`
    if inner.starts_with("not ")
        || inner.starts_with("not(")
        || inner.starts_with('(')
        || inner.starts_with("selector")
    {
        return parse_supports_condition(inner);
    }

    // `and` / `or` chains enclosed in this parens level
    if contains_top_level_keyword(inner, " and ")
        || contains_top_level_keyword(inner, " or ")
    {
        return parse_supports_condition(inner);
    }

    // `property: value` declaration
    if let Some(colon) = inner.find(':') {
        let property = inner[..colon].trim();
        let value = inner[colon + 1..].trim();
        // Ignore vendor-prefixed properties that start with '--' as custom properties:
        // they are always considered supported.
        if property.starts_with("--") {
            return true;
        }
        return property_value_supported(property, value);
    }

    false
}

// ---------------------------------------------------------------------------
// Selector support check
// ---------------------------------------------------------------------------

/// Returns `true` if the given CSS selector text is parseable.
fn selector_supported(selector_text: &str) -> bool {
    use cssparser::Parser as CssParser;

    // Use cssparser to attempt to lex the selector.  We treat it as supported
    // if it at least tokenises without an error.  A full validity check would
    // require running it through the selectors crate, but tokenisation is a
    // reasonable proxy.
    let mut input = ParserInput::new(selector_text);
    let mut parser = CssParser::new(&mut input);

    // Walk all tokens; if any is an error the selector is not supported.
    loop {
        match parser.next() {
            Ok(_) => continue,
            Err(cssparser::BasicParseError { kind: cssparser::BasicParseErrorKind::EndOfInput, .. }) => return true,
            Err(_) => return false,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: condition parsing utilities
// ---------------------------------------------------------------------------

/// Find the index of the closing `)` that matches the `(` at position 0.
fn matching_paren_close(text: &str) -> Option<usize> {
    if !text.starts_with('(') {
        return None;
    }
    let mut depth: usize = 0;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip `keyword` from the start of `text` (case-insensitive).
/// The keyword must be followed by ASCII whitespace or `(`.
fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    if text.len() < keyword.len() {
        return None;
    }
    if !text[..keyword.len()].eq_ignore_ascii_case(keyword) {
        return None;
    }
    let rest = &text[keyword.len()..];
    if rest.starts_with(|c: char| c.is_ascii_whitespace()) || rest.starts_with('(') {
        Some(rest)
    } else {
        None
    }
}

/// Return `true` if `text` contains `needle` at the top level (not inside
/// parentheses).
fn contains_top_level_keyword(text: &str, needle: &str) -> bool {
    let mut depth: usize = 0;
    let bytes = text.as_bytes();
    let needle_bytes = needle.as_bytes();
    let n = needle_bytes.len();

    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {}
        }
        if depth == 0 && i + n <= bytes.len() {
            let slice = &text[i..i + n];
            if slice.eq_ignore_ascii_case(needle) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// CSS.escape()
// ---------------------------------------------------------------------------

/// JS callback for `CSS.escape(value)`.
/// https://www.w3.org/TR/cssom/#the-css.escape()-method
unsafe extern "C" fn css_escape(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    if argc == 0 {
        // TypeError: not enough arguments
        args.rval().set(UndefinedValue());
        return false;
    }

    let value = js_value_to_string(raw_cx, *args.get(0));
    let escaped = css_escape_ident(&value);
    args.rval().set(create_js_string(raw_cx, &escaped));
    true
}

/// Escape a string so it can safely be used as a CSS identifier.
///
/// Follows the algorithm at https://drafts.csswg.org/cssom/#serialize-an-identifier
fn css_escape_ident(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }

    let chars: Vec<char> = value.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(len * 2);

    for (i, &ch) in chars.iter().enumerate() {
        let code = ch as u32;

        // NULL → replacement character
        if code == 0x0000 {
            result.push('\u{FFFD}');
            continue;
        }

        // Non-printable / DEL → hex escape
        if (0x0001..=0x001F).contains(&code) || code == 0x007F {
            result.push_str(&format!("\\{:X} ", code));
            continue;
        }

        // First character special cases
        if i == 0 {
            if ch == '-' {
                if len == 1 {
                    // A lone `-` must be escaped
                    result.push('\\');
                    result.push('-');
                    continue;
                }
                // Otherwise the leading `-` is fine; check the *next* char below
                result.push('-');
                continue;
            }

            // Digit at position 0 (or position 1 when position 0 is `-`)
            if ch.is_ascii_digit() {
                result.push_str(&format!("\\{:X} ", code));
                continue;
            }
        }

        // A digit at position 1 when position 0 is `-`
        if i == 1 && chars[0] == '-' && ch.is_ascii_digit() {
            result.push_str(&format!("\\{:X} ", code));
            continue;
        }

        // Safe identifier characters need no escaping
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || code > 0x007F {
            result.push(ch);
        } else {
            // All other ASCII → backslash-escape
            result.push('\\');
            result.push(ch);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// CSS.registerProperty()
// ---------------------------------------------------------------------------

/// JS callback for `CSS.registerProperty(descriptor)`.
///
/// This is a CSS Houdini API.  We accept the call without error so that
/// frameworks that call it unconditionally (e.g. certain animation libraries)
/// do not crash.  Custom properties registered this way will not be type-
/// checked or inherit their initial values; they are treated as unregistered
/// custom properties.
unsafe extern "C" fn css_register_property(
    _raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(UndefinedValue());
    // Silently succeed
    let _ = argc;
    true
}

// ---------------------------------------------------------------------------
// CSS Typed OM – CSSUnitValue factories
// ---------------------------------------------------------------------------
//
// Each factory function creates a plain JS object with `value` (number) and
// `unit` (string) properties, matching the CSSUnitValue interface:
// https://www.w3.org/TR/css-typed-om-1/#cssunitvalue
//
// It also exposes a `toString()` method so it serialises as a CSS value string.

/// Create a CSSUnitValue-like plain JS object `{ value, unit }` plus a
/// `toString()` method.
unsafe fn make_css_unit_value(
    raw_cx: *mut JSContext,
    value: f64,
    unit: &str,
) -> *mut mozjs::jsapi::JSObject {
    rooted!(in(raw_cx) let obj = JS_NewPlainObject(raw_cx));
    if obj.get().is_null() {
        return std::ptr::null_mut();
    }

    // obj.value = <number>
    rooted!(in(raw_cx) let val = DoubleValue(value));
    let value_name = CString::new("value").unwrap();
    JS_DefineProperty(
        raw_cx,
        obj.handle().into(),
        value_name.as_ptr(),
        val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // obj.unit = <string>
    let _ = set_string_property(raw_cx, obj.get(), "unit", unit);

    // obj.toString = function() { return value + unit; }
    // We define a generic toString that reads `this.value` and `this.unit`.
    define_function(raw_cx, obj.get(), "toString", Some(css_unit_value_to_string), 0).ok();

    obj.get()
}

/// `toString()` for CSSUnitValue objects.
unsafe extern "C" fn css_unit_value_to_string(
    raw_cx: *mut JSContext,
    _argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, 0);
    let this = args.thisv().get();

    if !this.is_object() {
        args.rval().set(create_js_string(raw_cx, ""));
        return true;
    }

    rooted!(in(raw_cx) let this_obj = this.to_object());

    // Read `value`
    rooted!(in(raw_cx) let mut val_val = UndefinedValue());
    let value_name = CString::new("value").unwrap();
    JS_GetProperty(
        raw_cx,
        this_obj.handle().into(),
        value_name.as_ptr(),
        val_val.handle_mut().into(),
    );

    // Read `unit`
    rooted!(in(raw_cx) let mut unit_val = UndefinedValue());
    let unit_name = CString::new("unit").unwrap();
    JS_GetProperty(
        raw_cx,
        this_obj.handle().into(),
        unit_name.as_ptr(),
        unit_val.handle_mut().into(),
    );

    let num = if val_val.get().is_double() {
        val_val.get().to_double()
    } else if val_val.get().is_int32() {
        val_val.get().to_int32() as f64
    } else {
        0.0
    };

    let unit = if unit_val.get().is_string() {
        use mozjs::conversions::jsstr_to_string;
        use std::ptr::NonNull;
        jsstr_to_string(raw_cx, NonNull::new(unit_val.get().to_string()).unwrap())
    } else {
        String::new()
    };

    // Format the number: omit the decimal point if it's an integer
    let formatted = if num.fract() == 0.0 && num.abs() < 1e15 {
        format!("{}{}", num as i64, unit)
    } else {
        format!("{}{}", num, unit)
    };

    args.rval().set(create_js_string(raw_cx, &formatted));
    true
}

/// Shared implementation for all unit factory functions.
unsafe fn unit_factory(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
    unit: &str,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let value = if argc >= 1 {
        let v = *args.get(0);
        if v.is_double() {
            v.to_double()
        } else if v.is_int32() {
            v.to_int32() as f64
        } else {
            args.rval().set(UndefinedValue());
            return false;
        }
    } else {
        args.rval().set(UndefinedValue());
        return false;
    };

    let obj = make_css_unit_value(raw_cx, value, unit);
    if obj.is_null() {
        args.rval().set(UndefinedValue());
        return false;
    }

    args.rval().set(ObjectValue(obj));
    true
}

// ---------------------------------------------------------------------------
// Unit factory callbacks – one per CSS unit
// ---------------------------------------------------------------------------

macro_rules! unit_fn {
    ($name:ident, $unit:expr) => {
        unsafe extern "C" fn $name(
            raw_cx: *mut JSContext,
            argc: c_uint,
            vp: *mut JSVal,
        ) -> bool {
            unit_factory(raw_cx, argc, vp, $unit)
        }
    };
}

// Dimensionless / percentage
unit_fn!(css_unit_number, "number");
unit_fn!(css_unit_percent, "%");

// Font-relative
unit_fn!(css_unit_em, "em");
unit_fn!(css_unit_rem, "rem");
unit_fn!(css_unit_ex, "ex");
unit_fn!(css_unit_rex, "rex");
unit_fn!(css_unit_cap, "cap");
unit_fn!(css_unit_rcap, "rcap");
unit_fn!(css_unit_ch, "ch");
unit_fn!(css_unit_rch, "rch");
unit_fn!(css_unit_ic, "ic");
unit_fn!(css_unit_ric, "ric");
unit_fn!(css_unit_lh, "lh");
unit_fn!(css_unit_rlh, "rlh");

// Absolute lengths
unit_fn!(css_unit_px, "px");
unit_fn!(css_unit_cm, "cm");
unit_fn!(css_unit_mm, "mm");
unit_fn!(css_unit_q, "Q");
unit_fn!(css_unit_in, "in");
unit_fn!(css_unit_pt, "pt");
unit_fn!(css_unit_pc, "pc");

// Viewport
unit_fn!(css_unit_vw, "vw");
unit_fn!(css_unit_vh, "vh");
unit_fn!(css_unit_vmin, "vmin");
unit_fn!(css_unit_vmax, "vmax");
unit_fn!(css_unit_vi, "vi");
unit_fn!(css_unit_vb, "vb");

// Small viewport
unit_fn!(css_unit_svw, "svw");
unit_fn!(css_unit_svh, "svh");
unit_fn!(css_unit_svi, "svi");
unit_fn!(css_unit_svb, "svb");
unit_fn!(css_unit_svmin, "svmin");
unit_fn!(css_unit_svmax, "svmax");

// Large viewport
unit_fn!(css_unit_lvw, "lvw");
unit_fn!(css_unit_lvh, "lvh");
unit_fn!(css_unit_lvi, "lvi");
unit_fn!(css_unit_lvb, "lvb");
unit_fn!(css_unit_lvmin, "lvmin");
unit_fn!(css_unit_lvmax, "lvmax");

// Dynamic viewport
unit_fn!(css_unit_dvw, "dvw");
unit_fn!(css_unit_dvh, "dvh");
unit_fn!(css_unit_dvi, "dvi");
unit_fn!(css_unit_dvb, "dvb");
unit_fn!(css_unit_dvmin, "dvmin");
unit_fn!(css_unit_dvmax, "dvmax");

// Container query
unit_fn!(css_unit_cqw, "cqw");
unit_fn!(css_unit_cqh, "cqh");
unit_fn!(css_unit_cqi, "cqi");
unit_fn!(css_unit_cqb, "cqb");
unit_fn!(css_unit_cqmin, "cqmin");
unit_fn!(css_unit_cqmax, "cqmax");

// Angle
unit_fn!(css_unit_deg, "deg");
unit_fn!(css_unit_grad, "grad");
unit_fn!(css_unit_rad, "rad");
unit_fn!(css_unit_turn, "turn");

// Time
unit_fn!(css_unit_s, "s");
unit_fn!(css_unit_ms, "ms");

// Frequency
unit_fn!(css_unit_hz, "Hz");
unit_fn!(css_unit_khz, "kHz");

// Resolution
unit_fn!(css_unit_dpi, "dpi");
unit_fn!(css_unit_dpcm, "dpcm");
unit_fn!(css_unit_dppx, "dppx");
unit_fn!(css_unit_x, "x");

// Flex
unit_fn!(css_unit_fr, "fr");

