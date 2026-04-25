use crate::error::{Result, VaultError};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Visibility {
    Public,
    Work,
    Personal,
    Private,
    Custom(String),
}

impl Visibility {
    pub fn from_label(s: &str) -> Result<Self> {
        match s {
            "public" => Ok(Self::Public),
            "work" => Ok(Self::Work),
            "personal" => Ok(Self::Personal),
            "private" => Ok(Self::Private),
            other => {
                if Self::is_valid_label(other) {
                    Ok(Self::Custom(other.to_string()))
                } else {
                    Err(VaultError::InvalidVisibility(other.to_string()))
                }
            }
        }
    }

    pub fn as_label(&self) -> &str {
        match self {
            Self::Public => "public",
            Self::Work => "work",
            Self::Personal => "personal",
            Self::Private => "private",
            Self::Custom(s) => s,
        }
    }

    pub fn is_private(&self) -> bool {
        matches!(self, Self::Private)
    }

    fn is_valid_label(s: &str) -> bool {
        let Some(first) = s.chars().next() else {
            return false;
        };
        if !first.is_ascii_lowercase() {
            return false;
        }
        s.chars()
            .skip(1)
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    }
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_label())
    }
}

impl Serialize for Visibility {
    fn serialize<S: serde::Serializer>(
        &self,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.as_label())
    }
}

impl<'de> Deserialize<'de> for Visibility {
    fn deserialize<D: serde::Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Visibility::from_label(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_builtins() {
        assert!(matches!(Visibility::from_label("public").unwrap(), Visibility::Public));
        assert!(matches!(Visibility::from_label("work").unwrap(), Visibility::Work));
        assert!(matches!(Visibility::from_label("personal").unwrap(), Visibility::Personal));
        assert!(matches!(Visibility::from_label("private").unwrap(), Visibility::Private));
    }

    #[test]
    fn parses_custom_labels() {
        let v = Visibility::from_label("medical").unwrap();
        assert_eq!(v.as_label(), "medical");
        assert!(!v.is_private());
    }

    #[test]
    fn rejects_invalid_labels() {
        for bad in ["", "Medical", "-lead", "1numeric-first", "has space"] {
            assert!(Visibility::from_label(bad).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn only_builtin_private_is_private() {
        // `private` the built-in is the hard floor.
        assert!(Visibility::from_label("private").unwrap().is_private());
        // A user-defined label that happens to contain "private" is not.
        assert!(!Visibility::from_label("semi-private").unwrap().is_private());
    }
}
