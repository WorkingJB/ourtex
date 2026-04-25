/// First non-empty `# heading`, falling back to the id.
/// Duplicates `ourtex-index::title` — the function is tiny and both crates
/// want the same rule. Keeping them independent avoids exposing an internal
/// helper from the index crate.
pub fn derive_title(body: &str, fallback_id: &str) -> String {
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    fallback_id.to_string()
}
