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
use crate::agent::{LLMResponse, Message, ToolCallRequest, TokenCallback};
use crate::tools::ToolDefinition;
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A pre-configured response from the mock backend
#[derive(Clone, Debug)]
pub enum MockResponse {
    /// Plain text response (no tool calls) — agent loop ends
    Text(String),
    /// Tool call request — agent will execute the tool and send result back
    ToolCall {
        id: String,
        name: String,
        args: Value,
    },
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
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                usage: None,
            },
            MockResponse::ToolCall { id, name, args } => LLMResponse {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id,
                    name,
                    arguments: args,
                }],
                finish_reason: "tool_calls".to_string(),
                usage: None,
            },
        })
    }
}
