use crate::error::{Result, VaultError};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentId(String);

impl DocumentId {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        if !Self::is_valid(&s) {
            return Err(VaultError::InvalidId(s));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_valid(s: &str) -> bool {
        let len = s.len();
        if !(1..=64).contains(&len) {
            return false;
        }
        let mut chars = s.chars();
        let first = chars.next().expect("length checked");
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return false;
        }
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for DocumentId {
    fn serialize<S: serde::Serializer>(
        &self,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DocumentId {
    fn deserialize<D: serde::Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        DocumentId::new(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_ids() {
        for ok in &["a", "0", "rel-jane-smith", "goal-q2-launch", "me"] {
            assert!(DocumentId::new(*ok).is_ok(), "expected {ok:?} to be valid");
        }
    }

    #[test]
    fn rejects_invalid_ids() {
        let too_long = "a".repeat(65);
        let bad = [
            "",                // empty
            "-leading-dash",   // leading dash
            "has_underscore",  // underscore
            "HasUppercase",
            "has space",
            too_long.as_str(), // too long
        ];
        for b in bad {
            assert!(DocumentId::new(b).is_err(), "expected {b:?} to be invalid");
        }
    }
}
