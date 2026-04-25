pub fn extract_title(body: &str, fallback_id: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_first_h1() {
        let body = "# Jane Smith\n\nMy manager.\n";
        assert_eq!(extract_title(body, "fallback"), "Jane Smith");
    }

    #[test]
    fn ignores_h2_and_beyond() {
        let body = "## Not an H1\n# Real Title\n";
        assert_eq!(extract_title(body, "fallback"), "Real Title");
    }

    #[test]
    fn falls_back_to_id() {
        assert_eq!(extract_title("", "my-id"), "my-id");
        assert_eq!(extract_title("just some body text\n", "my-id"), "my-id");
    }

    #[test]
    fn trims_leading_whitespace_on_heading_line() {
        assert_eq!(extract_title("   # Indented\n", "x"), "Indented");
    }
}
