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
                        // For bash: auto-allow read-only commands even under Prompt
                        if tool_call.name == "bash" {
                            if let Some(cmd) = tool_call.arguments.get("command").and_then(|v| v.as_str()) {
                                if crate::tools::bash::is_read_only(cmd) {
                                    tracing::debug!(command = cmd, "Auto-allowing read-only bash command under Prompt permission");
                                    None
                                } else {
                                    Some("Bash command requires user approval (read-only commands auto-allowed)")
                                }
                            } else {
                                Some("Tool requires user approval")
                            }
                        } else {
                            // Non-bash tools: headless = deny for safety
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
}
