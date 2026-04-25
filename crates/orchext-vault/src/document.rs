use crate::error::{Result, VaultError};
use crate::Frontmatter;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct Document {
    pub frontmatter: Frontmatter,
    pub body: String,
}

impl Document {
    pub fn parse(input: &str) -> Result<Self> {
        let (yaml, body) = split_frontmatter(input)?;
        let frontmatter: Frontmatter = serde_yml::from_str(yaml)?;
        Ok(Self {
            frontmatter,
            body: body.to_string(),
        })
    }

    pub fn serialize(&self) -> Result<String> {
        let yaml = serde_yml::to_string(&self.frontmatter)?;
        let mut out = String::with_capacity(yaml.len() + self.body.len() + 16);
        out.push_str("---\n");
        out.push_str(&yaml);
        if !yaml.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("---\n");
        out.push_str(&self.body);
        Ok(out)
    }

    pub fn version(&self) -> Result<String> {
        let serialized = self.serialize()?;
        let mut hasher = Sha256::new();
        hasher.update(serialized.as_bytes());
        Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
    }
}

fn split_frontmatter(input: &str) -> Result<(&str, &str)> {
    let after_open = input
        .strip_prefix("---\n")
        .or_else(|| input.strip_prefix("---\r\n"))
        .ok_or(VaultError::MissingFrontmatter)?;

    let mut offset = 0usize;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            let yaml = &after_open[..offset];
            let rest = &after_open[offset + line.len()..];
            return Ok((yaml, rest));
        }
        offset += line.len();
    }
    Err(VaultError::UnterminatedFrontmatter)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "---\n\
id: rel-jane-smith\n\
type: relationship\n\
visibility: work\n\
tags:\n\
  - manager\n\
  - acme\n\
links:\n\
  - goal-q2-launch\n\
created: 2026-04-18\n\
updated: 2026-04-18\n\
---\n\
# Jane Smith\n\
\n\
My manager at Acme.\n";

    #[test]
    fn parses_example() {
        let doc = Document::parse(EXAMPLE).unwrap();
        assert_eq!(doc.frontmatter.id.as_str(), "rel-jane-smith");
        assert_eq!(doc.frontmatter.type_, "relationship");
        assert_eq!(doc.frontmatter.visibility.as_label(), "work");
        assert_eq!(doc.frontmatter.tags, vec!["manager", "acme"]);
        assert!(doc.body.starts_with("# Jane Smith"));
    }

    #[test]
    fn round_trips() {
        let doc = Document::parse(EXAMPLE).unwrap();
        let serialized = doc.serialize().unwrap();
        let reparsed = Document::parse(&serialized).unwrap();
        assert_eq!(reparsed.frontmatter.id, doc.frontmatter.id);
        assert_eq!(reparsed.frontmatter.type_, doc.frontmatter.type_);
        assert_eq!(reparsed.frontmatter.tags, doc.frontmatter.tags);
        assert_eq!(reparsed.body, doc.body);
    }

    #[test]
    fn preserves_unknown_fields() {
        // Per FORMAT.md §3.4: x-* extensions are preserved on round-trip.
        let input = "---\n\
id: a\n\
type: preferences\n\
visibility: personal\n\
x-plugin-color: \"#ff8800\"\n\
x-plugin-priority: 3\n\
---\n\
body\n";
        let doc = Document::parse(input).unwrap();
        assert!(doc.frontmatter.extras.contains_key("x-plugin-color"));
        assert!(doc.frontmatter.extras.contains_key("x-plugin-priority"));

        let serialized = doc.serialize().unwrap();
        assert!(serialized.contains("x-plugin-color"));
        assert!(serialized.contains("x-plugin-priority"));
    }

    #[test]
    fn rejects_missing_frontmatter() {
        let err = Document::parse("no frontmatter here").unwrap_err();
        assert!(matches!(err, VaultError::MissingFrontmatter));
    }

    #[test]
    fn rejects_unterminated_frontmatter() {
        let err = Document::parse("---\nid: a\ntype: x\nvisibility: work\n").unwrap_err();
        assert!(matches!(err, VaultError::UnterminatedFrontmatter));
    }

    #[test]
    fn version_is_stable() {
        let doc = Document::parse(EXAMPLE).unwrap();
        let v1 = doc.version().unwrap();
        let v2 = doc.version().unwrap();
        assert_eq!(v1, v2);
        assert!(v1.starts_with("sha256:"));
    }
}
