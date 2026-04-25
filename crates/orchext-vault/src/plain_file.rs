use crate::driver::{Entry, VaultDriver};
use crate::error::{Result, VaultError};
use crate::{Document, DocumentId};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct PlainFileDriver {
    root: PathBuf,
}

impl PlainFileDriver {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    async fn locate(&self, id: &DocumentId) -> Result<PathBuf> {
        let mut type_dirs = tokio::fs::read_dir(&self.root).await?;
        while let Some(type_dir) = type_dirs.next_entry().await? {
            if !type_dir.file_type().await?.is_dir() {
                continue;
            }
            if is_hidden_or_reserved(&type_dir.file_name().to_string_lossy()) {
                continue;
            }
            let candidate = type_dir.path().join(format!("{}.md", id));
            if tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
                return Ok(candidate);
            }
        }
        Err(VaultError::NotFound(id.to_string()))
    }
}

fn is_hidden_or_reserved(name: &str) -> bool {
    // Skip `.ourtex/`, `.git/`, and anything else that starts with `.`.
    // FORMAT.md §1: seed and custom types are non-hidden top-level directories.
    name.starts_with('.')
}

#[async_trait]
impl VaultDriver for PlainFileDriver {
    async fn list(&self, type_filter: Option<&str>) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        let mut type_dirs = tokio::fs::read_dir(&self.root).await?;
        while let Some(type_dir) = type_dirs.next_entry().await? {
            if !type_dir.file_type().await?.is_dir() {
                continue;
            }
            let type_name_os = type_dir.file_name();
            let Some(type_name) = type_name_os.to_str() else {
                continue;
            };
            if is_hidden_or_reserved(type_name) {
                continue;
            }
            if let Some(filter) = type_filter {
                if type_name != filter {
                    continue;
                }
            }

            let mut files = tokio::fs::read_dir(type_dir.path()).await?;
            while let Some(file_entry) = files.next_entry().await? {
                let file_path = file_entry.path();
                if file_path.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let Some(stem) = file_path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(id) = DocumentId::new(stem) else {
                    tracing::debug!(
                        path = %file_path.display(),
                        "skipping file with invalid id"
                    );
                    continue;
                };
                out.push(Entry {
                    id,
                    type_: type_name.to_string(),
                    path: file_path,
                });
            }
        }
        Ok(out)
    }

    async fn read(&self, id: &DocumentId) -> Result<Document> {
        let path = self.locate(id).await?;
        let content = tokio::fs::read_to_string(&path).await?;
        Document::parse(&content)
    }

    async fn write(&self, id: &DocumentId, doc: &Document) -> Result<()> {
        if doc.frontmatter.id != *id {
            return Err(VaultError::InvalidId(format!(
                "id mismatch: path uses {id}, frontmatter says {}",
                doc.frontmatter.id
            )));
        }
        let type_dir = self.root.join(&doc.frontmatter.type_);
        tokio::fs::create_dir_all(&type_dir).await?;
        let path = type_dir.join(format!("{}.md", id));
        let serialized = doc.serialize()?;
        tokio::fs::write(&path, serialized).await?;
        Ok(())
    }

    async fn delete(&self, id: &DocumentId) -> Result<()> {
        let path = self.locate(id).await?;
        tokio::fs::remove_file(&path).await?;
        Ok(())
    }
}
