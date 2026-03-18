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
