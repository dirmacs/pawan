//! Lancor LLM backend — local inference via llama.cpp.
//!
//! Wraps `lancor::LlamaCppClient` (the OpenAI-compatible client from the
//! lancor llama.cpp toolkit) to satisfy `LlmBackend`. Local inference for
//! pawan workflows that don't need tool calling.
//!
//! ## Limitations
//!
//! Lancor's chat client is **text-only**: there is no native tool-calling
//! support in `lancor::ChatCompletionRequest` / `Message`. This backend
//! drops the `tools` argument from `generate()` and always returns an empty
//! `tool_calls` vec. For tool-using agents, point pawan's `openai_compat`
//! backend at a llama.cpp server's OpenAI-compatible endpoint
//! (`http://localhost:8080/v1`) — that path supports tool calling via the
//! standard OpenAI protocol.
//!
//! Use this backend for: commit message generation, skill distillation
//! summarization, plain-text Q&A, and any other LLM call that doesn't need
//! tools.

use crate::agent::backend::LlmBackend;
use crate::agent::{LLMResponse, Message, Role, TokenCallback, TokenUsage};
use crate::tools::ToolDefinition;
use crate::{PawanError, Result};
use async_trait::async_trait;
use lancor::{ChatCompletionRequest, LlamaCppClient, Message as LancorMessage};

/// Lancor-backed LLM client for local llama.cpp inference.
pub struct LancorBackend {
    client: LlamaCppClient,
    model: String,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
}

impl LancorBackend {
    /// Create a new lancor backend pointed at the given llama.cpp server URL.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        let client = LlamaCppClient::new(base_url)
            .map_err(|e| PawanError::Llm(format!("lancor client init failed: {e}")))?;
        Ok(Self {
            client,
            model: model.into(),
            temperature: None,
            max_tokens: None,
        })
    }

    /// Create a backend with an API key (for endpoints behind auth).
    pub fn with_api_key(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self> {
        let client = LlamaCppClient::with_api_key(base_url, api_key)
            .map_err(|e| PawanError::Llm(format!("lancor client init failed: {e}")))?;
        Ok(Self {
            client,
            model: model.into(),
            temperature: None,
            max_tokens: None,
        })
    }

    /// Set sampling temperature.
    pub fn temperature(mut self, t: f32) -> Self {
        self.temperature = Some(t);
        self
    }

    /// Set maximum tokens to generate.
    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }

    /// Convert pawan messages to lancor's text-only Message form.
    /// Tool messages are flattened into a `[tool result]`-prefixed user
    /// message so the LLM still sees the tool output.
    fn to_lancor_messages(messages: &[Message]) -> Vec<LancorMessage> {
        messages
            .iter()
            .map(|m| match m.role {
                Role::System => LancorMessage::system(&m.content),
                Role::User => LancorMessage::user(&m.content),
                Role::Assistant => LancorMessage::assistant(&m.content),
                Role::Tool => {
                    let body = m
                        .tool_result
                        .as_ref()
                        .map(|t| format!("[tool result] {}", t.content))
                        .unwrap_or_else(|| m.content.clone());
                    LancorMessage::user(body)
                }
            })
            .collect()
    }
}

#[async_trait]
impl LlmBackend for LancorBackend {
    async fn generate(
        &self,
        messages: &[Message],
        _tools: &[ToolDefinition],
        _on_token: Option<&TokenCallback>,
    ) -> Result<LLMResponse> {
        let mut req = ChatCompletionRequest::new(&self.model)
            .messages(Self::to_lancor_messages(messages));
        if let Some(t) = self.temperature {
            req = req.temperature(t);
        }
        if let Some(n) = self.max_tokens {
            req = req.max_tokens(n);
        }

        let resp = self
            .client
            .chat_completion(req)
            .await
            .map_err(|e| PawanError::Llm(format!("lancor chat_completion: {e}")))?;

        let content = resp
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        let finish_reason = resp
            .choices
            .first()
            .and_then(|c| c.finish_reason.clone())
            .unwrap_or_else(|| "stop".to_string());

        let usage = TokenUsage {
            prompt_tokens: resp.usage.prompt_tokens as u64,
            completion_tokens: resp.usage.completion_tokens.unwrap_or(0) as u64,
            total_tokens: resp.usage.total_tokens as u64,
            reasoning_tokens: 0,
            action_tokens: resp.usage.completion_tokens.unwrap_or(0) as u64,
        };

        Ok(LLMResponse {
            content,
            reasoning: None,
            tool_calls: vec![],
            finish_reason,
            usage: Some(usage),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ToolResultMessage;
    use serde_json::json;

    #[test]
    fn test_lancor_backend_constructor_succeeds() {
        let backend = LancorBackend::new("http://localhost:8080", "qwen3.5").unwrap();
        assert_eq!(backend.model, "qwen3.5");
        assert!(backend.temperature.is_none());
        assert!(backend.max_tokens.is_none());
    }

    #[test]
    fn test_lancor_backend_with_api_key_constructor() {
        let backend =
            LancorBackend::with_api_key("https://example.com", "secret", "model-x").unwrap();
        assert_eq!(backend.model, "model-x");
    }

    #[test]
    fn test_lancor_backend_builder_methods_chain() {
        let backend = LancorBackend::new("http://localhost:8080", "m")
            .unwrap()
            .temperature(0.7)
            .max_tokens(512);
        assert_eq!(backend.temperature, Some(0.7));
        assert_eq!(backend.max_tokens, Some(512));
    }

    #[test]
    fn test_to_lancor_messages_maps_roles_correctly() {
        let messages = vec![
            Message {
                role: Role::System,
                content: "you are an assistant".into(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::User,
                content: "hello".into(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Assistant,
                content: "hi there".into(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        let lm = LancorBackend::to_lancor_messages(&messages);
        assert_eq!(lm.len(), 3);
        assert_eq!(lm[0].role, "system");
        assert_eq!(lm[0].content, "you are an assistant");
        assert_eq!(lm[1].role, "user");
        assert_eq!(lm[1].content, "hello");
        assert_eq!(lm[2].role, "assistant");
        assert_eq!(lm[2].content, "hi there");
    }

    #[test]
    fn test_to_lancor_messages_flattens_tool_role_to_user() {
        let messages = vec![Message {
            role: Role::Tool,
            content: "raw content (ignored)".into(),
            tool_calls: vec![],
            tool_result: Some(ToolResultMessage {
                tool_call_id: "call_1".into(),
                content: json!({"files": ["a.rs", "b.rs"]}),
                success: true,
            }),
        }];
        let lm = LancorBackend::to_lancor_messages(&messages);
        assert_eq!(lm.len(), 1);
        assert_eq!(lm[0].role, "user", "Tool role must flatten to user");
        assert!(lm[0].content.contains("[tool result]"));
        assert!(lm[0].content.contains("a.rs"));
    }

    #[test]
    fn test_to_lancor_messages_empty_input_yields_empty_output() {
        let lm = LancorBackend::to_lancor_messages(&[]);
        assert!(lm.is_empty());
    }

    #[test]
    fn test_to_lancor_messages_tool_role_without_tool_result_falls_back_to_content() {
        // When role=Tool but tool_result is None (e.g. incomplete message),
        // the conversion must fall back to m.content rather than panicking.
        let messages = vec![Message {
            role: Role::Tool,
            content: "fallback text".into(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let lm = LancorBackend::to_lancor_messages(&messages);
        assert_eq!(lm.len(), 1);
        assert_eq!(lm[0].role, "user", "Tool role must still flatten to user");
        assert_eq!(
            lm[0].content, "fallback text",
            "content must fall back to m.content when tool_result is None"
        );
    }

    #[test]
    fn test_temperature_zero_is_stored_not_dropped() {
        // 0.0 is a valid temperature — must be preserved as Some(0.0), not treated as falsy.
        let backend = LancorBackend::new("http://localhost:8080", "m")
            .unwrap()
            .temperature(0.0);
        assert_eq!(
            backend.temperature,
            Some(0.0),
            "temperature(0.0) must set Some(0.0), not None"
        );
    }

    #[test]
    fn test_max_tokens_zero_is_stored_not_dropped() {
        // Callers may set max_tokens=0 as an explicit "no limit" signal.
        // Must be preserved as Some(0), not silently cleared.
        let backend = LancorBackend::new("http://localhost:8080", "m")
            .unwrap()
            .max_tokens(0);
        assert_eq!(
            backend.max_tokens,
            Some(0),
            "max_tokens(0) must set Some(0), not None"
        );
    }

    #[test]
    fn test_to_lancor_messages_preserves_order_across_all_four_roles() {
        let messages = vec![
            Message { role: Role::System, content: "sys".into(), tool_calls: vec![], tool_result: None },
            Message { role: Role::User, content: "usr".into(), tool_calls: vec![], tool_result: None },
            Message { role: Role::Assistant, content: "asst".into(), tool_calls: vec![], tool_result: None },
            Message {
                role: Role::Tool,
                content: "raw".into(),
                tool_calls: vec![],
                tool_result: Some(ToolResultMessage {
                    tool_call_id: "id".into(),
                    content: json!({"k": "v"}),
                    success: true,
                }),
            },
        ];
        let lm = LancorBackend::to_lancor_messages(&messages);
        assert_eq!(lm.len(), 4);
        assert_eq!(lm[0].role, "system");
        assert_eq!(lm[1].role, "user");
        assert_eq!(lm[2].role, "assistant");
        assert_eq!(lm[3].role, "user", "Tool must become user");
        assert!(lm[3].content.contains("[tool result]"));
    }

    #[test]
    fn test_lancor_backend_new_with_invalid_url_returns_error() {
        // Invalid URL should cause LlamaCppClient::new to fail
        let result = LancorBackend::new("not-a-valid-url", "model");
        assert!(result.is_err(), "Invalid URL should return error");
        match result {
            Err(PawanError::Llm(msg)) => {
                assert!(msg.contains("lancor client init failed"), "Error message should mention init failure");
            }
            _ => panic!("Expected PawanError::Llm variant"),
        }
    }

    #[test]
    fn test_lancor_backend_with_api_key_with_invalid_url_returns_error() {
        // Invalid URL should cause LlamaCppClient::with_api_key to fail
        let result = LancorBackend::with_api_key("not-a-valid-url", "key", "model");
        assert!(result.is_err(), "Invalid URL should return error");
        match result {
            Err(PawanError::Llm(msg)) => {
                assert!(msg.contains("lancor client init failed"), "Error message should mention init failure");
            }
            _ => panic!("Expected PawanError::Llm variant"),
        }
    }

    #[test]
    fn test_lancor_backend_new_with_empty_url_returns_error() {
        // Empty URL should cause failure
        let result = LancorBackend::new("", "model");
        assert!(result.is_err(), "Empty URL should return error");
    }

    #[test]
    fn test_lancor_backend_with_api_key_with_empty_url_returns_error() {
        // Empty URL should cause failure even with valid API key
        let result = LancorBackend::with_api_key("", "key", "model");
        assert!(result.is_err(), "Empty URL should return error");
    }

    #[test]
    fn test_lancor_backend_new_with_empty_model_still_succeeds() {
        // Empty model string is technically valid (up to caller to validate)
        let backend = LancorBackend::new("http://localhost:8080", "").unwrap();
        assert_eq!(backend.model, "");
    }

    #[test]
    fn test_lancor_backend_with_api_key_with_empty_model_still_succeeds() {
        // Empty model string is technically valid (up to caller to validate)
        let backend = LancorBackend::with_api_key("http://localhost:8080", "key", "").unwrap();
        assert_eq!(backend.model, "");
    }
}
