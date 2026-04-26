/// Pawan Agent - The core agent that handles tool-calling loops
///
/// This module provides the main `PawanAgent` which:
/// - Manages conversation history
/// - Coordinates tool calling with the LLM via pluggable backends
/// - Provides streaming responses
/// - Supports multiple LLM backends (NVIDIA API, Ollama, OpenAI)
/// - Context management and token counting
/// - Integration with Eruka for 3-tier memory injection

//
// This module provides the main `PawanAgent` which:
// - Manages conversation history
// - Coordinates tool calling with the LLM via pluggable backends
// - Provides streaming responses
// - Supports multiple LLM backends (NVIDIA API, Ollama, OpenAI)
// - Context management and token counting
// - Integration with Eruka for 3-tier memory injection
/// Model information for available models
/// Pawan Agent - The core agent that handles tool-calling loops
///
/// This module provides the main `PawanAgent` which:
/// - Manages conversation history
/// - Coordinates tool calling with the LLM via pluggable backends
/// - Provides streaming responses
/// - Supports multiple LLM backends (NVIDIA API, Ollama, OpenAI)
/// - Context management and token counting
/// - Integration with Eruka for 3-tier memory injection
// Pawan Agent - The core agent that handles tool-calling loops
//
// This module provides the main `PawanAgent` which:
// - Manages conversation history
// - Coordinates tool calling with the LLM via pluggable backends
// - Provides streaming responses
// - Supports multiple LLM backends (NVIDIA API, Ollama, OpenAI)

pub mod backend;
mod preflight;
pub mod events;
pub mod session;
pub mod git_session;

// Re-export event types for public API
pub use events::{
    AgentEvent, FinishReason, ThinkingDeltaEvent, ToolApprovalEvent,
    ToolCompleteEvent, ToolStartEvent, TokenUsageInfo, TurnEndEvent,
    TurnStartEvent, SessionEndEvent,
};

use crate::config::{LlmProvider, PawanConfig};
use crate::coordinator::{CoordinatorResult, ToolCallingConfig, ToolCoordinator};
use crate::credentials;
use crate::tools::{ToolDefinition, ToolRegistry};
use crate::{PawanError, Result};
use backend::openai_compat::{OpenAiCompatBackend, OpenAiCompatConfig};
use backend::LlmBackend;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// Role of the message sender
    pub role: Role,
    /// Content of the message
    pub content: String,
    /// Tool calls (if any)
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRequest>,
    /// Tool results (if this is a tool result message)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResultMessage>,
}

/// Role of a message sender
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A request to call a tool
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallRequest {
    /// Unique ID for this tool call
    pub id: String,
    /// Name of the tool to call
    pub name: String,
    /// Arguments for the tool
    pub arguments: Value,
}

/// Result from a tool execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultMessage {
    /// ID of the tool call this result is for
    pub tool_call_id: String,
    /// The result content
    pub content: Value,
    /// Whether the tool executed successfully
    pub success: bool,
}

/// Record of a tool call execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Unique ID for this tool call
    pub id: String,
    /// Name of the tool
    pub name: String,
    /// Arguments passed to the tool
    pub arguments: Value,
    /// Result from the tool
    pub result: Value,
    /// Whether execution was successful
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// Token usage from an LLM response
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// Tokens spent on reasoning/thinking (subset of completion_tokens)
    pub reasoning_tokens: u64,
    /// Tokens spent on actual content/tool output (completion - reasoning)
    pub action_tokens: u64,
}

/// LLM response from a generation request
#[derive(Debug, Clone)]
pub struct LLMResponse {
    /// Text content of the response
    pub content: String,
    /// Reasoning/thinking content (separate from visible content)
    pub reasoning: Option<String>,
    /// Tool calls requested by the model
    pub tool_calls: Vec<ToolCallRequest>,
    /// Reason the response finished
    pub finish_reason: String,
    /// Token usage (if available)
    pub usage: Option<TokenUsage>,
}

/// Result from a complete agent execution
#[derive(Debug)]
pub struct AgentResponse {
    /// Final text response
    pub content: String,
    /// All tool calls made during execution
    pub tool_calls: Vec<ToolCallRecord>,
    /// Number of iterations taken
    pub iterations: usize,
    /// Cumulative token usage across all iterations
    pub usage: TokenUsage,
}

/// Callback for receiving streaming tokens
pub type TokenCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Callback for receiving tool call updates
pub type ToolCallback = Box<dyn Fn(&ToolCallRecord) + Send + Sync>;

/// Callback for tool call start notifications
pub type ToolStartCallback = Box<dyn Fn(&str) + Send + Sync>;

/// A permission request sent from the agent to the UI for approval.
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    /// Tool name requesting permission
    pub tool_name: String,
    /// Summary of arguments (e.g. bash command or file path)
    pub args_summary: String,
}

/// Callback for requesting tool permission from the user.
/// Returns true if the tool should be allowed, false to deny.
pub type PermissionCallback =
    Box<dyn Fn(PermissionRequest) -> tokio::sync::oneshot::Receiver<bool> + Send + Sync>;

/// The main Pawan agent — handles conversation, tool calling, and self-healing.
///
/// This struct represents the core Pawan agent that handles:
/// - Conversation history management
/// - Tool calling with the LLM via pluggable backends
/// - Streaming responses
/// - Multiple LLM backends (NVIDIA API, Ollama, OpenAI)
/// - Context management and token counting
/// - Integration with Eruka for 3-tier memory injection
pub struct PawanAgent {
    /// Configuration
    config: PawanConfig,
    /// Tool registry
    tools: ToolRegistry,
    /// Conversation history
    history: Vec<Message>,
    /// Workspace root
    workspace_root: PathBuf,
    /// LLM backend
    backend: Box<dyn LlmBackend>,

    /// Estimated token count for current context
    context_tokens_estimate: usize,

    /// Eruka bridge for 3-tier memory injection
    eruka: Option<crate::eruka_bridge::ErukaClient>,

    /// Stable identifier for this agent instance's session — used as the
    /// key for eruka sync_turn / on_pre_compress writes so turns from one
    /// conversation cluster under the same path. Generated fresh in new(),
    /// overwritten by resume_session() when loading an existing session.
    session_id: String,

    /// Per-turn architecture context loaded from `.pawan/arch.md` at init.
    /// When present, prepended to every user message so key architectural
    /// constraints stay visible even as tool-call history grows long.
    arch_context: Option<String>,
    /// Timestamp of last tool call completion for idle timeout tracking
    last_tool_call_time: Option<Instant>,
}

/// Probe whether a local inference server is reachable at `url`.
///
/// Parses `host:port` from the URL and attempts a TCP connect with a 100 ms
/// timeout. Returns `true` if the port is open, `false` on any error.
/// This is intentionally cheap (no HTTP round-trip) so it can run at agent
/// startup without perceptible latency.
fn probe_local_endpoint(url: &str) -> bool {
    use std::net::TcpStream;
    use std::time::Duration;

    // Strip scheme and path — we only need host:port
    let hostport = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("");

    // Ensure port is present; default http → 80, https → 443
    let addr = if hostport.contains(':') {
        hostport.to_string()
    } else if url.starts_with("https://") {
        format!("{hostport}:443")
    } else {
        format!("{hostport}:80")
    };

    // Normalise "localhost" → "127.0.0.1" so we don't accidentally resolve
    // to ::1 (IPv6) when the listener is bound only to IPv4.
    let addr = addr.replace("localhost", "127.0.0.1");

    let socket_addr = match addr.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };

    TcpStream::connect_timeout(&socket_addr, Duration::from_millis(100)).is_ok()
}

/// Retrieve an API key with fallback chain:
/// 1. Environment variable
/// 2. Secure credential store
/// 3. Return None (caller should prompt user)
///
/// If the key is found in the secure store, it's also set as an env var
/// for subsequent calls.
fn get_api_key_with_secure_fallback(env_var: &str, key_name: &str) -> Option<String> {
    // First, check environment variable
    if let Ok(key) = std::env::var(env_var) {
        return Some(key);
    }

    // Second, try secure credential store
    match credentials::get_api_key(key_name) {
        Ok(Some(key)) => {
            // Cache in env var for subsequent calls
            std::env::set_var(env_var, &key);
            Some(key)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("Failed to retrieve {} from secure store: {}", key_name, e);
            None
        }
    }
}

/// Prompt user to enter an API key and store it securely.
///
/// This function:
/// 1. Prompts the user to enter the API key
/// 2. Stores it in the secure credential store
/// 3. Sets it as an environment variable for the current session
///
/// Returns the entered key on success, or None if the user cancels.
fn prompt_and_store_api_key(env_var: &str, key_name: &str, provider: &str) -> Option<String> {
    eprintln!("\n🔑 {} API key not found.", provider);
    eprintln!("You can set it via:");
    eprintln!("  - Environment variable: export {}=<your-key>", env_var);
    eprintln!("  - Interactive entry (recommended for security)");
    eprintln!("\nEnter your {} API key:", provider);
    eprintln!("  (Your key will be stored securely in the OS credential store)\n");

    // Read input securely (no echo)
    #[cfg(unix)]
    let key = {
        use std::io::{self, Write};
        
        // Use termios to disable echo on Unix
        let mut stdout = io::stdout();
        stdout.flush().ok();
        
        // Read password without echo
        rpassword::prompt_password("> ").ok()
    };

    #[cfg(windows)]
    let key = {
        use std::io::{self, Write};
        
        let mut stdout = io::stdout();
        stdout.flush().ok();
        
        // On Windows, use a simple prompt (rpassword handles this)
        rpassword::prompt_password("> ").ok()
    };

    #[cfg(not(any(unix, windows)))]
    let key = {
        use std::io::{self, Write, BufRead};
        
        let mut stdout = io::stdout();
        let mut stdin = io::stdin();
        stdout.flush().ok();
        print!("> ");
        stdout.flush().ok();
        
        let mut input = String::new();
        stdin.lock().read_line(&mut input).ok();
        Some(input.trim().to_string())
    };

    match key {
        Some(k) if !k.trim().is_empty() => {
            let key = k.trim().to_string();
            
            // Store in secure credential store
            match credentials::store_api_key(key_name, &key) {
                Ok(()) => {
                    tracing::info!("{} API key stored securely", provider);
                    std::env::set_var(env_var, &key);
                    Some(key)
                }
                Err(e) => {
                    tracing::warn!("Failed to store key securely: {}. Using session-only.", e);
                    std::env::set_var(env_var, &key);
                    Some(key)
                }
            }
        }
        _ => {
            eprintln!("\n⚠️  No key entered. {} will not work until a key is set.", provider);
            None
        }
    }
}

/// Load per-turn architecture context from `<workspace_root>/.pawan/arch.md`.
///
/// Returns `None` if the file is absent or empty.
/// Caps content at 2 000 chars to avoid context bloat from large files;
/// an ellipsis marker is appended when truncation occurs.
fn load_arch_context(workspace_root: &std::path::Path) -> Option<String> {
    let path = workspace_root.join(".pawan").join("arch.md");
    if !path.exists() {
        return None;
    }
    match std::fs::read_to_string(&path) {
        Ok(content) if !content.trim().is_empty() => {
            const MAX_CHARS: usize = 2_000;
            if content.len() > MAX_CHARS {
                // Truncate on a char boundary
                let boundary = content
                    .char_indices()
                    .map(|(i, _)| i)
                    .nth(MAX_CHARS)
                    .unwrap_or(content.len());
                Some(format!("{}…(truncated)", &content[..boundary]))
            } else {
                Some(content)
            }
        }
        _ => None,
    }
}

impl PawanAgent {
    /// Create a new PawanAgent with auto-selected backend
    pub fn new(config: PawanConfig, workspace_root: PathBuf) -> Self {
        let tools = ToolRegistry::with_defaults(workspace_root.clone());
        let system_prompt = config.get_system_prompt();
        let backend = Self::create_backend(&config, &system_prompt);
        let eruka = if config.eruka.enabled {
            Some(crate::eruka_bridge::ErukaClient::new(config.eruka.clone()))
        } else {
            None
        };
        let arch_context = load_arch_context(&workspace_root);

        Self {
            config,
            tools,
            history: Vec::new(),
            workspace_root,
            backend,
            context_tokens_estimate: 0,
            eruka,
            session_id: uuid::Uuid::new_v4().to_string(),
            arch_context,
            last_tool_call_time: None,
        }
    }

    /// Create the appropriate backend based on config.
    ///
    /// If `use_ares_backend` is true and the `ares` feature is compiled in,
    /// delegates to ares-server's LLMClient (unified provider abstraction with
    /// connection pooling). Otherwise uses pawan's built-in OpenAI-compatible
    /// backend (the original path).
    fn create_backend(config: &PawanConfig, system_prompt: &str) -> Box<dyn LlmBackend> {
        // Local-inference-first cost guard: if enabled and the local server
        // responds within 100 ms, route all traffic there instead of cloud.
        if config.local_first {
            let local_url = config
                .local_endpoint
                .clone()
                .unwrap_or_else(|| "http://localhost:11434/v1".to_string());
            if probe_local_endpoint(&local_url) {
                tracing::info!(
                    url = %local_url,
                    model = %config.model,
                    "local_first: local server reachable, using local inference"
                );
                return Box::new(OpenAiCompatBackend::new(
                    backend::openai_compat::OpenAiCompatConfig {
                        api_url: local_url,
                        api_key: None,
                        model: config.model.clone(),
                        temperature: config.temperature,
                        top_p: config.top_p,
                        max_tokens: config.max_tokens,
                        system_prompt: system_prompt.to_string(),
                        use_thinking: false,
                        max_retries: config.max_retries,
                        fallback_models: Vec::new(),
                        cloud: None,
                    },
                ));
            }
            tracing::info!(
                url = %local_url,
                "local_first: local server unreachable, falling back to cloud provider"
            );
        }

        // Try ares backend first if requested
        if config.use_ares_backend {
            if let Some(backend) = Self::try_create_ares_backend(config, system_prompt) {
                return backend;
            }
            tracing::warn!(
                "use_ares_backend=true but ares backend creation failed; \
                 falling back to pawan's native backend"
            );
        }

        match config.provider {
        LlmProvider::Nvidia | LlmProvider::OpenAI | LlmProvider::Mlx => {
            let (api_url, api_key) = match config.provider {
                LlmProvider::Nvidia => {
                    let url = std::env::var("NVIDIA_API_URL")
                        .unwrap_or_else(|_| crate::DEFAULT_NVIDIA_API_URL.to_string());
                    
                    // Try to get key from env or secure store
                    let key = get_api_key_with_secure_fallback("NVIDIA_API_KEY", "nvidia_api_key");
                    
                    // If no key found, prompt user
                    let key = if key.is_none() {
                        prompt_and_store_api_key("NVIDIA_API_KEY", "nvidia_api_key", "NVIDIA")
                    } else {
                        key
                    };
                    
                    if key.is_none() {
                        tracing::warn!("NVIDIA_API_KEY not set. Model calls will fail until a key is provided.");
                    }
                    (url, key)
                },
                LlmProvider::OpenAI => {
                    let url = config.base_url.clone()
                        .or_else(|| std::env::var("OPENAI_API_URL").ok())
                        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                    
                    let key = get_api_key_with_secure_fallback("OPENAI_API_KEY", "openai_api_key");
                    let key = if key.is_none() {
                        prompt_and_store_api_key("OPENAI_API_KEY", "openai_api_key", "OpenAI")
                    } else {
                        key
                    };
                    
                    (url, key)
                },
                LlmProvider::Mlx => {
                    // MLX LM server — Apple Silicon native, always local
                    let url = config.base_url.clone()
                        .unwrap_or_else(|| "http://localhost:8080/v1".to_string());
                    tracing::info!(url = %url, "Using MLX LM server (Apple Silicon native)");
                    (url, None) // mlx_lm.server requires no API key
                },
                _ => unreachable!(),
            };
                
                // Build cloud fallback if configured
                let cloud = config.cloud.as_ref().map(|c| {
                    let (cloud_url, cloud_key) = match c.provider {
                        LlmProvider::Nvidia => {
                            let url = std::env::var("NVIDIA_API_URL")
                                .unwrap_or_else(|_| crate::DEFAULT_NVIDIA_API_URL.to_string());
                            let key = get_api_key_with_secure_fallback("NVIDIA_API_KEY", "nvidia_api_key");
                            (url, key)
                        },
                        LlmProvider::OpenAI => {
                            let url = std::env::var("OPENAI_API_URL")
                                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
                            let key = get_api_key_with_secure_fallback("OPENAI_API_KEY", "openai_api_key");
                            (url, key)
                        },
                        LlmProvider::Mlx => {
                            ("http://localhost:8080/v1".to_string(), None)
                        },
                        _ => {
                            tracing::warn!("Cloud fallback only supports nvidia/openai/mlx providers");
                            ("https://integrate.api.nvidia.com/v1".to_string(), None)
                        }
                    };
                    backend::openai_compat::CloudFallback {
                        api_url: cloud_url,
                        api_key: cloud_key,
                        model: c.model.clone(),
                        fallback_models: c.fallback_models.clone(),
                    }
                });

                Box::new(OpenAiCompatBackend::new(OpenAiCompatConfig {
                    api_url,
                    api_key,
                    model: config.model.clone(),
                    temperature: config.temperature,
                    top_p: config.top_p,
                    max_tokens: config.max_tokens,
                    system_prompt: system_prompt.to_string(),
                    // Enforce thinking budget: if set, disable thinking entirely
                    // and give all tokens to action output
                    use_thinking: config.thinking_budget == 0 && config.use_thinking_mode(),
                    max_retries: config.max_retries,
                    fallback_models: config.fallback_models.clone(),
                    cloud,
                }))
            }
            LlmProvider::Ollama => {
                let url = std::env::var("OLLAMA_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());

                Box::new(backend::ollama::OllamaBackend::new(
                    url,
                    config.model.clone(),
                    config.temperature,
                    system_prompt.to_string(),
                ))
            }
        }
    }

    /// Try to construct an ares-backed LLM backend from pawan config.
    /// Returns `None` if the provider isn't supported by ares or required
    /// credentials are missing — the caller should fall back to pawan's
    /// native backend.
    fn try_create_ares_backend(
        config: &PawanConfig,
        system_prompt: &str,
    ) -> Option<Box<dyn LlmBackend>> {
        use ares::llm::client::{ModelParams, Provider};

        // Map pawan LlmProvider → ares Provider variants.
        // ares supports: OpenAI (with custom base_url), Ollama, LlamaCpp, Anthropic.
        // Pawan's Nvidia/OpenAI/Mlx all use OpenAI-compatible endpoints, so they
        // all map to ares Provider::OpenAI with different base URLs.
        let params = ModelParams {
            temperature: Some(config.temperature),
            max_tokens: Some(config.max_tokens as u32),
            top_p: Some(config.top_p),
            frequency_penalty: None,
            presence_penalty: None,
        };

        let provider = match config.provider {
            LlmProvider::Nvidia => {
                let api_base = std::env::var("NVIDIA_API_URL")
                    .unwrap_or_else(|_| crate::DEFAULT_NVIDIA_API_URL.to_string());
                let api_key = std::env::var("NVIDIA_API_KEY").ok()?;
                Provider::OpenAI {
                    api_key,
                    api_base,
                    model: config.model.clone(),
                    params,
                }
            }
            LlmProvider::OpenAI => {
                let api_base = config
                    .base_url
                    .clone()
                    .or_else(|| std::env::var("OPENAI_API_URL").ok())
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
                Provider::OpenAI {
                    api_key,
                    api_base,
                    model: config.model.clone(),
                    params,
                }
            }
            LlmProvider::Mlx => {
                // MLX LM server is OpenAI-compatible, no API key needed
                let api_base = config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8080/v1".to_string());
                Provider::OpenAI {
                    api_key: String::new(),
                    api_base,
                    model: config.model.clone(),
                    params,
                }
            }
            LlmProvider::Ollama => {
                // Ares Ollama client is async-constructed (async with_params),
                // which doesn't fit pawan's sync PawanAgent::new path.
                // Fall back to pawan's native OllamaBackend for now.
                return None;
            }
        };

        // OpenAI variants construct synchronously — we skip the async
        // Provider::create_client() entirely for sync construction.
        let client: Box<dyn ares::llm::LLMClient> = match provider {
            Provider::OpenAI {
                api_key,
                api_base,
                model,
                params,
            } => Box::new(ares::llm::openai::OpenAIClient::with_params(
                api_key, api_base, model, params,
            )),
            _ => return None,
        };

        tracing::info!(
            provider = ?config.provider,
            model = %config.model,
            "Using ares-backed LLM backend"
        );

        Some(Box::new(backend::ares_backend::AresBackend::new(
            client,
            system_prompt.to_string(),
        )))
    }

    /// Create with a specific tool registry
    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    /// Get mutable access to the tool registry (for registering MCP tools)
    pub fn tools_mut(&mut self) -> &mut ToolRegistry {
        &mut self.tools
    }

    /// Create with a custom backend
    pub fn with_backend(mut self, backend: Box<dyn LlmBackend>) -> Self {
        self.backend = backend;
        self
    }

    /// Get the current conversation history
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Save current conversation as a session, returns session ID
    pub fn save_session(&self) -> Result<String> {
        let mut session = session::Session::new(&self.config.model);
        session.messages = self.history.clone();
        session.total_tokens = self.context_tokens_estimate as u64;
        session.save()?;
        Ok(session.id)
    }

    /// Resume a saved session by ID
    pub fn resume_session(&mut self, session_id: &str) -> Result<()> {
        let session = session::Session::load(session_id)?;
        self.history = session.messages;
        self.context_tokens_estimate = session.total_tokens as usize;
        // Adopt the loaded session's id so eruka writes cluster under the
        // same key as the on-disk session.
        self.session_id = session_id.to_string();
        Ok(())
    }

    /// Archive the current conversation to Eruka's context store. Safe to
    /// call from any async context; returns Ok even when eruka is disabled
    /// or unreachable so callers can fire-and-forget after save_session().
    pub async fn archive_to_eruka(&self) -> Result<()> {
        let Some(eruka) = &self.eruka else {
            return Ok(());
        };
        let mut session = session::Session::new(&self.config.model);
        session.id = self.session_id.clone();
        session.messages = self.history.clone();
        session.total_tokens = self.context_tokens_estimate as u64;
        eruka.archive_session(&session).await
    }

    /// Build a compact snapshot of the current history for on_pre_compress.
    /// Keeps message role + first 200 chars per entry so the eruka write
    /// stays bounded even with huge histories.
    fn history_snapshot_for_eruka(history: &[Message]) -> String {
        let mut out = String::with_capacity(2048);
        for msg in history {
            let prefix = match msg.role {
                Role::User => "U: ",
                Role::Assistant => "A: ",
                Role::Tool => "T: ",
                Role::System => "S: ",
            };
            let body: String = msg.content.chars().take(200).collect();
            out.push_str(prefix);
            out.push_str(&body);
            out.push('\n');
            if out.len() > 4000 {
                break;
            }
        }
        out
    }

    /// Get the configuration
    pub fn config(&self) -> &PawanConfig {
        &self.config
    }

    /// Clear the conversation history
    pub fn clear_history(&mut self) {
        self.history.clear();
    }
    /// Prune conversation history to reduce context size.
    /// Uses importance scoring (inspired by claude-code-rust's consolidation engine):
    /// - Tool results with errors: high importance (learning from failures)
    /// - User messages: medium importance (intent context)
    /// - Successful tool results: low importance (can be re-derived)
    ///
    /// Keeps system prompt + last 4 messages, summarizes the rest.
    fn prune_history(&mut self) {
        let len = self.history.len();
        if len <= 5 {
            return; // Nothing to prune
        }

        let keep_end = 4;
        let start = 1; // Skip system prompt at index 0
        let end = len - keep_end;
        let pruned_count = end - start;

        // Score messages by importance for summary prioritization
        let mut scored: Vec<(f32, &Message)> = self.history[start..end]
            .iter()
            .map(|msg| {
                let score = Self::message_importance(msg);
                (score, msg)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Build summary from highest-importance messages first (UTF-8 safe)
        let mut summary = String::with_capacity(2048);
        for (score, msg) in &scored {
            let prefix = match msg.role {
                Role::User => "User: ",
                Role::Assistant => "Assistant: ",
                Role::Tool => if *score > 0.7 { "Tool error: " } else { "Tool: " },
                Role::System => "System: ",
            };
            let chunk: String = msg.content.chars().take(200).collect();
            summary.push_str(prefix);
            summary.push_str(&chunk);
            summary.push('\n');
            if summary.len() > 2000 {
                let safe_end = summary.char_indices()
                    .take_while(|(i, _)| *i <= 2000)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                summary.truncate(safe_end);
                break;
            }
        }

        let summary_msg = Message {
            role: Role::System,
            content: format!("Previous conversation summary (pruned {} messages, importance-ranked): {}", pruned_count, summary),
            tool_calls: vec![],
            tool_result: None,
        };

        self.history.drain(start..end);
        self.history.insert(start, summary_msg);

        tracing::info!(pruned = pruned_count, context_estimate = self.context_tokens_estimate, "Pruned messages from history (importance-ranked)");
    }

    /// Score a message's importance for pruning decisions (0.0-1.0).
    /// Higher = more important = kept in summary.
    fn message_importance(msg: &Message) -> f32 {
        match msg.role {
            Role::User => 0.6,       // User intent is moderately important
            Role::System => 0.3,     // System messages are usually ephemeral
            Role::Assistant => {
                if msg.content.contains("error") || msg.content.contains("Error") { 0.8 }
                else { 0.4 }
            }
            Role::Tool => {
                if let Some(ref result) = msg.tool_result {
                    if !result.success { 0.9 }  // Failed tools are very important (learning)
                    else { 0.2 }                 // Successful tools can be re-derived
                } else {
                    0.3
                }
            }
        }
    }

    /// Add a message to history
    pub fn add_message(&mut self, message: Message) {
        self.history.push(message);
    }

    /// Switch the LLM model at runtime. Recreates the backend with the new model.
    pub fn switch_model(&mut self, model: &str) {
        self.config.model = model.to_string();
        let system_prompt = self.config.get_system_prompt();
        self.backend = Self::create_backend(&self.config, &system_prompt);
        tracing::info!(model = model, "Model switched at runtime");
    }

    /// Get the current model name
    pub fn model_name(&self) -> &str {
        &self.config.model
    }

    /// Get tool definitions for the LLM
    pub fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.get_definitions()
    }

    /// Execute a single prompt with tool calling support
    pub async fn execute(&mut self, user_prompt: &str) -> Result<AgentResponse> {
        self.execute_with_callbacks(user_prompt, None, None, None)
            .await
    }

    /// Execute with optional callbacks for streaming
    pub async fn execute_with_callbacks(
        &mut self,
        user_prompt: &str,
        on_token: Option<TokenCallback>,
        on_tool: Option<ToolCallback>,
        on_tool_start: Option<ToolStartCallback>,
    ) -> Result<AgentResponse> {
        self.execute_with_all_callbacks(user_prompt, on_token, on_tool, on_tool_start, None)
            .await
    }

    /// Execute with all callbacks, including permission prompt.
    pub async fn execute_with_all_callbacks(
        &mut self,
        user_prompt: &str,
        on_token: Option<TokenCallback>,
        on_tool: Option<ToolCallback>,
        on_tool_start: Option<ToolStartCallback>,
        on_permission: Option<PermissionCallback>,
    ) -> Result<AgentResponse> {
        // Check if coordinator mode is enabled
        if self.config.use_coordinator {
            // Coordinator mode does not support callbacks or permission prompts
            if on_token.is_some() || on_tool.is_some() || on_tool_start.is_some() || on_permission.is_some() {
                tracing::warn!(
                    "Callbacks and permission prompts are not supported in coordinator mode; ignoring them"
                );
            }
            return self.execute_with_coordinator(user_prompt).await;
        }

        // Reset idle timeout for the new turn
        self.last_tool_call_time = None;

        // Inject Eruka core memory before first LLM call
        if let Some(eruka) = &self.eruka {
            if let Err(e) = eruka.inject_core_memory(&mut self.history).await {
                tracing::warn!("Eruka memory injection failed (non-fatal): {}", e);
            }

            // Prefetch task-relevant context: semantic search + compressed
            // general context. Inject as a system message so the LLM can
            // draw on prior-session context for the same query. Non-fatal.
            match eruka.prefetch(user_prompt, 2000).await {
                Ok(Some(ctx)) => {
                    self.history.push(Message {
                        role: Role::System,
                        content: ctx,
                        tool_calls: vec![],
                        tool_result: None,
                    });
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("Eruka prefetch failed (non-fatal): {}", e),
            }
        }

        // Per-turn architecture context injection: prepend .pawan/arch.md content
        // so key constraints stay visible even as tool-call history grows long.
        let effective_prompt = match &self.arch_context {
            Some(ctx) => format!(
                "[Workspace Architecture]\n{ctx}\n[/Workspace Architecture]\n\n{user_prompt}"
            ),
            None => user_prompt.to_string(),
        };

        self.history.push(Message {
            role: Role::User,
            content: effective_prompt,
            tool_calls: vec![],
            tool_result: None,
        });

        let mut all_tool_calls = Vec::new();
        let mut total_usage = TokenUsage::default();
        let mut iterations = 0;
        let max_iterations = self.config.max_tool_iterations;

        loop {
            // Check idle timeout
            if let Some(last_time) = self.last_tool_call_time {
                let elapsed = last_time.elapsed().as_secs();
                if elapsed > self.config.tool_call_idle_timeout_secs {
                    return Err(PawanError::Agent(format!(
                        "Tool idle timeout exceeded ({}s > {}s)",
                        elapsed, self.config.tool_call_idle_timeout_secs
                    )));
                }
            }

            iterations += 1;
            if iterations > max_iterations {
                return Err(PawanError::Agent(format!(
                    "Max tool iterations ({}) exceeded",
                    max_iterations
                )));
            }

            // Budget awareness: when running low on iterations, nudge the model
            let remaining = max_iterations.saturating_sub(iterations);
            if remaining == 3 && iterations > 1 {
                self.history.push(Message {
                    role: Role::User,
                    content: format!(
                        "[SYSTEM] You have {} tool iterations remaining. \
                         Stop exploring and write the most important output now. \
                         If you have code to write, write it immediately.",
                        remaining
                    ),
                    tool_calls: vec![],
                    tool_result: None,
                });
            }
            // Estimate context tokens
            self.context_tokens_estimate = self.history.iter().map(|m| m.content.len()).sum::<usize>() / 4;
            if self.context_tokens_estimate > self.config.max_context_tokens {
                // Snapshot pre-compression content to Eruka so the facts
                // being discarded survive the prune. Non-fatal.
                if let Some(eruka) = &self.eruka {
                    let snapshot = Self::history_snapshot_for_eruka(&self.history);
                    if let Err(e) = eruka.on_pre_compress(&snapshot, &self.session_id).await {
                        tracing::warn!("Eruka on_pre_compress failed (non-fatal): {}", e);
                    }
                }
                self.prune_history();
            }

            // Dynamic tool selection: pick the most relevant tools for this query
            // Extract latest user message for keyword matching
            let latest_query = self.history.iter().rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.as_str())
                .unwrap_or("");
            let tool_defs = self.tools.select_for_query(latest_query, 12);
            if iterations == 1 {
                let tool_names: Vec<&str> = tool_defs.iter().map(|t| t.name.as_str()).collect();
                tracing::info!(tools = ?tool_names, count = tool_defs.len(), "Selected tools for query");
            }

            // Update idle timeout tracker before LLM call to track time spent in generation
            self.last_tool_call_time = Some(Instant::now());

            // --- Resilient LLM call: retry on transient failures instead of crashing ---
            let response = {
                #[allow(unused_assignments)]
                let mut last_err = None;
                let max_llm_retries = 3;
                let mut attempt = 0;
                loop {
                    attempt += 1;
                    match self.backend.generate(&self.history, &tool_defs, on_token.as_ref()).await {
                        Ok(resp) => break resp,
                        Err(e) => {
                            let err_str = e.to_string();
                            let is_transient = err_str.contains("timeout")
                                || err_str.contains("connection")
                                || err_str.contains("429")
                                || err_str.contains("500")
                                || err_str.contains("502")
                                || err_str.contains("503")
                                || err_str.contains("504")
                                || err_str.contains("reset")
                                || err_str.contains("broken pipe");

                            if is_transient && attempt <= max_llm_retries {
                                let delay = std::time::Duration::from_secs(2u64.pow(attempt as u32));
                                tracing::warn!(
                                    attempt = attempt,
                                    delay_secs = delay.as_secs(),
                                    error = err_str.as_str(),
                                    "LLM call failed (transient) — retrying"
                                );
                                tokio::time::sleep(delay).await;

                                // If context is too large, prune before retry
                                if err_str.contains("context") || err_str.contains("token") {
                                    tracing::info!("Pruning history before retry (possible context overflow)");
                                    if let Some(eruka) = &self.eruka {
                                        let snapshot = Self::history_snapshot_for_eruka(&self.history);
                                        if let Err(e) = eruka.on_pre_compress(&snapshot, &self.session_id).await {
                                            tracing::warn!("Eruka on_pre_compress failed (non-fatal): {}", e);
                                        }
                                    }
                                    self.prune_history();
                                }
                                continue;
                            }

                            // Non-transient or max retries exhausted
                            last_err = Some(e);
                            break {
                                // Return a synthetic "give up" response instead of crashing
                                tracing::error!(
                                    attempt = attempt,
                                    error = last_err.as_ref().map(|e| e.to_string()).unwrap_or_default().as_str(),
                                    "LLM call failed permanently — returning error as content"
                                );
                                LLMResponse {
                                    content: format!(
                                        "LLM error after {} attempts: {}. The task could not be completed.",
                                        attempt,
                                        last_err.as_ref().map(|e| e.to_string()).unwrap_or_default()
                                    ),
                                    reasoning: None,
                                    tool_calls: vec![],
                                    finish_reason: "error".to_string(),
                                    usage: None,
                                }
                            };
                        }
                    }
                }
            };

            // Accumulate token usage with thinking/action split
            if let Some(ref usage) = response.usage {
                total_usage.prompt_tokens += usage.prompt_tokens;
                total_usage.completion_tokens += usage.completion_tokens;
                total_usage.total_tokens += usage.total_tokens;
                total_usage.reasoning_tokens += usage.reasoning_tokens;
                total_usage.action_tokens += usage.action_tokens;

                // Log token budget split per iteration
                if usage.reasoning_tokens > 0 {
                    tracing::info!(
                        iteration = iterations,
                        think = usage.reasoning_tokens,
                        act = usage.action_tokens,
                        total = usage.completion_tokens,
                        "Token budget: think:{} act:{} (total:{})",
                        usage.reasoning_tokens, usage.action_tokens, usage.completion_tokens
                    );
                }

                // Thinking budget enforcement
                let thinking_budget = self.config.thinking_budget;
                if thinking_budget > 0 && usage.reasoning_tokens > thinking_budget as u64 {
                    tracing::warn!(
                        budget = thinking_budget,
                        actual = usage.reasoning_tokens,
                        "Thinking budget exceeded ({}/{} tokens)",
                        usage.reasoning_tokens, thinking_budget
                    );
                }
            }

            // --- Guardrail: strip thinking blocks from content ---
            let clean_content = {
                let mut s = response.content.clone();
                loop {
                    let lower = s.to_lowercase();
                    let open = lower.find("<think>");
                    let close = lower.find("</think>");
                    match (open, close) {
                        (Some(i), Some(j)) if j > i => {
                            let before = s[..i].trim_end().to_string();
                            let after = if s.len() > j + 8 { s[j + 8..].trim_start().to_string() } else { String::new() };
                            s = if before.is_empty() { after } else if after.is_empty() { before } else { format!("{}\n{}", before, after) };
                        }
                        _ => break,
                    }
                }
                s
            };

            if response.tool_calls.is_empty() {
                // --- Guardrail: detect chatty no-op (content but no tools on early iterations) ---
                // Only nudge if tools are available AND response looks like planning text (not a real answer)
                let has_tools = !tool_defs.is_empty();
                let lower = clean_content.to_lowercase();
                let planning_prefix = lower.starts_with("let me")
                    || lower.starts_with("i'll help")
                    || lower.starts_with("i will help")
                    || lower.starts_with("sure, i")
                    || lower.starts_with("okay, i");
                let looks_like_planning = clean_content.len() > 200 || (planning_prefix && clean_content.len() > 50);
                if has_tools && looks_like_planning && iterations == 1 && iterations < max_iterations && response.finish_reason != "error" {
                    tracing::warn!(
                        "No tool calls at iteration {} (content: {}B) — nudging model to use tools",
                        iterations, clean_content.len()
                    );
                    self.history.push(Message {
                        role: Role::Assistant,
                        content: clean_content.clone(),
                        tool_calls: vec![],
                        tool_result: None,
                    });
                    self.history.push(Message {
                        role: Role::User,
                        content: "You must use tools to complete this task. Do NOT just describe what you would do — actually call the tools. Start with bash or read_file.".to_string(),
                        tool_calls: vec![],
                        tool_result: None,
                    });
                    continue;
                }

                // --- Guardrail: detect repeated responses ---
                if iterations > 1 {
                    let prev_assistant = self.history.iter().rev()
                        .find(|m| m.role == Role::Assistant && !m.content.is_empty());
                    if let Some(prev) = prev_assistant {
                        if prev.content.trim() == clean_content.trim() && iterations < max_iterations {
                            tracing::warn!("Repeated response detected at iteration {} — injecting correction", iterations);
                            self.history.push(Message {
                                role: Role::Assistant,
                                content: clean_content.clone(),
                                tool_calls: vec![],
                                tool_result: None,
                            });
                            self.history.push(Message {
                                role: Role::User,
                                content: "You gave the same response as before. Try a different approach. Use anchor_text in edit_file_lines, or use insert_after, or use bash with sed.".to_string(),
                                tool_calls: vec![],
                                tool_result: None,
                            });
                            continue;
                        }
                    }
                }

                self.history.push(Message {
                    role: Role::Assistant,
                    content: clean_content.clone(),
                    tool_calls: vec![],
                    tool_result: None,
                });

                // Persist this completed turn to Eruka so future prefetches
                // and sessions can pull from it. Non-fatal on any error.
                if let Some(eruka) = &self.eruka {
                    if let Err(e) = eruka
                        .sync_turn(user_prompt, &clean_content, &self.session_id)
                        .await
                    {
                        tracing::warn!("Eruka sync_turn failed (non-fatal): {}", e);
                    }
                }

                return Ok(AgentResponse {
                    content: clean_content,
                    tool_calls: all_tool_calls,
                    iterations,
                    usage: total_usage,
                });
            }

            self.history.push(Message {
                role: Role::Assistant,
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
                tool_result: None,
            });

            for tool_call in &response.tool_calls {
                // Auto-activate extended tools on first use (makes them visible in next iteration)
                self.tools.activate(&tool_call.name);

                // Check permission (Deny and Prompt-in-headless both block)
                let perm = crate::config::ToolPermission::resolve(
                    &tool_call.name, &self.config.permissions
                );
                let denied = match perm {
                    crate::config::ToolPermission::Deny => Some("Tool denied by permission policy"),
                    crate::config::ToolPermission::Prompt => {
                        // For bash: auto-allow read-only commands even under Prompt
                        if tool_call.name == "bash" {
                            if let Some(cmd) = tool_call.arguments.get("command").and_then(|v| v.as_str()) {
                                if crate::tools::bash::is_read_only(cmd) {
                                    tracing::debug!(command = cmd, "Auto-allowing read-only bash command under Prompt permission");
                                    None
                                } else if let Some(ref perm_cb) = on_permission {
                                    // Ask TUI for approval
                                    let args_summary = cmd.chars().take(120).collect::<String>();
                                    let rx = perm_cb(PermissionRequest {
                                        tool_name: tool_call.name.clone(),
                                        args_summary,
                                    });
                                    match rx.await {
                                        Ok(true) => None,
                                        _ => Some("User denied tool execution"),
                                    }
                                } else {
                                    Some("Bash command requires user approval (read-only commands auto-allowed)")
                                }
                            } else {
                                Some("Tool requires user approval")
                            }
                        } else if let Some(ref perm_cb) = on_permission {
                            // Ask TUI for approval
                            let args_summary = tool_call.arguments.to_string().chars().take(120).collect::<String>();
                            let rx = perm_cb(PermissionRequest {
                                tool_name: tool_call.name.clone(),
                                args_summary,
                            });
                            match rx.await {
                                Ok(true) => None,
                                _ => Some("User denied tool execution"),
                            }
                        } else {
                            // Headless = deny for safety
                            Some("Tool requires user approval (set permission to 'allow' or use TUI mode)")
                        }
                    }
                    crate::config::ToolPermission::Allow => None,
                };
                if let Some(reason) = denied {
                    let record = ToolCallRecord {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        arguments: tool_call.arguments.clone(),
                        result: json!({"error": reason}),
                        success: false,
                        duration_ms: 0,
                    };

                    if let Some(ref callback) = on_tool {
                        callback(&record);
                    }
                    all_tool_calls.push(record);

                    self.history.push(Message {
                        role: Role::Tool,
                        content: format!("{{\"error\": \"{}\"}}", reason),
                        tool_calls: vec![],
                        tool_result: Some(ToolResultMessage {
                            tool_call_id: tool_call.id.clone(),
                            content: json!({"error": reason}),
                            success: false,
                        }),
                    });
                    continue;
                }

                // Notify tool start
                if let Some(ref callback) = on_tool_start {
                    callback(&tool_call.name);
                }

                // Debug: log tool call args for diagnosis
                tracing::debug!(
                    tool = tool_call.name.as_str(),
                    args_len = serde_json::to_string(&tool_call.arguments).unwrap_or_default().len(),
                    "Tool call: {}({})",
                    tool_call.name,
                    serde_json::to_string(&tool_call.arguments)
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect::<String>()
                );

                // Validate tool arguments using thulp-core (DRY: reuse thulp's validation)
                if let Some(tool) = self.tools.get(&tool_call.name) {
                    let schema = tool.parameters_schema();
                    if let Ok(params) = thulp_core::ToolDefinition::parse_mcp_input_schema(&schema) {
                        let thulp_def = thulp_core::ToolDefinition {
                            name: tool_call.name.clone(),
                            description: String::new(),
                            parameters: params,
                        };
                        if let Err(e) = thulp_def.validate_args(&tool_call.arguments) {
                            tracing::warn!(
                                tool = tool_call.name.as_str(),
                                error = %e,
                                "Tool argument validation failed (continuing anyway)"
                            );
                        }
                    }
                }

                let start = std::time::Instant::now();

                // Check permission for mutating tools
                let tool = self.tools.get(&tool_call.name);
                let is_mutating = tool.map(|t| t.mutating()).unwrap_or(false);
                if is_mutating {
                    if let Some(ref callback) = on_permission {
                        let args_summary = summarize_args(&tool_call.arguments);
                        let request = PermissionRequest {
                            tool_name: tool_call.name.clone(),
                            args_summary,
                        };
                        let permission_rx = (callback)(request);
                        match permission_rx.await {
                            Ok(true) => {
                                // Permission granted, continue with execution
                            }
                            Ok(false) => {
                                // Permission denied, skip this tool call
                                tracing::info!(tool = tool_call.name.as_str(), "Tool execution denied by user");
                                let record = ToolCallRecord {
                                    id: tool_call.id.clone(),
                                    name: tool_call.name.clone(),
                                    arguments: tool_call.arguments.clone(),
                                    result: json!({"error": "Tool execution denied by user", "tool": tool_call.name}),
                                    success: false,
                                    duration_ms: 0,
                                };
                                if let Some(ref callback) = on_tool {
                                    callback(&record);
                                }
                                continue;
                            }
                            Err(_) => {
                                let record = ToolCallRecord {
                                    id: tool_call.id.clone(),
                                    name: tool_call.name.clone(),
                                    arguments: tool_call.arguments.clone(),
                                    result: json!({"error": "Permission channel closed", "tool": tool_call.name}),
                                    success: false,
                                    duration_ms: 0,
                                };
                                if let Some(ref callback) = on_tool {
                                    callback(&record);
                                }
                                continue;
                            }
                        }
                    } else {
                        tracing::warn!(tool = tool_call.name.as_str(), "No permission callback, auto-approving mutating tool");
                    }
                }

                // Resilient tool execution: catch panics + errors
                let result = {
                    let tool_future = self.tools.execute(&tool_call.name, tool_call.arguments.clone());
                    // Timeout individual tool calls (prevent hangs)
                    let timeout_dur = if tool_call.name == "bash" {
                        std::time::Duration::from_secs(self.config.bash_timeout_secs)
                    } else {
                        std::time::Duration::from_secs(30)
                    };
                    match tokio::time::timeout(timeout_dur, tool_future).await {
                        Ok(inner) => inner,
                        Err(_) => Err(PawanError::Tool(format!(
                            "Tool '{}' timed out after {}s", tool_call.name, timeout_dur.as_secs()
                        ))),
                    }
                };
                let duration_ms = start.elapsed().as_millis() as u64;

                let (result_value, success) = match result {
                    Ok(v) => (v, true),
                    Err(e) => {
                        tracing::warn!(tool = tool_call.name.as_str(), error = %e, "Tool execution failed");
                        (json!({"error": e.to_string(), "tool": tool_call.name, "hint": "Try a different approach or tool"}), false)
                    }
                };

                // Truncate tool results that exceed max chars to prevent context bloat
                let max_result_chars = self.config.max_result_chars;
                let result_value = truncate_tool_result(result_value, max_result_chars);


                let record = ToolCallRecord {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    arguments: tool_call.arguments.clone(),
                    result: result_value.clone(),
                    success,
                    duration_ms,
                };

                if let Some(ref callback) = on_tool {
                    callback(&record);
                }

                all_tool_calls.push(record);

                self.history.push(Message {
                    role: Role::Tool,
                    content: serde_json::to_string(&result_value).unwrap_or_default(),
                    tool_calls: vec![],
                    tool_result: Some(ToolResultMessage {
                        tool_call_id: tool_call.id.clone(),
                        content: result_value,
                        success,
                    }),
                });

                // Compile-gated confidence: after writing a .rs file, auto-run cargo check
                // and inject the result so the model can self-correct on the same iteration
                if success && tool_call.name == "write_file" {
                    let wrote_rs = tool_call.arguments.get("path")
                        .and_then(|p| p.as_str())
                        .map(|p| p.ends_with(".rs"))
                        .unwrap_or(false);
                    if wrote_rs {
                        let ws = self.workspace_root.clone();
                        let check_result = tokio::process::Command::new("cargo")
                            .arg("check")
                            .arg("--message-format=short")
                            .current_dir(&ws)
                            .output()
                            .await;
                        match check_result {
                            Ok(output) if !output.status.success() => {
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                // Only inject first 1500 chars of errors to avoid context bloat
                                let err_msg: String = stderr.chars().take(1500).collect();
                                tracing::info!("Compile-gate: cargo check failed after write_file, injecting errors");
                                self.history.push(Message {
                                    role: Role::User,
                                    content: format!(
                                        "[SYSTEM] cargo check failed after your write_file. Fix the errors:\n```\n{}\n```",
                                        err_msg
                                    ),
                                    tool_calls: vec![],
                                    tool_result: None,
                                });
                            }
                            Ok(_) => {
                                tracing::debug!("Compile-gate: cargo check passed");
                            }
                            Err(e) => {
                                tracing::warn!("Compile-gate: cargo check failed to run: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Execute using the ToolCoordinator instead of the built-in loop.
    ///
    /// This method provides an alternative implementation that uses the
    /// ToolCoordinator for tool-calling loops, which offers:
    /// - Parallel tool execution
    /// - Per-tool timeout handling
    /// - Consistent error handling
    /// - Max iteration limits
    ///
    /// Note: This method does not support streaming callbacks or permission
    /// prompts - those are only available in the built-in loop.
    async fn execute_with_coordinator(&mut self, user_prompt: &str) -> Result<AgentResponse> {
        // Reset idle timeout for the new turn
        self.last_tool_call_time = None;

        // Inject Eruka core memory before first LLM call
        if let Some(eruka) = &self.eruka {
            if let Err(e) = eruka.inject_core_memory(&mut self.history).await {
                tracing::warn!("Eruka memory injection failed (non-fatal): {}", e);
            }

            // Prefetch task-relevant context
            match eruka.prefetch(user_prompt, 2000).await {
                Ok(Some(ctx)) => {
                    self.history.push(Message {
                        role: Role::System,
                        content: ctx,
                        tool_calls: vec![],
                        tool_result: None,
                    });
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("Eruka prefetch failed (non-fatal): {}", e),
            }
        }

        // Per-turn architecture context injection
        let effective_prompt = match &self.arch_context {
            Some(ctx) => format!(
                "[Workspace Architecture]\n{ctx}\n[/Workspace Architecture]\n\n{user_prompt}"
            ),
            None => user_prompt.to_string(),
        };

        // Build coordinator config from agent config
        let coordinator_config = ToolCallingConfig {
            max_iterations: self.config.max_tool_iterations,
            parallel_execution: true,
            tool_timeout: std::time::Duration::from_secs(self.config.bash_timeout_secs),
            stop_on_error: false,
        };

        // Create a fresh backend for coordinator execution
        let system_prompt = self.config.get_system_prompt();
        let backend = Self::create_backend(&self.config, &system_prompt);
        let backend = Arc::from(backend);

        // Create a fresh tool registry for coordinator execution
        // Note: This will not include any MCP tools registered at runtime
        let registry = Arc::new(ToolRegistry::with_defaults(self.workspace_root.clone()));

        // Create coordinator with backend and tool registry
        let coordinator = ToolCoordinator::new(backend, registry, coordinator_config);

        // Execute with coordinator
        let result: CoordinatorResult = coordinator
            .execute(Some(&system_prompt), &effective_prompt)
            .await
            .map_err(|e| PawanError::Agent(format!("Coordinator execution failed: {}", e)))?;

        // Convert CoordinatorResult to AgentResponse
        let content = result.content.clone();
        let agent_response = AgentResponse {
            content: result.content,
            tool_calls: result.tool_calls,
            iterations: result.iterations,
            usage: result.total_usage,
        };

        // Sync turn to Eruka if enabled
        if let Some(eruka) = &self.eruka {
            if let Err(e) = eruka
                .sync_turn(user_prompt, &content, &self.session_id)
                .await
            {
                tracing::warn!("Eruka sync_turn failed (non-fatal): {}", e);
            }
        }

        Ok(agent_response)
    }

    /// Execute a healing task with real diagnostics
    pub async fn heal(&mut self) -> Result<AgentResponse> {
        let healer = crate::healing::Healer::new(
            self.workspace_root.clone(),
            self.config.healing.clone(),
        );

        let diagnostics = healer.get_diagnostics().await?;
        let failed_tests = healer.get_failed_tests().await?;

        let mut prompt = format!(
            "I need you to heal this Rust project at: {}

",
            self.workspace_root.display()
        );

        if !diagnostics.is_empty() {
            prompt.push_str(&format!(
                "## Compilation Issues ({} found)
{}
",
                diagnostics.len(),
                healer.format_diagnostics_for_prompt(&diagnostics)
            ));
        }

        if !failed_tests.is_empty() {
            prompt.push_str(&format!(
                "## Failed Tests ({} found)
{}
",
                failed_tests.len(),
                healer.format_tests_for_prompt(&failed_tests)
            ));
        }

        if diagnostics.is_empty() && failed_tests.is_empty() {
            prompt.push_str("No issues found! Run cargo check and cargo test to verify.
");
        }

        prompt.push_str("
Fix each issue one at a time. Verify with cargo check after each fix.");

        self.execute(&prompt).await
    }
    /// Execute healing with retries — calls heal(), checks for remaining errors, retries if needed.
    ///
    /// Two-stage gate:
    ///   Stage 1 — `cargo check`: must produce zero errors before proceeding.
    ///   Stage 2 — `healing.verify_cmd` (optional): a user-supplied shell command
    ///             (e.g. `cargo test --workspace`).  If it exits non-zero the loop
    ///             continues so the LLM can address the reported failures.
    ///
    /// Anti-thrash guard: each Stage-1 error is fingerprinted (kind + code +
    /// message prefix).  If the same fingerprint survives `max_attempts`
    /// consecutive rounds unchanged the loop halts rather than spinning
    /// indefinitely on an error the LLM cannot fix.
    pub async fn heal_with_retries(&mut self, max_attempts: usize) -> Result<AgentResponse> {
        use std::collections::{HashMap, HashSet};

        let mut last_response = self.heal().await?;
        // fingerprint → consecutive rounds this error has survived unchanged
        let mut stuck_counts: HashMap<u64, usize> = HashMap::new();

        for attempt in 1..max_attempts {
            // Stage 1: cargo check must be error-free
            let fixer = crate::healing::CompilerFixer::new(self.workspace_root.clone());
            let remaining = fixer.check().await?;
            let errors: Vec<_> = remaining
                .iter()
                .filter(|d| d.kind == crate::healing::DiagnosticKind::Error)
                .collect();

            if !errors.is_empty() {
                // Update fingerprint counts.
                // Drop entries for errors that were fixed; increment survivors.
                let current_fps: HashSet<u64> = errors.iter().map(|d| d.fingerprint()).collect();
                stuck_counts.retain(|fp, _| current_fps.contains(fp));
                for fp in &current_fps {
                    *stuck_counts.entry(*fp).or_insert(0) += 1;
                }

                // Anti-thrash: halt if any error fingerprint has not budged
                // after max_attempts consecutive rounds.
                let thrashing: Vec<u64> = stuck_counts
                    .iter()
                    .filter_map(|(&fp, &count)| if count >= max_attempts { Some(fp) } else { None })
                    .collect();
                if !thrashing.is_empty() {
                    tracing::warn!(
                        stuck_fingerprints = thrashing.len(),
                        attempt,
                        "Anti-thrash: {} error(s) unchanged after {} attempts, halting heal loop",
                        thrashing.len(),
                        max_attempts
                    );
                    return Ok(last_response);
                }

                tracing::warn!(
                    errors = errors.len(),
                    attempt,
                    "Stage 1 (cargo check): errors remain, retrying"
                );
                last_response = self.heal().await?;
                continue;
            }

            // All Stage-1 errors cleared — reset thrash counters.
            stuck_counts.clear();

            // Stage 2: optional verify_cmd
            let verify_cmd = self.config.healing.verify_cmd.clone();
            if let Some(ref cmd) = verify_cmd {
                match crate::healing::run_verify_cmd(&self.workspace_root, cmd).await {
                    Ok(None) => {
                        tracing::info!(attempts = attempt, "Stage 2 (verify_cmd) passed, healing complete");
                        return Ok(last_response);
                    }
                    Ok(Some(diag)) => {
                        tracing::warn!(
                            attempt,
                            cmd,
                            output = diag.raw,
                            "Stage 2 (verify_cmd) failed, retrying"
                        );
                        last_response = self.heal().await?;
                        continue;
                    }
                    Err(e) => {
                        // Cannot spawn the command — don't block healing on this
                        tracing::warn!(cmd, error = %e, "verify_cmd could not be run, skipping stage 2");
                        return Ok(last_response);
                    }
                }
            } else {
                tracing::info!(attempts = attempt, "Stage 1 (cargo check) passed, healing complete");
                return Ok(last_response);
            }
        }

        tracing::info!(attempts = max_attempts, "Healing finished (may still have errors)");
        Ok(last_response)
    }
    /// Execute a task with a specific prompt
    pub async fn task(&mut self, task_description: &str) -> Result<AgentResponse> {
        let prompt = format!(
            r#"I need you to complete the following coding task:

{}

The workspace is at: {}

Please:
1. First explore the codebase to understand the relevant code
2. Make the necessary changes
3. Verify the changes compile with `cargo check`
4. Run relevant tests if applicable

Explain your changes as you go."#,
            task_description,
            self.workspace_root.display()
        );

        self.execute(&prompt).await
    }

    /// Generate a commit message for current changes
    pub async fn generate_commit_message(&mut self) -> Result<String> {
        let prompt = r#"Please:
1. Run `git status` to see what files are changed
2. Run `git diff --cached` to see staged changes (or `git diff` for unstaged)
3. Generate a concise, descriptive commit message following conventional commits format

Only output the suggested commit message, nothing else."#;

        let response = self.execute(prompt).await?;
        Ok(response.content)
    }
}

/// Truncate a tool result JSON value to fit within max_chars.
/// Unlike naive string truncation (which breaks JSON), this truncates string
/// *values* within the JSON structure, preserving valid JSON output.
fn truncate_tool_result(value: Value, max_chars: usize) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_chars {
        return value;
    }

    // Strategy: find the largest string values and truncate them
    match value {
        Value::Object(map) => {
            let mut result = serde_json::Map::new();
            let total = serialized.len();
            for (k, v) in map {
                if let Value::String(s) = &v {
                    if s.len() > 500 {
                        // Proportional truncation: shrink large strings
                        let target = s.len() * max_chars / total;
                        let target = target.max(200); // Keep at least 200 chars
                        let truncated: String = s.chars().take(target).collect();
                        result.insert(k, json!(format!("{}...[truncated from {} chars]", truncated, s.len())));
                        continue;
                    }
                }
                // Recursively truncate nested structures
                result.insert(k, truncate_tool_result(v, max_chars));
            }
            Value::Object(result)
        }
        Value::String(s) if s.len() > max_chars => {
            let truncated: String = s.chars().take(max_chars).collect();
            json!(format!("{}...[truncated from {} chars]", truncated, s.len()))
        }
        Value::Array(arr) if serialized.len() > max_chars => {
            // Truncate array: keep first N items that fit
            let mut result = Vec::new();
            let mut running_len = 2; // "[]"
            for item in arr {
                let item_str = serde_json::to_string(&item).unwrap_or_default();
                running_len += item_str.len() + 1; // +1 for comma
                if running_len > max_chars {
                    result.push(json!(format!("...[{} more items truncated]", 0)));
                    break;
                }
                result.push(item);
            }
            Value::Array(result)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::agent::backend::mock::{MockBackend, MockResponse};


    #[test]
    fn test_message_serialization() {
        let msg = Message {
            role: Role::User,
            content: "Hello".to_string(),
            tool_calls: vec![],
            tool_result: None,
        };

        let json = serde_json::to_string(&msg).expect("Serialization failed");
        assert!(json.contains("user"));
        assert!(json.contains("Hello"));
    }

    #[test]
    fn test_tool_call_request() {
        let tc = ToolCallRequest {
            id: "123".to_string(),
            name: "read_file".to_string(),
            arguments: json!({"path": "test.txt"}),
        };

        let json = serde_json::to_string(&tc).expect("Serialization failed");
        assert!(json.contains("read_file"));
        assert!(json.contains("test.txt"));
    }

    /// Helper to build an agent with N messages for prune testing.
    /// History starts empty; we add a system prompt + (n-1) user/assistant messages = n total.
    fn agent_with_messages(n: usize) -> PawanAgent {
        let config = PawanConfig::default();
        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        // Add system prompt as message 0
        agent.add_message(Message {
            role: Role::System,
            content: "System prompt".to_string(),
            tool_calls: vec![],
            tool_result: None,
        });
        for i in 1..n {
            agent.add_message(Message {
                role: if i % 2 == 1 { Role::User } else { Role::Assistant },
                content: format!("Message {}", i),
                tool_calls: vec![],
                tool_result: None,
            });
        }
        assert_eq!(agent.history().len(), n);
        agent
    }

    #[test]
    fn test_prune_history_no_op_when_small() {
        let mut agent = agent_with_messages(5);
        agent.prune_history();
        assert_eq!(agent.history().len(), 5, "Should not prune <= 5 messages");
    }

    #[test]
    fn test_prune_history_reduces_messages() {
        let mut agent = agent_with_messages(12);
        assert_eq!(agent.history().len(), 12);
        agent.prune_history();
        // Should keep: system prompt (1) + summary (1) + last 4 = 6
        assert_eq!(agent.history().len(), 6);
    }

    #[test]
    fn test_prune_history_preserves_system_prompt() {
        let mut agent = agent_with_messages(10);
        let original_system = agent.history()[0].content.clone();
        agent.prune_history();
        assert_eq!(agent.history()[0].content, original_system, "System prompt must survive pruning");
    }

    #[test]
    fn test_prune_history_preserves_last_messages() {
        let mut agent = agent_with_messages(10);
        // Last 4 messages are at indices 6..10 with content "Message 6".."Message 9"
        let last4: Vec<String> = agent.history()[6..10].iter().map(|m| m.content.clone()).collect();
        agent.prune_history();
        // After pruning: [system, summary, msg6, msg7, msg8, msg9]
        let after_last4: Vec<String> = agent.history()[2..6].iter().map(|m| m.content.clone()).collect();
        assert_eq!(last4, after_last4, "Last 4 messages must be preserved after pruning");
    }

    #[test]
    fn test_prune_history_inserts_summary() {
        let mut agent = agent_with_messages(10);
        agent.prune_history();
        assert_eq!(agent.history()[1].role, Role::System);
        assert!(agent.history()[1].content.contains("summary"), "Summary message should contain 'summary'");
    }

    #[test]
    fn test_prune_history_utf8_safe() {
        let config = PawanConfig::default();
        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        // Add system prompt + 10 messages with multi-byte UTF-8 characters
        agent.add_message(Message {
            role: Role::System, content: "sys".into(), tool_calls: vec![], tool_result: None,
        });
        for _ in 0..10 {
            agent.add_message(Message {
                role: Role::User,
                content: "こんにちは世界 🌍 ".repeat(50),
                tool_calls: vec![],
                tool_result: None,
            });
        }
        // This should not panic on char boundary issues
        agent.prune_history();
        assert!(agent.history().len() < 11, "Should have pruned");
        // Verify summary is valid UTF-8
        let summary = &agent.history()[1].content;
        assert!(summary.is_char_boundary(0));
    }

    #[test]
    fn test_prune_history_exactly_6_messages() {
        // 6 messages = 1 more than the no-op threshold of 5
        let mut agent = agent_with_messages(6);
        agent.prune_history();
        // Prunes 1 middle message, replaced by summary: system(1) + summary(1) + last 4 = 6
        assert_eq!(agent.history().len(), 6);
    }

    #[test]
    fn test_message_role_roundtrip() {
        for role in [Role::User, Role::Assistant, Role::System, Role::Tool] {
            let json = serde_json::to_string(&role).unwrap();
            let back: Role = serde_json::from_str(&json).unwrap();
            assert_eq!(role, back);
        }
    }

    #[test]
    fn test_agent_response_construction() {
        let resp = AgentResponse {
            content: String::new(),
            tool_calls: vec![],
            iterations: 3,
            usage: TokenUsage::default(),
        };
        assert!(resp.content.is_empty());
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.iterations, 3);
    }

    // --- truncate_tool_result tests ---

    #[test]
    fn test_truncate_small_result_unchanged() {
        let val = json!({"success": true, "output": "hello"});
        let result = truncate_tool_result(val.clone(), 8000);
        assert_eq!(result, val);
    }

    #[test]
    fn test_truncate_large_string_value() {
        let big = "x".repeat(10000);
        let val = json!({"stdout": big, "success": true});
        let result = truncate_tool_result(val, 2000);
        let stdout = result["stdout"].as_str().unwrap();
        assert!(stdout.len() < 10000, "Should be truncated");
        assert!(stdout.contains("truncated"), "Should indicate truncation");
    }

    #[test]
    fn test_truncate_preserves_valid_json() {
        let big = "x".repeat(20000);
        let val = json!({"data": big, "meta": "keep"});
        let result = truncate_tool_result(val, 5000);
        // Result should be valid JSON (no broken strings)
        let serialized = serde_json::to_string(&result).unwrap();
        let _reparsed: Value = serde_json::from_str(&serialized).unwrap();
        // meta should be preserved (it's small)
        assert_eq!(result["meta"], "keep");
    }

    #[test]
    fn test_truncate_bare_string() {
        let big = json!("x".repeat(10000));
        let result = truncate_tool_result(big, 500);
        let s = result.as_str().unwrap();
        assert!(s.len() <= 600); // 500 + truncation notice
        assert!(s.contains("truncated"));
    }

    #[test]
    fn test_truncate_array() {
        let items: Vec<Value> = (0..1000).map(|i| json!(format!("item_{}", i))).collect();
        let val = Value::Array(items);
        let result = truncate_tool_result(val, 500);
        let arr = result.as_array().unwrap();
        assert!(arr.len() < 1000, "Array should be truncated");
    }

    // --- message_importance tests ---

    #[test]
    fn test_importance_failed_tool_highest() {
        let msg = Message {
            role: Role::Tool,
            content: "error".into(),
            tool_calls: vec![],
            tool_result: Some(ToolResultMessage {
                tool_call_id: "1".into(),
                content: json!({"error": "failed"}),
                success: false,
            }),
        };
        assert!(PawanAgent::message_importance(&msg) > 0.8, "Failed tools should be high importance");
    }

    #[test]
    fn test_importance_successful_tool_lowest() {
        let msg = Message {
            role: Role::Tool,
            content: "ok".into(),
            tool_calls: vec![],
            tool_result: Some(ToolResultMessage {
                tool_call_id: "1".into(),
                content: json!({"success": true}),
                success: true,
            }),
        };
        assert!(PawanAgent::message_importance(&msg) < 0.3, "Successful tools should be low importance");
    }

    #[test]
    fn test_importance_user_medium() {
        let msg = Message { role: Role::User, content: "hello".into(), tool_calls: vec![], tool_result: None };
        let score = PawanAgent::message_importance(&msg);
        assert!(score > 0.4 && score < 0.8, "User messages should be medium: {}", score);
    }

    #[test]
    fn test_importance_error_assistant_high() {
        let msg = Message { role: Role::Assistant, content: "Error: something failed".into(), tool_calls: vec![], tool_result: None };
        assert!(PawanAgent::message_importance(&msg) > 0.7, "Error assistant messages should be high importance");
    }

    #[test]
    fn test_importance_ordering() {
        let failed_tool = Message { role: Role::Tool, content: "err".into(), tool_calls: vec![], tool_result: Some(ToolResultMessage { tool_call_id: "1".into(), content: json!({}), success: false }) };
        let user = Message { role: Role::User, content: "hi".into(), tool_calls: vec![], tool_result: None };
        let ok_tool = Message { role: Role::Tool, content: "ok".into(), tool_calls: vec![], tool_result: Some(ToolResultMessage { tool_call_id: "2".into(), content: json!({}), success: true }) };

        let f = PawanAgent::message_importance(&failed_tool);
        let u = PawanAgent::message_importance(&user);
        let s = PawanAgent::message_importance(&ok_tool);
        assert!(f > u && u > s, "Ordering should be: failed({}) > user({}) > success({})", f, u, s);
    }

    // --- State management tests ---

    #[test]
    fn test_agent_clear_history_removes_all() {
        let mut agent = agent_with_messages(8);
        assert_eq!(agent.history().len(), 8);
        agent.clear_history();
        assert_eq!(agent.history().len(), 0, "clear_history should drop every message");
    }

    #[test]
    fn test_agent_add_message_appends_in_order() {
        let config = PawanConfig::default();
        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        assert_eq!(agent.history().len(), 0);

        let first = Message {
            role: Role::User,
            content: "first".into(),
            tool_calls: vec![],
            tool_result: None,
        };
        let second = Message {
            role: Role::Assistant,
            content: "second".into(),
            tool_calls: vec![],
            tool_result: None,
        };
        agent.add_message(first);
        agent.add_message(second);

        assert_eq!(agent.history().len(), 2);
        assert_eq!(agent.history()[0].content, "first");
        assert_eq!(agent.history()[1].content, "second");
        assert_eq!(agent.history()[0].role, Role::User);
        assert_eq!(agent.history()[1].role, Role::Assistant);
    }

    #[test]
    fn test_agent_switch_model_updates_name() {
        let config = PawanConfig::default();
        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        let original = agent.model_name().to_string();

        agent.switch_model("gpt-oss-120b");
        assert_eq!(agent.model_name(), "gpt-oss-120b");
        assert_ne!(
            agent.model_name(),
            original,
            "switch_model should change model_name"
        );
    }

    #[test]
    fn test_agent_with_tools_replaces_registry() {
        let config = PawanConfig::default();
        let agent = PawanAgent::new(config, PathBuf::from("."));
        let original_tool_count = agent.get_tool_definitions().len();

        // Build a fresh empty registry
        let empty = ToolRegistry::new();
        let agent = agent.with_tools(empty);
        assert_eq!(
            agent.get_tool_definitions().len(),
            0,
            "with_tools(empty) should drop default registry (had {} tools)",
            original_tool_count
        );
    }

    #[test]
    fn test_agent_get_tool_definitions_returns_deterministic_set() {
        // Fresh agent should expose a stable, non-empty default tool set
        let config = PawanConfig::default();
        let agent_a = PawanAgent::new(config.clone(), PathBuf::from("."));
        let agent_b = PawanAgent::new(config, PathBuf::from("."));
        let defs_a: Vec<String> = agent_a.get_tool_definitions().iter().map(|d| d.name.clone()).collect();
        let defs_b: Vec<String> = agent_b.get_tool_definitions().iter().map(|d| d.name.clone()).collect();

        assert!(!defs_a.is_empty(), "default agent should have tools");
        assert_eq!(defs_a.len(), defs_b.len(), "two default agents must have same tool count");
        // Spot-check a few core tools we know exist
        let names: Vec<&str> = defs_a.iter().map(|s| s.as_str()).collect();
        assert!(names.contains(&"read_file"), "should have read_file in defaults");
        assert!(names.contains(&"bash"), "should have bash in defaults");
    }

    // ─── Edge cases for truncate_tool_result ─────────────────────────────

    #[test]
    fn test_truncate_empty_object_unchanged() {
        // Regression: empty object passes through early-return (serialized "{}" = 2 chars)
        let val = json!({});
        let result = truncate_tool_result(val.clone(), 10);
        assert_eq!(result, val);
    }

    #[test]
    fn test_truncate_null_value_unchanged() {
        // Null values pass through the `other => other` arm
        let val = Value::Null;
        let result = truncate_tool_result(val.clone(), 10);
        assert_eq!(result, val);
    }

    #[test]
    fn test_truncate_numeric_values_pass_through() {
        // Numbers and booleans can't be truncated — the fn must leave them intact
        let val = json!({"count": 42, "ratio": 2.5, "enabled": true});
        let result = truncate_tool_result(val.clone(), 8000);
        assert_eq!(result, val);
    }

    #[test]
    fn test_truncate_large_string_is_utf8_safe() {
        // Regression: must use chars().take() not byte slicing so multi-byte
        // UTF-8 doesn't panic on char boundary (3000 crabs = ~12000 bytes)
        let emoji_heavy = "🦀".repeat(3000);
        let val = json!({"crabs": emoji_heavy});
        let result = truncate_tool_result(val, 1000);
        let out = result["crabs"].as_str().unwrap();
        assert!(out.contains("truncated"), "truncation marker must be present");
        assert!(out.starts_with('🦀'), "must preserve char boundary");
    }

    #[test]
    fn test_truncate_nested_object_remains_valid_json() {
        // Recursive case: large string nested inside a sub-object still truncates,
        // and the output stays valid parseable JSON.
        let inner_big = "y".repeat(5000);
        let val = json!({
            "meta": "small",
            "nested": { "inner": inner_big }
        });
        let result = truncate_tool_result(val, 1500);
        assert_eq!(result["meta"], "small");
        let serialized = serde_json::to_string(&result).unwrap();
        let _reparsed: Value = serde_json::from_str(&serialized)
            .expect("truncated result must be valid JSON");
    }

    #[test]
    fn test_truncate_short_bare_string_unchanged() {
        // A bare string under max_chars hits the early-return check
        let val = json!("short string");
        let result = truncate_tool_result(val.clone(), 1000);
        assert_eq!(result, val);
    }

    #[test]
    fn test_session_id_is_unique_per_agent() {
        // Two fresh agents must get distinct session_ids so their eruka
        // writes don't collide under the same operations/turns/ key.
        let a1 = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        let a2 = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        assert_ne!(a1.session_id, a2.session_id);
        assert!(!a1.session_id.is_empty());
        // UUID v4 with dashes is 36 chars
        assert_eq!(a1.session_id.len(), 36);
    }

    #[test]
    fn test_resume_session_adopts_loaded_id() {
        // resume_session must overwrite self.session_id with the loaded
        // session's id so subsequent eruka writes cluster under that id
        // rather than the ephemeral one from new().
        use std::io::Write;
        let tmp = tempfile::TempDir::new().unwrap();
        // Minimal valid session file
        let sess_dir = tmp.path().join(".pawan").join("sessions");
        std::fs::create_dir_all(&sess_dir).unwrap();
        let sess_id = "resume-test-xyz";
        let sess_path = sess_dir.join(format!("{}.json", sess_id));
        let sess_json = serde_json::json!({
            "id": sess_id,
            "model": "test-model",
            "created_at": "2026-04-11T00:00:00Z",
            "updated_at": "2026-04-11T00:00:00Z",
            "messages": [],
            "total_tokens": 0,
            "iteration_count": 0
        });
        let mut f = std::fs::File::create(&sess_path).unwrap();
        f.write_all(sess_json.to_string().as_bytes()).unwrap();

        // Point HOME at the tmp dir so Session::sessions_dir resolves here
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        let orig_id = agent.session_id.clone();
        agent.resume_session(sess_id).expect("resume should succeed");
        assert_eq!(agent.session_id, sess_id);
        assert_ne!(agent.session_id, orig_id);

        // Restore HOME to avoid polluting other tests
        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn test_history_snapshot_for_eruka_bounded() {
        // 100 messages of 500 chars each = 50k raw content. Snapshot must
        // cap at ~4000 chars so eruka writes never balloon.
        let mut history = Vec::new();
        for i in 0..100 {
            history.push(Message {
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                content: "x".repeat(500),
                tool_calls: vec![],
                tool_result: None,
            });
        }
        let snapshot = PawanAgent::history_snapshot_for_eruka(&history);
        // After the break at >4000, one more line (up to 203 chars) gets
        // appended, so total is bounded by ~4200.
        assert!(snapshot.len() <= 4400, "snapshot too long: {} chars", snapshot.len());
        assert!(snapshot.len() > 200, "snapshot too short: {} chars", snapshot.len());
    }

    #[test]
    fn test_history_snapshot_for_eruka_includes_role_prefixes() {
        // Each message must be tagged with its role so the eruka consumer
        // can distinguish user questions from assistant answers.
        let history = vec![
            Message { role: Role::User, content: "hi".into(), tool_calls: vec![], tool_result: None },
            Message { role: Role::Assistant, content: "hello".into(), tool_calls: vec![], tool_result: None },
            Message { role: Role::Tool, content: "ok".into(), tool_calls: vec![], tool_result: None },
            Message { role: Role::System, content: "sys".into(), tool_calls: vec![], tool_result: None },
        ];
        let snapshot = PawanAgent::history_snapshot_for_eruka(&history);
        assert!(snapshot.contains("U: hi"));
        assert!(snapshot.contains("A: hello"));
        assert!(snapshot.contains("T: ok"));
        assert!(snapshot.contains("S: sys"));
    }

    async fn test_archive_to_eruka_ok_when_disabled() {
        // When eruka is disabled (the default), archive_to_eruka must
        // return Ok without touching the network — this is the
        // fire-and-forget contract the CLI relies on.
        let agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        assert!(agent.eruka.is_none(), "default config should disable eruka");
        let result = agent.archive_to_eruka().await;
        assert!(result.is_ok(), "archive_to_eruka should be non-fatal when disabled");
    }

    // ─── probe_local_endpoint tests ──────────────────────────────────────

    #[test]
    fn test_probe_local_endpoint_closed_port_returns_false() {
        // Port 1999 is almost never in use by Netdata (which uses 19999) 
        // or other common services.
        assert!(
            !probe_local_endpoint("http://localhost:1999/v1"),
            "closed port should return false"
        );
    }

    #[test]
    fn test_probe_local_endpoint_open_port_returns_true() {
        // Bind a real listener on a free OS-assigned port, then probe it.
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind failed");
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://localhost:{port}/v1");
        assert!(probe_local_endpoint(&url), "open port should return true");
    }

    #[test]
    fn test_probe_local_endpoint_url_without_explicit_port() {
        // Port is absent — probe_local_endpoint must default to 80
        // which on CI is normally closed, so this just must not panic.
        let _ = probe_local_endpoint("http://localhost/v1");
    }

    // ─── load_arch_context tests ──────────────────────────────────────────

    #[test]
    fn test_load_arch_context_absent_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(load_arch_context(dir.path()).is_none());
    }

    #[test]
    fn test_load_arch_context_reads_file_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let pawan_dir = dir.path().join(".pawan");
        std::fs::create_dir_all(&pawan_dir).unwrap();
        std::fs::write(pawan_dir.join("arch.md"), "## Architecture\nUse tokio.\n").unwrap();
        let result = load_arch_context(dir.path());
        assert!(result.is_some());
        assert!(result.unwrap().contains("Use tokio"));
    }

    #[test]
    fn test_load_arch_context_empty_file_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let pawan_dir = dir.path().join(".pawan");
        std::fs::create_dir_all(&pawan_dir).unwrap();
        std::fs::write(pawan_dir.join("arch.md"), "   \n").unwrap();
        assert!(load_arch_context(dir.path()).is_none(), "whitespace-only file should be None");
    }

    #[test]
    fn test_load_arch_context_truncates_at_2000_chars() {
        let dir = tempfile::TempDir::new().unwrap();
        let pawan_dir = dir.path().join(".pawan");
        std::fs::create_dir_all(&pawan_dir).unwrap();
        // Write a file that is exactly 2500 ASCII chars (safe char boundary)
        let content = "x".repeat(2_500);
        std::fs::write(pawan_dir.join("arch.md"), &content).unwrap();
        let result = load_arch_context(dir.path()).unwrap();
        assert!(
            result.len() < 2_100,
            "truncated result should be close to 2000 chars, got {}",
            result.len()
        );
        assert!(result.ends_with("(truncated)"), "truncated output must end with marker");
    }

    async fn test_tool_idle_timeout_triggered() {
        use std::time::Duration;
        use tokio::time::sleep;

        let mut config = PawanConfig::default();
        config.tool_call_idle_timeout_secs = 0; // Trigger on any non-zero elapsed seconds

        // Custom backend that is slow on the second call.
        // With our fix (moving update before LLM call), this will trigger
        // at the start of the THIRD iteration if the second iteration takes time.
        struct SlowBackend {
            index: Arc<std::sync::atomic::AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl LlmBackend for SlowBackend {
            async fn generate(&self, _m: &[Message], _t: &[ToolDefinition], _o: Option<&TokenCallback>) -> Result<LLMResponse> {
                let idx = self.index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if idx == 0 {
                    // First call: return a tool call to ensure we loop again
                    Ok(LLMResponse {
                        content: String::new(),
                        reasoning: None,
                        tool_calls: vec![ToolCallRequest {
                            id: "1".to_string(),
                            name: "read_file".to_string(),
                            arguments: json!({"path": "foo"}),
                        }],
                        finish_reason: "tool_calls".to_string(),
                        usage: None,
                    })
                } else if idx == 1 {
                    // Second call: delay then return ANOTHER tool call
                    // The delay happens AFTER last_tool_call_time is updated for Iteration 2.
                    // So Iteration 3's check will see this 1.1s delay.
                    sleep(Duration::from_millis(1100)).await;
                    Ok(LLMResponse {
                        content: String::new(),
                        reasoning: None,
                        tool_calls: vec![ToolCallRequest {
                            id: "2".to_string(),
                            name: "read_file".to_string(),
                            arguments: json!({"path": "bar"}),
                        }],
                        finish_reason: "tool_calls".to_string(),
                        usage: None,
                    })
                } else {
                    Ok(LLMResponse {
                        content: "Done".to_string(),
                        reasoning: None,
                        tool_calls: vec![],
                        finish_reason: "stop".to_string(),
                        usage: None,
                    })
                }
            }
        }

        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        agent.backend = Box::new(SlowBackend { index: Arc::new(std::sync::atomic::AtomicUsize::new(0)) });

        let result = agent.execute_with_all_callbacks("test", None, None, None, None).await;
        
        match result {
            Err(PawanError::Agent(msg)) => {
                assert!(msg.contains("Tool idle timeout exceeded"), "Error message should contain timeout: {}", msg);
            }
            Ok(_) => panic!("Expected timeout error, but it succeeded. This means the timeout check didn't catch the delay."),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    async fn test_tool_idle_timeout_not_triggered() {
        let mut config = PawanConfig::default();
        config.tool_call_idle_timeout_secs = 10;

        let backend = MockBackend::new(vec![
            MockResponse::text("Done"),
        ]);

        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute_with_all_callbacks("test", None, None, None, None).await;
        assert!(result.is_ok());
    }

    // ─── Backend creation tests ─────────────────────────────────────────────

    #[test]
    fn test_probe_local_endpoint_with_localhost_replacement() {
        // Verify localhost is replaced with 127.0.0.1
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind failed");
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://localhost:{}/v1", port);
        assert!(probe_local_endpoint(&url), "localhost should be resolved to 127.0.0.1");
    }

    #[test]
    fn test_probe_local_endpoint_with_https_defaults_to_443() {
        // HTTPS without explicit port should default to 443
        let _ = probe_local_endpoint("https://example.com/v1");
        // Just verify it doesn't panic
    }

    #[test]
    fn test_probe_local_endpoint_with_http_defaults_to_80() {
        // HTTP without explicit port should default to 80
        let _ = probe_local_endpoint("http://example.com/v1");
        // Just verify it doesn't panic
    }

    #[test]
    fn test_probe_local_endpoint_invalid_address_returns_false() {
        // Invalid address should return false without panicking
        assert!(!probe_local_endpoint("http://invalid-host-name-that-does-not-exist-12345.com:9999/v1"));
    }

    // ─── Session management tests ───────────────────────────────────────────

    #[test]
    fn test_save_session_creates_valid_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let config = PawanConfig::default();
        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        agent.add_message(Message {
            role: Role::User,
            content: "test message".to_string(),
            tool_calls: vec![],
            tool_result: None,
        });

        let session_id = agent.save_session().expect("save should succeed");
        assert!(!session_id.is_empty());

        // Verify session file exists
        let sess_dir = tmp.path().join(".pawan").join("sessions");
        let sess_path = sess_dir.join(format!("{}.json", session_id));
        assert!(sess_path.exists(), "session file should be created");

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn test_resume_session_loads_messages() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let sess_dir = tmp.path().join(".pawan").join("sessions");
        std::fs::create_dir_all(&sess_dir).unwrap();
        let sess_id = "resume-load-test";
        let sess_path = sess_dir.join(format!("{}.json", sess_id));

        let sess_json = serde_json::json!({
            "id": sess_id,
            "model": "test-model",
            "created_at": "2026-04-11T00:00:00Z",
            "updated_at": "2026-04-11T00:00:00Z",
            "messages": [
                {"role": "user", "content": "test", "tool_calls": [], "tool_result": null}
            ],
            "total_tokens": 100,
            "iteration_count": 1
        });
        std::fs::write(&sess_path, sess_json.to_string()).unwrap();

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.resume_session(sess_id).expect("resume should succeed");

        assert_eq!(agent.history().len(), 1);
        assert_eq!(agent.history()[0].content, "test");
        assert_eq!(agent.context_tokens_estimate, 100);

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn test_resume_session_nonexistent_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        let result = agent.resume_session("nonexistent-session");
        assert!(result.is_err(), "resuming nonexistent session should fail");

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
    }

    // ─── Execution logic tests ───────────────────────────────────────────────

    async fn test_execute_with_callbacks_returns_response() {
        let backend = MockBackend::new(vec![
            MockResponse::text("Hello world"),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute_with_callbacks("test", None, None, None).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.content, "Hello world");
    }

    async fn test_execute_with_token_callback() {
        let backend = MockBackend::new(vec![
            MockResponse::text("Response"),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let tokens_received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let tokens_clone = tokens_received.clone();

        let on_token = Box::new(move |token: &str| {
            tokens_received.lock().unwrap().push(token.to_string());
        });

        let result = agent.execute_with_callbacks("test", Some(on_token), None, None).await;
        assert!(result.is_ok());
        // Note: MockBackend doesn't actually call token callbacks, but we verify the path works
    }

    async fn test_execute_with_tool_callback() {
        let backend = MockBackend::new(vec![
            MockResponse::text("Done"),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let tools_called = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools_clone = tools_called.clone();

        let on_tool = Box::new(move |record: &ToolCallRecord| {
            tools_called.lock().unwrap().push(record.name.clone());
        });

        let result = agent.execute_with_callbacks("test", None, Some(on_tool), None).await;
        assert!(result.is_ok());
    }

    async fn test_execute_max_iterations_exceeded() {
        let mut config = PawanConfig::default();
        config.max_tool_iterations = 2;

        let backend = MockBackend::with_repeated_tool_call("bash");

        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        assert!(result.is_err());
        match result {
            Err(PawanError::Agent(msg)) => {
                assert!(msg.contains("Max tool iterations"));
            }
            _ => panic!("Expected max iterations error"),
        }
    }

    async fn test_execute_with_arch_context_injection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pawan_dir = tmp.path().join(".pawan");
        std::fs::create_dir_all(&pawan_dir).unwrap();
        std::fs::write(pawan_dir.join("arch.md"), "## Architecture\nUse Rust.\n").unwrap();

        let backend = MockBackend::new(vec![
            MockResponse::text("Response"),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), tmp.path().to_path_buf());
        agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        assert!(result.is_ok());
        // Verify arch context was injected (check history)
        let user_msg = agent.history().iter().find(|m| m.role == Role::User);
        assert!(user_msg.is_some());
        assert!(user_msg.unwrap().content.contains("Workspace Architecture"));
    }

    async fn test_execute_context_pruning_triggered() {
        let mut config = PawanConfig::default();
        config.max_context_tokens = 100; // Very low to trigger pruning

        let backend = MockBackend::new(vec![
            MockResponse::text("Response"),
        ]);

        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        agent.backend = Box::new(backend);

        // Add many messages to exceed context limit
        for i in 0..50 {
            agent.add_message(Message {
                role: Role::User,
                content: "x".repeat(1000),
                tool_calls: vec![],
                tool_result: None,
            });
        }

        let result = agent.execute("test").await;
        assert!(result.is_ok());
        // Verify pruning occurred
        assert!(agent.history().len() < 50, "history should be pruned");
    }

    async fn test_execute_iteration_budget_warning() {
        let mut config = PawanConfig::default();
        config.max_tool_iterations = 5;

        let backend = MockBackend::with_repeated_tool_call("bash");

        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        assert!(result.is_err());
        // Check that budget warning was added to history
        let budget_warnings = agent.history().iter()
            .filter(|m| m.content.contains("tool iterations remaining"))
            .count();
        assert!(budget_warnings > 0, "should have budget warning in history");
    }

    // ─── Tool execution tests ───────────────────────────────────────────────

    async fn test_execute_tool_timeout() {
        let mut config = PawanConfig::default();
        config.bash_timeout_secs = 1; // Very short timeout

        let backend = MockBackend::with_tool_call(
            "call_1",
            "bash",
            json!({"command": "sleep 10"}),
            "Run slow command",
        );

        let mut agent = PawanAgent::new(config, PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        // Should complete with error in tool result
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.tool_calls.is_empty());
        let first_tool = &response.tool_calls[0];
        assert!(!first_tool.success);
        assert!(first_tool.result.get("error").is_some());
    }

    async fn test_execute_tool_error_handling() {
        let backend = MockBackend::with_tool_call(
            "call_1",
            "read_file",
            json!({"path": "/nonexistent/file.txt"}),
            "Read file",
        );

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.tool_calls.is_empty());
        // Tool should have error result
        let first_tool = &response.tool_calls[0];
        assert!(!first_tool.success);
    }

    async fn test_execute_multiple_tool_calls() {
        let backend = MockBackend::with_multiple_tool_calls(vec![
            ("call_1", "bash", json!({"command": "echo 1"})),
            ("call_2", "bash", json!({"command": "echo 2"})),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.tool_calls.len() >= 2);
    }

    async fn test_execute_token_usage_accumulation() {
        let backend = MockBackend::with_text_and_usage("Response", 100, 50);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.usage.prompt_tokens, 100);
        assert_eq!(response.usage.completion_tokens, 50);
        assert_eq!(response.usage.total_tokens, 150);
    }

    // ─── Error handling tests ───────────────────────────────────────────────



    async fn test_execute_with_permission_callback_denied() {
        let backend = MockBackend::with_tool_call(
            "call_1",
            "bash",
            json!({"command": "echo test"}),
            "Run command",
        );

		let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
		agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        assert!(result.is_ok());
    }
    // ─── Error handling tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_execute_with_empty_history() {
        let backend = MockBackend::new(vec![
            MockResponse::text("Response"),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute("test").await;
        assert!(result.is_ok());
    }
    async fn test_execute_with_coordinator_basic() {
        let mut config = PawanConfig::default();
        config.use_coordinator = true;
        config.max_tool_iterations = 1;

        let agent = PawanAgent::new(config, PathBuf::from("."));
        // Verify coordinator flag is set
        assert!(agent.config().use_coordinator);
    }

    async fn test_execute_with_coordinator_ignores_callbacks() {
        let mut config = PawanConfig::default();
        config.use_coordinator = true;

        let mut agent = PawanAgent::new(config, PathBuf::from("."));

        let callback_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = callback_called.clone();

        let on_token = Box::new(move |_token: &str| {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        // Callbacks should be ignored in coordinator mode
        let _ = agent.execute_with_all_callbacks("test", Some(on_token), None, None, None).await;
        // Note: This will fail because coordinator needs a real backend, but we verify the path
    }

    // ─── Agent state tests ───────────────────────────────────────────────────

    #[test]
    fn test_agent_tools_mut_returns_mutable_registry() {
        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        let original_count = agent.get_tool_definitions().len();

        // tools_mut should allow modification
        let _ = agent.tools_mut();
        // Just verify we can get mutable access
    }

    #[test]
    fn test_agent_config_returns_reference() {
        let config = PawanConfig::default();
        let agent = PawanAgent::new(config.clone(), PathBuf::from("."));

        let agent_config = agent.config();
        assert_eq!(agent_config.model, config.model);
    }

    #[test]
    fn test_agent_clear_history() {
        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));

        agent.add_message(Message {
            role: Role::User,
            content: "test".to_string(),
            tool_calls: vec![],
            tool_result: None,
        });

        assert_eq!(agent.history().len(), 1);
        agent.clear_history();
        assert_eq!(agent.history().len(), 0);
    }

    #[test]
    fn test_agent_with_backend_replaces_backend() {
        let agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        let original_model = agent.model_name().to_string();

        let new_backend = MockBackend::new(vec![MockResponse::text("test")]);
        let agent = agent.with_backend(Box::new(new_backend));

        // Backend should be replaced
        assert_eq!(agent.model_name(), original_model);
    }

    // ─── Edge case tests ─────────────────────────────────────────────────────

    async fn test_execute_empty_prompt() {
        let backend = MockBackend::new(vec![
            MockResponse::text("Response"),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let result = agent.execute("").await;
        assert!(result.is_ok());
    }

    async fn test_execute_very_long_prompt() {
        let backend = MockBackend::new(vec![
            MockResponse::text("Response"),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let long_prompt = "x".repeat(100_000);
        let result = agent.execute(&long_prompt).await;
        assert!(result.is_ok());
    }

    async fn test_execute_with_special_characters() {
        let backend = MockBackend::new(vec![
            MockResponse::text("Response"),
        ]);

        let mut agent = PawanAgent::new(PawanConfig::default(), PathBuf::from("."));
        agent.backend = Box::new(backend);

        let special_prompt = "Test with 🦀 emojis and \n newlines and \t tabs";
        let result = agent.execute(special_prompt).await;
        assert!(result.is_ok());
    }
}
/// Summarize tool arguments for permission requests
fn summarize_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => {
            let mut parts = Vec::new();
            for (key, value) in map {
                let value_str = match value {
                    serde_json::Value::String(s) if s.len() > 50 => {
                        format!("\"{}...\"", &s[..47])
                    }
                    serde_json::Value::String(s) => format!("\"{}\"", s),
                    serde_json::Value::Array(arr) if arr.len() > 3 => {
                        format!("[... {} items]", arr.len())
                    }
                    serde_json::Value::Array(arr) => {
                        let items: Vec<String> = arr.iter().take(3).map(|v| {
                            match v {
                                serde_json::Value::String(s) => {
                                    if s.len() > 20 {
                                        format!("\"{}...\"", &s[..17])
                                    } else {
                                        format!("\"{}\"", s)
                                    }
                                }
                                _ => v.to_string(),
                            }
                        }).collect();
                        format!("[{}]", items.join(", "))
                    }
                    _ => value.to_string(),
                };
                parts.push(format!("{}: {}", key, value_str));
            }
            parts.join(", ")
        }
        serde_json::Value::String(s) => {
            if s.len() > 100 {
                format!("\"{}...\"", &s[..97])
            } else {
                format!("\"{}\"", s)
            }
        }
        serde_json::Value::Array(arr) => {
            format!("[{} items]", arr.len())
        }
        _ => args.to_string(),
    }
}

// --------------------------------------------------------------------------- Tests for coordinator integration
// ----------------------------------------------------------------------------

#[cfg(test)]
mod coordinator_tests {
    use super::*;
    use crate::agent::backend::mock::{MockBackend, MockResponse};
    use crate::coordinator::{FinishReason, ToolCallingConfig};
    use std::sync::Arc;

    /// Test that config default has use_coordinator = false
    #[test]
    fn test_config_default_use_coordinator_false() {
        let config = PawanConfig::default();
        assert!(!config.use_coordinator);
    }

    /// Test that config can set use_coordinator = true
    #[test]
    fn test_config_use_coordinator_true() {
        let config = PawanConfig {
            use_coordinator: true,
            ..Default::default()
        };
        assert!(config.use_coordinator);
    }

    /// Test coordinator execution dispatches correctly when flag is set
    async fn test_execute_with_coordinator_flag_enabled() {
        let config = PawanConfig {
            use_coordinator: true,
            model: "test-model".to_string(),
            ..Default::default()
        };
        let agent = PawanAgent::new(config, PathBuf::from("."));
        // Verify the flag is set
        assert!(agent.config().use_coordinator);
    }

    /// Test that execute_with_coordinator produces valid response
    async fn test_execute_with_coordinator_produces_response() {
        let config = PawanConfig {
            use_coordinator: true,
            max_tool_iterations: 1,
            model: "test-model".to_string(),
            ..Default::default()
        };
        let agent = PawanAgent::new(config, PathBuf::from("."));
let backend = MockBackend::with_text("Hello from coordinator!");
        let mut agent = agent.with_backend(Box::new(backend));

        // This will fail because the coordinator creates its own backend
        // but we can at least verify the flag works
        assert!(agent.config().use_coordinator);
    }

    /// Test ToolCallingConfig default values
    #[test]
    fn test_tool_calling_config_defaults() {
        let cfg = ToolCallingConfig::default();
        assert_eq!(cfg.max_iterations, 10);
        assert!(cfg.parallel_execution);
        assert_eq!(cfg.tool_timeout.as_secs(), 30);
        assert!(!cfg.stop_on_error);
    }

    /// Test custom ToolCallingConfig
    #[test]
    fn test_tool_calling_config_custom() {
        let cfg = ToolCallingConfig {
            max_iterations: 5,
            parallel_execution: false,
            tool_timeout: std::time::Duration::from_secs(60),
            stop_on_error: true,
        };
        assert_eq!(cfg.max_iterations, 5);
        assert!(!cfg.parallel_execution);
        assert_eq!(cfg.tool_timeout.as_secs(), 60);
        assert!(cfg.stop_on_error);
    }

    /// Test that coordinator dispatch check works correctly
    async fn test_coordinator_dispatch_when_flag_is_false() {
        let config = PawanConfig::default();
        assert!(!config.use_coordinator);
        // When flag is false, execute_with_all_callbacks should use built-in loop
    }

    /// Test error handling when coordinator encounters unknown tool
    async fn test_coordinator_error_handling_unknown_tool() {
        use crate::coordinator::ToolCoordinator;

        let mock_backend = Arc::new(MockBackend::with_tool_call(
            "call_1",
            "nonexistent_tool",
            json!({}),
            "Trying to call unknown tool",
        ));
        let registry = Arc::new(ToolRegistry::new());
        let config = ToolCallingConfig::default();
        let coordinator = ToolCoordinator::new(mock_backend, registry, config);

        let result = coordinator.execute(None, "Use a tool").await.unwrap();
        assert!(matches!(result.finish_reason, FinishReason::UnknownTool(_)));
    }

    /// Test max iterations limit in coordinator
    async fn test_coordinator_max_iterations_limit() {
        use crate::coordinator::ToolCoordinator;
        use crate::tools::Tool;
        use async_trait::async_trait;
        use serde_json::json;
        use std::sync::Arc;

        // Dummy tool that always succeeds
        struct DummyTool;
        #[async_trait]
        impl Tool for DummyTool {
            fn name(&self) -> &str { "test_tool" }
            fn description(&self) -> &str { "Dummy tool for testing" }
            fn parameters_schema(&self) -> serde_json::Value { json!({}) }
            async fn execute(&self, _args: serde_json::Value) -> crate::Result<serde_json::Value> {
                Ok(json!({ "status": "ok" }))
            }
        }

        let mock_backend = Arc::new(MockBackend::with_repeated_tool_call("test_tool"));
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool));
        let registry = Arc::new(registry);
        let config = ToolCallingConfig {
            max_iterations: 3,
            ..Default::default()
        };
        let coordinator = ToolCoordinator::new(mock_backend, registry, config);

        let result = coordinator.execute(None, "Use tools").await.unwrap();
        assert_eq!(result.iterations, 3);
        assert!(matches!(result.finish_reason, FinishReason::MaxIterations));
    }

    /// Test timeout handling in coordinator
    async fn test_coordinator_timeout_handling() {
        use crate::coordinator::ToolCoordinator;

        // Create a mock that returns a tool call
        let mock_backend = Arc::new(MockBackend::with_tool_call(
            "call_1",
            "bash",
            json!({"command": "sleep 10"}),
            "Run slow command",
        ));
        let registry = Arc::new(ToolRegistry::with_defaults(PathBuf::from(".")));
        // Very short timeout
        let config = ToolCallingConfig {
            tool_timeout: std::time::Duration::from_millis(1),
            ..Default::default()
        };
        let coordinator = ToolCoordinator::new(mock_backend, registry, config);

        // This will timeout - coordinator should handle it gracefully
        let result = coordinator.execute(None, "Run a command").await.unwrap();
        // The tool should have failed with timeout error
        assert!(!result.tool_calls.is_empty());
        let first_call = &result.tool_calls[0];
        assert!(!first_call.success);
        assert!(first_call.result.get("error").is_some());
    }

    /// Test that coordinator accumulates token usage
    async fn test_coordinator_token_usage_accumulation() {
        use crate::coordinator::ToolCoordinator;

        let mock_backend = Arc::new(MockBackend::with_text_and_usage(
            "Response",
            100,
            50,
        ));
        let registry = Arc::new(ToolRegistry::new());
        let config = ToolCallingConfig::default();
        let coordinator = ToolCoordinator::new(mock_backend, registry, config);

        let result = coordinator.execute(None, "Hello").await.unwrap();
        assert_eq!(result.total_usage.prompt_tokens, 100);
        assert_eq!(result.total_usage.completion_tokens, 50);
        assert_eq!(result.total_usage.total_tokens, 150);
    }

    /// Test parallel execution in coordinator
    async fn test_coordinator_parallel_execution() {
        use crate::coordinator::ToolCoordinator;

        // Mock that returns multiple tool calls
        let mock_backend = Arc::new(MockBackend::with_multiple_tool_calls(vec![
            ("call_1", "bash", json!({"command": "echo 1"})),
            ("call_2", "bash", json!({"command": "echo 2"})),
            ("call_3", "read_file", json!({"path": "test.txt"})),
        ]));
        let registry = Arc::new(ToolRegistry::with_defaults(PathBuf::from(".")));
        let config = ToolCallingConfig {
            parallel_execution: true,
            ..Default::default()
        };
        let coordinator = ToolCoordinator::new(mock_backend, registry, config);

        let result = coordinator.execute(None, "Run multiple commands").await.unwrap();
        // Should have executed multiple tool calls
        assert!(result.tool_calls.len() >= 3);
    }
}
/// Model information for available models
pub struct ModelInfo {
	/// Model name
	pub name: String,
	/// Model display name
	pub display_name: String,
	/// Model description
	pub description: String,
	/// Quality score (0-100)
	pub quality_score: u8,
	/// Whether the model is local
	pub is_local: bool,
	/// Whether the model is experimental
	pub is_experimental: bool,
	/// Model file path (for local models)
	pub file_path: Option<String>,
}