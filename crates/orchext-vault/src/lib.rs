#![forbid(unsafe_code)]

mod document;
mod driver;
mod error;
mod frontmatter;
mod id;
mod plain_file;
mod visibility;

pub use document::Document;
pub use driver::{Entry, VaultDriver};
pub use error::{Result, VaultError};
pub use frontmatter::Frontmatter;
pub use id::DocumentId;
pub use plain_file::PlainFileDriver;
pub use visibility::Visibility;
