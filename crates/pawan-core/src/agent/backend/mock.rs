//! Mock LLM backend for testing
//!
//! Allows testing the full agent tool-calling loop without a real LLM.
//! Pre-load a sequence of responses; the backend returns them in order.
//!
//! ```rust
//! use pawan::agent::backend::mock::{MockBackend, MockResponse};
//! use pawan::agent::PawanAgent;
//! use pawan::config::PawanConfig;
//! use serde_json::json;
//!
//! let backend = MockBackend::new(vec![
//!     MockResponse::text("Hello from mock!"),
//! ]);
//! let mut agent = PawanAgent::new(PawanConfig::default(), ".".into())
//!     .with_backend(Box::new(backend));
//! ```

use crate::agent::backend::LlmBackend;
use crate::agent::{LLMResponse, Message, TokenCallback, TokenUsage, ToolCallRequest};
use crate::tools::ToolDefinition;
use crate::Result;
use async_trait::async_trait;
#[allow(unused_imports)]
use serde_json::json;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A pre-configured response from the mock backend
#[derive(Clone, Debug)]
pub enum MockResponse {
    /// Plain text response (no tool calls) — agent loop ends
    Text(String),
    /// Plain text with token usage
    TextWithUsage { text: String, usage: TokenUsage },
    /// Tool call request — agent will execute the tool and send result back
    ToolCall {
        id: String,
        name: String,
        args: Value,
    },
    /// Multiple tool calls in a single turn (concurrent execution)
    ToolSequence(Vec<ToolCallRequest>),
}

impl MockResponse {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    pub fn tool_call(name: impl Into<String>, args: Value) -> Self {
        Self::ToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            args,
        }
    }

    pub fn tool_sequence(calls: Vec<(&str, Value)>) -> Self {
        Self::ToolSequence(
            calls
                .into_iter()
                .map(|(name, args)| ToolCallRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.to_string(),
                    arguments: args,
                })
                .collect(),
        )
    }
}

// ---------------------------------------------------------------------------
// Scenario-in-prompt detection (ported from claw-code parity harness)
// ---------------------------------------------------------------------------

/// Named test scenarios that the mock backend can detect from prompt content.
/// Embed `PARITY_SCENARIO: <name>` in user messages to trigger deterministic behavior.
#[derive(Debug, Clone, PartialEq)]
pub enum MockScenario {
    /// Simple text response, no tool calls
    TextOnly,
    /// Single read_file tool call, then text completion
    ReadFileRoundtrip,
    /// Bash command execution, then text completion
    BashRoundtrip,
    /// Multiple tools in one turn (read_file + grep_search)
    MultiToolTurn,
    /// Three-step: read → edit → verify
    EditRoundtrip,
}

impl MockScenario {
    /// Detect scenario from message content. Looks for `PARITY_SCENARIO: <name>`.
    pub fn detect(messages: &[Message]) -> Option<Self> {
        for msg in messages {
            if let Some(pos) = msg.content.find("PARITY_SCENARIO:") {
                let rest = msg.content[pos + 16..].trim();
                let name = rest.split_whitespace().next().unwrap_or("");
                return match name {
                    "text_only" => Some(Self::TextOnly),
                    "read_file_roundtrip" => Some(Self::ReadFileRoundtrip),
                    "bash_roundtrip" => Some(Self::BashRoundtrip),
                    "multi_tool_turn" => Some(Self::MultiToolTurn),
                    "edit_roundtrip" => Some(Self::EditRoundtrip),
                    _ => None,
                };
            }
        }
        None
    }

    /// Get the pre-configured response sequence for this scenario.
    pub fn responses(&self) -> Vec<MockResponse> {
        match self {
            Self::TextOnly => vec![MockResponse::text("Scenario complete: text only")],
            Self::ReadFileRoundtrip => vec![
                MockResponse::tool_call("read_file", serde_json::json!({"path": "src/lib.rs"})),
                MockResponse::text("I read the file successfully."),
            ],
            Self::BashRoundtrip => vec![
                MockResponse::tool_call("bash", serde_json::json!({"command": "echo hello"})),
                MockResponse::text("Command executed successfully."),
            ],
            Self::MultiToolTurn => vec![
                MockResponse::tool_sequence(vec![
                    ("read_file", serde_json::json!({"path": "Cargo.toml"})),
                    ("grep_search", serde_json::json!({"pattern": "version"})),
                ]),
                MockResponse::text("Found version info in both files."),
            ],
            Self::EditRoundtrip => vec![
                MockResponse::tool_call("read_file", serde_json::json!({"path": "test.rs"})),
                MockResponse::tool_call(
                    "edit_file",
                    serde_json::json!({
                        "path": "test.rs",
                        "old_string": "old",
                        "new_string": "new"
                    }),
                ),
                MockResponse::text("Edit complete."),
            ],
        }
    }
}

/// Create a MockBackend pre-loaded with responses for a detected scenario.
pub fn mock_from_scenario(scenario: MockScenario) -> MockBackend {
    MockBackend::new(scenario.responses())
}

/// Mock LLM backend — returns pre-configured responses in sequence.
///
/// After all responses are consumed, returns an empty text response
/// so the agent loop terminates cleanly rather than panicking.
pub struct MockBackend {
    responses: Arc<Vec<MockResponse>>,
    index: Arc<AtomicUsize>,
}

impl MockBackend {
    pub fn new(responses: Vec<MockResponse>) -> Self {
        Self {
            responses: Arc::new(responses),
            index: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Single-response convenience constructor
    pub fn with_text(text: impl Into<String>) -> Self {
        Self::new(vec![MockResponse::text(text)])
    }

    /// Convenience constructor for a tool call response
    pub fn with_tool_call(id: &str, name: &str, args: Value, content: &str) -> Self {
        Self::new(vec![
            MockResponse::ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                args,
            },
            MockResponse::Text(content.to_string()),
        ])
    }

    /// Convenience constructor for repeated tool calls (never stops)
    pub fn with_repeated_tool_call(name: &str) -> Self {
        let mut responses = Vec::new();
        for i in 0..32 {
            responses.push(MockResponse::ToolCall {
                id: format!("call_{i}"),
                name: name.to_string(),
                args: json!({}),
            });
        }
        Self::new(responses)
    }

    /// Convenience constructor for multiple tool calls in a single turn
    pub fn with_multiple_tool_calls(calls: Vec<(&str, &str, Value)>) -> Self {
        let tool_calls: Vec<ToolCallRequest> = calls
            .into_iter()
            .map(|(id, name, args)| ToolCallRequest {
                id: id.to_string(),
                name: name.to_string(),
                arguments: args,
            })
            .collect();
        Self::new(vec![
            MockResponse::ToolSequence(tool_calls),
            MockResponse::Text("Done".to_string()),
        ])
    }

    /// Convenience constructor for text response with token usage
    pub fn with_text_and_usage(text: &str, prompt_tokens: u64, completion_tokens: u64) -> Self {
        let reasoning_tokens = completion_tokens / 3;
        let action_tokens = completion_tokens - reasoning_tokens;
        let usage = TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            reasoning_tokens,
            action_tokens,
        };
        Self::new(vec![MockResponse::TextWithUsage {
            text: text.to_string(),
            usage,
        }])
    }
}

#[async_trait]
impl LlmBackend for MockBackend {
    async fn generate(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _on_token: Option<&TokenCallback>,
    ) -> Result<LLMResponse> {
        let idx = self.index.fetch_add(1, Ordering::SeqCst);

        let response = self.responses.get(idx).cloned().unwrap_or_else(|| {
            // All pre-configured responses consumed — return empty text to end loop
            MockResponse::Text(String::new())
        });

        Ok(match response {
            MockResponse::Text(content) => LLMResponse {
                content,
                reasoning: None,
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                usage: None,
            },
            MockResponse::TextWithUsage { text, usage } => LLMResponse {
                content: text,
                reasoning: None,
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                usage: Some(usage),
            },
            MockResponse::ToolCall { id, name, args } => LLMResponse {
                content: String::new(),
                reasoning: None,
                tool_calls: vec![ToolCallRequest {
                    id,
                    name,
                    arguments: args,
                }],
                finish_reason: "tool_calls".to_string(),
                usage: None,
            },
            MockResponse::ToolSequence(calls) => LLMResponse {
                content: String::new(),
                reasoning: None,
                tool_calls: calls,
                finish_reason: "tool_calls".to_string(),
                usage: None,
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_detect_text_only() {
        let messages = vec![Message {
            role: crate::agent::Role::User,
            content: "PARITY_SCENARIO: text_only\nDo something".into(),
            tool_calls: vec![],
            tool_result: None,
        }];
        assert_eq!(
            MockScenario::detect(&messages),
            Some(MockScenario::TextOnly)
        );
    }

    #[test]
    fn test_scenario_detect_read_file() {
        let messages = vec![Message {
            role: crate::agent::Role::User,
            content: "Please PARITY_SCENARIO: read_file_roundtrip".into(),
            tool_calls: vec![],
            tool_result: None,
        }];
        assert_eq!(
            MockScenario::detect(&messages),
            Some(MockScenario::ReadFileRoundtrip)
        );
    }

    #[test]
    fn test_scenario_detect_none() {
        let messages = vec![Message {
            role: crate::agent::Role::User,
            content: "Just a normal message".into(),
            tool_calls: vec![],
            tool_result: None,
        }];
        assert_eq!(MockScenario::detect(&messages), None);
    }

    #[test]
    fn test_scenario_detect_unknown() {
        let messages = vec![Message {
            role: crate::agent::Role::User,
            content: "PARITY_SCENARIO: nonexistent_scenario".into(),
            tool_calls: vec![],
            tool_result: None,
        }];
        assert_eq!(MockScenario::detect(&messages), None);
    }

    #[test]
    fn test_scenario_responses_text_only() {
        let responses = MockScenario::TextOnly.responses();
        assert_eq!(responses.len(), 1);
        assert!(matches!(&responses[0], MockResponse::Text(_)));
    }

    #[test]
    fn test_scenario_responses_read_file() {
        let responses = MockScenario::ReadFileRoundtrip.responses();
        assert_eq!(responses.len(), 2);
        assert!(
            matches!(&responses[0], MockResponse::ToolCall { name, .. } if name == "read_file")
        );
        assert!(matches!(&responses[1], MockResponse::Text(_)));
    }

    #[test]
    fn test_scenario_responses_multi_tool() {
        let responses = MockScenario::MultiToolTurn.responses();
        assert_eq!(responses.len(), 2);
        assert!(matches!(&responses[0], MockResponse::ToolSequence(calls) if calls.len() == 2));
    }

    #[test]
    fn test_scenario_responses_edit_roundtrip() {
        let responses = MockScenario::EditRoundtrip.responses();
        assert_eq!(responses.len(), 3);
        assert!(
            matches!(&responses[0], MockResponse::ToolCall { name, .. } if name == "read_file")
        );
        assert!(
            matches!(&responses[1], MockResponse::ToolCall { name, .. } if name == "edit_file")
        );
        assert!(matches!(&responses[2], MockResponse::Text(_)));
    }

    #[test]
    fn test_mock_from_scenario() {
        let backend = mock_from_scenario(MockScenario::TextOnly);
        assert_eq!(backend.responses.len(), 1);
    }

    #[test]
    fn test_tool_sequence_constructor() {
        let resp = MockResponse::tool_sequence(vec![
            ("read_file", serde_json::json!({"path": "a.rs"})),
            ("bash", serde_json::json!({"command": "ls"})),
        ]);
        if let MockResponse::ToolSequence(calls) = resp {
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].name, "read_file");
            assert_eq!(calls[1].name, "bash");
        } else {
            panic!("Expected ToolSequence");
        }
    }

    #[tokio::test]
    async fn test_mock_backend_tool_sequence() {
        let backend = MockBackend::new(vec![
            MockResponse::tool_sequence(vec![
                ("read_file", serde_json::json!({"path": "a.rs"})),
                ("grep_search", serde_json::json!({"pattern": "fn"})),
            ]),
            MockResponse::text("Done"),
        ]);

        let resp = backend.generate(&[], &[], None).await.unwrap();
        assert_eq!(resp.tool_calls.len(), 2);
        assert_eq!(resp.finish_reason, "tool_calls");

        let resp2 = backend.generate(&[], &[], None).await.unwrap();
        assert_eq!(resp2.content, "Done");
        assert!(resp2.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_mock_backend_exhausted() {
        let backend = MockBackend::new(vec![MockResponse::text("first")]);
        let r1 = backend.generate(&[], &[], None).await.unwrap();
        assert_eq!(r1.content, "first");

        let r2 = backend.generate(&[], &[], None).await.unwrap();
        assert_eq!(r2.content, ""); // exhausted, returns empty
    }
}
