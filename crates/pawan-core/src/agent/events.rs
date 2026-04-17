//! Typed event stream for agent UI rendering.
//!
//! Provides a sealed enum of agent events that decouples the agent loop
//! from TUI rendering. All rendering in pawan-cli subscribes to these events.

use serde::{Deserialize, Serialize};

/// A turn in the agent loop has started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStartEvent {
    /// The user prompt that started this turn
    pub prompt: String,
    /// Timestamp when the turn started (Unix epoch seconds)
    pub timestamp_secs: u64,
}

/// The model is producing a thinking delta (partial content).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingDeltaEvent {
    /// The delta content
    pub content: String,
    /// Whether this is the first delta
    pub is_first: bool,
}

/// A tool call was approved (either auto-approved or user-confirmed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolApprovalEvent {
    /// Tool call ID
    pub call_id: String,
    /// Tool name
    pub tool_name: String,
    /// Arguments passed to the tool
    pub arguments: serde_json::Value,
    /// Whether this was auto-approved (read-only tool)
    pub auto_approved: bool,
}

/// A tool has started executing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStartEvent {
    /// Tool call ID
    pub call_id: String,
    /// Tool name
    pub tool_name: String,
    /// Arguments passed to the tool
    pub arguments: serde_json::Value,
}

/// A tool has completed execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCompleteEvent {
    /// Tool call ID
    pub call_id: String,
    /// Tool name
    pub tool_name: String,
    /// Result from the tool execution
    pub result: String,
    /// Whether the tool executed successfully
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// A turn in the agent loop has ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnEndEvent {
    /// The final content from the model
    pub content: String,
    /// Number of tool calls in this turn
    pub tool_call_count: usize,
    /// Total iterations used
    pub iterations: usize,
    /// Token usage for this turn
    pub usage: TokenUsageInfo,
}

/// Token usage information.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsageInfo {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// The agent session has ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEndEvent {
    /// The final response content
    pub content: String,
    /// Total iterations used
    pub total_iterations: usize,
    /// Total token usage
    pub usage: TokenUsageInfo,
    /// Why the session ended
    pub finish_reason: FinishReason,
}

/// Why the agent session ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "message")]
pub enum FinishReason {
    /// The model produced a final response without needing more tool calls
    Stop,
    /// Hit the maximum iteration limit
    MaxIterations,
    /// An error occurred
    Error(String),
    /// An unknown tool was called
    UnknownTool(String),
    /// Session was cancelled by user
    Cancelled,
}

/// A typed event emitted by the agent during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum AgentEvent {
    /// Turn started
    TurnStart(TurnStartEvent),
    /// Thinking delta (partial content)
    ThinkingDelta(ThinkingDeltaEvent),
    /// Tool call approved
    ToolApproval(ToolApprovalEvent),
    /// Tool started executing
    ToolStart(ToolStartEvent),
    /// Tool completed
    ToolComplete(ToolCompleteEvent),
    /// Turn ended
    TurnEnd(TurnEndEvent),
    /// Session ended
    SessionEnd(SessionEndEvent),
}

impl AgentEvent {
    /// Create a TurnStart event
    pub fn turn_start(prompt: &str) -> Self {
        AgentEvent::TurnStart(TurnStartEvent {
            prompt: prompt.to_string(),
            timestamp_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
    }

    /// Create a ThinkingDelta event
    pub fn thinking_delta(content: &str, is_first: bool) -> Self {
        AgentEvent::ThinkingDelta(ThinkingDeltaEvent {
            content: content.to_string(),
            is_first,
        })
    }

    /// Create a ToolApproval event
    pub fn tool_approval(call_id: &str, tool_name: &str, arguments: serde_json::Value, auto_approved: bool) -> Self {
        AgentEvent::ToolApproval(ToolApprovalEvent {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments,
            auto_approved,
        })
    }

    /// Create a ToolStart event
    pub fn tool_start(call_id: &str, tool_name: &str, arguments: serde_json::Value) -> Self {
        AgentEvent::ToolStart(ToolStartEvent {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments,
        })
    }

    /// Create a ToolComplete event
    pub fn tool_complete(call_id: &str, tool_name: &str, result: &str, success: bool, duration_ms: u64) -> Self {
        AgentEvent::ToolComplete(ToolCompleteEvent {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            result: result.to_string(),
            success,
            duration_ms,
        })
    }

    /// Create a TurnEnd event
    pub fn turn_end(content: &str, tool_call_count: usize, iterations: usize, usage: TokenUsageInfo) -> Self {
        AgentEvent::TurnEnd(TurnEndEvent {
            content: content.to_string(),
            tool_call_count,
            iterations,
            usage,
        })
    }

    /// Create a SessionEnd event
    pub fn session_end(content: &str, total_iterations: usize, usage: TokenUsageInfo, reason: FinishReason) -> Self {
        AgentEvent::SessionEnd(SessionEndEvent {
            content: content.to_string(),
            total_iterations,
            usage,
            finish_reason: reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_start_event() {
        let event = AgentEvent::turn_start("Hello, world!");
        assert!(matches!(event, AgentEvent::TurnStart(_)));
    }

    #[test]
    fn test_thinking_delta_event() {
        let event = AgentEvent::thinking_delta("Hello", true);
        assert!(matches!(event, AgentEvent::ThinkingDelta(_)));
    }

    #[test]
    fn test_tool_approval_event() {
        let args = serde_json::json!({"command": "ls"});
        let event = AgentEvent::tool_approval("call_1", "bash", args, true);
        assert!(matches!(event, AgentEvent::ToolApproval(_)));
    }

    #[test]
    fn test_tool_start_event() {
        let args = serde_json::json!({"command": "ls"});
        let event = AgentEvent::tool_start("call_1", "bash", args);
        assert!(matches!(event, AgentEvent::ToolStart(_)));
    }

    #[test]
    fn test_tool_complete_event() {
        let event = AgentEvent::tool_complete("call_1", "bash", "file1\nfile2", true, 100);
        assert!(matches!(event, AgentEvent::ToolComplete(_)));
    }

    #[test]
    fn test_turn_end_event() {
        let usage = TokenUsageInfo {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        };
        let event = AgentEvent::turn_end("Done!", 2, 5, usage);
        assert!(matches!(event, AgentEvent::TurnEnd(_)));
    }

    #[test]
    fn test_session_end_event() {
        let usage = TokenUsageInfo::default();
        let event = AgentEvent::session_end("Goodbye!", 10, usage, FinishReason::Stop);
        assert!(matches!(event, AgentEvent::SessionEnd(_)));
    }

    #[test]
    fn test_finish_reason_variants() {
        let stop = FinishReason::Stop;
        let max_iter = FinishReason::MaxIterations;
        let error = FinishReason::Error("something went wrong".to_string());
        let unknown = FinishReason::UnknownTool("fake_tool".to_string());
        let cancelled = FinishReason::Cancelled;

        assert!(matches!(stop, FinishReason::Stop));
        assert!(matches!(max_iter, FinishReason::MaxIterations));
        assert!(matches!(error, FinishReason::Error(_)));
        assert!(matches!(unknown, FinishReason::UnknownTool(_)));
        assert!(matches!(cancelled, FinishReason::Cancelled));
    }

    #[test]
    fn test_event_serialization() {
        let event = AgentEvent::turn_start("test prompt");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("TurnStart"));
        assert!(json.contains("test prompt"));
    }
}