//! Session conversion, export, handoff, and autosave.

use pawan::agent::session::Session;
use pawan::agent::{Message, Role, ToolCallRecord, ToolCallRequest};
use std::time::Instant;

use super::state::App;
use super::types::*;

impl<'a> App<'a> {
    pub(crate) fn messages_from_session(messages: Vec<Message>) -> Vec<DisplayMessage> {
        messages
            .into_iter()
            .map(|msg| {
                let mut blocks = Vec::new();
                if !msg.content.is_empty() {
                    blocks.push(ContentBlock::Text {
                        content: msg.content.clone(),
                        streaming: false,
                    });
                }
                for tc in &msg.tool_calls {
                    blocks.push(ContentBlock::ToolCall {
                        name: tc.name.clone(),
                        args_summary: summarize_args(&tc.arguments),
                        state: Box::new(ToolBlockState::Running),
                    });
                }
                if let Some(tr) = msg.tool_result {
                    let record = ToolCallRecord {
                        id: String::new(),
                        name: String::new(),
                        arguments: serde_json::Value::Null,
                        result: tr.content.clone(),
                        success: tr.success,
                        duration_ms: 0,
                    };
                    blocks.push(ContentBlock::ToolCall {
                        name: String::new(),
                        args_summary: String::new(),
                        state: Box::new(ToolBlockState::Done {
                            record,
                            expanded: true,
                        }),
                    });
                }
                DisplayMessage {
                    role: msg.role.clone(),
                    blocks,
                    timestamp: std::time::Instant::now(),
                    cached_block_lines: None,
                }
            })
            .collect()
    }

    pub(crate) fn export_conversation(
        &self,
        path: &str,
        format: ExportFormat,
    ) -> std::result::Result<usize, String> {
        match format {
            ExportFormat::Markdown => self.export_as_markdown(path),
            ExportFormat::Html => self.export_as_html(path),
            ExportFormat::Json => self.export_as_json(path),
            ExportFormat::Txt => self.export_as_txt(path),
        }
    }

    pub(crate) fn export_as_markdown(&self, path: &str) -> std::result::Result<usize, String> {
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        writeln!(f, "# Pawan Session\n").map_err(|e| e.to_string())?;
        writeln!(f, "**Model:** {}\n", self.model_name).map_err(|e| e.to_string())?;
        for msg in &self.messages {
            let role = match msg.role {
                Role::User => "**You**",
                Role::Assistant => "**Pawan**",
                _ => "**System**",
            };
            writeln!(f, "### {}\n", role).map_err(|e| e.to_string())?;
            writeln!(f, "{}\n", msg.text_content()).map_err(|e| e.to_string())?;
            let tool_records = msg.tool_records();
            if !tool_records.is_empty() {
                writeln!(
                    f,
                    "<details><summary>Tool calls ({})</summary>\n",
                    tool_records.len()
                )
                .map_err(|e| e.to_string())?;
                for tc in tool_records {
                    let status = if tc.success { "ok" } else { "err" };
                    writeln!(f, "- `{}` ({}) — {}ms", tc.name, status, tc.duration_ms)
                        .map_err(|e| e.to_string())?;
                    // Include arguments if available
                    if let Some(args) = tc.arguments.as_object() {
                        if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                            writeln!(f, "  - Command: `{}`", cmd).map_err(|e| e.to_string())?;
                        }
                    }
                    // Include result if available
                    if let Some(result_str) = tc.result.as_str() {
                        writeln!(f, "  - Result: {}", result_str).map_err(|e| e.to_string())?;
                    }
                }
                writeln!(f, "\n</details>\n").map_err(|e| e.to_string())?;
            }
        }
        writeln!(
            f,
            "---\n*Tokens: {} total ({} prompt, {} completion)*",
            self.total_tokens, self.total_prompt_tokens, self.total_completion_tokens
        )
        .map_err(|e| e.to_string())?;
        Ok(self.messages.len())
    }

    pub(crate) fn export_as_html(&self, path: &str) -> std::result::Result<usize, String> {
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        write!(f, "{}", Self::build_html_header(&self.model_name))
            .map_err(|e| e.to_string())?;
        for msg in &self.messages {
            write!(f, "{}", Self::render_message_html(msg))
                .map_err(|e| e.to_string())?;
        }
        write!(
            f,
            "{}",
            Self::build_html_footer(
                self.total_tokens,
                self.total_prompt_tokens,
                self.total_completion_tokens,
            )
        )
        .map_err(|e| e.to_string())?;
        Ok(self.messages.len())
    }
    pub(crate) fn export_as_json(&self, path: &str) -> std::result::Result<usize, String> {
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut output = serde_json::json!({
            "model": self.model_name,
            "total_tokens": self.total_tokens,
            "prompt_tokens": self.total_prompt_tokens,
            "completion_tokens": self.total_completion_tokens,
            "messages": []
        });
        for msg in &self.messages {
            let msg_obj = serde_json::json!({
                "role": format!("{:?}", msg.role),
                "content": msg.text_content(),
                "tool_calls": msg.tool_records().iter()
                    .map(|tc| serde_json::json!({
                        "name": tc.name,
                        "success": tc.success,
                        "duration_ms": tc.duration_ms,
                    }))
                    .collect::<Vec<_>>(),
            });
            if let Some(messages) = output.get_mut("messages") {
                if let Some(messages_array) = messages.as_array_mut() {
                    messages_array.push(msg_obj);
                }
            }
        }
        writeln!(
            f,
            "{}",
            serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
        Ok(self.messages.len())
    }

    pub(crate) fn export_as_txt(&self, path: &str) -> std::result::Result<usize, String> {
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        writeln!(f, "Pawan Session\n").map_err(|e| e.to_string())?;
        writeln!(f, "Model: {}\n", self.model_name).map_err(|e| e.to_string())?;
        for msg in &self.messages {
            let role = match msg.role {
                Role::User => "You",
                Role::Assistant => "Pawan",
                _ => "System",
            };
            writeln!(f, "[{}]\n", role).map_err(|e| e.to_string())?;
            writeln!(f, "{}\n", msg.text_content()).map_err(|e| e.to_string())?;
            let tool_records = msg.tool_records();
            if !tool_records.is_empty() {
                writeln!(f, "Tool calls ({}):\n", tool_records.len()).map_err(|e| e.to_string())?;
                for tc in tool_records {
                    let status = if tc.success { "ok" } else { "err" };
                    writeln!(f, "  - {} ({}) — {}ms\n", tc.name, status, tc.duration_ms)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        writeln!(
            f,
            "---\nTokens: {} total ({} prompt, {} completion)\n",
            self.total_tokens, self.total_prompt_tokens, self.total_completion_tokens
        )
        .map_err(|e| e.to_string())?;
        Ok(self.messages.len())
    }

    /// Helper function to escape HTML special characters
    pub(crate) fn html_escape(s: &str) -> String {
        s.replace("&", "&amp;")
            .replace("<", "&lt;")
            .replace(">", "&gt;")
            .replace("\"", "&quot;")
    }
    // ── HTML export helpers ──────────────────────────────────────────────

    /// Produces the HTML document head, styles, and opening body with title.
    fn build_html_header(model_name: &str) -> String {
        format!(
            "<!DOCTYPE html>\n\
             <html lang='en'>\n\
             <head>\n\
             <meta charset='UTF-8'>\n\
             <meta name='viewport' content='width=device-width, initial-scale=1.0'>\n\
             <title>Pawan Session</title>\n\
             <style>\n\
             body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; line-height: 1.6; }}\n\
             .message {{ margin: 20px 0; padding: 15px; border-radius: 8px; }}\n\
             .user {{ background-color: #e3f2fd; }}\n\
             .assistant {{ background-color: #f3e5f5; }}\n\
             .system {{ background-color: #f5f5f5; }}\n\
             .role {{ font-weight: bold; margin-bottom: 10px; }}\n\
             .content {{ white-space: pre-wrap; }}\n\
             .tool-calls {{ margin-top: 10px; padding: 10px; background-color: #fff3cd; border-radius: 4px; }}\n\
             .footer {{ margin-top: 30px; padding-top: 20px; border-top: 1px solid #ddd; color: #666; }}\n\
             </style>\n\
             </head>\n\
             <body>\n\
             <h1>Pawan Session</h1>\n\
             <p><strong>Model:</strong> {}</p>\n",
            model_name
        )
    }

    /// Closes the HTML document with a token-usage footer.
    fn build_html_footer(total_tokens: u64, prompt_tokens: u64, completion_tokens: u64) -> String {
        format!(
            "<div class='footer'>\n\
             Tokens: {} total ({} prompt, {} completion)\n\
             </div>\n\
             </body>\n\
             </html>\n",
            total_tokens, prompt_tokens, completion_tokens
        )
    }

    /// Renders a single message as an HTML div block.
    fn render_message_html(msg: &DisplayMessage) -> String {
        let class = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            _ => "system",
        };
        let role_name = match msg.role {
            Role::User => "You",
            Role::Assistant => "Pawan",
            _ => "System",
        };
        let tool_records = msg.tool_records();
        let tool_html = if tool_records.is_empty() {
            String::new()
        } else {
            Self::render_tool_calls_html(&tool_records)
        };
        format!(
            "  <div class='message {}'>\n\
             <div class='role'>{}</div>\n\
             <div class='content'>{}</div>\n\
             {}  </div>\n",
            class,
            role_name,
            Self::html_escape(&msg.text_content()),
            tool_html
        )
    }

    /// Renders tool call records as an HTML block.
    fn render_tool_calls_html(tool_records: &[&ToolCallRecord]) -> String {
        let mut out = format!(
            "    <div class='tool-calls'>\n\
             <strong>Tool calls ({}):</strong>\n",
            tool_records.len()
        );
        for tc in tool_records {
            let status = if tc.success { "✓" } else { "✗" };
            out.push_str(&format!(
                "    {} `{}` — {}ms\n",
                status, tc.name, tc.duration_ms
            ));
        }
        out.push_str("    </div>\n");
        out
    }

    // ── Handoff prompt helpers ────────────────────────────────────────────

    /// Extracts the last `count` messages as role/content preview pairs.
    fn extract_recent_context(messages: &[DisplayMessage], count: usize) -> Vec<String> {
        let recent_count = messages.len().min(count);
        messages
            .iter()
            .rev()
            .take(recent_count)
            .rev()
            .map(|msg| {
                let role = match msg.role {
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                    _ => "System",
                };
                let content = msg.text_content();
                let preview = if content.len() > 200 {
                    format!("{}...", &content[..200])
                } else {
                    content
                };
                format!("**{}:** {}", role, preview)
            })
            .collect()
    }

    /// Parses message content for file path references.
    fn extract_mentioned_files(messages: &[DisplayMessage]) -> Vec<String> {
        use std::collections::HashSet;
        let mut file_paths = HashSet::new();

        for msg in messages {
            let content = msg.text_content();
            for line in content.lines() {
                if line.contains(".rs")
                    || line.contains(".ts")
                    || line.contains(".js")
                    || line.contains(".py")
                    || line.contains(".go")
                    || line.contains(".java")
                    || (line.contains("/")
                        && (line.contains("src") || line.contains("lib") || line.contains("test")))
                {
                    for word in line.split_whitespace() {
                        if word.ends_with(".rs")
                            || word.ends_with(".ts")
                            || word.ends_with(".js")
                            || word.ends_with(".py")
                            || word.ends_with(".go")
                            || word.ends_with(".java")
                            || (word.contains("/")
                                && (word.contains("src") || word.contains("lib")))
                        {
                            file_paths.insert(
                                word.trim_matches(['\"', '\'', '(', ')', ',', ':'])
                                    .to_string(),
                            );
                        }
                    }
                }
            }
        }

        let mut paths: Vec<_> = file_paths.into_iter().collect();
        paths.sort();
        paths
    }

    /// Finds TODO/task items in messages.
    fn extract_task_items(messages: &[DisplayMessage]) -> Vec<String> {
        let mut tasks = Vec::new();
        for msg in messages {
            let content = msg.text_content();
            for line in content.lines() {
                if line.starts_with("-")
                    || line.starts_with("*")
                    || line.contains("TODO")
                    || line.contains("implement")
                    || line.contains("fix")
                    || line.contains("add")
                    || line.contains("create")
                {
                    tasks.push(line.trim().to_string());
                }
            }
        }
        tasks
    }

    /// Finds constraint/rules mentions in messages.
    fn extract_constraints(messages: &[DisplayMessage]) -> Vec<String> {
        let mut constraints = Vec::new();
        for msg in messages {
            let content = msg.text_content();
            for line in content.lines() {
                if line.contains("MUST")
                    || line.contains("MUST NOT")
                    || line.contains("should")
                    || line.contains("constraint")
                    || line.contains("requirement")
                {
                    constraints.push(line.trim().to_string());
                }
            }
        }
        constraints
    }

    /// Assembles the handoff prompt header.
    fn build_prompt_header(
        model_name: &str,
        msg_count: usize,
        tool_calls: u32,
        files_edited: u32,
    ) -> Vec<String> {
        vec![
            "# Session Handoff".to_string(),
            String::new(),
            format!("**Model:** {}", model_name),
            format!("**Messages:** {}", msg_count),
            format!("**Tool calls:** {}", tool_calls),
            format!("**Files edited:** {}", files_edited),
            String::new(),
        ]
    }

    // ── Composed handoff prompt ───────────────────────────────────────────

    /// This strips noise while preserving file paths, constraints, and key context
    pub(crate) fn generate_handoff_prompt(&self) -> String {
        if self.messages.is_empty() {
            return "No conversation context available.".to_string();
        }

        let file_paths = Self::extract_mentioned_files(&self.messages);
        let constraints = Self::extract_constraints(&self.messages);
        let key_tasks = Self::extract_task_items(&self.messages);

        let mut context_parts = Self::build_prompt_header(
            &self.model_name,
            self.messages.len(),
            self.session_tool_calls,
            self.session_files_edited,
        );

        if !file_paths.is_empty() {
            context_parts.push("## Files Referenced".to_string());
            for path in &file_paths {
                context_parts.push(format!("- {}", path));
            }
            context_parts.push(String::new());
        }

        if !constraints.is_empty() {
            context_parts.push("## Constraints".to_string());
            for constraint in constraints.iter().take(10) {
                context_parts.push(format!("- {}", constraint));
            }
            if constraints.len() > 10 {
                context_parts.push(format!("- ... and {} more", constraints.len() - 10));
            }
            context_parts.push(String::new());
        }

        if !key_tasks.is_empty() {
            context_parts.push("## Key Tasks".to_string());
            for task in key_tasks.iter().take(15) {
                context_parts.push(format!("- {}", task));
            }
            if key_tasks.len() > 15 {
                context_parts.push(format!("- ... and {} more", key_tasks.len() - 15));
            }
            context_parts.push(String::new());
        }

        context_parts.push("## Recent Context".to_string());
        for entry in Self::extract_recent_context(&self.messages, 3) {
            context_parts.push(entry);
        }

        context_parts.join("\n")
    }
    pub(crate) fn autosave(&mut self) {
        // Only autosave if there are messages to save
        if self.messages.is_empty() {
            return;
        }

        // Create or update session
        let mut session = if let Some(ref session_id) = self.current_session_id {
            // Load existing session and update it
            match Session::load(session_id) {
                Ok(mut s) => {
                    // Preserve existing metadata
                    s.model = self.model_name.clone();
                    s.tags = self.session_tags.clone();
                    s
                }
                Err(_) => {
                    // If load fails, create new session with same ID
                    Session::new_with_id(
                        session_id.clone(),
                        &self.model_name,
                        self.session_tags.clone(),
                    )
                }
            }
        } else {
            // No current session, create new one
            let new_session = Session::new_with_tags(&self.model_name, self.session_tags.clone());
            self.current_session_id = Some(new_session.id.clone());
            new_session
        };

        // Convert DisplayMessage -> Message, extracting tool calls from blocks
        session.messages.clear();
        for dm in &self.messages {
            // Extract text content from blocks
            let mut text_content = String::new();
            let mut tool_calls = Vec::new();

            for block in &dm.blocks {
                match block {
                    ContentBlock::Text { content, .. } => {
                        if !text_content.is_empty() {
                            text_content.push('\n');
                        }
                        text_content.push_str(content.as_str());
                    }
                    ContentBlock::ToolCall { state, .. } => {
                        if let ToolBlockState::Done { ref record, .. } = state.as_ref() {
                            tool_calls.push(ToolCallRequest {
                                id: record.id.clone(),
                                name: record.name.clone(),
                                arguments: record.arguments.clone(),
                            });
                        }
                    }
                }
            }

            // Add message if non-empty content or has tool calls
            let has_content = !text_content.trim().is_empty();
            if has_content || !tool_calls.is_empty() {
                session.messages.push(Message {
                    role: dm.role.clone(),
                    content: text_content,
                    tool_calls,
                    tool_result: None,
                });
            }
        }

        // Save session
        match session.save() {
            Ok(path) => {
                eprintln!("Autosaved session to {}", path.display());
            }
            Err(e) => {
                eprintln!("Autosave failed: {}", e);
            }
        }
    }
}
