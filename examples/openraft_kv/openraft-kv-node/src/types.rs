use std::collections::BTreeMap;

use openraft::{
    RaftMetrics,
    alias::{SnapshotMetaOf, VoteOf},
    raft::InstallSnapshotRequest,
};
use serde::{Deserialize, Serialize};

use crate::TypeConfig;

/// Result shape used by the simple admin endpoints in this example.
pub type OpenRaftResult<T> = Result<T, String>;

/// Request body for a replicated write submitted through the leader.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenRaftKvWriteRequest {
    /// Application key to write.
    pub key: String,
    /// Value stored for the key.
    pub value: String,
    /// Client-side serial used by OpenRaft's example state machine.
    pub serial: u64,
}

/// Response body returned after a replicated write is committed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenRaftKvWriteResponse {
    /// Previous value stored under the key, if any.
    pub previous: Option<String>,
}

/// Request body for a key lookup.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenRaftKvReadRequest {
    /// Application key to look up.
    pub key: String,
}

/// Response body returned by a key lookup.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenRaftKvReadResponse {
    /// Current value stored under the key, if any.
    pub value: Option<String>,
}

/// Admin request used to register a learner in the current cluster.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddLearnerRequest {
    /// OpenRaft node identifier for the learner.
    pub node_id: u64,
    /// Advertised Raft address for the learner.
    pub addr: String,
}

/// Admin request used to promote the cluster to a concrete voter set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeMembershipRequest {
    /// Full voter set that should own the cluster after the change.
    pub voters: Vec<u64>,
}

/// Snapshot of one node's externally visible Raft and application state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenRaftKvState {
    /// Stable OpenRaft node identifier.
    pub node_id: u64,
    /// Advertised Raft address for this node.
    pub public_addr: String,
    /// Current OpenRaft role rendered as text.
    pub role: String,
    /// Leader known by this node, if any.
    pub current_leader: Option<u64>,
    /// Current term reported by this node.
    pub current_term: u64,
    /// Highest log index stored locally.
    pub last_log_index: Option<u64>,
    /// Highest log index applied to the state machine.
    pub last_applied_index: Option<u64>,
    /// Current voter set reported by this node.
    pub voters: Vec<u64>,
    /// Application state machine contents.
    pub kv: BTreeMap<String, String>,
}

/// JSON representation used for full-snapshot replication over HTTP.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallFullSnapshotBody {
    /// Vote bundled with the snapshot transfer.
    pub vote: VoteOf<TypeConfig>,
    /// Snapshot metadata describing the transferred state.
    pub meta: SnapshotMetaOf<TypeConfig>,
    /// Serialized state machine bytes.
    pub data: Vec<u8>,
}

/// Serialized result of a vote RPC.
pub type VoteRpcResult = Result<openraft::raft::VoteResponse<TypeConfig>, String>;
/// Serialized result of an append-entries RPC.
pub type AppendRpcResult = Result<openraft::raft::AppendEntriesResponse<TypeConfig>, String>;
/// Serialized result of a full-snapshot RPC.
pub type SnapshotRpcResult = Result<openraft::raft::SnapshotResponse<TypeConfig>, String>;
/// JSON payload returned by the metrics endpoint.
pub type MetricsResult = Result<RaftMetrics<TypeConfig>, String>;
/// JSON payload returned by `/admin/init`.
pub type InitResult = Result<(), String>;
/// JSON payload returned by `/admin/add-learner`.
pub type AddLearnerResult = Result<(), String>;
/// JSON payload returned by `/admin/change-membership`.
pub type ChangeMembershipResult = Result<(), String>;
/// Request type accepted by the snapshot endpoint.
pub type InstallSnapshotBody = InstallSnapshotRequest<TypeConfig>;
