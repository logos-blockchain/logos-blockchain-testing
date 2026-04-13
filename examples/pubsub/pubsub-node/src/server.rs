use std::{collections::HashSet, sync::Arc};

use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::Message},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use tracing::{debug, warn};

use crate::{
    config::PubSubConfig,
    state::{PubSubState, Snapshot, TopicEvent, TopicsStateView},
};

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Publish { topic: String, payload: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame {
    Subscribed { topic: String },
    Unsubscribed { topic: String },
    Published { id: crate::state::EventId },
    Event { event: TopicEvent },
    Error { message: String },
}

pub async fn start_server(config: PubSubConfig, state: PubSubState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/topics/state", get(topics_state))
        .route("/internal/snapshot", get(snapshot))
        .route("/ws", get(ws_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(Arc::new(state.clone()));

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    state.set_ready(true).await;
    tracing::info!(node_id = state.node_id(), %addr, "pubsub node ready");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_live() -> (StatusCode, Json<HealthResponse>) {
    (StatusCode::OK, Json(HealthResponse { status: "alive" }))
}

async fn health_ready(State(state): State<Arc<PubSubState>>) -> (StatusCode, Json<HealthResponse>) {
    if state.is_ready().await {
        (StatusCode::OK, Json(HealthResponse { status: "ready" }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "not-ready",
            }),
        )
    }
}

async fn topics_state(State(state): State<Arc<PubSubState>>) -> Json<TopicsStateView> {
    Json(state.topics_state().await)
}

async fn snapshot(State(state): State<Arc<PubSubState>>) -> Json<Snapshot> {
    Json(state.snapshot().await)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<PubSubState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: axum::extract::ws::WebSocket, state: Arc<PubSubState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut topics = HashSet::new();
    let mut events = state.subscribe_events();

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientFrame>(&text) {
                            Ok(frame) => handle_client_frame(frame, &state, &mut topics, &mut sender).await,
                            Err(error) => {
                                let _ = send_frame(&mut sender, &ServerFrame::Error { message: format!("invalid frame: {error}") }).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        debug!(%error, "ws receive error");
                        break;
                    }
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) if topics.contains(&event.topic) => {
                        if send_frame(&mut sender, &ServerFrame::Event { event }).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "ws subscriber lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn handle_client_frame(
    frame: ClientFrame,
    state: &PubSubState,
    topics: &mut HashSet<String>,
    sender: &mut futures_util::stream::SplitSink<axum::extract::ws::WebSocket, Message>,
) {
    match frame {
        ClientFrame::Subscribe { topic } => {
            topics.insert(topic.clone());
            let _ = send_frame(sender, &ServerFrame::Subscribed { topic }).await;
        }
        ClientFrame::Unsubscribe { topic } => {
            topics.remove(&topic);
            let _ = send_frame(sender, &ServerFrame::Unsubscribed { topic }).await;
        }
        ClientFrame::Publish { topic, payload } => {
            let event = state.publish_local(topic, payload).await;
            let _ = send_frame(sender, &ServerFrame::Published { id: event.id }).await;
        }
    }
}

async fn send_frame(
    sender: &mut futures_util::stream::SplitSink<axum::extract::ws::WebSocket, Message>,
    frame: &ServerFrame,
) -> Result<(), axum::Error> {
    let payload = serde_json::to_string(frame)
        .map_err(|error| axum::Error::new(std::io::Error::other(error.to_string())))?;
    sender.send(Message::Text(payload)).await
}
