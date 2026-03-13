//! OpenAI-compatible LLM backend (NVIDIA NIM, OpenAI, DeepSeek, etc.)

use super::LlmBackend;
use crate::agent::{LLMResponse, Message, Role, TokenCallback, ToolCallRequest, ToolDefinition};
use crate::{PawanError, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Configuration for OpenAI-compatible backend
pub struct OpenAiCompatConfig {
    pub api_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: usize,
    pub system_prompt: String,
    pub use_thinking: bool,
}

/// Backend for OpenAI-compatible APIs (NVIDIA NIM, OpenAI, DeepSeek)
pub struct OpenAiCompatBackend {
    http_client: reqwest::Client,
    cfg: OpenAiCompatConfig,
}

impl OpenAiCompatBackend {
    pub fn new(cfg: OpenAiCompatConfig) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            cfg,
        }
    }

    fn build_messages(&self, messages: &[Message]) -> Vec<Value> {
        let mut out = vec![json!({
            "role": "system",
            "content": self.cfg.system_prompt
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
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": serde_json::to_string(&tc.arguments).unwrap_or_default()
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
                            "tool_call_id": tool_result.tool_call_id,
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

    async fn non_streaming(&self, request: reqwest::RequestBuilder) -> Result<LLMResponse> {
        let response = request
            .send()
            .await
            .map_err(|e| PawanError::Llm(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(PawanError::Llm(format!(
                "API request failed ({}): {}",
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
        request: reqwest::RequestBuilder,
        on_token: Option<&TokenCallback>,
    ) -> Result<LLMResponse> {
        let response = request
            .send()
            .await
            .map_err(|e| PawanError::Llm(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(PawanError::Llm(format!(
                "API request failed ({}): {}",
                status, text
            )));
        }

        let mut content = String::new();
        let mut tool_calls: Vec<ToolCallRequest> = Vec::new();
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

                let line = line.trim();
                if line.is_empty() || line == "data: [DONE]" {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(json) = serde_json::from_str::<Value>(data) {
                        if let Some(choices) = json.get("choices").and_then(|v| v.as_array()) {
                            for choice in choices {
                                if let Some(delta) = choice.get("delta") {
                                    if let Some(c) = delta.get("content").and_then(|v| v.as_str()) {
                                        if let Some(callback) = on_token {
                                            callback(c);
                                        }
                                        content.push_str(c);
                                    }

                                    if let Some(tc_array) =
                                        delta.get("tool_calls").and_then(|v| v.as_array())
                                    {
                                        for tc in tc_array {
                                            let index = tc
                                                .get("index")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0)
                                                as usize;

                                            while tool_calls.len() <= index {
                                                tool_calls.push(ToolCallRequest {
                                                    id: String::new(),
                                                    name: String::new(),
                                                    arguments: json!({}),
                                                });
                                            }

                                            if let Some(id) = tc.get("id").and_then(|v| v.as_str())
                                            {
                                                tool_calls[index].id = id.to_string();
                                            }
                                            if let Some(func) = tc.get("function") {
                                                if let Some(name) =
                                                    func.get("name").and_then(|v| v.as_str())
                                                {
                                                    tool_calls[index].name = name.to_string();
                                                }
                                                if let Some(args) =
                                                    func.get("arguments").and_then(|v| v.as_str())
                                                {
                                                    let current = tool_calls[index]
                                                        .arguments
                                                        .as_str()
                                                        .unwrap_or("");
                                                    tool_calls[index].arguments =
                                                        json!(format!("{}{}", current, args));
                                                }
                                            }
                                        }
                                    }
                                }

                                if let Some(reason) =
                                    choice.get("finish_reason").and_then(|v| v.as_str())
                                {
                                    finish_reason = reason.to_string();
                                }
                            }
                        }
                    }
                }
            }
        }

        // Parse tool call arguments from JSON strings
        for tc in &mut tool_calls {
            if let Some(args_str) = tc.arguments.as_str() {
                if let Ok(parsed) = serde_json::from_str::<Value>(args_str) {
                    tc.arguments = parsed;
                }
            }
            if tc.id.is_empty() {
                tc.id = uuid::Uuid::new_v4().to_string();
            }
        }

        tool_calls.retain(|tc| !tc.name.is_empty());

        if !tool_calls.is_empty() {
            finish_reason = "tool_calls".to_string();
        }

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
        })
    }

    fn parse_response(&self, json: &Value) -> Result<LLMResponse> {
        let choices = json
            .get("choices")
            .and_then(|v| v.as_array())
            .ok_or_else(|| PawanError::Llm("No choices in response".into()))?;

        let choice = choices
            .first()
            .ok_or_else(|| PawanError::Llm("Empty choices array".into()))?;

        let message = choice
            .get("message")
            .ok_or_else(|| PawanError::Llm("No message in choice".into()))?;

        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut tool_calls = Vec::new();
        let finish_reason = choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("stop")
            .to_string();

        if let Some(tc_array) = message.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tc_array {
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(func) = tc.get("function") {
                    let name = func
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let arguments =
                        if let Some(args_str) = func.get("arguments").and_then(|v| v.as_str()) {
                            serde_json::from_str(args_str).unwrap_or(json!({}))
                        } else {
                            func.get("arguments").cloned().unwrap_or(json!({}))
                        };

                    tool_calls.push(ToolCallRequest {
                        id: if id.is_empty() {
                            uuid::Uuid::new_v4().to_string()
                        } else {
                            id
                        },
                        name,
                        arguments,
                    });
                }
            }
        }

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
        })
    }
}

#[async_trait]
impl LlmBackend for OpenAiCompatBackend {
    async fn generate(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        on_token: Option<&TokenCallback>,
    ) -> Result<LLMResponse> {
        let api_messages = self.build_messages(messages);
        let api_tools = self.build_tools(tools);

        let mut request_body = json!({
            "model": self.cfg.model,
            "messages": api_messages,
            "tools": api_tools,
            "temperature": self.cfg.temperature,
            "top_p": self.cfg.top_p,
            "max_tokens": self.cfg.max_tokens,
            "stream": on_token.is_some()
        });

        if self.cfg.use_thinking {
            request_body["chat_template_kwargs"] = json!({ "thinking": true });
        }

        request_body["seed"] = json!(42);

        let url = format!("{}/chat/completions", self.cfg.api_url);
        let mut request = self.http_client.post(&url).json(&request_body);

        if let Some(ref api_key) = self.cfg.api_key {
            request = request.header("Authorization", format!("Bearer {}", api_key));
        }

        if on_token.is_some() {
            self.streaming(request, on_token).await
        } else {
            self.non_streaming(request).await
        }
    }
}
