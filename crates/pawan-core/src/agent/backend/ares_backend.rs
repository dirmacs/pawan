//! Ares LLM backend — uses ares-server's LLMClient for generation
//!
//! Feature-gated behind `ares` feature flag.

use super::LlmBackend;
use crate::agent::{
    LLMResponse, Message, Role, TokenCallback, TokenUsage, ToolCallRequest, ToolDefinition,
};
use crate::{PawanError, Result};
use async_trait::async_trait;

/// Backend that wraps an ares `LLMClient` for LLM generation
pub struct AresBackend {
    client: Box<dyn ares::llm::LLMClient>,
    system_prompt: String,
}

impl AresBackend {
    /// Create from an existing ares LLMClient
    pub fn new(client: Box<dyn ares::llm::LLMClient>, system_prompt: String) -> Self {
        Self {
            client,
            system_prompt,
        }
    }

    /// Create from ares Provider configuration
    pub async fn from_provider(
        provider: ares::llm::Provider,
        system_prompt: String,
    ) -> Result<Self> {
        let client = provider
            .create_client()
            .await
            .map_err(|e| PawanError::Llm(format!("Failed to create ares LLM client: {}", e)))?;
        Ok(Self::new(client, system_prompt))
    }

    /// Convert pawan messages to ares ConversationMessages
    fn to_ares_messages(
        &self,
        messages: &[Message],
    ) -> Vec<ares::llm::coordinator::ConversationMessage> {
        let mut out = vec![ares::llm::coordinator::ConversationMessage {
            role: ares::llm::coordinator::MessageRole::System,
            content: self.system_prompt.clone(),
            tool_calls: vec![],
            tool_call_id: None,
        }];

        for msg in messages {
            let role = match msg.role {
                Role::System => ares::llm::coordinator::MessageRole::System,
                Role::User => ares::llm::coordinator::MessageRole::User,
                Role::Assistant => ares::llm::coordinator::MessageRole::Assistant,
                Role::Tool => ares::llm::coordinator::MessageRole::Tool,
            };

            let tool_calls: Vec<ares::types::ToolCall> = msg
                .tool_calls
                .iter()
                .map(|tc| ares::types::ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .collect();

            let tool_call_id = msg.tool_result.as_ref().map(|tr| tr.tool_call_id.clone());

            out.push(ares::llm::coordinator::ConversationMessage {
                role,
                content: msg.content.clone(),
                tool_calls,
                tool_call_id,
            });
        }

        out
    }

    /// Convert pawan ToolDefinitions (typed) to ares ToolDefinitions (JSON-schema form)
    fn to_ares_tools(&self, tools: &[ToolDefinition]) -> Vec<ares::types::ToolDefinition> {
        tools
            .iter()
            .map(|t| ares::types::ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.to_mcp_input_schema(),
            })
            .collect()
    }
    /// Convert ares LLMResponse to pawan LLMResponse
    fn from_ares_response(&self, resp: ares::llm::LLMResponse) -> LLMResponse {
        let tool_calls: Vec<ToolCallRequest> = resp
            .tool_calls
            .iter()
            .map(|tc| ToolCallRequest {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            })
            .collect();

        let usage = resp.usage.as_ref().map(|u| TokenUsage {
            prompt_tokens: u.prompt_tokens as u64,
            completion_tokens: u.completion_tokens as u64,
            total_tokens: u.total_tokens as u64,
            reasoning_tokens: 0,
            action_tokens: u.completion_tokens as u64,
        });

        LLMResponse {
            content: resp.content,
            reasoning: None,
            tool_calls,
            finish_reason: resp.finish_reason,
            usage,
        }
    }
}

#[async_trait]
impl LlmBackend for AresBackend {
    async fn generate(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        _on_token: Option<&TokenCallback>,
    ) -> Result<LLMResponse> {
        let ares_messages = self.to_ares_messages(messages);
        let ares_tools = self.to_ares_tools(tools);

        let response = self
            .client
            .generate_with_tools_and_history(&ares_messages, &ares_tools)
            .await
            .map_err(|e| PawanError::Llm(format!("Ares LLM generation failed: {}", e)))?;

        Ok(self.from_ares_response(response))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ToolResultMessage;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex;

    // -----------------------------------------------------------------------
    // Mock implementations for ares types
    // -----------------------------------------------------------------------

    #[derive(Clone, Debug)]
    struct MockLLMResponse {
        content: String,
        tool_calls: Vec<ares::types::ToolCall>,
        finish_reason: String,
        usage: Option<ares::llm::client::TokenUsage>,
    }

    #[derive(Clone)]
    struct MockLLMClient {
        response: Arc<Mutex<Option<MockLLMResponse>>>,
    }

    impl MockLLMClient {
        fn new() -> Self {
            Self {
                response: Arc::new(Mutex::new(None)),
            }
        }

        fn set_response(&self, response: MockLLMResponse) {
            *self.response.lock().unwrap() = Some(response);
        }
    }

    #[async_trait]
    impl ares::llm::LLMClient for MockLLMClient {
        async fn generate(&self, _prompt: &str) -> ares::types::Result<String> {
            Ok("default".to_string())
        }

        async fn generate_with_system(
            &self,
            _system: &str,
            _prompt: &str,
        ) -> ares::types::Result<String> {
            Ok("default".to_string())
        }

        async fn generate_with_history(
            &self,
            _messages: &[(String, String)],
        ) -> ares::types::Result<ares::llm::LLMResponse> {
            Ok(ares::llm::LLMResponse {
                content: "default".to_string(),
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                usage: None,
            })
        }

        async fn generate_with_tools(
            &self,
            _prompt: &str,
            _tools: &[ares::types::ToolDefinition],
        ) -> ares::types::Result<ares::llm::LLMResponse> {
            Ok(ares::llm::LLMResponse {
                content: "default".to_string(),
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                usage: None,
            })
        }

        async fn generate_with_tools_and_history(
            &self,
            _messages: &[ares::llm::coordinator::ConversationMessage],
            _tools: &[ares::types::ToolDefinition],
        ) -> ares::types::Result<ares::llm::LLMResponse> {
            let resp = self.response.lock().unwrap().clone();
            match resp {
                Some(r) => Ok(ares::llm::LLMResponse {
                    content: r.content,
                    tool_calls: r.tool_calls,
                    finish_reason: r.finish_reason,
                    usage: r.usage,
                }),
                None => Ok(ares::llm::LLMResponse {
                    content: "default response".to_string(),
                    tool_calls: vec![],
                    finish_reason: "stop".to_string(),
                    usage: None,
                }),
            }
        }

        async fn stream(
            &self,
            _prompt: &str,
        ) -> ares::types::Result<
            Box<dyn futures::Stream<Item = ares::types::Result<String>> + Send + Unpin>,
        > {
            Ok(Box::new(futures::stream::empty()))
        }

        async fn stream_with_system(
            &self,
            _system: &str,
            _prompt: &str,
        ) -> ares::types::Result<
            Box<dyn futures::Stream<Item = ares::types::Result<String>> + Send + Unpin>,
        > {
            Ok(Box::new(futures::stream::empty()))
        }

        async fn stream_with_history(
            &self,
            _messages: &[(String, String)],
        ) -> ares::types::Result<
            Box<dyn futures::Stream<Item = ares::types::Result<String>> + Send + Unpin>,
        > {
            Ok(Box::new(futures::stream::empty()))
        }

        fn model_name(&self) -> &str {
            "mock"
        }
    }

    // -----------------------------------------------------------------------
    // Constructor tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_new() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "test system prompt".to_string());
        assert_eq!(backend.system_prompt, "test system prompt");
    }

    #[test]
    fn test_new_empty_system_prompt() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, String::new());
        assert_eq!(backend.system_prompt, "");
    }

    // -----------------------------------------------------------------------
    // to_ares_messages tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_to_ares_messages_empty() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let messages: Vec<Message> = vec![];
        let result = backend.to_ares_messages(&messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "system");
        assert!(matches!(
            result[0].role,
            ares::llm::coordinator::MessageRole::System
        ));
    }

    #[test]
    fn test_to_ares_messages_with_system_prompt() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "You are a helpful assistant".to_string());
        let messages: Vec<Message> = vec![];
        let result = backend.to_ares_messages(&messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "You are a helpful assistant");
    }

    #[test]
    fn test_to_ares_messages_all_roles() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let messages = vec![
            Message {
                role: Role::System,
                content: "system message".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::User,
                content: "user message".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Assistant,
                content: "assistant message".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Tool,
                content: "tool result".to_string(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        let result = backend.to_ares_messages(&messages);
        assert_eq!(result.len(), 5); // 1 system prompt + 4 messages
        assert!(matches!(
            result[0].role,
            ares::llm::coordinator::MessageRole::System
        ));
        assert!(matches!(
            result[1].role,
            ares::llm::coordinator::MessageRole::System
        ));
        assert!(matches!(
            result[2].role,
            ares::llm::coordinator::MessageRole::User
        ));
        assert!(matches!(
            result[3].role,
            ares::llm::coordinator::MessageRole::Assistant
        ));
        assert!(matches!(
            result[4].role,
            ares::llm::coordinator::MessageRole::Tool
        ));
    }

    #[test]
    fn test_to_ares_messages_with_tool_calls() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let messages = vec![Message {
            role: Role::Assistant,
            content: "I'll use a tool".to_string(),
            tool_calls: vec![ToolCallRequest {
                id: "call_123".to_string(),
                name: "read_file".to_string(),
                arguments: json!({"path": "test.rs"}),
            }],
            tool_result: None,
        }];
        let result = backend.to_ares_messages(&messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].tool_calls.len(), 1);
        assert_eq!(result[1].tool_calls[0].id, "call_123");
        assert_eq!(result[1].tool_calls[0].name, "read_file");
    }

    #[test]
    fn test_to_ares_messages_with_tool_call_id() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let messages = vec![Message {
            role: Role::Tool,
            content: "tool output".to_string(),
            tool_calls: vec![],
            tool_result: Some(ToolResultMessage {
                tool_call_id: "call_123".to_string(),
                content: json!({"result": "ok"}),
                success: true,
            }),
        }];
        let result = backend.to_ares_messages(&messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].tool_call_id, Some("call_123".to_string()));
    }

    #[test]
    fn test_to_ares_messages_empty_tool_calls() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let messages = vec![Message {
            role: Role::Assistant,
            content: "no tools".to_string(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let result = backend.to_ares_messages(&messages);
        assert_eq!(result.len(), 2);
        assert!(result[1].tool_calls.is_empty());
    }

    // -----------------------------------------------------------------------
    // to_ares_tools tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_to_ares_tools_empty() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let tools: Vec<ToolDefinition> = vec![];
        let result = backend.to_ares_tools(&tools);
        assert!(result.is_empty());
    }

    #[test]
    fn test_to_ares_tools_single() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let schema = json!({"type": "object", "properties": {"path": {"type": "string"}}});
        let tools = vec![ToolDefinition {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: thulp_core::ToolDefinition::parse_mcp_input_schema(&schema).unwrap(),
        }];
        let result = backend.to_ares_tools(&tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "test_tool");
        assert_eq!(result[0].description, "A test tool");
    }

    #[test]
    fn test_to_ares_tools_multiple() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let schema1 = json!({"type": "object"});
        let schema2 = json!({"type": "object"});
        let tools = vec![
            ToolDefinition {
                name: "tool1".to_string(),
                description: "First tool".to_string(),
                parameters: thulp_core::ToolDefinition::parse_mcp_input_schema(&schema1).unwrap(),
            },
            ToolDefinition {
                name: "tool2".to_string(),
                description: "Second tool".to_string(),
                parameters: thulp_core::ToolDefinition::parse_mcp_input_schema(&schema2).unwrap(),
            },
        ];
        let result = backend.to_ares_tools(&tools);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "tool1");
        assert_eq!(result[1].name, "tool2");
    }

    // -----------------------------------------------------------------------
    // from_ares_response tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_from_ares_response_content() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let ares_resp = ares::llm::LLMResponse {
            content: "test response".to_string(),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
            usage: None,
        };
        let result = backend.from_ares_response(ares_resp);
        assert_eq!(result.content, "test response");
        assert_eq!(result.finish_reason, "stop");
        assert!(result.tool_calls.is_empty());
        assert!(result.usage.is_none());
    }

    #[test]
    fn test_from_ares_response_with_tool_calls() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let ares_resp = ares::llm::LLMResponse {
            content: String::new(),
            tool_calls: vec![ares::types::ToolCall {
                id: "call_1".to_string(),
                name: "test_tool".to_string(),
                arguments: json!({"arg": "value"}),
            }],
            finish_reason: "tool_calls".to_string(),
            usage: None,
        };
        let result = backend.from_ares_response(ares_resp);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_1");
        assert_eq!(result.tool_calls[0].name, "test_tool");
        assert_eq!(result.finish_reason, "tool_calls");
    }

    #[test]
    fn test_from_ares_response_with_usage() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let ares_resp = ares::llm::LLMResponse {
            content: "response".to_string(),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
            usage: Some(ares::llm::client::TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            }),
        };
        let result = backend.from_ares_response(ares_resp);
        assert!(result.usage.is_some());
        let usage = result.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total_tokens, 30);
        assert_eq!(usage.reasoning_tokens, 0);
        assert_eq!(usage.action_tokens, 20);
    }

    #[test]
    fn test_from_ares_response_no_usage() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let ares_resp = ares::llm::LLMResponse {
            content: "response".to_string(),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
            usage: None,
        };
        let result = backend.from_ares_response(ares_resp);
        assert!(result.usage.is_none());
    }

    #[test]
    fn test_from_ares_response_empty_tool_call_args() {
        let client = Box::new(MockLLMClient::new()) as Box<dyn ares::llm::LLMClient>;
        let backend = AresBackend::new(client, "system".to_string());
        let ares_resp = ares::llm::LLMResponse {
            content: String::new(),
            tool_calls: vec![ares::types::ToolCall {
                id: "call_1".to_string(),
                name: "test_tool".to_string(),
                arguments: json!({}),
            }],
            finish_reason: "tool_calls".to_string(),
            usage: None,
        };
        let result = backend.from_ares_response(ares_resp);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].arguments, json!({}));
    }

    // -----------------------------------------------------------------------
    // LlmBackend::generate tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_generate_text_response() {
        let mock_client = MockLLMClient::new();
        mock_client.set_response(MockLLMResponse {
            content: "Hello!".to_string(),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
            usage: None,
        });
        let backend = AresBackend::new(
            Box::new(mock_client) as Box<dyn ares::llm::LLMClient>,
            "system".to_string(),
        );
        let messages = vec![Message {
            role: Role::User,
            content: "Hello".to_string(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let result = backend.generate(&messages, &[], None).await.unwrap();
        assert_eq!(result.content, "Hello!");
        assert_eq!(result.finish_reason, "stop");
        assert!(result.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_generate_with_tool_calls() {
        let mock_client = MockLLMClient::new();
        mock_client.set_response(MockLLMResponse {
            content: String::new(),
            tool_calls: vec![ares::types::ToolCall {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: json!({"path": "test.rs"}),
            }],
            finish_reason: "tool_calls".to_string(),
            usage: None,
        });
        let backend = AresBackend::new(
            Box::new(mock_client) as Box<dyn ares::llm::LLMClient>,
            "system".to_string(),
        );
        let messages = vec![Message {
            role: Role::User,
            content: "Read test.rs".to_string(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let result = backend.generate(&messages, &[], None).await.unwrap();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "read_file");
        assert_eq!(result.finish_reason, "tool_calls");
    }

    #[tokio::test]
    async fn test_generate_with_usage() {
        let mock_client = MockLLMClient::new();
        mock_client.set_response(MockLLMResponse {
            content: "Response with usage".to_string(),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
            usage: Some(ares::llm::client::TokenUsage {
                prompt_tokens: 15,
                completion_tokens: 25,
                total_tokens: 40,
            }),
        });
        let backend = AresBackend::new(
            Box::new(mock_client) as Box<dyn ares::llm::LLMClient>,
            "system".to_string(),
        );
        let messages = vec![Message {
            role: Role::User,
            content: "Test".to_string(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let result = backend.generate(&messages, &[], None).await.unwrap();
        assert!(result.usage.is_some());
        let usage = result.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 15);
        assert_eq!(usage.completion_tokens, 25);
        assert_eq!(usage.total_tokens, 40);
    }

    #[tokio::test]
    async fn test_generate_empty_messages() {
        let mock_client = MockLLMClient::new();
        mock_client.set_response(MockLLMResponse {
            content: "Response".to_string(),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
            usage: None,
        });
        let backend = AresBackend::new(
            Box::new(mock_client) as Box<dyn ares::llm::LLMClient>,
            "system".to_string(),
        );
        let messages: Vec<Message> = vec![];
        let result = backend.generate(&messages, &[], None).await.unwrap();
        assert_eq!(result.content, "Response");
    }

    #[tokio::test]
    async fn test_generate_with_tools() {
        let mock_client = MockLLMClient::new();
        mock_client.set_response(MockLLMResponse {
            content: "I'll use a tool".to_string(),
            tool_calls: vec![ares::types::ToolCall {
                id: "call_1".to_string(),
                name: "bash".to_string(),
                arguments: json!({"command": "ls"}),
            }],
            finish_reason: "tool_calls".to_string(),
            usage: None,
        });
        let backend = AresBackend::new(
            Box::new(mock_client) as Box<dyn ares::llm::LLMClient>,
            "system".to_string(),
        );
        let schema = json!({"type": "object", "properties": {"command": {"type": "string"}}});
        let messages = vec![Message {
            role: Role::User,
            content: "List files".to_string(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let tools = vec![ToolDefinition {
            name: "bash".to_string(),
            description: "Execute bash command".to_string(),
            parameters: thulp_core::ToolDefinition::parse_mcp_input_schema(&schema).unwrap(),
        }];
        let result = backend.generate(&messages, &tools, None).await.unwrap();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "bash");
    }
}
