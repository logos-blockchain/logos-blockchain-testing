//! Axum server that exposes the OpenRaft example node and its admin endpoints.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use openraft::{Config, Raft, SnapshotPolicy, type_config::async_runtime::WatchReceiver};
use openraft_memstore::{ClientRequest, MemLogStore, MemStateMachine, new_mem_store};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{
    TypeConfig,
    config::OpenRaftKvNodeConfig,
    network::HttpNetworkFactory,
    types::{
        AddLearnerRequest, AppendRpcResult, ChangeMembershipRequest, InitResult,
        InstallSnapshotBody, MetricsResult, OpenRaftKvReadRequest, OpenRaftKvReadResponse,
        OpenRaftKvState, OpenRaftKvWriteRequest, OpenRaftKvWriteResponse, SnapshotRpcResult,
        VoteRpcResult,
    },
};

type KnownNodes = Arc<RwLock<BTreeMap<u64, String>>>;

/// Shared state used by the HTTP handlers exposed by one node.
#[derive(Clone)]
pub struct AppState {
    config: OpenRaftKvNodeConfig,
    raft: Raft<TypeConfig, Arc<MemStateMachine>>,
    state_machine: Arc<MemStateMachine>,
    known_nodes: KnownNodes,
}

impl AppState {
    /// Builds the application state for one node process.
    pub fn new(
        config: OpenRaftKvNodeConfig,
        raft: Raft<TypeConfig, Arc<MemStateMachine>>,
        state_machine: Arc<MemStateMachine>,
        known_nodes: KnownNodes,
    ) -> Self {
        Self {
            config,
            raft,
            state_machine,
            known_nodes,
        }
    }
}

/// Starts one OpenRaft-backed HTTP node.
pub async fn run_server(config: OpenRaftKvNodeConfig) -> anyhow::Result<()> {
    let raft_config = Arc::new(
        Config {
            cluster_name: "openraft-kv".to_owned(),
            heartbeat_interval: config.heartbeat_interval_ms,
            election_timeout_min: config.election_timeout_min_ms,
            election_timeout_max: config.election_timeout_max_ms,
            snapshot_policy: SnapshotPolicy::Never,
            ..Default::default()
        }
        .validate()?,
    );

    let known_nodes = Arc::new(RwLock::new(known_nodes(&config)));

    let (log_store, state_machine): (Arc<MemLogStore>, Arc<MemStateMachine>) = new_mem_store();
    let network = HttpNetworkFactory::new(known_nodes.clone());

    let raft = Raft::new(
        config.node_id,
        raft_config,
        network,
        log_store,
        state_machine.clone(),
    )
    .await?;

    let app_state = AppState::new(config.clone(), raft, state_machine, known_nodes);
    let app = router(app_state);
    let address = std::net::SocketAddr::from(([0, 0, 0, 0], config.http_port));

    info!(
        node_id = config.node_id,
        public_addr = %config.public_addr,
        peers = ?config.peer_addrs,
        %address,
        "starting openraft kv node"
    );

    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn router(app_state: AppState) -> Router {
    let app_routes = Router::new()
        .route("/healthz", get(healthz))
        .route("/state", get(cluster_state))
        .route("/kv/write", post(write))
        .route("/kv/read", post(read));

    let admin_routes = Router::new()
        .route("/admin/init", post(init))
        .route("/admin/add-learner", post(add_learner))
        .route("/admin/change-membership", post(change_membership))
        .route("/admin/metrics", get(metrics));

    let raft_routes = Router::new()
        .route("/raft/vote", post(vote))
        .route("/raft/append", post(append))
        .route("/raft/snapshot", post(snapshot));

    app_routes
        .merge(admin_routes)
        .merge(raft_routes)
        .layer(TraceLayer::new_for_http())
        .with_state(app_state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn cluster_state(State(app): State<AppState>) -> Result<Json<OpenRaftKvState>, StatusCode> {
    let metrics = app.raft.metrics().borrow_watched().clone();

    let sm = app.state_machine.get_state_machine().await;

    let voters = metrics
        .membership_config
        .membership()
        .voter_ids()
        .collect::<Vec<_>>();

    let kv = sm.client_status.into_iter().collect::<BTreeMap<_, _>>();

    Ok(Json(OpenRaftKvState {
        node_id: app.config.node_id,
        public_addr: app.config.public_addr.clone(),
        role: format!("{:?}", metrics.state),
        current_leader: metrics.current_leader,
        current_term: metrics.current_term,
        last_log_index: metrics.last_log_index,
        last_applied_index: metrics.last_applied.as_ref().map(|log_id| log_id.index()),
        voters,
        kv,
    }))
}

async fn metrics(State(app): State<AppState>) -> Json<MetricsResult> {
    Json(Ok(app.raft.metrics().borrow_watched().clone()))
}

async fn init(State(app): State<AppState>) -> Json<InitResult> {
    let members = BTreeSet::from([app.config.node_id]);

    Json(
        app.raft
            .initialize(members)
            .await
            .map_err(|err| err.to_string()),
    )
}

async fn add_learner(
    State(app): State<AppState>,
    Json(request): Json<AddLearnerRequest>,
) -> Json<InitResult> {
    let mut known_nodes = app.known_nodes.write().await;
    known_nodes.insert(request.node_id, request.addr.clone());
    drop(known_nodes);

    Json(
        app.raft
            .add_learner(request.node_id, (), true)
            .await
            .map(|_| ())
            .map_err(|err| err.to_string()),
    )
}

async fn change_membership(
    State(app): State<AppState>,
    Json(request): Json<ChangeMembershipRequest>,
) -> Json<InitResult> {
    Json(
        app.raft
            .change_membership(request.voters.into_iter().collect::<BTreeSet<_>>(), false)
            .await
            .map(|_| ())
            .map_err(|err| err.to_string()),
    )
}

async fn write(
    State(app): State<AppState>,
    Json(request): Json<OpenRaftKvWriteRequest>,
) -> Json<Result<OpenRaftKvWriteResponse, String>> {
    let result = app
        .raft
        .client_write(ClientRequest {
            client: request.key,
            serial: request.serial,
            status: request.value,
        })
        .await
        .map(|response| OpenRaftKvWriteResponse {
            previous: response.response().0.clone(),
        })
        .map_err(|err| err.to_string());

    Json(result)
}

async fn read(
    State(app): State<AppState>,
    Json(request): Json<OpenRaftKvReadRequest>,
) -> Json<Result<OpenRaftKvReadResponse, String>> {
    let sm = app.state_machine.get_state_machine().await;

    Json(Ok(OpenRaftKvReadResponse {
        value: sm.client_status.get(&request.key).cloned(),
    }))
}

async fn vote(
    State(app): State<AppState>,
    Json(request): Json<openraft::raft::VoteRequest<TypeConfig>>,
) -> Json<VoteRpcResult> {
    Json(app.raft.vote(request).await.map_err(|err| err.to_string()))
}

async fn append(
    State(app): State<AppState>,
    Json(request): Json<openraft::raft::AppendEntriesRequest<TypeConfig>>,
) -> Json<AppendRpcResult> {
    Json(
        app.raft
            .append_entries(request)
            .await
            .map_err(|err| err.to_string()),
    )
}

async fn snapshot(
    State(app): State<AppState>,
    Json(request): Json<InstallSnapshotBody>,
) -> Json<SnapshotRpcResult> {
    let snapshot = openraft::alias::SnapshotOf::<TypeConfig> {
        meta: request.meta,
        snapshot: std::io::Cursor::new(request.data),
    };

    Json(
        app.raft
            .install_full_snapshot(request.vote, snapshot)
            .await
            .map_err(|err| err.to_string()),
    )
}

fn known_nodes(config: &OpenRaftKvNodeConfig) -> BTreeMap<u64, String> {
    let mut known_nodes = config.peer_addrs.clone();
    known_nodes.insert(config.node_id, config.public_addr.clone());
    known_nodes
}
