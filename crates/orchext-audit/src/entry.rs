use crate::error::{AuditError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const OWNER_ACTOR: &str = "owner";
pub const TOKEN_ACTOR_PREFIX: &str = "tok:";
pub const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Owner,
    Token(String),
}

impl Actor {
    pub fn as_encoded(&self) -> String {
        match self {
            Self::Owner => OWNER_ACTOR.to_string(),
            Self::Token(id) => format!("{TOKEN_ACTOR_PREFIX}{id}"),
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        if s == OWNER_ACTOR {
            return Ok(Self::Owner);
        }
        if let Some(id) = s.strip_prefix(TOKEN_ACTOR_PREFIX) {
            if id.is_empty() {
                return Err(AuditError::InvalidActor(s.to_string()));
            }
            return Ok(Self::Token(id.to_string()));
        }
        Err(AuditError::InvalidActor(s.to_string()))
    }
}

impl Serialize for Actor {
    fn serialize<S: serde::Serializer>(
        &self,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.as_encoded())
    }
}

impl<'de> Deserialize<'de> for Actor {
    fn deserialize<D: serde::Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Actor::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Denied,
    Error,
}

impl Outcome {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::Error => "error",
        }
    }
}

/// An input record; the writer fills in seq, ts, and hash fields.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub actor: Actor,
    pub action: String,
    pub document_id: Option<String>,
    pub scope_used: Vec<String>,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub actor: Actor,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    pub scope_used: Vec<String>,
    pub outcome: Outcome,
    pub prev_hash: String,
    pub hash: String,
}

#[derive(Serialize)]
struct HashInput<'a> {
    seq: u64,
    ts: &'a DateTime<Utc>,
    actor: String,
    action: &'a str,
    document_id: Option<&'a str>,
    scope_used: &'a [String],
    outcome: &'a str,
    prev_hash: &'a str,
}

impl AuditEntry {
    pub(crate) fn new(
        seq: u64,
        ts: DateTime<Utc>,
        record: AuditRecord,
        prev_hash: String,
    ) -> Result<Self> {
        let hash = compute_hash(
            seq,
            &ts,
            &record.actor,
            &record.action,
            record.document_id.as_deref(),
            &record.scope_used,
            record.outcome,
            &prev_hash,
        )?;
        Ok(Self {
            seq,
            ts,
            actor: record.actor,
            action: record.action,
            document_id: record.document_id,
            scope_used: record.scope_used,
            outcome: record.outcome,
            prev_hash,
            hash,
        })
    }

    pub fn recompute_hash(&self) -> Result<String> {
        compute_hash(
            self.seq,
            &self.ts,
            &self.actor,
            &self.action,
            self.document_id.as_deref(),
            &self.scope_used,
            self.outcome,
            &self.prev_hash,
        )
    }
}

fn compute_hash(
    seq: u64,
    ts: &DateTime<Utc>,
    actor: &Actor,
    action: &str,
    document_id: Option<&str>,
    scope_used: &[String],
    outcome: Outcome,
    prev_hash: &str,
) -> Result<String> {
    let input = HashInput {
        seq,
        ts,
        actor: actor.as_encoded(),
        action,
        document_id,
        scope_used,
        outcome: outcome.as_str(),
        prev_hash,
    };
    let bytes = serde_json::to_vec(&input)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_roundtrips() {
        assert_eq!(Actor::Owner.as_encoded(), "owner");
        assert_eq!(Actor::parse("owner").unwrap(), Actor::Owner);

        let t = Actor::Token("abc123".to_string());
        assert_eq!(t.as_encoded(), "tok:abc123");
        assert_eq!(Actor::parse("tok:abc123").unwrap(), t);
    }

    #[test]
    fn actor_parse_rejects_unknown_shape() {
        assert!(Actor::parse("").is_err());
        assert!(Actor::parse("random").is_err());
        assert!(Actor::parse("tok:").is_err());
    }
}
