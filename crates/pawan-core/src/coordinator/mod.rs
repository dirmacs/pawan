//! Multi-turn tool coordinator — data types.
//!
//! Provides a provider-agnostic orchestration layer for agent tool-calling
//! loops: send a prompt with tool definitions, handle tool call requests,
//! execute tools, feed results back, repeat until the model produces a final
//! response or hits an iteration cap.
//!
//! This module currently defines the data types only. The [`ToolCoordinator`]
//! runtime and the LLM client trait are introduced in a follow-up change so
//! the types can be reviewed and tested on their own first.
//!
//! Types reused from [`crate::agent`]:
//! - [`ToolCallRequest`] — what the model asks for
//! - [`ToolCallRecord`]  — what actually happened
//! - [`TokenUsage`]      — accumulated counts
//!
//! New types introduced here:
//! - [`ToolCallingConfig`]   — iteration / parallelism / timeout knobs
//! - [`FinishReason`]        — why the session ended
//! - [`MessageRole`]         — system / user / assistant / tool
//! - [`ConversationMessage`] — a single turn in the history
//! - [`CoordinatorResult`]   — everything the caller gets back
//!
//! ## Design notes
//!
//! - [`ToolCallRecord`] is reused from [`crate::agent`] rather than duplicated.
//!   Failed tool calls land in `result` as a `{"error": "..."}` JSON object
//!   with `success: false`, matching pawan's existing agent loop — there's no
//!   separate `error` field on the record.
//! - [`ConversationMessage::tool_call_id`] is only populated on [`MessageRole::Tool`]
//!   turns and links the result back to the assistant message that requested it.

use crate::agent::{ToolCallRecord, ToolCallRequest, TokenUsage};
use serde::{Deserialize, Serialize};
use std::time::Duration;

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
}
