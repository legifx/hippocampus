//! The JSON-RPC surface shared by the daemon and every client.

pub mod client;
pub mod protocol;
pub mod transport;

pub use protocol::{method, Request, Response, RpcError};
