//! OpenRaft-backed key-value node used by the `examples-simple-clusters`
//! branch.

/// HTTP client for interacting with one OpenRaft node.
pub mod client;
/// YAML node configuration used by TF and the node binary.
pub mod config;
mod network;
/// Axum server bootstrap and request handlers for one node process.
pub mod server;
/// Shared request, response, and state payload types.
pub mod types;

/// Re-export of the node HTTP client.
pub use client::OpenRaftKvClient;
/// Re-export of the node YAML config type.
pub use config::OpenRaftKvNodeConfig;
/// Re-export of the public request and state payloads.
pub use types::{
    AddLearnerRequest, ChangeMembershipRequest, OpenRaftKvReadRequest, OpenRaftKvReadResponse,
    OpenRaftKvState, OpenRaftKvWriteRequest, OpenRaftKvWriteResponse,
};

/// OpenRaft type configuration shared by the in-memory log and state machine.
pub type TypeConfig = openraft_memstore::TypeConfig;
