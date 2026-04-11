//! LLM Backend trait and implementations
//!
//! Abstracts over different LLM providers (NVIDIA NIM, Ollama, OpenAI).

pub mod mock;
pub mod ollama;
pub mod openai_compat;

#[cfg(feature = "ares")]
pub mod ares_backend;

#[cfg(feature = "lancor")]
pub mod lancor;

use crate::agent::{LLMResponse, Message, TokenCallback, ToolDefinition};
use crate::Result;
use async_trait::async_trait;

/// Trait for LLM backends that can generate responses
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Generate a response given messages and available tools
    async fn generate(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        on_token: Option<&TokenCallback>,
    ) -> Result<LLMResponse>;
}
