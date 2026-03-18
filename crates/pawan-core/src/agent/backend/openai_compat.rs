//! OpenAI-compatible LLM backend (NVIDIA NIM, OpenAI, DeepSeek, etc.)

use super::LlmBackend;
use crate::agent::{
    LLMResponse, Message, Role, TokenCallback, TokenUsage, ToolCallRequest, ToolDefinition,
};
use crate::{PawanError, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Cloud fallback endpoint — different API URL/key for hybrid routing
pub struct CloudFallback {
    pub api_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub fallback_models: Vec<String>,
}

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
    pub max_retries: usize,
    pub fallback_models: Vec<String>,
    /// Optional cloud fallback for hybrid local+cloud routing
    pub cloud: Option<CloudFallback>,
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

    /// Check if a model supports the `chat_template_kwargs` parameter.
    /// Only Qwen-family models on NIM support this. Mistral, LLaMA, and others reject it with 400.
    fn supports_chat_template_kwargs(model: &str) -> bool {
        let m = model.to_lowercase();
        m.contains("qwen") || m.contains("deepseek")
    }

    /// Check if a model supports tool use (function calling).
    /// Models known NOT to support tools on NIM: mistral-small, llama-3.1-8b, etc.
    /// Models known to support tools: devstral, qwen, deepseek, nemotron, llama-3.1-70b+
    fn supports_tool_use(model: &str) -> bool {
        let m = model.to_lowercase();
        // Explicit deny list — models that reject tools on NIM
        if m.contains("mistral-small") || m.contains("mistral-7b") {
            return false;
        }
        // Everything else: assume tool use support (fail gracefully in retry loop)
        true
    }

    /// Calculate exponential backoff delay with jitter
    /// Start at 1s, double each time, with ±20% random jitter
    fn calculate_backoff_delay(attempt: usize) -> std::time::Duration {
        let base_secs = (1u64 << attempt) as f64; // 1, 2, 4, 8, ...
        let jitter = 0.8 + (rand::random::<f64>() * 0.4); // 0.8 to 1.2
        std::time::Duration::from_secs_f64(base_secs * jitter)
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
        // StepFun/Qwen models may interleave <think>...</think> tokens inside arguments
        for tc in &mut tool_calls {
            if let Some(args_str) = tc.arguments.as_str() {
                // Strip think blocks from arguments before JSON parse
                let clean_args = Self::strip_think_from_str(args_str);
                if let Ok(parsed) = serde_json::from_str::<Value>(&clean_args) {
                    tc.arguments = parsed;
                } else if let Ok(parsed) = serde_json::from_str::<Value>(args_str) {
                    // Fallback: try original if stripping broke the JSON
                    tc.arguments = parsed;
                }
            }
            if tc.id.is_empty() {
                tc.id = uuid::Uuid::new_v4().to_string();
            }
        }

        tool_calls.retain(|tc| !tc.name.is_empty());

        // Fallback: devstral/Mistral models sometimes stream [TOOL_CALLS] text
        // instead of structured tool_call deltas.
        if tool_calls.is_empty() {
            tool_calls = Self::parse_mistral_tool_calls(&content);
        }

        if !tool_calls.is_empty() {
            finish_reason = "tool_calls".to_string();
        }

        // Strip think blocks from content (StepFun/Qwen interleave reasoning)
        let content = Self::strip_think_from_str(&content);

        // Streaming responses don't include usage in individual chunks
        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage: None,
        })
    }

    /// Strip <think>...</think> blocks from a string. Handles case-insensitive,
    /// nested blocks, and blocks interleaved with content (StepFun pattern).
    fn strip_think_from_str(s: &str) -> String {
        let mut out = s.to_string();
        loop {
            let lower = out.to_lowercase();
            let open = lower.find("<think>");
            let close = lower.find("</think>");
            match (open, close) {
                (Some(i), Some(j)) if j > i => {
                    let before = out[..i].trim_end().to_string();
                    let after = if out.len() > j + 8 {
                        out[j + 8..].trim_start().to_string()
                    } else {
                        String::new()
                    };
                    out = if before.is_empty() && after.is_empty() {
                        String::new()
                    } else if before.is_empty() {
                        after
                    } else if after.is_empty() {
                        before
                    } else {
                        format!("{} {}", before, after)
                    };
                }
                _ => break,
            }
        }
        out
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

        let raw_content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = Self::strip_think_from_str(raw_content);

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
                            // Strip think blocks from arguments (StepFun interleaves reasoning)
                            let clean = Self::strip_think_from_str(args_str);
                            serde_json::from_str(&clean)
                                .or_else(|_| serde_json::from_str(args_str))
                                .unwrap_or(json!({}))
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

        // Fallback: devstral/Mistral models sometimes embed tool calls in text content
        // instead of the structured tool_calls API field. Parse [TOOL_CALLS] format.
        if tool_calls.is_empty() {
            tool_calls = Self::parse_mistral_tool_calls(&content);
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

    /// Parse Mistral text-format tool calls embedded in content.
    ///
    /// Mistral/devstral models non-deterministically emit tool calls as text instead
    /// of structured API `tool_calls`. Two observed variants:
    ///
    /// Variant 1 (standard):  `[TOOL_CALLS] [{"name":"func","arguments":{...}}]`
    /// Variant 2 (compact):   `[TOOL_CALLS]func_name{"key":"value"}`
    fn parse_mistral_tool_calls(content: &str) -> Vec<ToolCallRequest> {
        const MARKER: &str = "[TOOL_CALLS]";
        let Some(pos) = content.find(MARKER) else {
            return vec![];
        };

        let after = content[pos + MARKER.len()..].trim_start();

        // Variant 1: JSON array — [{"name":..., "arguments":...}, ...]
        if after.starts_with('[') {
            let bracket_end = Self::find_matching_bracket(after, '[', ']');
            if bracket_end > 0 {
                if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&after[..bracket_end]) {
                    let calls: Vec<ToolCallRequest> = arr
                        .iter()
                        .filter_map(|tc| {
                            let name = tc.get("name")?.as_str()?.to_string();
                            if name.is_empty() {
                                return None;
                            }
                            let arguments = tc.get("arguments").cloned().unwrap_or(json!({}));
                            Some(ToolCallRequest {
                                id: uuid::Uuid::new_v4().to_string(),
                                name,
                                arguments,
                            })
                        })
                        .collect();
                    if !calls.is_empty() {
                        return calls;
                    }
                }
            }
        }

        // Variant 2: compact — func_name{"key":"value"}
        if let Some(brace_pos) = after.find('{') {
            let name = after[..brace_pos].trim();
            let is_valid_ident = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_');
            if is_valid_ident {
                let json_part = &after[brace_pos..];
                let brace_end = Self::find_matching_bracket(json_part, '{', '}');
                if brace_end > 0 {
                    if let Ok(arguments) = serde_json::from_str::<Value>(&json_part[..brace_end]) {
                        return vec![ToolCallRequest {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: name.to_string(),
                            arguments,
                        }];
                    }
                }
            }
        }

        vec![]
    }

    /// Find the end index of a balanced bracket pair starting at position 0.
    /// Returns the byte index after the closing bracket, or 0 if not found.
    fn find_matching_bracket(s: &str, open: char, close: char) -> usize {
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escape = false;
        for (i, ch) in s.char_indices() {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' && in_string {
                escape = true;
                continue;
            }
            if ch == '"' {
                in_string = !in_string;
                continue;
            }
            if in_string {
                continue;
            }
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    return i + ch.len_utf8();
                }
            }
        }
        0
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
            "temperature": self.cfg.temperature,
            "top_p": self.cfg.top_p,
            "max_tokens": self.cfg.max_tokens,
            "stream": on_token.is_some()
        });

        // Only include tools if non-empty AND model supports tool use
        if !api_tools.is_empty() && Self::supports_tool_use(&self.cfg.model) {
            request_body["tools"] = json!(api_tools);
        }

        // Only send chat_template_kwargs for models that support it (Qwen family).
        // Mistral, LLaMA, and other models reject this parameter with 400 errors.
        if Self::supports_chat_template_kwargs(&self.cfg.model) {
            if self.cfg.use_thinking {
                request_body["chat_template_kwargs"] = json!({ "thinking": true });
            } else {
                request_body["chat_template_kwargs"] = json!({ "enable_thinking": false });
            }
        }

        request_body["seed"] = json!(42);

        // Build model chain: primary model + fallback models (same provider)
        let mut model_chains: Vec<(String, Option<String>, Vec<String>)> = vec![
            (self.cfg.api_url.clone(), self.cfg.api_key.clone(), {
                let mut m = vec![self.cfg.model.clone()];
                m.extend(self.cfg.fallback_models.clone());
                m
            }),
        ];

        // Add cloud fallback chain if configured (different provider/URL)
        if let Some(ref cloud) = self.cfg.cloud {
            let mut cloud_models = vec![cloud.model.clone()];
            cloud_models.extend(cloud.fallback_models.clone());
            model_chains.push((cloud.api_url.clone(), cloud.api_key.clone(), cloud_models));
        }

        let mut last_error = None;
        let max_retries = self.cfg.max_retries;

        for (chain_idx, (api_url, api_key, models)) in model_chains.iter().enumerate() {
            let url = format!("{}/chat/completions", api_url);
            let is_cloud = chain_idx > 0;

            for model in models {
                request_body["model"] = json!(model);

                // Dynamically add/remove chat_template_kwargs based on model support
                if Self::supports_chat_template_kwargs(model) {
                    if !request_body.get("chat_template_kwargs").is_some() {
                        if self.cfg.use_thinking {
                            request_body["chat_template_kwargs"] = json!({ "thinking": true });
                        } else {
                            request_body["chat_template_kwargs"] = json!({ "enable_thinking": false });
                        }
                    }
                } else {
                    request_body.as_object_mut().map(|o| o.remove("chat_template_kwargs"));
                }

                // Dynamically add/remove tools based on model support
                if Self::supports_tool_use(model) {
                    if !api_tools.is_empty() && !request_body.get("tools").is_some() {
                        request_body["tools"] = json!(api_tools);
                    }
                } else {
                    request_body.as_object_mut().map(|o| o.remove("tools"));
                }

                for attempt in 0..=max_retries {
                    let mut request = self.http_client.post(&url).json(&request_body);

                    if let Some(ref key) = api_key {
                        request = request.header("Authorization", format!("Bearer {}", key));
                    }

                    let result = if on_token.is_some() {
                        self.streaming(request, on_token).await
                    } else {
                        self.non_streaming(request).await
                    };

                    match result {
                        Ok(response) => {
                            if is_cloud {
                                tracing::info!(model = model.as_str(), "Succeeded via cloud fallback");
                            }
                            return Ok(response);
                        }
                        Err(err) => {
                            last_error = Some(err);

                            if let Some(PawanError::Llm(ref msg)) = last_error.as_ref() {
                                if (msg.contains("429") || msg.contains("500") || msg.contains("501") ||
                                    msg.contains("502") || msg.contains("503") || msg.contains("504")) &&
                                    attempt < max_retries {
                                    let delay = Self::calculate_backoff_delay(attempt);
                                    tracing::warn!(
                                        attempt = attempt + 1,
                                        model = model.as_str(),
                                        delay_ms = delay.as_millis() as u64,
                                        "Retrying after LLM API error"
                                    );
                                    tokio::time::sleep(delay).await;
                                    continue;
                                }
                            }
                            break;
                        }
                    }
                }

                tracing::warn!(
                    model = model.as_str(),
                    cloud = is_cloud,
                    "Model exhausted retries, trying next"
                );
            }

            if self.cfg.cloud.is_some() && !is_cloud {
                tracing::warn!("Local models exhausted — falling back to cloud");
            }
        }

        Err(last_error.expect("No error recorded in retry loop"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            max_retries: 3,
            fallback_models: vec![],
            cloud: None,
        });

        let json = json!({"choices": []});
        assert!(backend.parse_response(&json).is_err());
    }

    #[test]
    fn test_parse_mistral_tool_calls_array_format() {
        let content = r#"[TOOL_CALLS] [{"name":"edit_file","arguments":{"path":"/tmp/test.rs","content":"fn main() {}"}}]"#;
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "edit_file");
        assert_eq!(calls[0].arguments["path"], "/tmp/test.rs");
    }

    #[test]
    fn test_parse_mistral_tool_calls_compact_format() {
        let content = r#"[TOOL_CALLS]read_file{"path":"/opt/pawan/src/main.rs"}"#;
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "/opt/pawan/src/main.rs");
    }

    #[test]
    fn test_parse_mistral_tool_calls_no_marker() {
        let content = "No tool calls here, just regular text.";
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_mistral_tool_calls_multiple() {
        let content = r#"[TOOL_CALLS] [{"name":"read_file","arguments":{"path":"a.rs"}},{"name":"read_file","arguments":{"path":"b.rs"}}]"#;
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["path"], "a.rs");
        assert_eq!(calls[1].arguments["path"], "b.rs");
    }

    #[test]
    fn test_parse_mistral_tool_calls_with_preamble() {
        let content = "I'll edit the file now.\n[TOOL_CALLS]shell_exec{\"command\":\"cargo check\"}";
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].arguments["command"], "cargo check");
    }

    #[test]
    fn test_parse_response_falls_back_to_mistral_format() {
        let backend = OpenAiCompatBackend::new(OpenAiCompatConfig {
            api_url: "http://localhost".into(),
            api_key: None,
            model: "test".into(),
            temperature: 1.0,
            top_p: 0.95,
            max_tokens: 100,
            system_prompt: "test".into(),
            use_thinking: false,
            max_retries: 3,
            fallback_models: vec![],
            cloud: None,
        });

        // No structured tool_calls, but content has [TOOL_CALLS] marker
        let json = json!({
            "choices": [{
                "message": {
                    "content": "[TOOL_CALLS] [{\"name\":\"read_file\",\"arguments\":{\"path\":\"/tmp/x.rs\"}}]",
                    "role": "assistant"
                },
                "finish_reason": "stop"
            }]
        });

        let response = backend.parse_response(&json).unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.tool_calls[0].arguments["path"], "/tmp/x.rs");
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
            max_retries: 3,
            fallback_models: vec![],
            cloud: None,
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

    #[test]
    fn test_calculate_backoff_delay() {
        // Test that backoff delays follow exponential pattern with jitter
        let delay_0 = OpenAiCompatBackend::calculate_backoff_delay(0);
        let delay_1 = OpenAiCompatBackend::calculate_backoff_delay(1);
        let delay_2 = OpenAiCompatBackend::calculate_backoff_delay(2);

        // Base delays should be around 1s, 2s, 4s (with ±20% jitter)
        assert!(delay_0.as_millis() >= 800 && delay_0.as_millis() <= 1200, 
                "Delay 0 should be ~1s with jitter: {}ms", delay_0.as_millis());
        assert!(delay_1.as_millis() >= 1600 && delay_1.as_millis() <= 2400, 
                "Delay 1 should be ~2s with jitter: {}ms", delay_1.as_millis());
        assert!(delay_2.as_millis() >= 3200 && delay_2.as_millis() <= 4800,
                "Delay 2 should be ~4s with jitter: {}ms", delay_2.as_millis());
    }
}
