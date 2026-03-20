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

    /// Convert pawan ToolDefinitions to ares ToolDefinitions
    fn to_ares_tools(&self, tools: &[ToolDefinition]) -> Vec<ares::types::ToolDefinition> {
        tools
            .iter()
            .map(|t| ares::types::ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
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
