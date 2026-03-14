//! OpenAI-compatible LLM backend (NVIDIA NIM, OpenAI, DeepSeek, etc.)

use super::LlmBackend;
use crate::agent::{
    LLMResponse, Message, Role, TokenCallback, TokenUsage, ToolCallRequest, ToolDefinition,
};
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
            return Err(PawanError::Llm(Self::format_api_error(status, &text)));
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
            return Err(PawanError::Llm(Self::format_api_error(status, &text)));
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

        // Streaming responses don't include usage in individual chunks
        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage: None,
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

        // Parse usage from response
        let usage = Self::parse_usage(json);

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage,
        })
    }

    /// Parse API error response body for a user-friendly message
    fn format_api_error(status: reqwest::StatusCode, body: &str) -> String {
        // Try to extract message from JSON error body
        let detail = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|json| {
                // Common patterns: { "error": { "message": "..." } } or { "detail": "..." } or { "message": "..." }
                json.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| json.get("detail").and_then(|v| v.as_str()).map(String::from))
                    .or_else(|| json.get("message").and_then(|v| v.as_str()).map(String::from))
            });

        let hint = match status.as_u16() {
            401 => " (check your API key)",
            403 => " (forbidden — check API key permissions)",
            404 => " (model not found or endpoint incorrect)",
            429 => " (rate limited — try again shortly)",
            500..=599 => " (server error — retry later)",
            _ => "",
        };

        match detail {
            Some(msg) => format!("API error {}{}: {}", status, hint, msg),
            None if body.is_empty() => format!("API error {}{}", status, hint),
            None => format!("API error {}{}: {}", status, hint, body),
        }
    }

    fn parse_usage(json: &Value) -> Option<TokenUsage> {
        let u = json.get("usage")?;
        Some(TokenUsage {
            prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            completion_tokens: u
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
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

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn test_format_api_error_401_with_json_body() {
        let body = r#"{"error":{"message":"Invalid API key"}}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::UNAUTHORIZED, body);
        assert!(result.contains("Invalid API key"));
        assert!(result.contains("check your API key"));
        assert!(result.contains("401"));
    }

    #[test]
    fn test_format_api_error_429_rate_limited() {
        let body = r#"{"error":{"message":"Rate limit exceeded"}}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::TOO_MANY_REQUESTS, body);
        assert!(result.contains("rate limited"));
        assert!(result.contains("Rate limit exceeded"));
    }

    #[test]
    fn test_format_api_error_404_model_not_found() {
        let body = r#"{"detail":"Model not found"}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::NOT_FOUND, body);
        assert!(result.contains("Model not found"));
        assert!(result.contains("model not found or endpoint incorrect"));
    }

    #[test]
    fn test_format_api_error_403_forbidden() {
        let body = r#"{"message":"Forbidden"}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::FORBIDDEN, body);
        assert!(result.contains("Forbidden"));
        assert!(result.contains("check API key permissions"));
    }

    #[test]
    fn test_format_api_error_500_server() {
        let body = r#"{"error":{"message":"Internal server error"}}"#;
        let result =
            OpenAiCompatBackend::format_api_error(StatusCode::INTERNAL_SERVER_ERROR, body);
        assert!(result.contains("retry later"));
    }

    #[test]
    fn test_format_api_error_empty_body() {
        let result = OpenAiCompatBackend::format_api_error(StatusCode::BAD_GATEWAY, "");
        assert!(result.contains("502"));
        assert!(result.contains("retry later"));
        // Should not contain trailing ": " for empty body
        assert!(!result.ends_with(": "));
    }

    #[test]
    fn test_format_api_error_plain_text_body() {
        let result = OpenAiCompatBackend::format_api_error(
            StatusCode::BAD_REQUEST,
            "something went wrong",
        );
        assert!(result.contains("something went wrong"));
        assert!(result.contains("400"));
    }

    #[test]
    fn test_format_api_error_no_hint_for_unknown_status() {
        let body = r#"{"error":{"message":"weird error"}}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::IM_A_TEAPOT, body);
        assert!(result.contains("weird error"));
        // No hint for 418
        assert!(!result.contains("check"));
        assert!(!result.contains("retry"));
    }

    #[test]
    fn test_parse_response_valid() {
        let backend = OpenAiCompatBackend::new(OpenAiCompatConfig {
            api_url: "http://localhost".into(),
            api_key: None,
            model: "test".into(),
            temperature: 1.0,
            top_p: 0.95,
            max_tokens: 100,
            system_prompt: "test".into(),
            use_thinking: false,
        });

        let json = json!({
            "choices": [{
                "message": {
                    "content": "Hello, world!",
                    "role": "assistant"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        let response = backend.parse_response(&json).unwrap();
        assert_eq!(response.content, "Hello, world!");
        assert_eq!(response.finish_reason, "stop");
        assert!(response.tool_calls.is_empty());
        assert_eq!(response.usage.unwrap().total_tokens, 15);
    }

    #[test]
    fn test_parse_response_with_tool_calls() {
        let backend = OpenAiCompatBackend::new(OpenAiCompatConfig {
            api_url: "http://localhost".into(),
            api_key: None,
            model: "test".into(),
            temperature: 1.0,
            top_p: 0.95,
            max_tokens: 100,
            system_prompt: "test".into(),
            use_thinking: false,
        });

        let json = json!({
            "choices": [{
                "message": {
                    "content": "",
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "tc_123",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"test.rs\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let response = backend.parse_response(&json).unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.tool_calls[0].id, "tc_123");
    }

    #[test]
    fn test_parse_response_no_choices() {
        let backend = OpenAiCompatBackend::new(OpenAiCompatConfig {
            api_url: "http://localhost".into(),
            api_key: None,
            model: "test".into(),
            temperature: 1.0,
            top_p: 0.95,
            max_tokens: 100,
            system_prompt: "test".into(),
            use_thinking: false,
        });

        let json = json!({"choices": []});
        assert!(backend.parse_response(&json).is_err());
    }

    #[test]
    fn test_build_messages() {
        let backend = OpenAiCompatBackend::new(OpenAiCompatConfig {
            api_url: "http://localhost".into(),
            api_key: None,
            model: "test".into(),
            temperature: 1.0,
            top_p: 0.95,
            max_tokens: 100,
            system_prompt: "You are helpful.".into(),
            use_thinking: false,
        });

        let messages = vec![
            Message {
                role: Role::User,
                content: "Hello".into(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];

        let api_messages = backend.build_messages(&messages);
        assert_eq!(api_messages.len(), 2); // system + user
        assert_eq!(api_messages[0]["role"], "system");
        assert_eq!(api_messages[1]["role"], "user");
        assert_eq!(api_messages[1]["content"], "Hello");
    }
}
