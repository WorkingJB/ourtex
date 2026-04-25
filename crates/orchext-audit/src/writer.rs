use crate::entry::{AuditEntry, AuditRecord, ZERO_HASH};
use crate::error::{AuditError, Result};
use crate::verify::scan;
use chrono::Utc;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub struct AuditWriter {
    path: PathBuf,
    state: Mutex<WriterState>,
}

struct WriterState {
    next_seq: u64,
    last_hash: String,
}

impl AuditWriter {
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let state = recover_state(&path).await?;
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn append(&self, record: AuditRecord) -> Result<AuditEntry> {
        let mut state = self.state.lock().await;
        let entry = AuditEntry::new(state.next_seq, Utc::now(), record, state.last_hash.clone())?;

        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(&line).await?;
        file.flush().await?;

        state.next_seq += 1;
        state.last_hash = entry.hash.clone();

        Ok(entry)
    }
}

async fn recover_state(path: &Path) -> Result<WriterState> {
    match tokio::fs::metadata(path).await {
        Ok(_) => {
            let report = scan(path).await?;
            Ok(WriterState {
                next_seq: report.last_seq.map(|s| s + 1).unwrap_or(0),
                last_hash: report.last_hash.unwrap_or_else(|| ZERO_HASH.to_string()),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WriterState {
            next_seq: 0,
            last_hash: ZERO_HASH.to_string(),
        }),
        Err(e) => Err(AuditError::Io(e)),
    }
}
