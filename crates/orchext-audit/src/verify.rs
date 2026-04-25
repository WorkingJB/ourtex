use crate::entry::{AuditEntry, ZERO_HASH};
use crate::error::{AuditError, Result};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub total_entries: u64,
    pub last_seq: Option<u64>,
    pub last_hash: Option<String>,
}

pub async fn verify(path: impl AsRef<Path>) -> Result<VerifyReport> {
    scan(path.as_ref()).await
}

pub(crate) async fn scan(path: &Path) -> Result<VerifyReport> {
    let file = tokio::fs::File::open(path).await?;
    let mut lines = BufReader::new(file).lines();

    let mut line_no: u64 = 0;
    let mut expected_seq: u64 = 0;
    let mut prev_hash = ZERO_HASH.to_string();
    let mut last_hash: Option<String> = None;
    let mut last_seq: Option<u64> = None;
    let mut total: u64 = 0;

    while let Some(line) = lines.next_line().await? {
        line_no += 1;
        if line.trim().is_empty() {
            continue;
        }
        let entry: AuditEntry = serde_json::from_str(&line).map_err(|e| AuditError::Malformed {
            line: line_no,
            reason: e.to_string(),
        })?;

        if entry.seq != expected_seq {
            return Err(AuditError::ChainBroken {
                seq: entry.seq,
                reason: format!("expected seq {expected_seq}, got {}", entry.seq),
            });
        }
        if entry.prev_hash != prev_hash {
            return Err(AuditError::ChainBroken {
                seq: entry.seq,
                reason: "prev_hash does not match previous entry's hash".to_string(),
            });
        }
        let recomputed = entry.recompute_hash()?;
        if recomputed != entry.hash {
            return Err(AuditError::ChainBroken {
                seq: entry.seq,
                reason: "stored hash does not match recomputed hash".to_string(),
            });
        }

        prev_hash = entry.hash.clone();
        last_hash = Some(entry.hash.clone());
        last_seq = Some(entry.seq);
        expected_seq = entry.seq + 1;
        total += 1;
    }

    Ok(VerifyReport {
        total_entries: total,
        last_seq,
        last_hash,
    })
}

pub struct Iter {
    inner: tokio::io::Lines<BufReader<tokio::fs::File>>,
}

impl Iter {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = tokio::fs::File::open(path.as_ref()).await?;
        Ok(Self {
            inner: BufReader::new(file).lines(),
        })
    }

    pub async fn next(&mut self) -> Result<Option<AuditEntry>> {
        loop {
            match self.inner.next_line().await? {
                None => return Ok(None),
                Some(line) if line.trim().is_empty() => continue,
                Some(line) => {
                    let entry: AuditEntry = serde_json::from_str(&line)?;
                    return Ok(Some(entry));
                }
            }
        }
    }
}
