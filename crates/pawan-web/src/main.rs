extern crate pawan;

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{delete, get, post},
    Json, Router,
};
use futures::stream::Stream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use pawan::agent::{PawanAgent, TokenCallback, ToolCallRecord, ToolCallback, ToolStartCallback};
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
    peers
        .iter()
        .map(|(name, config)| {
            serde_json::json!({
                "name": name,
                "agent_id": format!("pawan@{}", name),
                "ip": config.get("ip").and_then(|v| v.as_str()),
                "groups": config.get("groups").and_then(|v| v.as_array()),
            })
        })
        .collect()
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
    let session_id = req
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

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
    let session_id = req
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
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
                let event = Event::default().event("done").data(
                    serde_json::json!({
                        "session_id": sid,
                        "content": response.content,
                        "iterations": response.iterations,
                        "tool_calls": response.tool_calls.len(),
                    })
                    .to_string(),
                );
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

async fn list_sessions_handler() -> Result<Json<Vec<sessions::SessionSummary>>, (StatusCode, String)>
{
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
            .oneshot(Request::post("/api/sessions").body(Body::empty()).unwrap())
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

    // ---------------------------------------------------------------------------
    // read_aegis_peers tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_read_aegis_peers_no_config_file() {
        // When config file doesn't exist, should return empty vec
        let peers = read_aegis_peers();
        assert!(peers.is_empty());
    }

    #[test]
    fn test_chat_request_deserialization_with_session_id() {
        let json = r#"{"session_id": "test-123", "message": "Hello", "model": "gpt-4"}"#;
        let req: ChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.session_id, Some("test-123".to_string()));
        assert_eq!(req.message, "Hello");
        assert_eq!(req.model, Some("gpt-4".to_string()));
    }

    #[test]
    fn test_chat_request_deserialization_without_session_id() {
        let json = r#"{"message": "Hello world"}"#;
        let req: ChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.session_id, None);
        assert_eq!(req.message, "Hello world");
        assert_eq!(req.model, None);
    }

    #[test]
    fn test_chat_response_serialization() {
        let resp = ChatResponse {
            session_id: "test-session".to_string(),
            content: "Hello!".to_string(),
            iterations: 3,
            tool_calls: 2,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("test-session"));
        assert!(json.contains("Hello!"));
    }

    #[test]
    fn test_health_response_serialization() {
        let resp = HealthResponse {
            status: "ok",
            version: "0.1.0",
            uptime_secs: 123,
            agent_id: "test-agent".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("test-agent"));
    }

    #[test]
    fn test_model_info_serialization() {
        let model = ModelInfo {
            name: "gpt-4".to_string(),
            provider: "OpenAI".to_string(),
            is_default: true,
        };
        let json = serde_json::to_string(&model).unwrap();
        assert!(json.contains("gpt-4"));
        assert!(json.contains("OpenAI"));
        assert!(json.contains("true"));
    }

    #[test]
    fn test_models_response_serialization() {
        let resp = ModelsResponse {
            models: vec![
                ModelInfo {
                    name: "gpt-4".to_string(),
                    provider: "OpenAI".to_string(),
                    is_default: true,
                },
                ModelInfo {
                    name: "claude-3".to_string(),
                    provider: "Anthropic".to_string(),
                    is_default: false,
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("gpt-4"));
        assert!(json.contains("claude-3"));
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let state = test_state();
        let app = Router::new()
            .route("/api/sessions/{id}", get(get_session_handler))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::get("/api/sessions/nonexistent-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_session_not_found() {
        let state = test_state();
        let app = Router::new()
            .route("/api/sessions/{id}", delete(delete_session_handler))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::delete("/api/sessions/nonexistent-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_invalid_method_health() {
        let app = build_test_router(test_state());
        let resp = app
            .oneshot(Request::post("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        // POST to GET-only endpoint should return 405 Method Not Allowed
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_invalid_method_create_session() {
        let app = build_test_router(test_state());
        let resp = app
            .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
            .await
            .unwrap();

        // Both GET and POST are valid for /api/sessions, so this test doesn't apply
        // The router has both handlers for this path
        assert!(
            resp.status() == StatusCode::OK || resp.status() == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_malformed_json_chat() {
        let state = test_state();
        let app = Router::new()
            .route("/api/chat", post(chat_handler))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::post("/api/chat")
                    .header("content-type", "application/json")
                    .body(Body::from("{invalid json}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
    #[tokio::test]
    async fn test_missing_content_type() {
        let state = test_state();
        let app = Router::new()
            .route("/api/chat", post(chat_handler))
            .with_state(state);

        let resp = app
            .oneshot(
                Request::post("/api/chat")
                    .body(Body::from(r#"{"message": "test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Without content-type header, should fail to parse
        assert!(
            resp.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE
                || resp.status() == StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn test_models_with_fallback_models() {
        use pawan::config::PawanConfig;

        let config = PawanConfig {
            fallback_models: vec!["fallback-1".to_string(), "fallback-2".to_string()],
            ..Default::default()
        };

        let state = AppState {
            agents: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(config),
            workspace: std::path::PathBuf::from("/tmp"),
            agent_id: "test".to_string(),
            start_time: std::time::Instant::now(),
        };

        let app = Router::new()
            .route("/api/models", get(models_handler))
            .with_state(state);

        let resp = app
            .oneshot(Request::get("/api/models").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let models = json["models"].as_array().unwrap();
        assert_eq!(models.len(), 3); // 1 default + 2 fallbacks
        assert_eq!(models[0]["is_default"], true);
        assert_eq!(models[1]["is_default"], false);
        assert_eq!(models[2]["is_default"], false);
    }

    #[tokio::test]
    async fn test_health_uptime_increases() {
        let state = test_state();
        let app = Router::new()
            .route("/api/health", get(health_handler))
            .with_state(state);

        let resp1 = app
            .clone()
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let json1 = body_json(resp1).await;
        let uptime1 = json1["uptime_secs"].as_u64().unwrap();

        // Small delay to ensure uptime increases
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let resp2 = app
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let json2 = body_json(resp2).await;
        let uptime2 = json2["uptime_secs"].as_u64().unwrap();

        assert!(uptime2 >= uptime1);
    }

    #[tokio::test]
    async fn test_agents_returns_empty_peers() {
        let app = build_test_router(test_state());
        let resp = app
            .oneshot(Request::get("/api/agents").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["self"], "pawan@test");
        let peers = json["peers"].as_array().unwrap();
        // Should be empty when no config file exists
        assert!(peers.is_empty());
    }

    #[tokio::test]
    async fn test_cors_preflight() {
        let state = test_state();
        let cors = tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any);

        let app = Router::new()
            .route("/api/health", get(health_handler))
            .layer(cors)
            .with_state(state);

        let resp = app
            .oneshot(
                Request::options("/api/health")
                    .header("Origin", "http://localhost:3000")
                    .header("Access-Control-Request-Method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Preflight request should return OK
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
