//! Pawan Agent - The core agent that handles tool-calling loops
//!
//! This module provides the main `PawanAgent` which:
//! - Manages conversation history
//! - Coordinates tool calling with the LLM via pluggable backends
//! - Provides streaming responses
//! - Supports multiple LLM backends (NVIDIA API, Ollama, OpenAI)

pub mod backend;
mod preflight;
pub mod session;
pub mod git_session;

use crate::config::{LlmProvider, PawanConfig};
use crate::tools::{ToolDefinition, ToolRegistry};
use crate::{PawanError, Result};
use backend::openai_compat::{OpenAiCompatBackend, OpenAiCompatConfig};
use backend::LlmBackend;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    /// Unique ID for this tool call
    pub id: String,
    /// Name of the tool to call
    pub name: String,
    /// Arguments for the tool
    pub arguments: Value,
}

/// Result from a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
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

        Self {
            config,
            tools,
            history: Vec::new(),
            workspace_root,
            backend,
            context_tokens_estimate: 0,
            eruka,
        }
    }

    /// Create the appropriate backend based on config
    fn create_backend(config: &PawanConfig, system_prompt: &str) -> Box<dyn LlmBackend> {
        match config.provider {
            LlmProvider::Nvidia | LlmProvider::OpenAI | LlmProvider::Mlx => {
                let (api_url, api_key) = match config.provider {
                    LlmProvider::Nvidia => {
                        let url = std::env::var("NVIDIA_API_URL")
                            .unwrap_or_else(|_| crate::DEFAULT_NVIDIA_API_URL.to_string());
                        let key = std::env::var("NVIDIA_API_KEY").ok();
                        if key.is_none() {
                            tracing::warn!("NVIDIA_API_KEY not set. Add it to .env or export it.");
                        }
                        (url, key)
                    },
                    LlmProvider::OpenAI => {
                        let url = config.base_url.clone()
                            .or_else(|| std::env::var("OPENAI_API_URL").ok())
                            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                        let key = std::env::var("OPENAI_API_KEY").ok();
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
                            let key = std::env::var("NVIDIA_API_KEY").ok();
                            (url, key)
                        },
                        LlmProvider::OpenAI => {
                            let url = std::env::var("OPENAI_API_URL")
                                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
                            let key = std::env::var("OPENAI_API_KEY").ok();
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
        Ok(())
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
    /// Keeps the first message (system prompt) and last 4 messages,
    /// replaces everything in between with a summary message.
    fn prune_history(&mut self) {
        let len = self.history.len();
        if len <= 5 {
            return; // Nothing to prune
        }

        let keep_end = 4;
        let start = 1; // Skip system prompt at index 0
        let end = len - keep_end;
        let pruned_count = end - start;

        // Build summary from middle messages (UTF-8 safe truncation)
        let mut summary = String::with_capacity(2048);
        for msg in &self.history[start..end] {
            let chunk: String = msg.content.chars().take(200).collect();
            summary.push_str(&chunk);
            summary.push('\n');
            if summary.len() > 2000 {
                // Truncate at char boundary
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
            content: format!("Previous conversation summary (pruned): {}", summary),
            tool_calls: vec![],
            tool_result: None,
        };

        // Replace middle messages in-place with drain (avoids clone + clear + extend)
        self.history.drain(start..end);
        self.history.insert(start, summary_msg);

        tracing::info!(pruned = pruned_count, context_estimate = self.context_tokens_estimate, "Pruned messages from history");
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
        // Inject Eruka core memory before first LLM call
        if let Some(eruka) = &self.eruka {
            if let Err(e) = eruka.inject_core_memory(&mut self.history).await {
                tracing::warn!("Eruka memory injection failed (non-fatal): {}", e);
            }
        }

        self.history.push(Message {
            role: Role::User,
            content: user_prompt.to_string(),
            tool_calls: vec![],
            tool_result: None,
        });

        let mut all_tool_calls = Vec::new();
        let mut total_usage = TokenUsage::default();
        let mut iterations = 0;
        let max_iterations = self.config.max_tool_iterations;

        loop {
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
                        // In headless mode (no TUI), Prompt = deny for safety.
                        // TUI mode overrides this via the callback.
                        Some("Tool requires user approval (set permission to 'allow' or use TUI mode)")
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

                let start = std::time::Instant::now();

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
                let result_value = {
                    let result_str = serde_json::to_string(&result_value).unwrap_or_default();
                    if result_str.len() > max_result_chars {
                        // UTF-8 safe truncation
                        let truncated: String = result_str.chars().take(max_result_chars).collect();
                        let truncated = truncated.as_str();
                        serde_json::from_str(truncated).unwrap_or_else(|_| {
                            json!({"content": format!("{}...[truncated from {} chars]", truncated, result_str.len())})
                        })
                    } else {
                        result_value
                    }
                };


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
    /// Execute healing with retries — calls heal(), checks for remaining errors, retries if needed
    pub async fn heal_with_retries(&mut self, max_attempts: usize) -> Result<AgentResponse> {
        let mut last_response = self.heal().await?;

        for attempt in 1..max_attempts {
            let fixer = crate::healing::CompilerFixer::new(self.workspace_root.clone());
            let remaining = fixer.check().await?;
            let errors: Vec<_> = remaining.iter().filter(|d| d.kind == crate::healing::DiagnosticKind::Error).collect();

            if errors.is_empty() {
                tracing::info!(attempts = attempt, "Healing complete");
                return Ok(last_response);
            }

            tracing::warn!(errors = errors.len(), attempt = attempt, "Errors remain after heal attempt, retrying");
            last_response = self.heal().await?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
