pub(crate) const MAX_CHARS: usize = 120;

/// Names share trimming and Unicode/control-character limits; each store owns
/// uniqueness and reports its own domain error.
pub(crate) fn normalize(name: &str) -> Option<&str> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= MAX_CHARS && !name.chars().any(char::is_control))
        .then_some(name)
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/unit/names.rs"));
}
