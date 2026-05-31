//! Agent execution loop — tool calling, streaming, and coordinator dispatch.

use super::{
    prepare_recalled_context, AgentResponse, LLMResponse, Message, PawanAgent, PermissionCallback,
    PermissionRequest, Role, TokenCallback, TokenUsage, ToolCallback, ToolCallRecord,
    ToolCallRequest, ToolResultMessage, ToolStartCallback,
};
use crate::coordinator::{CoordinatorResult, ToolCallingConfig, ToolCoordinator};
use crate::tools::ToolRegistry;
use crate::{PawanError, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

/// Truncate a tool result JSON value to fit within max_chars.
/// Unlike naive string truncation (which breaks JSON), this truncates string
/// *values* within the JSON structure, preserving valid JSON output.
pub(crate) fn truncate_tool_result(value: Value, max_chars: usize) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_chars {
        return value;
    }

    // Strategy: find the largest string values and truncate them
    match value {
        Value::Object(map) => {
            let mut result = serde_json::Map::new();
            let total = serialized.len();
            for (k, v) in map {
                if let Value::String(s) = &v {
                    if s.len() > 500 {
                        // Proportional truncation: shrink large strings
                        let target = s.len() * max_chars / total;
                        let target = target.max(200); // Keep at least 200 chars
                        let truncated: String = s.chars().take(target).collect();
                        result.insert(
                            k,
                            json!(format!(
                                "{}...[truncated from {} chars]",
                                truncated,
                                s.len()
                            )),
                        );
                        continue;
                    }
                }
                // Recursively truncate nested structures
                result.insert(k, truncate_tool_result(v, max_chars));
            }
            Value::Object(result)
        }
        Value::String(s) if s.len() > max_chars => {
            let truncated: String = s.chars().take(max_chars).collect();
            json!(format!(
                "{}...[truncated from {} chars]",
                truncated,
                s.len()
            ))
        }
        Value::Array(arr) if serialized.len() > max_chars => {
            // Truncate array: keep first N items that fit
            let mut result = Vec::new();
            let mut running_len = 2; // "[]"
            for item in arr {
                let item_str = serde_json::to_string(&item).unwrap_or_default();
                running_len += item_str.len() + 1; // +1 for comma
                if running_len > max_chars {
                    result.push(json!(format!("...[{} more items truncated]", 0)));
                    break;
                }
                result.push(item);
            }
            Value::Array(result)
        }
        other => other,
    }
}

/// Summarize tool arguments for permission requests
pub(crate) fn summarize_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => {
            let mut parts = Vec::new();
            for (key, value) in map {
                let value_str = match value {
                    serde_json::Value::String(s) if s.len() > 50 => {
                        format!("\"{}...\"", &s[..47])
                    }
                    serde_json::Value::String(s) => format!("\"{}\"", s),
                    serde_json::Value::Array(arr) if arr.len() > 3 => {
                        format!("[... {} items]", arr.len())
                    }
                    serde_json::Value::Array(arr) => {
                        let items: Vec<String> = arr
                            .iter()
                            .take(3)
                            .map(|v| match v {
                                serde_json::Value::String(s) => {
                                    if s.len() > 20 {
                                        format!("\"{}...\"", &s[..17])
                                    } else {
                                        format!("\"{}\"", s)
                                    }
                                }
                                _ => v.to_string(),
                            })
                            .collect();
                        format!("[{}]", items.join(", "))
                    }
                    _ => value.to_string(),
                };
                parts.push(format!("{}: {}", key, value_str));
            }
            parts.join(", ")
        }
        serde_json::Value::String(s) => {
            if s.len() > 100 {
                format!("\"{}...\"", &s[..97])
            } else {
                format!("\"{}\"", s)
            }
        }
        serde_json::Value::Array(arr) => {
            format!("[{} items]", arr.len())
        }
        _ => args.to_string(),
    }
}


impl PawanAgent {
    /// Execute a single prompt with tool calling support
    pub async fn execute(&mut self, user_prompt: &str) -> Result<AgentResponse> {
        self.execute_with_callbacks(user_prompt, None, None, None)
            .await
    }

    /// Execute with optional callbacks for streaming
    pub async fn execute_with_callbacks(
        &mut self,
        user_prompt: &str,
        on_token: Option<TokenCallback>,
        on_tool: Option<ToolCallback>,
        on_tool_start: Option<ToolStartCallback>,
    ) -> Result<AgentResponse> {
        self.execute_with_all_callbacks(user_prompt, on_token, on_tool, on_tool_start, None)
            .await
    }

    /// Execute with all callbacks, including permission prompt.
    pub async fn execute_with_all_callbacks(
        &mut self,
        user_prompt: &str,
        on_token: Option<TokenCallback>,
        on_tool: Option<ToolCallback>,
        on_tool_start: Option<ToolStartCallback>,
        on_permission: Option<PermissionCallback>,
    ) -> Result<AgentResponse> {
        // Check if coordinator mode is enabled
        if self.config.use_coordinator {
            // Coordinator mode does not support callbacks or permission prompts
            if on_token.is_some()
                || on_tool.is_some()
                || on_tool_start.is_some()
                || on_permission.is_some()
            {
                tracing::warn!(
                    "Callbacks and permission prompts are not supported in coordinator mode; ignoring them"
                );
            }
            return self.execute_with_coordinator(user_prompt).await;
        }

        // Reset idle timeout for the new turn
        self.last_tool_call_time = None;

        // Inject Eruka core memory and prefetch context
        self.inject_eruka_context(user_prompt).await;

        // Build effective prompt with architecture context and push to history
        let effective_prompt = self.build_user_prompt(user_prompt)?;
        self.history.push(Message {
            role: Role::User,
            content: effective_prompt,
            tool_calls: vec![],
            tool_result: None,
        });

        let mut all_tool_calls = Vec::new();
        let mut total_usage = TokenUsage::default();
        let mut iterations = 0;
        let max_iterations = self.config.max_tool_iterations;

        loop {
            // Check idle timeout
            if let Some(last_time) = self.last_tool_call_time {
                let elapsed = last_time.elapsed().as_secs();
                if elapsed > self.config.tool_call_idle_timeout_secs {
                    return Err(PawanError::Agent(format!(
                        "Tool idle timeout exceeded ({}s > {}s)",
                        elapsed, self.config.tool_call_idle_timeout_secs
                    )));
                }
            }

            iterations += 1;
            if iterations > max_iterations {
                return Err(PawanError::Agent(format!(
                    "Max tool iterations ({}) exceeded",
                    max_iterations
                )));
            }

            // Budget nudge + context estimation + pruning
            self.apply_iteration_budgets(iterations, max_iterations).await;

            // Dynamic tool selection: pick the most relevant tools for this query
            let latest_query = self
                .history
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.as_str())
                .unwrap_or("");
            let tool_defs = self.tools.select_for_query(latest_query, 12);
            if iterations == 1 {
                let tool_names: Vec<&str> = tool_defs.iter().map(|t| t.name.as_str()).collect();
                tracing::info!(tools = ?tool_names, count = tool_defs.len(), "Selected tools for query");
            }

            // Update idle timeout tracker before LLM call to track time spent in generation
            self.last_tool_call_time = Some(Instant::now());

            // Resilient LLM call with retry on transient failures
            let response = self.call_llm_with_retry(&tool_defs, on_token.as_ref()).await;

            // Accumulate token usage with thinking/action split
            if let Some(ref usage) = response.usage {
                Self::accumulate_token_usage(
                    usage, &mut total_usage, iterations, self.config.thinking_budget,
                );
            }

            // Strip thinking blocks from content
            let clean_content = Self::strip_thinking_blocks(&response.content);

            if response.tool_calls.is_empty() {
                // Guardrails for no-tool responses; returns true if conversation is complete
                if self
                    .handle_no_tool_response(
                        &clean_content,
                        user_prompt,
                        &tool_defs,
                        iterations,
                        max_iterations,
                        &response.finish_reason,
                    )
                    .await
                {
                    self.history.push(Message {
                        role: Role::Assistant,
                        content: clean_content.clone(),
                        tool_calls: vec![],
                        tool_result: None,
                    });
                    // Persist this completed turn to Eruka
                    if let Some(eruka) = &self.eruka {
                        if let Err(e) = eruka
                            .sync_turn(user_prompt, &clean_content, &self.session_id)
                            .await
                        {
                            tracing::warn!("Eruka sync_turn failed (non-fatal): {}", e);
                        }
                    }
                    return Ok(AgentResponse {
                        content: clean_content,
                        tool_calls: all_tool_calls,
                        iterations,
                        usage: total_usage,
                    });
                }
                continue;
            }

            // Push assistant response to history
            self.history.push(Message {
                role: Role::Assistant,
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
                tool_result: None,
            });

            // Validate permissions, emit start events, partition into pending vs denied
            let (pending, mut ordered_records, mut ordered_tool_messages, mut ordered_compile_gate) =
                self.check_tool_permissions(
                    &response.tool_calls,
                    on_permission.as_ref(),
                    on_tool.as_ref(),
                    on_tool_start.as_ref(),
                )
                .await;

            // Execute pending tools in parallel
            if !pending.is_empty() {
                let results = Self::execute_pending_tools(
                    &self.tools,
                    self.config.bash_timeout_secs,
                    self.config.max_result_chars,
                    pending,
                    on_tool.as_ref(),
                )
                .await;
                for (idx, record, tool_msg, wrote_rs) in results {
                    ordered_records[idx] = Some(record);
                    ordered_tool_messages[idx] = Some(tool_msg);
                    ordered_compile_gate[idx] = wrote_rs;
                }
            }

            // Collect ordered results and run compile gates
            self.collect_tool_results(
                &mut all_tool_calls,
                &mut ordered_records,
                &mut ordered_tool_messages,
                &ordered_compile_gate,
                response.tool_calls.len(),
            )
            .await;
        }
    }
    // ─── Helper functions for execute_with_all_callbacks ───────────────

    /// Inject Eruka core memory and prefetch task-relevant context into history.
    async fn inject_eruka_context(&mut self, user_prompt: &str) {
        if let Some(eruka) = &self.eruka {
            let before_inject = self.history.len();
            if let Err(e) = eruka.inject_core_memory(&mut self.history).await {
                tracing::warn!("Eruka memory injection failed (non-fatal): {}", e);
            }

            for msg in self
                .history
                .iter_mut()
                .skip(before_inject)
                .filter(|m| m.role == Role::System)
            {
                let fenced = prepare_recalled_context("eruka_core_memory", &msg.content);
                if !fenced.is_empty() {
                    msg.content = fenced;
                }
            }

            // Prefetch task-relevant context: semantic search + compressed
            // general context. Inject as a system message so the LLM can
            // draw on prior-session context for the same query. Non-fatal.
            match eruka.prefetch(user_prompt, 2000).await {
                Ok(Some(ctx)) => {
                    let fenced = prepare_recalled_context("eruka_prefetch", &ctx);
                    if !fenced.is_empty() {
                        self.history.push(Message {
                            role: Role::System,
                            content: fenced,
                            tool_calls: vec![],
                            tool_result: None,
                        });
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("Eruka prefetch failed (non-fatal): {}", e),
            }
        }
    }

    /// Prepend workspace architecture context to the user prompt, if available.
    fn build_user_prompt(&self, user_prompt: &str) -> Result<String> {
        if let Some(err) = &self.arch_context_error {
            return Err(PawanError::Config(err.clone()));
        }
        Ok(match &self.arch_context {
            Some(ctx) => format!(
                "[Workspace Architecture]\n{ctx}\n[/Workspace Architecture]\n\n{user_prompt}"
            ),
            None => user_prompt.to_string(),
        })
    }

    /// Nudge the model when running low on tool iterations, estimate context
    /// size, and prune history if it exceeds the configured token budget.
    async fn apply_iteration_budgets(&mut self, iterations: usize, max_iterations: usize) {
        // Budget awareness: when running low on iterations, nudge the model
        let remaining = max_iterations.saturating_sub(iterations);
        if remaining == 3 && iterations > 1 {
            self.history.push(Message {
                role: Role::User,
                content: format!(
                    "[SYSTEM] You have {} tool iterations remaining. \
                     Stop exploring and write the most important output now. \
                     If you have code to write, write it immediately.",
                    remaining
                ),
                tool_calls: vec![],
                tool_result: None,
            });
        }
        // Estimate context tokens
        self.context_tokens_estimate =
            self.history.iter().map(|m| m.content.len()).sum::<usize>() / 4;
        if self.context_tokens_estimate > self.config.max_context_tokens {
            // Snapshot pre-compression content to Eruka so the facts
            // being discarded survive the prune. Non-fatal.
            if let Some(eruka) = &self.eruka {
                let snapshot = Self::history_snapshot_for_eruka(&self.history);
                if let Err(e) = eruka.on_pre_compress(&snapshot, &self.session_id).await {
                    tracing::warn!("Eruka on_pre_compress failed (non-fatal): {}", e);
                }
            }
            self.prune_history();
        }
    }

    /// Call the LLM backend with retry logic for transient failures.
    /// Returns a synthetic error response after exhausting retries.
    async fn call_llm_with_retry(
        &mut self,
        tool_defs: &[thulp_core::ToolDefinition],
        on_token: Option<&TokenCallback>,
    ) -> LLMResponse {
        let max_llm_retries = 3;
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self
                .backend
                .generate(&self.history, tool_defs, on_token)
                .await
            {
                Ok(resp) => return resp,
                Err(e) => {
                    let err_str = e.to_string();
                    let is_transient = err_str.contains("timeout")
                        || err_str.contains("connection")
                        || err_str.contains("429")
                        || err_str.contains("500")
                        || err_str.contains("502")
                        || err_str.contains("503")
                        || err_str.contains("504")
                        || err_str.contains("reset")
                        || err_str.contains("broken pipe");

                    if is_transient && attempt <= max_llm_retries {
                        let delay =
                            std::time::Duration::from_secs(2u64.pow(attempt as u32));
                        tracing::warn!(
                            attempt = attempt,
                            delay_secs = delay.as_secs(),
                            error = err_str.as_str(),
                            "LLM call failed (transient) — retrying"
                        );
                        tokio::time::sleep(delay).await;

                        // If context is too large, prune before retry
                        if err_str.contains("context") || err_str.contains("token") {
                            tracing::info!(
                                "Pruning history before retry (possible context overflow)"
                            );
                            if let Some(eruka) = &self.eruka {
                                let snapshot =
                                    Self::history_snapshot_for_eruka(&self.history);
                                if let Err(e) =
                                    eruka.on_pre_compress(&snapshot, &self.session_id).await
                                {
                                    tracing::warn!(
                                        "Eruka on_pre_compress failed (non-fatal): {}",
                                        e
                                    );
                                }
                            }
                            self.prune_history();
                        }
                        continue;
                    }

                    // Non-transient or max retries exhausted — return synthetic error
                    tracing::error!(
                        attempt = attempt,
                        error = err_str.as_str(),
                        "LLM call failed permanently — returning error as content"
                    );
                    return LLMResponse {
                        content: format!(
                            "LLM error after {} attempts: {}. The task could not be completed.",
                            attempt, err_str
                        ),
                        reasoning: None,
                        tool_calls: vec![],
                        finish_reason: "error".to_string(),
                        usage: None,
                    };
                }
            }
        }
    }

    /// Accumulate token usage from an LLM response and enforce thinking budget.
    fn accumulate_token_usage(
        usage: &TokenUsage,
        total_usage: &mut TokenUsage,
        iterations: usize,
        thinking_budget: usize,
    ) {
        total_usage.prompt_tokens += usage.prompt_tokens;
        total_usage.completion_tokens += usage.completion_tokens;
        total_usage.total_tokens += usage.total_tokens;
        total_usage.reasoning_tokens += usage.reasoning_tokens;
        total_usage.action_tokens += usage.action_tokens;

        // Log token budget split per iteration
        if usage.reasoning_tokens > 0 {
            tracing::info!(
                iteration = iterations,
                think = usage.reasoning_tokens,
                act = usage.action_tokens,
                total = usage.completion_tokens,
                "Token budget: think:{} act:{} (total:{})",
                usage.reasoning_tokens,
                usage.action_tokens,
                usage.completion_tokens
            );
        }

        // Thinking budget enforcement
        if thinking_budget > 0 && usage.reasoning_tokens > thinking_budget as u64 {
            tracing::warn!(
                budget = thinking_budget,
                actual = usage.reasoning_tokens,
                "Thinking budget exceeded ({}/{} tokens)",
                usage.reasoning_tokens,
                thinking_budget
            );
        }
    }

    /// Strip `<think>` blocks from LLM response content.
    fn strip_thinking_blocks(content: &str) -> String {
        let mut s = content.to_string();
        loop {
            let lower = s.to_lowercase();
            let open = lower.find("<think>");
            let close = lower.find("</think>");
            match (open, close) {
                (Some(i), Some(j)) if j > i => {
                    let before = s[..i].trim_end().to_string();
                    let after = if s.len() > j + 8 {
                        s[j + 8..].trim_start().to_string()
                    } else {
                        String::new()
                    };
                    s = if before.is_empty() {
                        after
                    } else if after.is_empty() {
                        before
                    } else {
                        format!("{}\n{}", before, after)
                    };
                }
                _ => break,
            }
        }
        s
    }

    /// Handle guardrails for no-tool-call responses.
    ///
    /// Returns `true` if the conversation is complete (caller should return the
    /// response), `false` if the loop should continue with a correction message.
    async fn handle_no_tool_response(
        &mut self,
        clean_content: &str,
        _user_prompt: &str,
        tool_defs: &[thulp_core::ToolDefinition],
        iterations: usize,
        max_iterations: usize,
        finish_reason: &str,
    ) -> bool {
        // Guardrail: detect chatty no-op (content but no tools on early iterations)
        // Only nudge if tools are available AND response looks like planning text
        let has_tools = !tool_defs.is_empty();
        let lower = clean_content.to_lowercase();
        let planning_prefix = lower.starts_with("let me")
            || lower.starts_with("i'll help")
            || lower.starts_with("i will help")
            || lower.starts_with("sure, i")
            || lower.starts_with("okay, i");
        let looks_like_planning =
            clean_content.len() > 200 || (planning_prefix && clean_content.len() > 50);
        if has_tools
            && looks_like_planning
            && iterations == 1
            && iterations < max_iterations
            && finish_reason != "error"
        {
            tracing::warn!(
                "No tool calls at iteration {} (content: {}B) — nudging model to use tools",
                iterations,
                clean_content.len()
            );
            self.history.push(Message {
                role: Role::Assistant,
                content: clean_content.to_string(),
                tool_calls: vec![],
                tool_result: None,
            });
            self.history.push(Message {
                role: Role::User,
                content: "You must use tools to complete this task. Do NOT just describe what you would do — actually call the tools. Start with bash or read_file.".to_string(),
                tool_calls: vec![],
                tool_result: None,
            });
            return false;
        }

        // Guardrail: detect repeated responses
        if iterations > 1 {
            let prev_assistant = self
                .history
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant && !m.content.is_empty());
            if let Some(prev) = prev_assistant {
                if prev.content.trim() == clean_content.trim()
                    && iterations < max_iterations
                {
                    tracing::warn!(
                        "Repeated response detected at iteration {} — injecting correction",
                        iterations
                    );
                    self.history.push(Message {
                        role: Role::Assistant,
                        content: clean_content.to_string(),
                        tool_calls: vec![],
                        tool_result: None,
                    });
                    self.history.push(Message {
                        role: Role::User,
                        content: "You gave the same response as before. Try a different approach. Use anchor_text in edit_file_lines, or use insert_after, or use bash with sed.".to_string(),
                        tool_calls: vec![],
                        tool_result: None,
                    });
                    return false;
                }
            }
        }

        true
    }

    /// Validate permissions, emit start events, and partition tool calls into
    /// pending (ready to execute) vs denied.
    async fn check_tool_permissions(
        &mut self,
        tool_calls: &[ToolCallRequest],
        on_permission: Option<&PermissionCallback>,
        on_tool: Option<&ToolCallback>,
        on_tool_start: Option<&ToolStartCallback>,
    ) -> (
        Vec<(usize, ToolCallRequest)>,
        Vec<Option<ToolCallRecord>>,
        Vec<Option<Message>>,
        Vec<bool>,
    ) {
        let mut ordered_records: Vec<Option<ToolCallRecord>> =
            vec![None; tool_calls.len()];
        let mut ordered_tool_messages: Vec<Option<Message>> =
            vec![None; tool_calls.len()];
        let ordered_compile_gate: Vec<bool> = vec![false; tool_calls.len()];
        let mut pending: Vec<(usize, ToolCallRequest)> = Vec::new();

        for (idx, tool_call) in tool_calls.iter().cloned().enumerate() {
            self.tools.activate(&tool_call.name);

            let perm = crate::config::ToolPermission::resolve(
                &tool_call.name,
                &self.config.permissions,
            );
            let denied = match perm {
                crate::config::ToolPermission::Deny => Some("Tool denied by permission policy"),
                crate::config::ToolPermission::Prompt => {
                    if tool_call.name == "bash" {
                        if let Some(cmd) =
                            tool_call.arguments.get("command").and_then(|v| v.as_str())
                        {
                            if crate::tools::bash::is_read_only(cmd) {
                                tracing::debug!(command = cmd, "Auto-allowing read-only bash command under Prompt permission");
                                None
                            } else if let Some(ref perm_cb) = on_permission {
                                let args_summary = cmd.chars().take(120).collect::<String>();
                                let rx = perm_cb(PermissionRequest {
                                    tool_name: tool_call.name.clone(),
                                    args_summary,
                                });
                                match rx.await {
                                    Ok(true) => None,
                                    _ => Some("User denied tool execution"),
                                }
                            } else {
                                Some("Bash command requires user approval (read-only commands auto-allowed)")
                            }
                        } else {
                            Some("Tool requires user approval")
                        }
                    } else if let Some(ref perm_cb) = on_permission {
                        let args_summary = tool_call
                            .arguments
                            .to_string()
                            .chars()
                            .take(120)
                            .collect::<String>();
                        let rx = perm_cb(PermissionRequest {
                            tool_name: tool_call.name.clone(),
                            args_summary,
                        });
                        match rx.await {
                            Ok(true) => None,
                            _ => Some("User denied tool execution"),
                        }
                    } else {
                        Some("Tool requires user approval (set permission to allow or use TUI mode)")
                    }
                }
                crate::config::ToolPermission::Allow => None,
            };

            if let Some(reason) = denied {
                let record = ToolCallRecord {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    arguments: tool_call.arguments.clone(),
                    result: json!({"error": reason}),
                    success: false,
                    duration_ms: 0,
                };
                if let Some(ref callback) = on_tool {
                    callback(&record);
                }
                ordered_records[idx] = Some(record);
                ordered_tool_messages[idx] = Some(Message {
                    role: Role::Tool,
                    content: serde_json::to_string(&json!({"error": reason}))
                        .unwrap_or_default(),
                    tool_calls: vec![],
                    tool_result: Some(ToolResultMessage {
                        tool_call_id: tool_call.id.clone(),
                        content: json!({"error": reason}),
                        success: false,
                    }),
                });
                continue;
            }

            if let Some(ref callback) = on_tool_start {
                callback(&tool_call.name);
            }

            if let Some(tool) = self.tools.get(&tool_call.name) {
                let schema = tool.parameters_schema();
                if let Ok(params) = thulp_core::ToolDefinition::parse_mcp_input_schema(&schema)
                {
                    let thulp_def = thulp_core::ToolDefinition {
                        name: tool_call.name.clone(),
                        description: String::new(),
                        parameters: params,
                    };
                    if let Err(e) = thulp_def.validate_args(&tool_call.arguments) {
                        tracing::warn!(tool = tool_call.name.as_str(), error = %e, "Tool argument validation failed (continuing anyway)");
                    }
                }
            }

            let tool = self.tools.get(&tool_call.name);
            let is_mutating = tool.map(|t| t.mutating()).unwrap_or(false);
            if is_mutating {
                if let Some(ref callback) = on_permission {
                    let args_summary = summarize_args(&tool_call.arguments);
                    let request = PermissionRequest {
                        tool_name: tool_call.name.clone(),
                        args_summary,
                    };
                    let permission_rx = (callback)(request);
                    match permission_rx.await {
                        Ok(true) => {}
                        Ok(false) => {
                            let record = ToolCallRecord {
                                id: tool_call.id.clone(),
                                name: tool_call.name.clone(),
                                arguments: tool_call.arguments.clone(),
                                result: json!({"error": "Tool execution denied by user", "tool": tool_call.name}),
                                success: false,
                                duration_ms: 0,
                            };
                            if let Some(ref callback) = on_tool {
                                callback(&record);
                            }
                            ordered_records[idx] = Some(record);
                            ordered_tool_messages[idx] = Some(Message {
                                role: Role::Tool,
                                content: serde_json::to_string(&json!({"error": "Tool execution denied by user", "tool": tool_call.name})).unwrap_or_default(),
                                tool_calls: vec![],
                                tool_result: Some(ToolResultMessage {
                                    tool_call_id: tool_call.id.clone(),
                                    content: json!({"error": "Tool execution denied by user", "tool": tool_call.name}),
                                    success: false,
                                }),
                            });
                            continue;
                        }
                        Err(_) => {
                            let record = ToolCallRecord {
                                id: tool_call.id.clone(),
                                name: tool_call.name.clone(),
                                arguments: tool_call.arguments.clone(),
                                result: json!({"error": "Permission channel closed", "tool": tool_call.name}),
                                success: false,
                                duration_ms: 0,
                            };
                            if let Some(ref callback) = on_tool {
                                callback(&record);
                            }
                            ordered_records[idx] = Some(record);
                            ordered_tool_messages[idx] = Some(Message {
                                role: Role::Tool,
                                content: serde_json::to_string(&json!({"error": "Permission channel closed", "tool": tool_call.name})).unwrap_or_default(),
                                tool_calls: vec![],
                                tool_result: Some(ToolResultMessage {
                                    tool_call_id: tool_call.id.clone(),
                                    content: json!({"error": "Permission channel closed", "tool": tool_call.name}),
                                    success: false,
                                }),
                            });
                            continue;
                        }
                    }
                } else {
                    tracing::warn!(
                        tool = tool_call.name.as_str(),
                        "No permission callback, auto-approving mutating tool"
                    );
                }
            }

            pending.push((idx, tool_call));
        }

        (pending, ordered_records, ordered_tool_messages, ordered_compile_gate)
    }

    /// Execute pending tool calls in parallel with per-tool timeout handling.
    async fn execute_pending_tools(
        tools: &ToolRegistry,
        bash_timeout_secs: u64,
        max_result_chars: usize,
        pending: Vec<(usize, ToolCallRequest)>,
        on_tool: Option<&ToolCallback>,
    ) -> Vec<(usize, ToolCallRecord, Message, bool)> {
        use futures::{stream, StreamExt};

        let on_tool_cb = on_tool;
        let max_parallel = std::cmp::max(1, 10);
        stream::iter(pending)
            .map(|(idx, tool_call)| async move {
                let start = std::time::Instant::now();

                let result = {
                    let tool_future = tools.execute(&tool_call.name, tool_call.arguments.clone());
                    let timeout_dur = if tool_call.name == "bash" {
                        std::time::Duration::from_secs(bash_timeout_secs)
                    } else {
                        std::time::Duration::from_secs(30)
                    };
                    match tokio::time::timeout(timeout_dur, tool_future).await {
                        Ok(inner) => inner,
                        Err(_) => Err(PawanError::Tool(format!(
                            "Tool {} timed out after {}s",
                            tool_call.name,
                            timeout_dur.as_secs()
                        ))),
                    }
                };

                let duration_ms = start.elapsed().as_millis() as u64;
                let (mut result_value, success) = match result {
                    Ok(v) => (v, true),
                    Err(e) => {
                        tracing::warn!(tool = tool_call.name.as_str(), error = %e, "Tool execution failed");
                        (json!({"error": e.to_string(), "tool": tool_call.name, "hint": "Try a different approach or tool"}), false)
                    }
                };

                result_value = truncate_tool_result(result_value, max_result_chars);

                let record = ToolCallRecord {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    arguments: tool_call.arguments.clone(),
                    result: result_value.clone(),
                    success,
                    duration_ms,
                };

                if let Some(ref cb) = on_tool_cb {
                    cb(&record);
                }

                let tool_msg = Message {
                    role: Role::Tool,
                    content: serde_json::to_string(&result_value).unwrap_or_default(),
                    tool_calls: vec![],
                    tool_result: Some(ToolResultMessage {
                        tool_call_id: tool_call.id.clone(),
                        content: result_value,
                        success,
                    }),
                };

                let wrote_rs = success
                    && tool_call.name == "write_file"
                    && tool_call
                        .arguments
                        .get("path")
                        .and_then(|p| p.as_str())
                        .map(|p| p.ends_with(".rs"))
                        .unwrap_or(false);

                (idx, record, tool_msg, wrote_rs)
            })
            .buffer_unordered(max_parallel)
            .collect::<Vec<_>>()
            .await
    }

    /// Collect ordered tool results into the history and all_tool_calls list,
    /// and apply compile gates (cargo check after .rs file writes).
    async fn collect_tool_results(
        &mut self,
        all_tool_calls: &mut Vec<ToolCallRecord>,
        ordered_records: &mut [Option<ToolCallRecord>],
        ordered_tool_messages: &mut [Option<Message>],
        ordered_compile_gate: &[bool],
        tool_calls_count: usize,
    ) {
        for i in 0..tool_calls_count {
            if let Some(record) = ordered_records[i].take() {
                all_tool_calls.push(record);
            }
            if let Some(msg) = ordered_tool_messages[i].take() {
                self.history.push(msg);
            }

            if ordered_compile_gate[i] {
                let ws = self.workspace_root.clone();
                let check_result = tokio::process::Command::new("cargo")
                    .arg("check")
                    .arg("--message-format=short")
                    .current_dir(&ws)
                    .output()
                    .await;
                match check_result {
                    Ok(output) if !output.status.success() => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let err_msg: String = stderr.chars().take(1500).collect();
                        tracing::info!("Compile-gate: cargo check failed after write_file, injecting errors");
                        self.history.push(Message {
                            role: Role::User,
                            content: format!(
                                "[SYSTEM] cargo check failed after your write_file. Fix the errors:\n{}",
                                err_msg
                            ),
                            tool_calls: vec![],
                            tool_result: None,
                        });
                    }
                    Ok(_) => {
                        tracing::debug!("Compile-gate: cargo check passed");
                    }
                    Err(e) => {
                        tracing::warn!("Compile-gate: cargo check failed to run: {}", e);
                    }
                }
            }
        }
    }

    /// Execute using the ToolCoordinator instead of the built-in loop.
    ///
    /// This method provides an alternative implementation that uses the
    /// ToolCoordinator for tool-calling loops, which offers:
    /// - Parallel tool execution
    /// - Per-tool timeout handling
    /// - Consistent error handling
    /// - Max iteration limits
    ///
    /// Note: This method does not support streaming callbacks or permission
    /// prompts - those are only available in the built-in loop.
    async fn execute_with_coordinator(&mut self, user_prompt: &str) -> Result<AgentResponse> {
        // Reset idle timeout for the new turn
        self.last_tool_call_time = None;

        // Inject Eruka core memory before first LLM call
        if let Some(eruka) = &self.eruka {
            let before_inject = self.history.len();
            if let Err(e) = eruka.inject_core_memory(&mut self.history).await {
                tracing::warn!("Eruka memory injection failed (non-fatal): {}", e);
            }

            for msg in self
                .history
                .iter_mut()
                .skip(before_inject)
                .filter(|m| m.role == Role::System)
            {
                let fenced = prepare_recalled_context("eruka_core_memory", &msg.content);
                if !fenced.is_empty() {
                    msg.content = fenced;
                }
            }

            // Prefetch task-relevant context
            match eruka.prefetch(user_prompt, 2000).await {
                Ok(Some(ctx)) => {
                    let fenced = prepare_recalled_context("eruka_prefetch", &ctx);
                    if !fenced.is_empty() {
                        self.history.push(Message {
                            role: Role::System,
                            content: fenced,
                            tool_calls: vec![],
                            tool_result: None,
                        });
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("Eruka prefetch failed (non-fatal): {}", e),
            }
        }

        // Per-turn architecture context injection

        if let Some(err) = &self.arch_context_error {
            return Err(PawanError::Config(err.clone()));
        }

        let effective_prompt = match &self.arch_context {
            Some(ctx) => format!(
                "[Workspace Architecture]\n{ctx}\n[/Workspace Architecture]\n\n{user_prompt}"
            ),
            None => user_prompt.to_string(),
        };

        // Build coordinator config from agent config
        let coordinator_config = ToolCallingConfig {
            max_iterations: self.config.max_tool_iterations,
            parallel_execution: true,
            max_parallel_tools: 10,
            tool_timeout: std::time::Duration::from_secs(self.config.bash_timeout_secs),
            stop_on_error: false,
        };

        // Create a fresh backend for coordinator execution
        let system_prompt = self.config.get_system_prompt_checked()?;
        let backend = Self::create_backend(&self.config, &system_prompt);
        let backend = Arc::from(backend);

        // Create a fresh tool registry for coordinator execution
        // Note: This will not include any MCP tools registered at runtime
        let registry = Arc::new(ToolRegistry::with_defaults(self.workspace_root.clone()));

        // Create coordinator with backend and tool registry
        let coordinator = ToolCoordinator::new(backend, registry, coordinator_config);

        // Execute with coordinator
        let result: CoordinatorResult = coordinator
            .execute(Some(&system_prompt), &effective_prompt)
            .await
            .map_err(|e| PawanError::Agent(format!("Coordinator execution failed: {}", e)))?;

        // Convert CoordinatorResult to AgentResponse
        let content = result.content.clone();
        let agent_response = AgentResponse {
            content: result.content,
            tool_calls: result.tool_calls,
            iterations: result.iterations,
            usage: result.total_usage,
        };

        // Sync turn to Eruka if enabled
        if let Some(eruka) = &self.eruka {
            if let Err(e) = eruka
                .sync_turn(user_prompt, &content, &self.session_id)
                .await
            {
                tracing::warn!("Eruka sync_turn failed (non-fatal): {}", e);
            }
        }

        Ok(agent_response)
    }

    /// Execute a healing task with real diagnostics
    pub async fn heal(&mut self) -> Result<AgentResponse> {
        let healer =
            crate::healing::Healer::new(self.workspace_root.clone(), self.config.healing.clone());

        let diagnostics = healer.get_diagnostics().await?;
        let failed_tests = healer.get_failed_tests().await?;

        let mut prompt = format!(
            "I need you to heal this Rust project at: {}

",
            self.workspace_root.display()
        );

        if !diagnostics.is_empty() {
            prompt.push_str(&format!(
                "## Compilation Issues ({} found)
{}
",
                diagnostics.len(),
                healer.format_diagnostics_for_prompt(&diagnostics)
            ));
        }

        if !failed_tests.is_empty() {
            prompt.push_str(&format!(
                "## Failed Tests ({} found)
{}
",
                failed_tests.len(),
                healer.format_tests_for_prompt(&failed_tests)
            ));
        }

        if diagnostics.is_empty() && failed_tests.is_empty() {
            prompt.push_str(
                "No issues found! Run cargo check and cargo test to verify.
",
            );
        }

        prompt.push_str(
            "
Fix each issue one at a time. Verify with cargo check after each fix.",
        );

        self.execute(&prompt).await
    }
    /// Execute healing with retries — calls heal(), checks for remaining errors, retries if needed.
    ///
    /// Two-stage gate:
    ///   Stage 1 — `cargo check`: must produce zero errors before proceeding.
    ///   Stage 2 — `healing.verify_cmd` (optional): a user-supplied shell command
    ///             (e.g. `cargo test --workspace`).  If it exits non-zero the loop
    ///             continues so the LLM can address the reported failures.
    ///
    /// Anti-thrash guard: each Stage-1 error is fingerprinted (kind + code +
    /// message prefix).  If the same fingerprint survives `max_attempts`
    /// consecutive rounds unchanged the loop halts rather than spinning
    /// indefinitely on an error the LLM cannot fix.
    pub async fn heal_with_retries(&mut self, max_attempts: usize) -> Result<AgentResponse> {
        use std::collections::{HashMap, HashSet};

        let mut last_response = self.heal().await?;
        // fingerprint → consecutive rounds this error has survived unchanged
        let mut stuck_counts: HashMap<u64, usize> = HashMap::new();

        for attempt in 1..max_attempts {
            // Stage 1: cargo check must be error-free
            let fixer = crate::healing::CompilerFixer::new(self.workspace_root.clone());
            let remaining = fixer.check().await?;
            let errors: Vec<_> = remaining
                .iter()
                .filter(|d| d.kind == crate::healing::DiagnosticKind::Error)
                .collect();

            if !errors.is_empty() {
                // Update fingerprint counts.
                // Drop entries for errors that were fixed; increment survivors.
                let current_fps: HashSet<u64> = errors.iter().map(|d| d.fingerprint()).collect();
                stuck_counts.retain(|fp, _| current_fps.contains(fp));
                for fp in &current_fps {
                    *stuck_counts.entry(*fp).or_insert(0) += 1;
                }

                // Anti-thrash: halt if any error fingerprint has not budged
                // after max_attempts consecutive rounds.
                let thrashing: Vec<u64> = stuck_counts
                    .iter()
                    .filter_map(|(&fp, &count)| {
                        if count >= max_attempts {
                            Some(fp)
                        } else {
                            None
                        }
                    })
                    .collect();
                if !thrashing.is_empty() {
                    tracing::warn!(
                        stuck_fingerprints = thrashing.len(),
                        attempt,
                        "Anti-thrash: {} error(s) unchanged after {} attempts, halting heal loop",
                        thrashing.len(),
                        max_attempts
                    );
                    return Ok(last_response);
                }

                tracing::warn!(
                    errors = errors.len(),
                    attempt,
                    "Stage 1 (cargo check): errors remain, retrying"
                );
                last_response = self.heal().await?;
                continue;
            }

            // All Stage-1 errors cleared — reset thrash counters.
            stuck_counts.clear();

            // Stage 2: optional verify_cmd
            let verify_cmd = self.config.healing.verify_cmd.clone();
            if let Some(ref cmd) = verify_cmd {
                match crate::healing::run_verify_cmd(&self.workspace_root, cmd).await {
                    Ok(None) => {
                        tracing::info!(
                            attempts = attempt,
                            "Stage 2 (verify_cmd) passed, healing complete"
                        );
                        return Ok(last_response);
                    }
                    Ok(Some(diag)) => {
                        tracing::warn!(
                            attempt,
                            cmd,
                            output = diag.raw,
                            "Stage 2 (verify_cmd) failed, retrying"
                        );
                        last_response = self.heal().await?;
                        continue;
                    }
                    Err(e) => {
                        // Cannot spawn the command — don't block healing on this
                        tracing::warn!(cmd, error = %e, "verify_cmd could not be run, skipping stage 2");
                        return Ok(last_response);
                    }
                }
            } else {
                tracing::info!(
                    attempts = attempt,
                    "Stage 1 (cargo check) passed, healing complete"
                );
                return Ok(last_response);
            }
        }

        tracing::info!(
            attempts = max_attempts,
            "Healing finished (may still have errors)"
        );
        Ok(last_response)
    }
    /// Execute a task with a specific prompt
    pub async fn task(&mut self, task_description: &str) -> Result<AgentResponse> {
        let prompt = format!(
            r#"I need you to complete the following coding task:

{}

The workspace is at: {}

Please:
1. First explore the codebase to understand the relevant code
2. Make the necessary changes
3. Verify the changes compile with `cargo check`
4. Run relevant tests if applicable

Explain your changes as you go."#,
            task_description,
            self.workspace_root.display()
        );

        self.execute(&prompt).await
    }

    /// Generate a commit message for current changes
    pub async fn generate_commit_message(&mut self) -> Result<String> {
        let prompt = r#"Please:
1. Run `git status` to see what files are changed
2. Run `git diff --cached` to see staged changes (or `git diff` for unstaged)
3. Generate a concise, descriptive commit message following conventional commits format

Only output the suggested commit message, nothing else."#;

        let response = self.execute(prompt).await?;
        Ok(response.content)
    }
}
