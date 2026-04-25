#![forbid(unsafe_code)]

mod entry;
mod error;
mod verify;
mod writer;

pub use entry::{Actor, AuditEntry, AuditRecord, Outcome};
pub use error::{AuditError, Result};
pub use verify::{verify, Iter, VerifyReport};
pub use writer::AuditWriter;
