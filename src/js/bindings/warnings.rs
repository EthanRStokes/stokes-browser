use tracing::warn;

/// Logs that a binding is intentionally partial/stubbed.
pub(crate) fn warn_stubbed_binding(binding: &str, detail: &str) {
    warn!("[JS][binding-warning] {binding} called on partial/stubbed binding ({detail})");
}

/// Logs that a binding returned a nullish value where a concrete object/value is expected.
pub(crate) fn warn_unexpected_nullish_return(
    binding: &str,
    actual: &str,
    expected: &str,
    detail: &str,
) {
    warn!(
        "[JS][binding-warning] {binding} returned {actual} unexpectedly (expected {expected}; {detail})"
    );
}

