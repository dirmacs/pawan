//! Multi-turn tool coordinator — data types and runtime.
//!
//! Provides a provider-agnostic orchestration layer for agent tool-calling
//! loops: send a prompt with tool definitions, handle tool call requests,
//! execute tools, feed results back, repeat until the model produces a final
//! response or hits an iteration cap.
//!
//! Types reused from [`crate::agent`]:
//! - [`ToolCallRequest`] — what the model asks for
//! - [`ToolCallRecord`]  — what actually happened
//! - [`TokenUsage`]      — accumulated counts
//!
//! Types defined here:
//! - [`ToolCallingConfig`]   — iteration / parallelism / timeout knobs
//! - [`FinishReason`]        — why the session ended
//! - [`MessageRole`]         — system / user / assistant / tool
//! - [`ConversationMessage`] — a single turn in the history
//! - [`CoordinatorResult`]   — everything the caller gets back
//! - [`ToolCoordinator`]     — the runtime that drives the LLM+tool loop
//!
//! ## Design notes
//!
//! - [`ToolCallRecord`] is reused from [`crate::agent`] rather than duplicated.
//!   Failed tool calls land in `result` as a `{"error": "..."}` JSON object
//!   with `success: false`, matching pawan's existing agent loop — there's no
//!   separate `error` field on the record.
//! - [`ConversationMessage::tool_call_id`] is only populated on [`MessageRole::Tool`]
//!   turns and links the result back to the assistant message that requested it.

use crate::agent::backend::LlmBackend;
use crate::agent::{Message, Role, ToolCallRecord, ToolCallRequest, ToolResultMessage, TokenUsage};
use crate::tools::ToolRegistry;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::timeout;

/// Configuration for tool calling coordination behavior.
///
/// Controls how the coordinator handles multi-turn tool calling — iteration
/// limits, parallelism, per-tool timeout, and error propagation.
#[derive(Debug, Clone)]
pub struct ToolCallingConfig {
    /// Maximum number of LLM iterations (round-trips) before stopping.
    /// This is *not* the max number of tool calls — one iteration may fan out
    /// into many parallel tool calls. Defaults to 10.
    pub max_iterations: usize,

    /// Whether to execute multiple tool calls in parallel within a single
    /// iteration. `false` forces sequential execution in the order the model
    /// requested. Defaults to `true`.
    pub parallel_execution: bool,

    /// Timeout for individual tool execution. Defaults to 30 seconds.
    pub tool_timeout: Duration,

    /// Whether to abort the whole session when any tool errors, or continue
    /// with remaining tools in the same iteration. Defaults to `false` —
    /// errors become `ToolCallRecord { success: false, ... }` and the loop
    /// keeps going.
    pub stop_on_error: bool,
}

impl Default for ToolCallingConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            parallel_execution: true,
            tool_timeout: Duration::from_secs(30),
            stop_on_error: false,
        }
    }
}

/// Reason why a tool coordination session ended.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FinishReason {
    /// Model produced a final response with no tool calls.
    Stop,
    /// Hit the configured [`ToolCallingConfig::max_iterations`] ceiling.
    MaxIterations,
    /// An unrecoverable error aborted the session. Recoverable errors land
    /// on individual [`ToolCallRecord`]s with `success: false` instead.
    Error(String),
    /// The model requested a tool that isn't registered in the tool registry.
    UnknownTool(String),
}

impl std::fmt::Display for FinishReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinishReason::Stop => write!(f, "stop"),
            FinishReason::MaxIterations => write!(f, "max_iterations"),
            FinishReason::Error(e) => write!(f, "error: {}", e),
            FinishReason::UnknownTool(t) => write!(f, "unknown_tool: {}", t),
        }
    }
}

/// Role of a message sender in a tool-calling conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    /// System instructions — sets the assistant's behavior.
    System,
    /// User message — the prompt driving the conversation.
    User,
    /// Assistant response — may include `tool_calls`.
    Assistant,
    /// Tool execution result — carries a `tool_call_id` linking it to the
    /// assistant message that requested it.
    Tool,
}

/// A single message in a tool-calling conversation.
///
/// Covers all four roles in [`MessageRole`]. For `Assistant` messages,
/// `tool_calls` may be populated with the requests the model wants executed.
/// For `Tool` messages, `tool_call_id` carries the ID of the request being
/// responded to and `content` holds the JSON-serialized result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    /// Who sent this message.
    pub role: MessageRole,

    /// Text content. For `Tool` messages this is a JSON-serialized result.
    pub content: String,

    /// Tool calls requested by the assistant. Empty for non-assistant messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRequest>,

    /// ID of the tool call this message is responding to. Present only on
    /// `Tool` role messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ConversationMessage {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// Create an assistant message, optionally carrying tool calls.
    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCallRequest>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    /// Create a tool-result message. The result value is JSON-serialized into
    /// `content`; if serialization fails the message falls back to `"{}"`.
    pub fn tool_result(tool_call_id: impl Into<String>, result: &serde_json::Value) -> Self {
        Self {
            role: MessageRole::Tool,
            content: serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string()),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Result of a complete tool coordination session.
///
/// Captures the final text response, every tool call made, iteration count,
/// end-of-session reason, accumulated token usage, and the full message
/// history (useful for debugging, distillation, and replay).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorResult {
    /// Final text response from the model.
    pub content: String,

    /// Every tool call executed during the session, in order.
    pub tool_calls: Vec<ToolCallRecord>,

    /// Number of LLM round-trips performed before the session ended.
    pub iterations: usize,

    /// Why the session ended.
    pub finish_reason: FinishReason,

    /// Accumulated token usage across every iteration.
    pub total_usage: TokenUsage,

    /// Full message history, in order, including tool-result turns. Useful
    /// for debugging, distillation, and training-data export.
    pub message_history: Vec<ConversationMessage>,
}

// ---------------------------------------------------------------------------
// Type bridge: ConversationMessage → agent::Message
// ---------------------------------------------------------------------------

/// Convert a [`ConversationMessage`] to the backend's [`Message`] type.
///
/// The coordinator tracks history in its own `ConversationMessage` type, but
/// `LlmBackend::generate()` expects `&[agent::Message]`. This function maps
/// the coordinator's richer type to the backend wire format:
///
/// - `Tool` role messages: parse `content` back to JSON and populate
///   `Message::tool_result` with a `ToolResultMessage`.
/// - `Assistant` messages: copy `tool_calls` directly (same type).
/// - `System`/`User` messages: straightforward role + content copy.
fn to_backend_message(msg: &ConversationMessage) -> Message {
    let role = match msg.role {
        MessageRole::System => Role::System,
        MessageRole::User => Role::User,
        MessageRole::Assistant => Role::Assistant,
        MessageRole::Tool => Role::Tool,
    };

    let tool_result = if msg.role == MessageRole::Tool {
        msg.tool_call_id.as_ref().map(|id| ToolResultMessage {
            tool_call_id: id.clone(),
            content: serde_json::from_str(&msg.content).unwrap_or(serde_json::Value::String(msg.content.clone())),
            success: true,
        })
    } else {
        None
    };

    Message {
        role,
        content: msg.content.clone(),
        tool_calls: msg.tool_calls.clone(),
        tool_result,
    }
}

// ---------------------------------------------------------------------------
// ToolCoordinator runtime
// ---------------------------------------------------------------------------

/// Runtime that drives the LLM + tool-calling loop.
///
/// Wraps a backend and a tool registry, sends prompts with tool definitions,
/// executes requested tools, feeds results back, and repeats until the model
/// produces a final text response or a halt condition fires.
///
/// # Example
///
/// ```rust,ignore
/// use pawan::coordinator::{ToolCoordinator, ToolCallingConfig};
/// use pawan::tools::ToolRegistry;
/// use std::sync::Arc;
///
/// let backend = Arc::new(my_backend);
/// let registry = Arc::new(ToolRegistry::new());
/// let coordinator = ToolCoordinator::new(backend, registry, ToolCallingConfig::default());
///
/// let result = coordinator.execute(Some("You are helpful."), "What is 2+2?").await?;
/// println!("{}", result.content);
/// ```
pub struct ToolCoordinator {
    backend: Arc<dyn LlmBackend>,
    registry: Arc<ToolRegistry>,
    config: ToolCallingConfig,
}

impl ToolCoordinator {
    /// Create a new `ToolCoordinator`.
    pub fn new(
        backend: Arc<dyn LlmBackend>,
        registry: Arc<ToolRegistry>,
        config: ToolCallingConfig,
    ) -> Self {
        Self { backend, registry, config }
    }

    /// Execute a tool-calling session starting from a plain prompt.
    ///
    /// Builds an initial `[system?, user]` message list and drives the loop.
    pub async fn execute(
        &self,
        system_prompt: Option<&str>,
        user_prompt: &str,
    ) -> crate::Result<CoordinatorResult> {
        let mut messages: Vec<ConversationMessage> = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(ConversationMessage::system(sys));
        }
        messages.push(ConversationMessage::user(user_prompt));
        self.execute_with_history(messages).await
    }

    /// Execute a tool-calling session from an existing message history.
    ///
    /// This is the primary loop: it calls the backend, dispatches tool calls,
    /// appends results to history, and repeats until the model emits a final
    /// text response or a halt condition fires.
    pub async fn execute_with_history(
        &self,
        mut messages: Vec<ConversationMessage>,
    ) -> crate::Result<CoordinatorResult> {
        let tool_defs = self.registry.get_definitions();
        let mut all_tool_calls: Vec<ToolCallRecord> = Vec::new();
        let mut total_usage = TokenUsage::default();

        for iteration in 0..self.config.max_iterations {
            // Convert coordinator messages to backend wire format.
            let backend_messages: Vec<Message> =
                messages.iter().map(to_backend_message).collect();

            // Call backend — no streaming callback needed for coordinator.
            let response = self
                .backend
                .generate(&backend_messages, &tool_defs, None)
                .await?;

            // Accumulate token usage.
            if let Some(usage) = &response.usage {
                total_usage.prompt_tokens += usage.prompt_tokens;
                total_usage.completion_tokens += usage.completion_tokens;
                total_usage.total_tokens += usage.total_tokens;
                total_usage.reasoning_tokens += usage.reasoning_tokens;
                total_usage.action_tokens += usage.action_tokens;
            }

            // Append the assistant turn to history.
            messages.push(ConversationMessage::assistant(
                &response.content,
                response.tool_calls.clone(),
            ));

            // No tool calls → model is done.
            if response.tool_calls.is_empty() {
                return Ok(CoordinatorResult {
                    content: response.content,
                    tool_calls: all_tool_calls,
                    iterations: iteration + 1,
                    finish_reason: FinishReason::Stop,
                    total_usage,
                    message_history: messages,
                });
            }

            // Empty response with tool calls is unusual but guard it.
            if response.content.is_empty() && response.tool_calls.is_empty() {
                return Ok(CoordinatorResult {
                    content: String::new(),
                    tool_calls: all_tool_calls,
                    iterations: iteration + 1,
                    finish_reason: FinishReason::Stop,
                    total_usage,
                    message_history: messages,
                });
            }

            // Validate all requested tools exist before executing any.
            for tc in &response.tool_calls {
                if !self.registry.has_tool(&tc.name) {
                    return Ok(CoordinatorResult {
                        content: response.content,
                        tool_calls: all_tool_calls,
                        iterations: iteration + 1,
                        finish_reason: FinishReason::UnknownTool(tc.name.clone()),
                        total_usage,
                        message_history: messages,
                    });
                }
            }

            // Execute tool calls (parallel or sequential per config).
            let records = self.execute_tool_calls(&response.tool_calls).await?;

            // If stop_on_error, check if any record failed.
            if self.config.stop_on_error {
                if let Some(failed) = records.iter().find(|r| !r.success) {
                    let err_msg = failed
                        .result
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tool error")
                        .to_string();
                    return Ok(CoordinatorResult {
                        content: response.content,
                        tool_calls: all_tool_calls,
                        iterations: iteration + 1,
                        finish_reason: FinishReason::Error(err_msg),
                        total_usage,
                        message_history: messages,
                    });
                }
            }

            // Append tool result messages and accumulate records.
            for record in records {
                messages.push(ConversationMessage::tool_result(&record.id, &record.result));
                all_tool_calls.push(record);
            }
        }

        // Hit max iterations.
        Ok(CoordinatorResult {
            content: messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default(),
            tool_calls: all_tool_calls,
            iterations: self.config.max_iterations,
            finish_reason: FinishReason::MaxIterations,
            total_usage,
            message_history: messages,
        })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    async fn execute_tool_calls(
        &self,
        calls: &[ToolCallRequest],
    ) -> crate::Result<Vec<ToolCallRecord>> {
        if self.config.parallel_execution {
            self.execute_parallel(calls).await
        } else {
            self.execute_sequential(calls).await
        }
    }

    async fn execute_parallel(
        &self,
        calls: &[ToolCallRequest],
    ) -> crate::Result<Vec<ToolCallRecord>> {
        let futures = calls.iter().map(|c| self.execute_single_tool(c));
        let results = join_all(futures).await;

        let mut records = Vec::with_capacity(results.len());
        for (i, res) in results.into_iter().enumerate() {
            match res {
                Ok(record) => records.push(record),
                Err(e) if self.config.stop_on_error => return Err(e),
                Err(e) => {
                    // Recover: turn the error into a failed ToolCallRecord.
                    let call = &calls[i];
                    records.push(ToolCallRecord {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        result: serde_json::json!({"error": e.to_string()}),
                        success: false,
                        duration_ms: 0,
                    });
                }
            }
        }
        Ok(records)
    }

    async fn execute_sequential(
        &self,
        calls: &[ToolCallRequest],
    ) -> crate::Result<Vec<ToolCallRecord>> {
        let mut records = Vec::with_capacity(calls.len());
        for call in calls {
            match self.execute_single_tool(call).await {
                Ok(record) => records.push(record),
                Err(e) if self.config.stop_on_error => return Err(e),
                Err(e) => {
                    records.push(ToolCallRecord {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        result: serde_json::json!({"error": e.to_string()}),
                        success: false,
                        duration_ms: 0,
                    });
                }
            }
        }
        Ok(records)
    }

    async fn execute_single_tool(&self, call: &ToolCallRequest) -> crate::Result<ToolCallRecord> {
        let start = Instant::now();

        let result = timeout(
            self.config.tool_timeout,
            self.registry.execute(&call.name, call.arguments.clone()),
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(value)) => Ok(ToolCallRecord {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                result: value,
                success: true,
                duration_ms,
            }),
            Ok(Err(e)) => Ok(ToolCallRecord {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                result: serde_json::json!({"error": e.to_string()}),
                success: false,
                duration_ms,
            }),
            Err(_elapsed) => Ok(ToolCallRecord {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                result: serde_json::json!({"error": "tool execution timed out"}),
                success: false,
                duration_ms,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_calling_config_default_values() {
        let cfg = ToolCallingConfig::default();
        assert_eq!(cfg.max_iterations, 10);
        assert!(cfg.parallel_execution);
        assert_eq!(cfg.tool_timeout, Duration::from_secs(30));
        assert!(!cfg.stop_on_error);
    }

    #[test]
    fn finish_reason_display_matches_snake_case_contract() {
        assert_eq!(FinishReason::Stop.to_string(), "stop");
        assert_eq!(FinishReason::MaxIterations.to_string(), "max_iterations");
        assert_eq!(
            FinishReason::Error("boom".into()).to_string(),
            "error: boom"
        );
        assert_eq!(
            FinishReason::UnknownTool("ghost".into()).to_string(),
            "unknown_tool: ghost"
        );
    }

    #[test]
    fn finish_reason_round_trips_through_json() {
        for variant in [
            FinishReason::Stop,
            FinishReason::MaxIterations,
            FinishReason::Error("oops".into()),
            FinishReason::UnknownTool("nope".into()),
        ] {
            let encoded = serde_json::to_string(&variant).unwrap();
            let decoded: FinishReason = serde_json::from_str(&encoded).unwrap();
            assert_eq!(
                decoded, variant,
                "{} did not round-trip through JSON",
                variant
            );
        }
    }

    #[test]
    fn message_role_serializes_as_lowercase_string() {
        assert_eq!(
            serde_json::to_string(&MessageRole::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Tool).unwrap(),
            "\"tool\""
        );
    }

    #[test]
    fn conversation_message_system_builder_sets_role_and_content() {
        let msg = ConversationMessage::system("you are an assistant");
        assert_eq!(msg.role, MessageRole::System);
        assert_eq!(msg.content, "you are an assistant");
        assert!(msg.tool_calls.is_empty());
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn conversation_message_user_builder_sets_role_and_content() {
        let msg = ConversationMessage::user("what is 2 + 2?");
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "what is 2 + 2?");
        assert!(msg.tool_calls.is_empty());
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn conversation_message_assistant_builder_preserves_tool_calls() {
        let calls = vec![ToolCallRequest {
            id: "call_1".into(),
            name: "search".into(),
            arguments: json!({"q": "rust"}),
        }];
        let msg = ConversationMessage::assistant("let me search", calls.clone());
        assert_eq!(msg.role, MessageRole::Assistant);
        assert_eq!(msg.content, "let me search");
        assert_eq!(msg.tool_calls.len(), 1);
        assert_eq!(msg.tool_calls[0].id, "call_1");
        assert_eq!(msg.tool_calls[0].name, "search");
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn conversation_message_tool_result_serializes_result_into_content() {
        let result = json!({"answer": 42, "units": "none"});
        let msg = ConversationMessage::tool_result("call_1", &result);
        assert_eq!(msg.role, MessageRole::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_1"));
        assert!(msg.tool_calls.is_empty());
        // Content is the JSON-serialized form — not the Display form.
        let parsed: serde_json::Value = serde_json::from_str(&msg.content).unwrap();
        assert_eq!(parsed, result);
    }

    #[test]
    fn conversation_message_tool_result_falls_back_on_serialize_failure() {
        // Any finite JSON value always serializes, so this test just pins the
        // contract that the function returns a ConversationMessage rather than
        // panicking. The "{}" fallback path is unreachable in practice but
        // documented in the function contract.
        let msg = ConversationMessage::tool_result("call_1", &json!(null));
        assert_eq!(msg.content, "null");
    }

    #[test]
    fn conversation_message_serde_skips_empty_tool_calls_and_none_id() {
        let msg = ConversationMessage::user("hi");
        let encoded = serde_json::to_string(&msg).unwrap();
        // Empty Vec<ToolCallRequest> skipped, None tool_call_id skipped.
        assert!(!encoded.contains("tool_calls"));
        assert!(!encoded.contains("tool_call_id"));
        assert!(encoded.contains("\"role\":\"user\""));
        assert!(encoded.contains("\"content\":\"hi\""));
    }

    #[test]
    fn coordinator_result_round_trips_through_json() {
        let result = CoordinatorResult {
            content: "done".into(),
            tool_calls: vec![ToolCallRecord {
                id: "call_1".into(),
                name: "echo".into(),
                arguments: json!({"text": "hi"}),
                result: json!({"text": "hi"}),
                success: true,
                duration_ms: 12,
            }],
            iterations: 2,
            finish_reason: FinishReason::Stop,
            total_usage: TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 20,
                total_tokens: 120,
                reasoning_tokens: 0,
                action_tokens: 20,
            },
            message_history: vec![
                ConversationMessage::system("be brief"),
                ConversationMessage::user("echo hi"),
                ConversationMessage::assistant(
                    "",
                    vec![ToolCallRequest {
                        id: "call_1".into(),
                        name: "echo".into(),
                        arguments: json!({"text": "hi"}),
                    }],
                ),
                ConversationMessage::tool_result("call_1", &json!({"text": "hi"})),
                ConversationMessage::assistant("done", vec![]),
            ],
        };
        let encoded = serde_json::to_string(&result).unwrap();
        let decoded: CoordinatorResult = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.content, "done");
        assert_eq!(decoded.iterations, 2);
        assert_eq!(decoded.finish_reason, FinishReason::Stop);
        assert_eq!(decoded.tool_calls.len(), 1);
        assert_eq!(decoded.tool_calls[0].id, "call_1");
        assert_eq!(decoded.message_history.len(), 5);
        assert_eq!(decoded.total_usage.total_tokens, 120);
    }

    // -----------------------------------------------------------------------
    // ToolCoordinator runtime tests
    // -----------------------------------------------------------------------

    /// No tools available — model replies with plain text on the first turn.
    /// Verifies that the coordinator terminates cleanly and returns the model
    /// text as `content` with `FinishReason::Stop` and zero tool calls.
    #[tokio::test]
    async fn execute_with_empty_registry_returns_model_response() {
        use crate::agent::backend::mock::MockBackend;

        let backend = Arc::new(MockBackend::with_text("Hello, world!"));
        let registry = Arc::new(ToolRegistry::new());
        let coordinator = ToolCoordinator::new(backend, registry, ToolCallingConfig::default());

        let result = coordinator
            .execute(None, "Say hello")
            .await
            .expect("coordinator should not error");

        assert_eq!(result.content, "Hello, world!");
        assert_eq!(result.finish_reason, FinishReason::Stop);
        assert_eq!(result.iterations, 1);
        assert!(result.tool_calls.is_empty());
        // History: [user, assistant]
        assert_eq!(result.message_history.len(), 2);
    }

    /// Pin the `ToolCallingConfig` defaults so regressions are caught.
    #[test]
    fn tool_calling_config_defaults_are_sensible() {
        let cfg = ToolCallingConfig::default();
        assert_eq!(cfg.max_iterations, 10, "max_iterations default changed");
        assert!(cfg.parallel_execution, "parallel_execution should default to true");
        assert_eq!(cfg.tool_timeout, Duration::from_secs(30), "tool_timeout default changed");
        assert!(!cfg.stop_on_error, "stop_on_error should default to false");
    }

    /// The coordinator must fire `FinishReason::MaxIterations` when the model
    /// keeps requesting tool calls and we exhaust the iteration budget.
    /// Uses a mock backend that always returns a tool-call response for a
    /// registered no-op tool, driving the loop to the configured cap.
    #[tokio::test]
    async fn coordinator_result_captures_finish_reason_max_iterations() {
        use crate::agent::backend::mock::{MockBackend, MockResponse};
        use async_trait::async_trait;
        use crate::tools::Tool;
        use serde_json::Value;

        // A trivial no-op tool that always succeeds.
        struct NoOpTool;

        #[async_trait]
        impl Tool for NoOpTool {
            fn name(&self) -> &str { "noop" }
            fn description(&self) -> &str { "does nothing" }
            fn parameters_schema(&self) -> Value {
                serde_json::json!({"type": "object", "properties": {}})
            }
            async fn execute(&self, _args: Value) -> crate::Result<Value> {
                Ok(serde_json::json!({"ok": true}))
            }
        }

        // Build a backend that always requests the noop tool (never gives a
        // final text response), so the loop runs until max_iterations.
        let responses: Vec<MockResponse> = (0..15)
            .map(|_| MockResponse::tool_call("noop", serde_json::json!({})))
            .collect();
        let backend = Arc::new(MockBackend::new(responses));

        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(NoOpTool));
        let registry = Arc::new(registry);

        let config = ToolCallingConfig {
            max_iterations: 3,
            parallel_execution: false,
            ..ToolCallingConfig::default()
        };
        let coordinator = ToolCoordinator::new(backend, registry, config);

        let result = coordinator
            .execute(None, "loop forever")
            .await
            .expect("coordinator should not hard-error");

        assert_eq!(
            result.finish_reason,
            FinishReason::MaxIterations,
            "expected MaxIterations, got {:?}",
            result.finish_reason
        );
        assert_eq!(result.iterations, 3);
        // Each iteration dispatches one noop tool call.
        assert_eq!(result.tool_calls.len(), 3);
        assert!(result.tool_calls.iter().all(|tc| tc.success));
    }
}
