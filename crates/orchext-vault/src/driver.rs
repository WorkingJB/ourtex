use crate::error::Result;
use crate::{Document, DocumentId};
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: DocumentId,
    pub type_: String,
    pub path: PathBuf,
}

#[async_trait]
pub trait VaultDriver: Send + Sync {
    async fn list(&self, type_: Option<&str>) -> Result<Vec<Entry>>;
    async fn read(&self, id: &DocumentId) -> Result<Document>;
    async fn write(&self, id: &DocumentId, doc: &Document) -> Result<()>;
    async fn delete(&self, id: &DocumentId) -> Result<()>;
}
