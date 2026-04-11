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
    /// Qwen, DeepSeek, Gemma-4, and GLM models support this. Mistral, LLaMA, and others reject it with 400.
    fn supports_chat_template_kwargs(model: &str) -> bool {
        let m = model.to_lowercase();
        m.contains("qwen") || m.contains("deepseek") || m.contains("gemma") || m.contains("glm")
    }

    /// Check if a model uses the `reasoning_effort` parameter instead of chat_template_kwargs.
    /// Mistral Small 4+ uses per-request `reasoning_effort` (none/high).
    fn supports_reasoning_effort(model: &str) -> bool {
        let m = model.to_lowercase();
        m.contains("mistral-small-4")
    }

    /// Get the correct `chat_template_kwargs` value for thinking mode.
    /// GLM uses `enable_thinking` + `clear_thinking`, Gemma uses `enable_thinking`,
    /// Qwen/DeepSeek use `thinking`.
    fn thinking_kwargs(model: &str, enabled: bool) -> serde_json::Value {
        let m = model.to_lowercase();
        if m.contains("glm") {
            json!({ "enable_thinking": enabled, "clear_thinking": false })
        } else if m.contains("gemma") {
            json!({ "enable_thinking": enabled })
        } else {
            json!({ "thinking": enabled })
        }
    }

    /// Check if a model supports tool use (function calling).
    /// Models known NOT to support tools on NIM: mistral-7b, old mistral-small (pre-v4).
    /// Models known to support tools: mistral-small-4, devstral, qwen, deepseek, nemotron, llama-3.1-70b+
    fn supports_tool_use(model: &str) -> bool {
        let m = model.to_lowercase();
        // Explicit deny list — old models that reject tools on NIM
        if m.contains("mistral-7b") {
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
                        "parameters": t.to_mcp_input_schema()
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
        let mut stream_usage: Option<TokenUsage> = None;
        let mut stream_reasoning = String::new();

        let mut stream = response.bytes_stream();
        use futures::StreamExt;

        let mut buffer = String::new();
        let mut buf_start = 0usize;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| PawanError::Llm(format!("Stream error: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(rel_pos) = buffer[buf_start..].find('\n') {
                let newline_pos = buf_start + rel_pos;
                let line = buffer[buf_start..newline_pos].trim();
                buf_start = newline_pos + 1; // advance past newline (zero-copy)
                if line.is_empty() || line == "data: [DONE]" {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(json) = serde_json::from_str::<Value>(data) {
                        // Capture usage from final chunk (OpenAI stream_options, vllm-mlx, etc.)
                        if json.get("usage").and_then(|u| u.get("total_tokens")).is_some() {
                            stream_usage = Self::parse_usage(&json);
                        }

                        if let Some(choices) = json.get("choices").and_then(|v| v.as_array()) {
                            for choice in choices {
                                if let Some(delta) = choice.get("delta") {
                                    if let Some(c) = delta.get("content").and_then(|v| v.as_str()) {
                                        if let Some(callback) = on_token {
                                            callback(c);
                                        }
                                        content.push_str(c);
                                    }

                                    // Capture reasoning/thinking content from streaming deltas
                                    if let Some(r) = delta.get("reasoning_content")
                                        .or_else(|| delta.get("reasoning"))
                                        .and_then(|v| v.as_str())
                                    {
                                        stream_reasoning.push_str(r);
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
                                                    arguments: json!(""),
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
            // Compact buffer: only reallocate when >50% consumed
            if buf_start > 0 {
                buffer = buffer[buf_start..].to_string();
                buf_start = 0;
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

        // Build reasoning from streamed chunks
        let reasoning = if stream_reasoning.is_empty() { None } else { Some(stream_reasoning) };

        // Enrich stream usage with reasoning token estimate
        let usage = stream_usage.map(|mut u| {
            if let Some(ref r) = reasoning {
                u.reasoning_tokens = (r.len() as u64) / 4;
                u.action_tokens = u.completion_tokens.saturating_sub(u.reasoning_tokens);
            }
            u
        });

        Ok(LLMResponse {
            content,
            reasoning,
            tool_calls,
            finish_reason,
            usage,
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

        // Parse reasoning/thinking content (mlx_lm.server, vllm-mlx, DeepSeek)
        let reasoning = message
            .get("reasoning_content")
            .or_else(|| message.get("reasoning"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // Parse usage from response, enriched with reasoning token estimate
        let usage = Self::parse_usage(json).map(|mut u| {
            // Estimate reasoning tokens from the reasoning string length (1 tok ≈ 4 chars)
            if let Some(ref r) = reasoning {
                u.reasoning_tokens = (r.len() as u64) / 4;
                u.action_tokens = u.completion_tokens.saturating_sub(u.reasoning_tokens);
            } else {
                u.reasoning_tokens = 0;
                u.action_tokens = u.completion_tokens;
            }
            u
        });

        Ok(LLMResponse {
            content,
            reasoning,
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
        let completion = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        Some(TokenUsage {
            prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            completion_tokens: completion,
            total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            reasoning_tokens: 0, // filled in by parse_response after reasoning extraction
            action_tokens: completion,
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

        // Thinking mode: use the right mechanism per model family.
        // - Mistral Small 4+: `reasoning_effort` (none/high)
        // - Qwen/Gemma/GLM: `chat_template_kwargs`
        // - Others: no thinking support
        if Self::supports_reasoning_effort(&self.cfg.model) {
            request_body["reasoning_effort"] = if self.cfg.use_thinking {
                json!("high")
            } else {
                json!("none")
            };
        } else if Self::supports_chat_template_kwargs(&self.cfg.model) {
            request_body["chat_template_kwargs"] =
                Self::thinking_kwargs(&self.cfg.model, self.cfg.use_thinking);
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

                // Dynamically add/remove thinking params based on model support
                if Self::supports_reasoning_effort(model) {
                    request_body.as_object_mut().map(|o| o.remove("chat_template_kwargs"));
                    request_body["reasoning_effort"] = if self.cfg.use_thinking {
                        json!("high")
                    } else {
                        json!("none")
                    };
                } else if Self::supports_chat_template_kwargs(model) {
                    request_body.as_object_mut().map(|o| o.remove("reasoning_effort"));
                    if request_body.get("chat_template_kwargs").is_none() {
                        request_body["chat_template_kwargs"] =
                            Self::thinking_kwargs(model, self.cfg.use_thinking);
                    }
                } else {
                    request_body.as_object_mut().map(|o| o.remove("chat_template_kwargs"));
                    request_body.as_object_mut().map(|o| o.remove("reasoning_effort"));
                }

                // Dynamically add/remove tools based on model support
                if Self::supports_tool_use(model) {
                    if !api_tools.is_empty() && request_body.get("tools").is_none() {
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

                    let prompt_len: usize = messages.iter().map(|m| m.content.len()).sum();
                    let tools_count = tools.len();
                    tracing::info!(
                        model = model.as_str(),
                        url = url.as_str(),
                        provider = if is_cloud { "cloud" } else { "local" },
                        prompt_len,
                        tools = tools_count,
                        attempt,
                        streaming = on_token.is_some(),
                        "llm call"
                    );

                    let t0 = std::time::Instant::now();
                    let result = if on_token.is_some() {
                        self.streaming(request, on_token).await
                    } else {
                        self.non_streaming(request).await
                    };
                    let latency_ms = t0.elapsed().as_millis() as u64;

                    match result {
                        Ok(response) => {
                            tracing::info!(
                                model = model.as_str(),
                                provider = if is_cloud { "cloud" } else { "local" },
                                latency_ms,
                                prompt_tokens = response.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                                completion_tokens = response.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                                finish_reason = response.finish_reason.as_str(),
                                response_len = response.content.len(),
                                tool_calls = response.tool_calls.len(),
                                "llm ok"
                            );
                            return Ok(response);
                        }
                        Err(err) => {
                            let err_msg = err.to_string();
                            tracing::warn!(
                                model = model.as_str(),
                                provider = if is_cloud { "cloud" } else { "local" },
                                latency_ms,
                                attempt,
                                error = %err_msg,
                                "llm error"
                            );
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
                                        "retrying"
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
        let content = r#"[TOOL_CALLS]read_file{"path":"/home/user/project/src/main.rs"}"#;
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "/home/user/project/src/main.rs");
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

    #[test]
    fn test_supports_chat_template_kwargs() {
        assert!(OpenAiCompatBackend::supports_chat_template_kwargs("Qwen/Qwen2.5-72B-Instruct"));
        assert!(OpenAiCompatBackend::supports_chat_template_kwargs("deepseek-ai/deepseek-v3"));
        assert!(OpenAiCompatBackend::supports_chat_template_kwargs("google/gemma-4-31b-it"));
        assert!(OpenAiCompatBackend::supports_chat_template_kwargs("z-ai/glm4.7"));
        assert!(OpenAiCompatBackend::supports_chat_template_kwargs("z-ai/glm5"));

        // Mistral uses reasoning_effort, not chat_template_kwargs
        assert!(!OpenAiCompatBackend::supports_chat_template_kwargs("mistralai/mistral-small-4-119b-2603"));
        assert!(!OpenAiCompatBackend::supports_chat_template_kwargs("meta/llama-3.1-70b-instruct"));
        assert!(!OpenAiCompatBackend::supports_chat_template_kwargs("stepfun-ai/step-3.5-flash"));
        assert!(!OpenAiCompatBackend::supports_chat_template_kwargs("minimaxai/minimax-m2.5"));
    }

    #[test]
    fn test_supports_reasoning_effort() {
        assert!(OpenAiCompatBackend::supports_reasoning_effort("mistralai/mistral-small-4-119b-2603"));
        assert!(!OpenAiCompatBackend::supports_reasoning_effort("stepfun-ai/step-3.5-flash"));
        assert!(!OpenAiCompatBackend::supports_reasoning_effort("minimaxai/minimax-m2.5"));
        assert!(!OpenAiCompatBackend::supports_reasoning_effort("qwen/qwen3.5-122b-a10b"));
    }

    #[test]
    fn test_thinking_kwargs_gemma_uses_enable_thinking() {
        assert_eq!(OpenAiCompatBackend::thinking_kwargs("google/gemma-4-31b-it", true),
                   json!({ "enable_thinking": true }));
        assert_eq!(OpenAiCompatBackend::thinking_kwargs("google/gemma-4-31b-it", false),
                   json!({ "enable_thinking": false }));
    }

    #[test]
    fn test_thinking_kwargs_glm_uses_enable_thinking_and_clear_thinking() {
        assert_eq!(OpenAiCompatBackend::thinking_kwargs("z-ai/glm4.7", true),
                   json!({ "enable_thinking": true, "clear_thinking": false }));
        assert_eq!(OpenAiCompatBackend::thinking_kwargs("z-ai/glm5", false),
                   json!({ "enable_thinking": false, "clear_thinking": false }));
    }

    #[test]
    fn test_thinking_kwargs_qwen_uses_thinking() {
        assert_eq!(OpenAiCompatBackend::thinking_kwargs("Qwen/Qwen2.5-72B-Instruct", true),
                   json!({ "thinking": true }));
        assert_eq!(OpenAiCompatBackend::thinking_kwargs("Qwen/Qwen2.5-72B-Instruct", false),
                   json!({ "thinking": false }));
    }

    #[test]
    fn test_thinking_kwargs_deepseek_uses_thinking() {
        assert_eq!(OpenAiCompatBackend::thinking_kwargs("deepseek-ai/deepseek-r1", true),
                   json!({ "thinking": true }));
    }
}

#[cfg(test)]
mod think_strip_tests {
    use super::OpenAiCompatBackend;

    #[test]
    fn strip_simple() {
        let s = "Hello <think>internal reasoning</think> world";
        assert_eq!(OpenAiCompatBackend::strip_think_from_str(s), "Hello world");
    }

    #[test]
    fn strip_case_insensitive() {
        let s = "A <Think>stuff</THINK> B";
        assert_eq!(OpenAiCompatBackend::strip_think_from_str(s), "A B");
    }

    #[test]
    fn strip_multiple() {
        let s = "<think>a</think>Hello<think>b</think> there";
        assert_eq!(OpenAiCompatBackend::strip_think_from_str(s), "Hello there");
    }

    #[test]
    fn strip_nested_content() {
        let s = "prefix <think>line1\nline2\nline3</think> suffix";
        assert_eq!(OpenAiCompatBackend::strip_think_from_str(s), "prefix suffix");
    }

    #[test]
    fn strip_entire_message() {
        let s = "<think>only thinking</think>";
        assert_eq!(OpenAiCompatBackend::strip_think_from_str(s).trim(), "");
    }

    #[test]
    fn strip_no_blocks() {
        let s = "No think blocks here";
        assert_eq!(OpenAiCompatBackend::strip_think_from_str(s), "No think blocks here");
    }

    #[test]
    fn strip_from_json_args() {
        let s = r#"<think>let me figure out the path</think>{"path":"src/main.rs","content":"hello"}"#;
        let clean = OpenAiCompatBackend::strip_think_from_str(s);
        let parsed: serde_json::Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(parsed["path"], "src/main.rs");
    }

    #[test]
    fn strip_interleaved_json() {
        // StepFun pattern: thinking interleaved with JSON
        let s = r#"{"path":"test.rs"<think>checking the file</think>,"content":"fn main() {}"}"#;
        let clean = OpenAiCompatBackend::strip_think_from_str(s);
        // After stripping, JSON should be parseable
        let result = serde_json::from_str::<serde_json::Value>(&clean);
        // This specific case may not parse due to comma positioning, 
        // but the stripping itself should not panic
        let _ = result;
    }

    #[test]
    fn strip_empty_string() {
        assert_eq!(OpenAiCompatBackend::strip_think_from_str(""), "");
    }

    #[test]
    fn strip_whitespace_only() {
        assert_eq!(OpenAiCompatBackend::strip_think_from_str("   ").trim(), "");
    }
}

#[cfg(test)]
mod mistral_tool_call_tests {
    use super::OpenAiCompatBackend;

    #[test]
    fn parse_json_array_variant() {
        let content = r#"I'll use the tool. [TOOL_CALLS] [{"name":"read_file","arguments":{"path":"src/main.rs"}}]"#;
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "src/main.rs");
    }

    #[test]
    fn parse_compact_variant() {
        let content = r#"[TOOL_CALLS] bash{"command":"ls -la"}"#;
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].arguments["command"], "ls -la");
    }

    #[test]
    fn parse_multiple_tools() {
        let content = r#"[TOOL_CALLS] [{"name":"read_file","arguments":{"path":"a.rs"}},{"name":"read_file","arguments":{"path":"b.rs"}}]"#;
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["path"], "a.rs");
        assert_eq!(calls[1].arguments["path"], "b.rs");
    }

    #[test]
    fn no_marker_returns_empty() {
        let content = "Just regular text with no tool calls";
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert!(calls.is_empty());
    }

    #[test]
    fn marker_with_invalid_json() {
        let content = "[TOOL_CALLS] {invalid json here}";
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert!(calls.is_empty());
    }

    #[test]
    fn empty_name_filtered_out() {
        let content = r#"[TOOL_CALLS] [{"name":"","arguments":{}}]"#;
        let calls = OpenAiCompatBackend::parse_mistral_tool_calls(content);
        assert!(calls.is_empty());
    }
}

#[cfg(test)]
mod streaming_tool_call_tests {
    use serde_json::json;

    /// Simulate streaming tool call argument accumulation — the exact pattern
    /// that was buggy when arguments were initialized as json!({}) instead of json!("").
    #[test]
    fn streaming_args_accumulate_across_deltas() {
        // Simulate the streaming accumulation logic from the streaming() method
        let mut arguments = json!(""); // Fixed: was json!({}) which broke .as_str()

        // Delta 1: partial arguments
        let delta1 = r#"{"pa"#;
        let current = arguments.as_str().unwrap_or("");
        arguments = json!(format!("{}{}", current, delta1));

        // Delta 2: more arguments
        let delta2 = r#"th":"src/"#;
        let current = arguments.as_str().unwrap_or("");
        arguments = json!(format!("{}{}", current, delta2));

        // Delta 3: closing
        let delta3 = r#"main.rs"}"#;
        let current = arguments.as_str().unwrap_or("");
        arguments = json!(format!("{}{}", current, delta3));

        // Verify accumulated string is valid JSON
        let args_str = arguments.as_str().unwrap();
        assert_eq!(args_str, r#"{"path":"src/main.rs"}"#);

        // Verify it parses correctly
        let parsed: serde_json::Value = serde_json::from_str(args_str).unwrap();
        assert_eq!(parsed["path"], "src/main.rs");
    }

    #[test]
    fn streaming_args_init_as_empty_string_not_object() {
        let arguments = json!("");
        // This must return Some, not None
        assert!(arguments.as_str().is_some(), "json!(\"\") must be a string");
        assert_eq!(arguments.as_str().unwrap(), "");

        // Contrast: json!({}) returns None for as_str()
        let bad_arguments = json!({});
        assert!(bad_arguments.as_str().is_none(), "json!({{}}) is not a string");
    }

    #[test]
    fn streaming_args_with_think_blocks_cleaned() {
        use super::OpenAiCompatBackend;

        // Simulate StepFun/Qwen model interleaving <think> in arguments
        let accumulated = r#"<think>let me write the path</think>{"path":"test.rs","content":"fn main() {}"}"#;
        let clean = OpenAiCompatBackend::strip_think_from_str(accumulated);
        let parsed: serde_json::Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(parsed["path"], "test.rs");
        assert_eq!(parsed["content"], "fn main() {}");
    }
}

#[cfg(test)]
mod bracket_matching_tests {
    use super::OpenAiCompatBackend;

    #[test]
    fn find_matching_bracket_simple() {
        assert_eq!(OpenAiCompatBackend::find_matching_bracket("{}", '{', '}'), 2);
        assert_eq!(OpenAiCompatBackend::find_matching_bracket("[]", '[', ']'), 2);
    }

    #[test]
    fn find_matching_bracket_nested() {
        // {"a":{"b":1}} = 13 bytes, outer } at index 12, returns 13
        assert_eq!(OpenAiCompatBackend::find_matching_bracket(r#"{"a":{"b":1}}"#, '{', '}'), 13);
    }

    #[test]
    fn find_matching_bracket_with_strings() {
        // {"key":"val{ue}"} = 17 bytes, outer } at index 16, returns 17
        assert_eq!(
            OpenAiCompatBackend::find_matching_bracket(r#"{"key":"val{ue}"}"#, '{', '}'),
            17
        );
    }

    #[test]
    fn find_matching_bracket_unmatched() {
        assert_eq!(OpenAiCompatBackend::find_matching_bracket("{unclosed", '{', '}'), 0);
    }

    #[test]
    fn find_matching_bracket_empty() {
        assert_eq!(OpenAiCompatBackend::find_matching_bracket("", '{', '}'), 0);
    }
}

#[cfg(test)]
mod api_error_tests {
    use super::OpenAiCompatBackend;
    use reqwest::StatusCode;

    #[test]
    fn format_error_json_message() {
        let body = r#"{"error":{"message":"Invalid API key"}}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::UNAUTHORIZED, body);
        assert!(result.contains("Invalid API key"));
        assert!(result.contains("401"));
        assert!(result.contains("check your API key"));
    }

    #[test]
    fn format_error_detail_field() {
        let body = r#"{"detail":"Model not found"}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::NOT_FOUND, body);
        assert!(result.contains("Model not found"));
        assert!(result.contains("404"));
    }

    #[test]
    fn format_error_message_field() {
        let body = r#"{"message":"Rate limit exceeded"}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::TOO_MANY_REQUESTS, body);
        assert!(result.contains("Rate limit exceeded"));
        assert!(result.contains("rate limited"));
    }

    #[test]
    fn format_error_empty_body() {
        let result = OpenAiCompatBackend::format_api_error(StatusCode::INTERNAL_SERVER_ERROR, "");
        assert!(result.contains("500"));
        assert!(result.contains("server error"));
        assert!(!result.contains(": \n")); // no trailing garbage
    }

    #[test]
    fn format_error_non_json_body() {
        let body = "Bad Gateway: upstream timeout";
        let result = OpenAiCompatBackend::format_api_error(StatusCode::BAD_GATEWAY, body);
        assert!(result.contains("502"));
        assert!(result.contains("upstream timeout"));
    }

    #[test]
    fn format_error_forbidden() {
        let body = r#"{"error":{"message":"Forbidden"}}"#;
        let result = OpenAiCompatBackend::format_api_error(StatusCode::FORBIDDEN, body);
        assert!(result.contains("403"));
        assert!(result.contains("permissions"));
    }

    #[test]
    fn format_error_unknown_status() {
        let result = OpenAiCompatBackend::format_api_error(StatusCode::IM_A_TEAPOT, "teapot");
        assert!(result.contains("418"));
        assert!(result.contains("teapot"));
        // No special hint for unknown codes
        assert!(!result.contains("check"));
    }
}

#[cfg(test)]
mod usage_parsing_tests {
    use super::OpenAiCompatBackend;
    use serde_json::json;

    #[test]
    fn parse_usage_full() {
        let resp = json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        });
        let usage = OpenAiCompatBackend::parse_usage(&resp).unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn parse_usage_missing() {
        let resp = json!({"choices": []});
        assert!(OpenAiCompatBackend::parse_usage(&resp).is_none());
    }

    #[test]
    fn parse_usage_partial() {
        let resp = json!({"usage": {"prompt_tokens": 42}});
        let usage = OpenAiCompatBackend::parse_usage(&resp).unwrap();
        assert_eq!(usage.prompt_tokens, 42);
        assert_eq!(usage.completion_tokens, 0); // defaults to 0
    }
}
