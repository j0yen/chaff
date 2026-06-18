/// Regenerable build artifact patterns.
///
/// Each entry is a path prefix (as returned by `git ls-files`) that
/// indicates a regenerable artifact committed into the index.
/// Exposed as a public constant so downstream chaff-* crates can
/// reuse the pattern set rather than re-encoding it.
pub const REGENERABLE: &[&str] = &[
    "target/",
    "node_modules/",
    ".venv/",
    "dist/",
    "__pycache__/",
    ".pytest_cache/",
];

/// File suffix patterns for regenerable artifacts.
pub const REGENERABLE_SUFFIXES: &[&str] = &[".o", ".rlib", ".rmeta"];

/// Returns true if the given git ls-files path matches any regenerable pattern.
pub fn is_regenerable(path: &str) -> bool {
    for prefix in REGENERABLE {
        if path.starts_with(prefix) {
            return true;
        }
    }
    for suffix in REGENERABLE_SUFFIXES {
        if path.ends_with(suffix) {
            return true;
        }
    }
    false
}

/// Returns the first matching regenerable prefix/pattern for a path, or None.
pub fn matching_pattern(path: &str) -> Option<&'static str> {
    for prefix in REGENERABLE {
        if path.starts_with(prefix) {
            return Some(prefix);
        }
    }
    for suffix in REGENERABLE_SUFFIXES {
        if path.ends_with(suffix) {
            return Some(suffix);
        }
    }
    None
}
