//! Ollama LLM backend (local inference)

use super::LlmBackend;
use crate::agent::{LLMResponse, Message, Role, TokenCallback, ToolCallRequest, ToolDefinition};
use crate::{PawanError, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Backend for local Ollama instances
pub struct OllamaBackend {
    http_client: reqwest::Client,
    api_url: String,
    model: String,
    temperature: f32,
    system_prompt: String,
}

impl OllamaBackend {
    pub fn new(api_url: String, model: String, temperature: f32, system_prompt: String) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            api_url,
            model,
            temperature,
            system_prompt,
        }
    }

    fn build_messages(&self, messages: &[Message]) -> Vec<Value> {
        let mut out = vec![json!({
            "role": "system",
            "content": self.system_prompt
        })];

        for msg in messages {
            match msg.role {
                Role::System => {
                    out.push(json!({ "role": "system", "content": msg.content }));
                }
                Role::User => {
                    out.push(json!({ "role": "user", "content": msg.content }));
                }
                Role::Assistant => {
                    if msg.tool_calls.is_empty() {
                        out.push(json!({ "role": "assistant", "content": msg.content }));
                    } else {
                        let tool_calls: Vec<Value> = msg
                            .tool_calls
                            .iter()
                            .map(|tc| {
                                json!({
                                    "function": {
                                        "name": tc.name,
                                        "arguments": tc.arguments
                                    }
                                })
                            })
                            .collect();
                        out.push(json!({
                            "role": "assistant",
                            "content": msg.content,
                            "tool_calls": tool_calls
                        }));
                    }
                }
                Role::Tool => {
                    if let Some(ref tool_result) = msg.tool_result {
                        out.push(json!({
                            "role": "tool",
                            "content": serde_json::to_string(&tool_result.content)
                                .unwrap_or_else(|_| tool_result.content.to_string())
                        }));
                    }
                }
            }
        }

        out
    }

    fn build_tools(&self, tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }
                })
            })
            .collect()
    }

    async fn non_streaming(&self, url: &str, body: Value) -> Result<LLMResponse> {
        let response = self
            .http_client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| PawanError::Llm(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(PawanError::Llm(format!(
                "Ollama request failed ({}): {}",
                status, text
            )));
        }

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| PawanError::Llm(format!("Failed to parse response: {}", e)))?;

        self.parse_response(&response_json)
    }

    async fn streaming(
        &self,
        url: &str,
        body: Value,
        on_token: Option<&TokenCallback>,
    ) -> Result<LLMResponse> {
        let response = self
            .http_client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| PawanError::Llm(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(PawanError::Llm(format!(
                "Ollama request failed ({}): {}",
                status, text
            )));
        }

        let mut content = String::new();
        let mut tool_calls = Vec::new();
        let mut finish_reason = "stop".to_string();

        let mut stream = response.bytes_stream();
        use futures::StreamExt;

        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| PawanError::Llm(format!("Stream error: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].to_string();
                buffer = buffer[newline_pos + 1..].to_string();

                if line.trim().is_empty() {
                    continue;
                }

                if let Ok(json) = serde_json::from_str::<Value>(&line) {
                    if let Some(msg) = json.get("message") {
                        if let Some(c) = msg.get("content").and_then(|v| v.as_str()) {
                            if let Some(callback) = on_token {
                                callback(c);
                            }
                            content.push_str(c);
                        }

                        if let Some(tc_array) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                            for tc in tc_array {
                                if let Some(func) = tc.get("function") {
                                    let name = func
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let arguments =
                                        func.get("arguments").cloned().unwrap_or(json!({}));

                                    tool_calls.push(ToolCallRequest {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        name,
                                        arguments,
                                    });
                                }
                            }
                            finish_reason = "tool_calls".to_string();
                        }
                    }

                    if json.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                        break;
                    }
                }
            }
        }

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage: None,
        })
    }

    fn parse_response(&self, json: &Value) -> Result<LLMResponse> {
        let message = json
            .get("message")
            .ok_or_else(|| PawanError::Llm("No message in response".into()))?;

        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut tool_calls = Vec::new();
        let mut finish_reason = "stop".to_string();

        if let Some(tc_array) = message.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tc_array {
                if let Some(func) = tc.get("function") {
                    let name = func
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = func.get("arguments").cloned().unwrap_or(json!({}));

                    tool_calls.push(ToolCallRequest {
                        id: uuid::Uuid::new_v4().to_string(),
                        name,
                        arguments,
                    });
                }
            }
            finish_reason = "tool_calls".to_string();
        }

        if let Some(reason) = json.get("done_reason").and_then(|v| v.as_str()) {
            finish_reason = reason.to_string();
        }

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage: None,
        })
    }
}

#[async_trait]
impl LlmBackend for OllamaBackend {
    async fn generate(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        on_token: Option<&TokenCallback>,
    ) -> Result<LLMResponse> {
        let api_messages = self.build_messages(messages);
        let api_tools = self.build_tools(tools);

        let request_body = json!({
            "model": self.model,
            "messages": api_messages,
            "tools": api_tools,
            "stream": on_token.is_some(),
            "options": {
                "temperature": self.temperature
            }
        });

        let url = format!("{}/api/chat", self.api_url);

        let prompt_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            model = self.model.as_str(),
            url = url.as_str(),
            provider = "ollama",
            prompt_len,
            tools = api_tools.len(),
            streaming = on_token.is_some(),
            "llm call"
        );

        let t0 = std::time::Instant::now();
        let result = if on_token.is_some() {
            self.streaming(&url, request_body, on_token).await
        } else {
            self.non_streaming(&url, request_body).await
        };
        let latency_ms = t0.elapsed().as_millis() as u64;

        match result {
            Ok(ref response) => {
                tracing::info!(
                    model = self.model.as_str(),
                    provider = "ollama",
                    latency_ms,
                    prompt_tokens = response.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                    completion_tokens = response.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                    finish_reason = response.finish_reason.as_str(),
                    response_len = response.content.len(),
                    "llm ok"
                );
            }
            Err(ref e) => {
                tracing::warn!(
                    model = self.model.as_str(),
                    provider = "ollama",
                    latency_ms,
                    error = %e,
                    "llm error"
                );
            }
        }
        result
    }
}
