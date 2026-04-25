use crate::error::{AuthError, Result};
use ourtex_vault::Visibility;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    labels: BTreeSet<String>,
}

impl Scope {
    pub fn new<I, S>(labels: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let labels: BTreeSet<String> = labels.into_iter().map(Into::into).collect();
        if labels.is_empty() {
            return Err(AuthError::EmptyScope);
        }
        for label in &labels {
            // Validate via Visibility::from_label — accepts built-ins and
            // well-formed custom labels; rejects empty or malformed strings.
            Visibility::from_label(label)
                .map_err(|_| AuthError::InvalidScope(label.clone()))?;
        }
        Ok(Self { labels })
    }

    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.labels.iter().map(String::as_str)
    }

    /// True iff a document with this visibility label is readable under this
    /// scope. The `private` hard floor is enforced because the match is
    /// strictly literal: only an explicit `private` in the scope set allows
    /// `private` documents, and no implicit promotion exists anywhere.
    pub fn allows_label(&self, visibility_label: &str) -> bool {
        self.labels.contains(visibility_label)
    }

    pub fn allows(&self, visibility: &Visibility) -> bool {
        self.allows_label(visibility.as_label())
    }

    pub fn includes_private(&self) -> bool {
        self.labels.contains("private")
    }

    /// Returns a scope narrowed to the intersection with `other`. Used when a
    /// request passes a `scope` argument that may only narrow the token's scope.
    pub fn narrow_to(&self, other: &[String]) -> Result<Self> {
        let other_set: BTreeSet<String> = other.iter().cloned().collect();
        let intersection: BTreeSet<String> =
            self.labels.intersection(&other_set).cloned().collect();
        if intersection.is_empty() {
            return Err(AuthError::EmptyScope);
        }
        Ok(Self {
            labels: intersection,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Read,
    ReadPropose,
}

impl Mode {
    pub fn allows_propose(&self) -> bool {
        matches!(self, Self::ReadPropose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_from_valid_labels() {
        let s = Scope::new(["work", "public"]).unwrap();
        assert!(s.allows_label("work"));
        assert!(s.allows_label("public"));
        assert!(!s.allows_label("personal"));
    }

    #[test]
    fn rejects_empty() {
        assert!(Scope::new(Vec::<String>::new()).is_err());
    }

    #[test]
    fn rejects_invalid_labels() {
        assert!(Scope::new(["UPPER"]).is_err());
        assert!(Scope::new([""]).is_err());
        assert!(Scope::new(["has space"]).is_err());
    }

    #[test]
    fn private_requires_explicit_private() {
        let s = Scope::new(["work", "public", "personal"]).unwrap();
        assert!(!s.allows_label("private"));
        assert!(!s.includes_private());

        let s = Scope::new(["private", "work"]).unwrap();
        assert!(s.allows_label("private"));
        assert!(s.includes_private());
    }

    #[test]
    fn custom_label_containing_private_is_not_the_floor() {
        // A custom label that textually contains "private" is not the
        // hard-floor built-in.
        let s = Scope::new(["semi-private"]).unwrap();
        assert!(!s.allows_label("private"));
        assert!(!s.includes_private());
        assert!(s.allows_label("semi-private"));
    }

    #[test]
    fn narrow_to_intersects() {
        let s = Scope::new(["work", "public"]).unwrap();
        let narrowed = s.narrow_to(&["work".to_string(), "personal".to_string()]).unwrap();
        assert!(narrowed.allows_label("work"));
        assert!(!narrowed.allows_label("public"));
        assert!(!narrowed.allows_label("personal")); // narrowing can't add
    }

    #[test]
    fn narrow_to_disjoint_errors() {
        let s = Scope::new(["work"]).unwrap();
        assert!(s.narrow_to(&["public".to_string()]).is_err());
    }
}
