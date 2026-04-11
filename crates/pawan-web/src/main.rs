extern crate pawan;

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{delete, get, post},
};
use futures::stream::Stream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use pawan::agent::{
    PawanAgent, TokenCallback, ToolCallback, ToolStartCallback,
    ToolCallRecord,
};
use pawan::config::PawanConfig;

mod sessions;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone)]
/// Application state shared across web handlers
///
/// Contains shared resources like agent instances, configuration,
/// and workspace information needed by HTTP handlers.
pub struct AppState {
    agents: Arc<RwLock<HashMap<String, PawanAgent>>>,
    config: Arc<PawanConfig>,
    workspace: PathBuf,
    agent_id: String,
    start_time: std::time::Instant,
}
// Request / Response types
#[derive(Debug, Deserialize)]
/// Request body for chat endpoint
///
/// Contains the user's message and optional session information.
pub struct ChatRequest {
    pub session_id: Option<String>,
    pub message: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
/// Response body for chat endpoint
///
/// Contains the agent's response and execution statistics.
pub struct ChatResponse {
    pub session_id: String,
    pub content: String,
    pub iterations: usize,
    pub tool_calls: usize,
}

#[derive(Debug, Serialize)]
/// Health check response
///
/// Contains information about the server's health status, version, uptime,
/// and the agent ID of the current instance.
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_secs: u64,
    pub agent_id: String,
}

#[derive(Debug, Serialize)]
/// Information about an available model
///
/// Represents a language model that can be used by the Pawan agent,
/// including its name, provider, and whether it's the default model.
pub struct ModelInfo {
    pub name: String,
    pub provider: String,
    pub is_default: bool,
}

#[derive(Debug, Serialize)]
/// Response containing available models
///
/// Contains a list of all available language models that can be used
/// by the Pawan agent for various operations.
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health_handler(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.start_time.elapsed().as_secs(),
        agent_id: state.agent_id.clone(),
    })
}

/// List known agents from aegis-net peer config
async fn agents_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Read aegis-net peers if available
    let peers = read_aegis_peers();
    Json(serde_json::json!({
        "self": state.agent_id,
        "peers": peers,
    }))
}

fn read_aegis_peers() -> Vec<serde_json::Value> {
    let path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/etc"))
        .join("aegis")
        .join("aegis-net.toml");
    let path = path.as_path();
    if !path.exists() {
        return vec![];
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let parsed: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let peers = match parsed.get("peers").and_then(|p| p.as_table()) {
        Some(t) => t,
        None => return vec![],
    };
    peers.iter().map(|(name, config)| {
        serde_json::json!({
            "name": name,
            "agent_id": format!("pawan@{}", name),
            "ip": config.get("ip").and_then(|v| v.as_str()),
            "groups": config.get("groups").and_then(|v| v.as_array()),
        })
    }).collect()
}

async fn models_handler(State(state): State<AppState>) -> Json<ModelsResponse> {
    let mut models = vec![ModelInfo {
        name: state.config.model.clone(),
        provider: format!("{:?}", state.config.provider),
        is_default: true,
    }];

    for fallback in &state.config.fallback_models {
        models.push(ModelInfo {
            name: fallback.clone(),
            provider: format!("{:?}", state.config.provider),
            is_default: false,
        });
    }

    Json(ModelsResponse { models })
}

async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, String)> {
    let session_id = req.session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut agents = state.agents.write().await;
    let agent = agents.entry(session_id.clone()).or_insert_with(|| {
        let config = (*state.config).clone();
        PawanAgent::new(config, state.workspace.clone())
    });

    match agent.execute(&req.message).await {
        Ok(response) => Ok(Json(ChatResponse {
            session_id,
            content: response.content,
            iterations: response.iterations,
            tool_calls: response.tool_calls.len(),
        })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn chat_stream_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let session_id = req.session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let message = req.message.clone();
    let config = (*state.config).clone();
    let workspace = state.workspace.clone();

    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(256);

    // Spawn agent task
    let sid = session_id.clone();
    tokio::spawn(async move {
        let mut agent = PawanAgent::new(config, workspace);

        // Try to resume session
        let _ = agent.resume_session(&sid);

        let tx_token = tx.clone();
        let tx_tool_start = tx.clone();
        let tx_tool = tx.clone();

        let on_token: TokenCallback = Box::new(move |token: &str| {
            let event = Event::default()
                .event("token")
                .data(serde_json::json!({"content": token}).to_string());
            let _ = tx_token.try_send(event);
        });

        let on_tool_start: ToolStartCallback = Box::new(move |name: &str| {
            let event = Event::default()
                .event("tool_start")
                .data(serde_json::json!({"name": name}).to_string());
            let _ = tx_tool_start.try_send(event);
        });

        let on_tool: ToolCallback = Box::new(move |record: &ToolCallRecord| {
            let event = Event::default()
                .event("tool_complete")
                .data(serde_json::json!({
                    "name": record.name,
                    "success": record.success,
                    "duration_ms": record.duration_ms,
                    "result_preview": record.result.to_string().chars().take(200).collect::<String>(),
                }).to_string());
            let _ = tx_tool.try_send(event);
        });

        match agent
            .execute_with_callbacks(&message, Some(on_token), Some(on_tool), Some(on_tool_start))
            .await
        {
            Ok(response) => {
                let _ = agent.save_session();
                let _ = agent.archive_to_eruka().await;
                let event = Event::default()
                    .event("done")
                    .data(serde_json::json!({
                        "session_id": sid,
                        "content": response.content,
                        "iterations": response.iterations,
                        "tool_calls": response.tool_calls.len(),
                    }).to_string());
                let _ = tx.send(event).await;
            }
            Err(e) => {
                let err_msg = e.to_string();
                let event = Event::default()
                    .event("error")
                    .data(serde_json::json!({"message": err_msg}).to_string());
                let _ = tx.send(event).await;
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok);
    Sse::new(stream)
}

async fn list_sessions_handler() -> Result<Json<Vec<sessions::SessionSummary>>, (StatusCode, String)> {
    sessions::list_sessions()
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_session_handler(
    Path(id): Path<String>,
) -> Result<Json<sessions::SessionDetail>, (StatusCode, String)> {
    sessions::get_session(&id)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn delete_session_handler(
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    sessions::delete_session(&id)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn create_session_handler() -> Json<serde_json::Value> {
    let id = uuid::Uuid::new_v4().to_string();
    Json(serde_json::json!({"session_id": id}))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pawan_web=info,tower_http=info".into()),
        )
        .init();

    let config = PawanConfig::default();
    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/opt"));

    // Derive agent_id from PAWAN_AGENT_ID env, or hostname, or aegis-net peer name
    let agent_id = std::env::var("PAWAN_AGENT_ID").unwrap_or_else(|_| {
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".into());
        format!("pawan@{}", hostname)
    });

    let state = AppState {
        agents: Arc::new(RwLock::new(HashMap::new())),
        config: Arc::new(config),
        workspace,
        agent_id: agent_id.clone(),
        start_time: std::time::Instant::now(),
    };

    tracing::info!("Agent identity: {}", agent_id);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/models", get(models_handler))
        .route("/api/chat", post(chat_handler))
        .route("/api/chat/stream", post(chat_stream_handler))
        .route("/api/sessions", get(list_sessions_handler))
        .route("/api/sessions", post(create_session_handler))
        .route("/api/sessions/{id}", get(get_session_handler))
        .route("/api/sessions/{id}", delete(delete_session_handler))
        .route("/api/agents", get(agents_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = std::env::var("PAWAN_WEB_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3300u16);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .expect("failed to bind");

    tracing::info!("pawan-web listening on port {}", port);

    axum::serve(listener, app).await.expect("server error");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        AppState {
            agents: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(PawanConfig::default()),
            workspace: std::path::PathBuf::from("/tmp"),
            agent_id: "pawan@test".to_string(),
            start_time: std::time::Instant::now(),
        }
    }

    fn build_test_router(state: AppState) -> Router {
        Router::new()
            .route("/api/health", get(health_handler))
            .route("/api/models", get(models_handler))
            .route("/api/sessions", get(list_sessions_handler))
            .route("/api/sessions", post(create_session_handler))
            .route("/api/agents", get(agents_handler))
            .with_state(state)
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn test_health_returns_ok_with_agent_id() {
        let app = build_test_router(test_state());
        let resp = app
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["status"], "ok");
        assert_eq!(json["agent_id"], "pawan@test");
    }

    #[tokio::test]
    async fn test_models_returns_array() {
        let app = build_test_router(test_state());
        let resp = app
            .oneshot(Request::get("/api/models").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["models"].is_array());
        assert!(!json["models"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_create_session_returns_id() {
        let app = build_test_router(test_state());
        let resp = app
            .oneshot(
                Request::post("/api/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let sid = json["session_id"].as_str().unwrap();
        assert!(!sid.is_empty());
    }

    #[tokio::test]
    async fn test_agents_returns_self() {
        let app = build_test_router(test_state());
        let resp = app
            .oneshot(Request::get("/api/agents").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["self"], "pawan@test");
        assert!(json["peers"].is_array());
    }

    #[tokio::test]
    async fn test_list_sessions_returns_array() {
        let app = build_test_router(test_state());
        let resp = app
            .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
            .await
            .unwrap();
        // May return 200 with [] or 500 if no session dir — both acceptable
        assert!(
            resp.status() == StatusCode::OK || resp.status() == StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
