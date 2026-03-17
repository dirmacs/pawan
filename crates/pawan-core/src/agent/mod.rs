//! Pawan Agent - The core agent that handles tool-calling loops
//!
//! This module provides the main `PawanAgent` which:
//! - Manages conversation history
//! - Coordinates tool calling with the LLM via pluggable backends
//! - Provides streaming responses
//! - Supports multiple LLM backends (NVIDIA API, Ollama, OpenAI)

pub mod backend;
pub mod session;

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
}

/// LLM response from a generation request
#[derive(Debug, Clone)]
pub struct LLMResponse {
    /// Text content of the response
    pub content: String,
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

/// The main Pawan agent
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
}

impl PawanAgent {
    /// Create a new PawanAgent with auto-selected backend
    pub fn new(config: PawanConfig, workspace_root: PathBuf) -> Self {
        let tools = ToolRegistry::with_defaults(workspace_root.clone());
        let system_prompt = config.get_system_prompt();
        let backend = Self::create_backend(&config, &system_prompt);
        Self {
            config,
            tools,
            history: Vec::new(),
            workspace_root,
            backend,
            context_tokens_estimate: 0,
        }
    }

    /// Create the appropriate backend based on config
    fn create_backend(config: &PawanConfig, system_prompt: &str) -> Box<dyn LlmBackend> {
        match config.provider {
            LlmProvider::Nvidia | LlmProvider::OpenAI => {
                let (api_url, api_key) = match config.provider {
                    LlmProvider::Nvidia => {
                        let url = std::env::var("NVIDIA_API_URL")
                            .unwrap_or_else(|_| crate::DEFAULT_NVIDIA_API_URL.to_string());
                        let key = std::env::var("NVIDIA_API_KEY").ok();
                        if key.is_none() {
                            eprintln!(
                                "Warning: NVIDIA_API_KEY not set. Add it to .env or export it."
                            );
                        }
                        (url, key)
                    }
                    LlmProvider::OpenAI => {
                        let url = std::env::var("OPENAI_API_URL")
                            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
                        let key = std::env::var("OPENAI_API_KEY").ok();
                        if key.is_none() {
                            eprintln!(
                                "Warning: OPENAI_API_KEY not set. Add it to .env or export it."
                            );
                        }
                        (url, key)
                    }
                    _ => unreachable!(),
                };

                Box::new(OpenAiCompatBackend::new(OpenAiCompatConfig {
                    api_url,
                    api_key,
                    model: config.model.clone(),
                    temperature: config.temperature,
                    top_p: config.top_p,
                    max_tokens: config.max_tokens,
                    system_prompt: system_prompt.to_string(),
                    use_thinking: config.use_thinking_mode(),
                    max_retries: config.max_retries,
                    fallback_models: config.fallback_models.clone(),
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

        // Build summary from middle messages
        let mut summary = String::new();
        for msg in &self.history[start..end] {
            let chunk = if msg.content.len() > 200 {
                &msg.content[..200]
            } else {
                &msg.content
            };
            summary.push_str(chunk);
            summary.push('\n');
            if summary.len() > 2000 {
                summary.truncate(2000);
                break;
            }
        }

        let summary_msg = Message {
            role: Role::System,
            content: format!("Previous conversation summary (pruned): {}", summary),
            tool_calls: vec![],
            tool_result: None,
        };

        // Keep first message, insert summary, then last 4
        let first = self.history[0].clone();
        let tail: Vec<Message> = self.history[len - keep_end..].to_vec();

        self.history.clear();
        self.history.push(first);
        self.history.push(summary_msg);
        self.history.extend(tail);

        eprintln!("[pawan] Pruned {} messages from history, context estimate was {}", pruned_count, self.context_tokens_estimate);
    }

    /// Add a message to history
    pub fn add_message(&mut self, message: Message) {
        self.history.push(message);
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
            // Estimate context tokens
            self.context_tokens_estimate = self.history.iter().map(|m| m.content.len()).sum::<usize>() / 4;
            if self.context_tokens_estimate > self.config.max_context_tokens {
                self.prune_history();
            }

            let tool_defs = self.tools.get_definitions();
            let response = self
                .backend
                .generate(&self.history, &tool_defs, on_token.as_ref())
                .await?;

            // Accumulate token usage
            if let Some(ref usage) = response.usage {
                total_usage.prompt_tokens += usage.prompt_tokens;
                total_usage.completion_tokens += usage.completion_tokens;
                total_usage.total_tokens += usage.total_tokens;
            }

            if response.tool_calls.is_empty() {
                self.history.push(Message {
                    role: Role::Assistant,
                    content: response.content.clone(),
                    tool_calls: vec![],
                    tool_result: None,
                });

                return Ok(AgentResponse {
                    content: response.content,
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
                // Check permission
                if let Some(crate::config::ToolPermission::Deny) =
                    self.config.permissions.get(&tool_call.name)
                {
                    let record = ToolCallRecord {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        arguments: tool_call.arguments.clone(),
                        result: json!({"error": "Tool denied by permission policy"}),
                        success: false,
                        duration_ms: 0,
                    };

                    if let Some(ref callback) = on_tool {
                        callback(&record);
                    }
                    all_tool_calls.push(record);

                    self.history.push(Message {
                        role: Role::Tool,
                        content: "{\"error\": \"Tool denied by permission policy\"}".to_string(),
                        tool_calls: vec![],
                        tool_result: Some(ToolResultMessage {
                            tool_call_id: tool_call.id.clone(),
                            content: json!({"error": "Tool denied by permission policy"}),
                            success: false,
                        }),
                    });
                    continue;
                }

                // Notify tool start
                if let Some(ref callback) = on_tool_start {
                    callback(&tool_call.name);
                }

                let start = std::time::Instant::now();

                let result = self
                    .tools
                    .execute(&tool_call.name, tool_call.arguments.clone())
                    .await;
                let duration_ms = start.elapsed().as_millis() as u64;

                let (result_value, success) = match result {
                    Ok(v) => (v, true),
                    Err(e) => (json!({"error": e.to_string()}), false),
                };

                // Truncate tool results that exceed max chars to prevent context bloat
                let max_result_chars = self.config.max_result_chars;
                let result_value = {
                    let result_str = serde_json::to_string(&result_value).unwrap_or_default();
                    if result_str.len() > max_result_chars {
                        let truncated = &result_str[..max_result_chars];
                        serde_json::from_str(truncated).unwrap_or_else(|_| {
                            json!({"content": format!("{}...[truncated from {} chars]", &result_str[..max_result_chars], result_str.len())})
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
                eprintln!("[pawan] Healing complete after {} attempt(s)", attempt);
                return Ok(last_response);
            }

            eprintln!("[pawan] {} errors remain after attempt {}, retrying heal...", errors.len(), attempt);
            last_response = self.heal().await?;
        }

        eprintln!("[pawan] Healing finished after {} attempts (may still have errors)", max_attempts);
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

        let json = serde_json::to_string(&msg).unwrap();
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

        let json = serde_json::to_string(&tc).unwrap();
        assert!(json.contains("read_file"));
        assert!(json.contains("test.txt"));
    }
}
