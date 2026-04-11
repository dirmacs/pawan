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
            reasoning: None,
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
            reasoning: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ToolResultMessage;

    fn make_backend() -> OllamaBackend {
        OllamaBackend::new(
            "http://localhost:11434".into(),
            "test-model".into(),
            0.7,
            "You are a helpful agent.".into(),
        )
    }

    #[test]
    fn build_messages_prepends_system_prompt_and_handles_all_roles() {
        let backend = make_backend();
        let input = vec![
            Message {
                role: Role::User,
                content: "fix the bug".into(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::Assistant,
                content: "Looking at it".into(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        let out = backend.build_messages(&input);
        // System prompt is always first, even if caller doesn't include one.
        assert_eq!(out.len(), 3, "system + 2 user/assistant messages");
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[0]["content"], "You are a helpful agent.");
        assert_eq!(out[1]["role"], "user");
        assert_eq!(out[1]["content"], "fix the bug");
        assert_eq!(out[2]["role"], "assistant");
        assert_eq!(out[2]["content"], "Looking at it");
        // Plain assistant message (no tool_calls) must NOT include the
        // tool_calls key at all — Ollama API rejects empty arrays for some
        // versions.
        assert!(out[2].get("tool_calls").is_none());
    }

    #[test]
    fn build_messages_assistant_with_tool_calls_emits_ollama_format() {
        let backend = make_backend();
        let input = vec![Message {
            role: Role::Assistant,
            content: "Reading file".into(),
            tool_calls: vec![ToolCallRequest {
                id: "tc-abc".into(),
                name: "read_file".into(),
                arguments: json!({"path": "src/lib.rs"}),
            }],
            tool_result: None,
        }];
        let out = backend.build_messages(&input);
        // system + assistant-with-tools
        let asst = &out[1];
        assert_eq!(asst["role"], "assistant");
        let tcs = asst["tool_calls"].as_array().expect("tool_calls array");
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0]["function"]["name"], "read_file");
        assert_eq!(tcs[0]["function"]["arguments"]["path"], "src/lib.rs");
    }

    #[test]
    fn build_messages_tool_role_serializes_result_content() {
        let backend = make_backend();
        let input = vec![Message {
            role: Role::Tool,
            content: "unused in tool role".into(),
            tool_calls: vec![],
            tool_result: Some(ToolResultMessage {
                tool_call_id: "tc-1".into(),
                content: json!({"ok": true, "rows": 42}),
                success: true,
            }),
        }];
        let out = backend.build_messages(&input);
        // system + tool
        let tool_msg = &out[1];
        assert_eq!(tool_msg["role"], "tool");
        let content = tool_msg["content"].as_str().unwrap();
        // serde_json::to_string produces a JSON-encoded string
        assert!(content.contains("\"rows\""), "content should have rows: {}", content);
        assert!(content.contains("42"), "content should have 42: {}", content);
    }

    #[test]
    fn build_tools_wraps_definitions_in_ollama_function_envelope() {
        let backend = make_backend();
        let tools = vec![ToolDefinition {
            name: "greet".into(),
            description: "Say hi".into(),
            parameters: json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        }];
        let out = backend.build_tools(&tools);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "function");
        assert_eq!(out[0]["function"]["name"], "greet");
        assert_eq!(out[0]["function"]["description"], "Say hi");
        assert_eq!(
            out[0]["function"]["parameters"]["properties"]["name"]["type"],
            "string"
        );
    }

    #[test]
    fn parse_response_plain_content() {
        let backend = make_backend();
        let json = serde_json::json!({
            "message": {
                "role": "assistant",
                "content": "hello world"
            }
        });
        let resp = backend.parse_response(&json).unwrap();
        assert_eq!(resp.content, "hello world");
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "stop", "default when no done_reason");
    }

    #[test]
    fn parse_response_with_tool_calls_sets_finish_reason() {
        let backend = make_backend();
        let json = serde_json::json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "function": {
                        "name": "read_file",
                        "arguments": {"path": "foo.rs"}
                    }
                }]
            }
        });
        let resp = backend.parse_response(&json).unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "read_file");
        assert_eq!(resp.tool_calls[0].arguments["path"], "foo.rs");
        assert_eq!(
            resp.finish_reason, "tool_calls",
            "presence of tool_calls must flip finish_reason"
        );
        // Each tool call gets a fresh UUID — non-empty, unique per call
        assert!(!resp.tool_calls[0].id.is_empty());
    }

    #[test]
    fn parse_response_without_message_returns_error() {
        let backend = make_backend();
        let json = serde_json::json!({"done": true});
        let err = backend.parse_response(&json).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("No message") || msg.contains("message"),
            "error message should mention missing message, got: {}",
            msg
        );
    }

    #[test]
    fn build_messages_system_role_stacks_on_implicit_system() {
        // The implicit system prompt is always the first entry. If the caller
        // also passes a Role::System message, it must be APPENDED, not
        // replace the implicit one. Historical regression: prior version
        // incorrectly overwrote the backend's system prompt.
        let backend = make_backend();
        let input = vec![Message {
            role: Role::System,
            content: "extra context: be terse".into(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let out = backend.build_messages(&input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[0]["content"], "You are a helpful agent.");
        assert_eq!(out[1]["role"], "system");
        assert_eq!(out[1]["content"], "extra context: be terse");
    }

    #[test]
    fn build_messages_tool_role_without_result_is_skipped() {
        // A Role::Tool message with tool_result = None produces no output —
        // the if-let-Some guard short-circuits. This keeps the conversation
        // array clean when there's a spurious tool role without a payload.
        let backend = make_backend();
        let input = vec![Message {
            role: Role::Tool,
            content: "no-op".into(),
            tool_calls: vec![],
            tool_result: None,
        }];
        let out = backend.build_messages(&input);
        // Only the implicit system prompt survives
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "system");
    }

    #[test]
    fn build_messages_multiple_tool_calls_preserved() {
        // All tool calls in a single assistant message must appear in the
        // tool_calls array, in order. Regression: earlier version only
        // emitted the first one.
        let backend = make_backend();
        let input = vec![Message {
            role: Role::Assistant,
            content: "doing 3 things".into(),
            tool_calls: vec![
                ToolCallRequest {
                    id: "a".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "a.rs"}),
                },
                ToolCallRequest {
                    id: "b".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "b.rs"}),
                },
                ToolCallRequest {
                    id: "c".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                },
            ],
            tool_result: None,
        }];
        let out = backend.build_messages(&input);
        let tcs = out[1]["tool_calls"].as_array().unwrap();
        assert_eq!(tcs.len(), 3);
        assert_eq!(tcs[0]["function"]["arguments"]["path"], "a.rs");
        assert_eq!(tcs[1]["function"]["arguments"]["path"], "b.rs");
        assert_eq!(tcs[2]["function"]["name"], "bash");
    }

    #[test]
    fn parse_response_done_reason_overrides_default_stop() {
        // When Ollama reports done_reason="length" (context truncated), the
        // parsed finish_reason must surface that — not silently pretend the
        // response completed cleanly. Downstream heal loop treats "length"
        // differently from "stop".
        let backend = make_backend();
        let json = serde_json::json!({
            "message": { "role": "assistant", "content": "partial..." },
            "done_reason": "length"
        });
        let resp = backend.parse_response(&json).unwrap();
        assert_eq!(resp.finish_reason, "length");
        assert_eq!(resp.content, "partial...");
    }

    #[test]
    fn parse_response_tool_call_missing_arguments_defaults_to_empty_object() {
        // Function with name but no arguments field — must NOT panic, must
        // default to an empty JSON object so downstream code can still call
        // `args["field"].as_str()` without a type error.
        let backend = make_backend();
        let json = serde_json::json!({
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "function": { "name": "stats" }
                }]
            }
        });
        let resp = backend.parse_response(&json).unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "stats");
        assert_eq!(resp.tool_calls[0].arguments, json!({}));
        assert_eq!(resp.finish_reason, "tool_calls");
    }

    #[test]
    fn parse_response_message_without_content_defaults_to_empty_string() {
        // If the message has no content field (just a role), content must
        // be "" not a parse error — some Ollama versions omit content when
        // the response is pure tool_calls.
        let backend = make_backend();
        let json = serde_json::json!({
            "message": { "role": "assistant" }
        });
        let resp = backend.parse_response(&json).unwrap();
        assert_eq!(resp.content, "");
        assert!(resp.tool_calls.is_empty());
    }
}
