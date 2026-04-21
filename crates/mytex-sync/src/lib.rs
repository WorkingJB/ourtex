//! Client-side sync for `mytex-server`.
//!
//! Exposes a `VaultDriver`-compatible `RemoteVaultDriver` so every
//! existing caller of the trait — including `mytex-index::Index::
//! reindex_from` and the desktop's Tauri commands — works unchanged
//! against a remote workspace. The typical open-workspace sequence
//! mirrors the local path:
//!
//! ```ignore
//! let client = RemoteClient::new(config);
//! let vault  = Arc::new(RemoteVaultDriver::new(client));
//! let index  = Index::open("/some/cache/index.sqlite").await?;
//! index.reindex_from(&*vault).await?;   // populates local cache
//! ```
//!
//! Thereafter, searches/lists go through `index` (local SQLite),
//! writes go through `vault.write_versioned(...)` (HTTP) followed
//! by `index.upsert(...)` to keep the cache consistent.

#![forbid(unsafe_code)]

pub mod client;
pub mod crypto;
pub mod driver;
pub mod error;
pub mod session;

pub use client::{RemoteClient, RemoteConfig};
pub use crypto::{CryptoState, InitCryptoResponse, PublishResponse};
pub use driver::{RemoteVaultDriver, WriteResponse};
pub use error::{Result, SyncError};
pub use session::{list_tenants, login, Account, LoginInput, LoginOutcome, SessionIssued, Tenant};
