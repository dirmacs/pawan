use super::{Message, Role, ToolDefinition};

impl super::PawanAgent {
    /// Pre-flight health check: verify the LLM backend is reachable before starting work.
    /// Sends a minimal "ping" message. Returns Ok(()) if the model responds.
    pub async fn preflight_check(&self) -> crate::Result<()> {
        let test = vec![Message {
            role: Role::User,
            content: "ping".into(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let tools: Vec<ToolDefinition> = vec![];
        match self.backend.generate(&test, &tools, None).await {
            Ok(_) => Ok(()),
            Err(e) => Err(crate::PawanError::Llm(format!("Model unreachable: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::backend::mock::MockBackend;
    use crate::agent::backend::LlmBackend;
    use crate::agent::{LLMResponse, PawanAgent, TokenCallback};
    use crate::config::PawanConfig;
    use async_trait::async_trait;
    use std::path::PathBuf;

    struct AlwaysFailBackend;

    #[async_trait]
    impl LlmBackend for AlwaysFailBackend {
        async fn generate(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            _on_token: Option<&TokenCallback>,
        ) -> crate::Result<LLMResponse> {
            Err(crate::PawanError::Llm("connection refused".into()))
        }
    }

    fn agent_with_backend(backend: Box<dyn LlmBackend>) -> PawanAgent {
        PawanAgent::new(PawanConfig::default(), PathBuf::from("."))
            .with_backend(backend)
    }

    #[tokio::test]
    async fn preflight_check_ok_when_backend_responds() {
        let agent = agent_with_backend(Box::new(MockBackend::with_text("pong")));
        assert!(agent.preflight_check().await.is_ok());
    }

    #[tokio::test]
    async fn preflight_check_ok_when_backend_returns_empty_text() {
        let agent = agent_with_backend(Box::new(MockBackend::with_text("")));
        assert!(agent.preflight_check().await.is_ok());
    }

    #[tokio::test]
    async fn preflight_check_errors_when_backend_fails() {
        let agent = agent_with_backend(Box::new(AlwaysFailBackend));
        let err = agent
            .preflight_check()
            .await
            .expect_err("failing backend must bubble out");
        assert!(
            matches!(err, crate::PawanError::Llm(_)),
            "expected Llm error variant, got {err:?}"
        );
    }

    #[tokio::test]
    async fn preflight_check_error_message_mentions_unreachable() {
        let agent = agent_with_backend(Box::new(AlwaysFailBackend));
        let err = agent.preflight_check().await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Model unreachable"),
            "error should be wrapped with 'Model unreachable', got: {msg}"
        );
    }

    #[tokio::test]
    async fn preflight_check_does_not_mutate_agent_history() {
        let agent = agent_with_backend(Box::new(MockBackend::with_text("pong")));
        let before = agent.history().len();
        agent.preflight_check().await.unwrap();
        assert_eq!(
            agent.history().len(),
            before,
            "preflight must not persist the ping message into agent history"
        );
    }
}
