//! Core wire types for the agent protocol.
//!
//! Pure data — no business logic. Every type here is `Send + Sync` and either
//! `Clone` or cheaply constructible. Kept in a separate module so the 3 kLOC
//! `mod.rs` does not have to recompile just because a callback signature changes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

/// Result from a tool execution (embedded in a [`Message`] with `Role::Tool`)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultMessage {
    /// ID of the tool call this result is for
    pub tool_call_id: String,
    /// The result content
    pub content: Value,
    /// Whether the tool executed successfully
    pub success: bool,
}

/// Record of a completed tool call (accumulated by the agent loop)
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
    /// Wall-clock duration in milliseconds
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

/// Result from a complete agent run (returned by [`super::PawanAgent::execute`])
#[derive(Debug)]
pub struct AgentResponse {
    /// Final text response
    pub content: String,
    /// All tool calls made during execution
    pub tool_calls: Vec<ToolCallRecord>,
    /// Number of LLM round-trips taken
    pub iterations: usize,
    /// Cumulative token usage across all iterations
    pub usage: TokenUsage,
}

/// Streaming token callback — called with each partial token as it arrives.
pub type TokenCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Tool call update callback — called once per completed tool call record.
pub type ToolCallback = Box<dyn Fn(&ToolCallRecord) + Send + Sync>;

/// Tool-start notification callback — called with the tool name when execution begins.
pub type ToolStartCallback = Box<dyn Fn(&str) + Send + Sync>;

/// A permission request forwarded from the agent to the UI.
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    /// Tool name requesting permission
    pub tool_name: String,
    /// Human-readable summary of what the tool will do (e.g. the bash command)
    pub args_summary: String,
}

/// Permission callback — the UI returns a one-shot receiver that resolves to
/// `true` (allow) or `false` (deny) once the user responds.
pub type PermissionCallback =
    Box<dyn Fn(PermissionRequest) -> tokio::sync::oneshot::Receiver<bool> + Send + Sync>;
