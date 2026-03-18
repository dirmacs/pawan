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
    AgentResponse, Message, PawanAgent, Role, TokenCallback, ToolCallback, ToolStartCallback,
    ToolCallRecord,
};
use pawan::config::PawanConfig;

mod sessions;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    agents: Arc<RwLock<HashMap<String, PawanAgent>>>,
    config: Arc<PawanConfig>,
    workspace: PathBuf,
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub session_id: Option<String>,
    pub message: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub session_id: String,
    pub content: String,
    pub iterations: usize,
    pub tool_calls: usize,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub provider: String,
    pub is_default: bool,
}

#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: 0, // TODO: track real uptime
    })
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

    let state = AppState {
        agents: Arc::new(RwLock::new(HashMap::new())),
        config: Arc::new(config),
        workspace,
    };

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
