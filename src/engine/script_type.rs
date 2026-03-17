use crate::engine::js_provider::ScriptKind;

fn script_type_essence(script_type: &str) -> &str {
    script_type.split(';').next().unwrap_or("").trim()
}

fn is_javascript_mime_essence(essence: &str) -> bool {
    essence.eq_ignore_ascii_case("text/javascript")
        || essence.eq_ignore_ascii_case("application/javascript")
        || essence.eq_ignore_ascii_case("text/ecmascript")
        || essence.eq_ignore_ascii_case("application/ecmascript")
        // Legacy JavaScript MIME types still recognized by browsers.
        || essence.eq_ignore_ascii_case("text/javascript1.0")
        || essence.eq_ignore_ascii_case("text/javascript1.1")
        || essence.eq_ignore_ascii_case("text/javascript1.2")
        || essence.eq_ignore_ascii_case("text/javascript1.3")
        || essence.eq_ignore_ascii_case("text/javascript1.4")
        || essence.eq_ignore_ascii_case("text/javascript1.5")
        || essence.eq_ignore_ascii_case("text/jscript")
        || essence.eq_ignore_ascii_case("text/livescript")
        || essence.eq_ignore_ascii_case("text/x-ecmascript")
        || essence.eq_ignore_ascii_case("text/x-javascript")
}

/// Returns the executable script kind for a `<script type>` attribute.
///
/// Browser parity:
/// - missing/empty type => classic script
/// - `module` => module script
/// - JavaScript MIME types (optionally with parameters) => classic script
/// - any other non-empty type => data block (non-executable)
pub(crate) fn executable_script_kind(script_type: Option<&str>) -> Option<ScriptKind> {
    if matches!(script_type.map(str::trim), Some(t) if t.eq_ignore_ascii_case("module")) {
        return Some(ScriptKind::Module);
    }

    match script_type.map(str::trim).filter(|t| !t.is_empty()) {
        None => Some(ScriptKind::Classic),
        Some(value) => {
            if is_javascript_mime_essence(script_type_essence(value)) {
                Some(ScriptKind::Classic)
            } else {
                None
            }
        }
    }
}
