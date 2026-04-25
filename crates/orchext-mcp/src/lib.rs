#![forbid(unsafe_code)]

pub mod error;
pub mod ratelimit;
pub mod resources;
pub mod rpc;
pub mod server;
pub mod title;
pub mod tools;
pub mod watch;

pub use error::{McpError, Result};
pub use rpc::{Id, Request, Response};
pub use server::{Server, PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION};
