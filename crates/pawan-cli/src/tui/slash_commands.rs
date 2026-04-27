//! Slash command entrypoints and fuzzy catalog helpers.

#![allow(unused_imports)]

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pawan::agent::session::{RetentionPolicy, SearchResult, Session, SessionSummary};
use pawan::agent::{AgentResponse, Message, PawanAgent, Role, ToolCallRecord, ToolCallRequest};
use pawan::config::TuiConfig;
use pawan::{PawanError, Result};
use ratatui::style::Style;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use ratatui_textarea::{Input, TextArea};
use regex::Regex;
use std::io::{self, Stdout};
use std::sync::OnceLock;
use std::time::Instant;
use tokio::sync::mpsc;

use super::app::{App, SlashCommand, SlashCommandRegistry};
use super::types::*;

impl<'a> App<'a> {
    /// Shared entry used by all registered `SlashCommand.handler` pointers
    pub fn universal_slash_entry(app: &mut App<'_>, args: &[&str]) -> Result<()> {
        let c: String = app
            .slash_inflight
            .as_deref()
            .ok_or_else(|| PawanError::Agent("internal: missing slash context".to_string()))?
            .to_string();
        let a = args.first().copied().unwrap_or("");
        app.slash_route(&c, a);
        Ok(())
    }

    /// Handle slash commands locally without sending to the agent
    pub(crate) fn handle_slash_command(&mut self, cmd: &str) {
        let s = cmd.trim();
        let normalized: String = if let Some(rest) = s.strip_prefix(':') {
            if rest.is_empty() {
                "/".to_string()
            } else {
                format!("/{rest}")
            }
        } else {
            s.to_string()
        };
        let parts: Vec<&str> = normalized.splitn(2, ' ').collect();
        let command = parts[0];
        let arg = parts.get(1).map(|x| x.trim()).unwrap_or("");

        if self.slash_registry.get(command).is_none() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!(
                    "Unknown command: {}. Type /help for available commands.",
                    command
                ),
            ));
            return;
        }

        if let Some(sc) = self.slash_registry.get(command) {
            self.slash_inflight = Some(sc.name.clone());
            let a: [&str; 1] = [arg];
            let sargs: &[&str] = if arg.is_empty() { &[] } else { &a };
            let res = (sc.handler)(self, sargs);
            self.slash_inflight = None;
            if let Err(e) = res {
                self.messages
                    .push(DisplayMessage::new_text(Role::System, e.to_string()));
            }
        }
    }

    pub(crate) fn slash_route(&mut self, command: &str, arg: &str) {
        match command {
            "/clear" | "/c" => {
                self.messages.clear();
                self.status = "Cleared".to_string();
            }
            "/model" | "/m" => {
                if arg.is_empty() {
                    // Open visual model selector
                    self.load_available_models();
                    self.model_picker.visible = true;
                    self.model_picker.query.clear();
                    self.model_picker.selected = 0;
                } else {
                    self.switch_model(arg.to_string());
                }
            }
            "/tools" | "/t" => {
                self.messages.push(DisplayMessage::new_text(Role::System,
                    "Core: bash, read_file, write_file, edit_file, ast_grep, glob_search, grep_search\n\
                     Standard: git (status/diff/add/commit/log/blame/branch/checkout/stash), agents, edit modes\n\
                     Extended: rg, fd, sd, tree, mise, zoxide, lsp\n\
                     MCP: mcp_daedra_web_search, mcp_daedra_visit_page"));
            }
            "/search" | "/s" => {
                if arg.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Usage: /search <query>",
                    ));
                } else {
                    let search_prompt = format!(
                        "Use mcp_daedra_web_search to search for '{}' and report the results",
                        arg
                    );
                    self.messages.push(DisplayMessage::new_text(
                        Role::User,
                        format!("/search {}", arg),
                    ));
                    self.processing = true;
                    self.status = format!("Searching: {}", arg);
                    let _ = self.cmd_tx.send(AgentCommand::Execute(search_prompt));
                }
            }
            "/handoff" => {
                if self.messages.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "No conversation to handoff. Start chatting first.",
                    ));
                    self.status = "Nothing to handoff".to_string();
                } else {
                    let handoff_prompt = self.generate_handoff_prompt();
                    self.messages.clear();
                    self.scroll = 0;
                    self.messages
                        .push(DisplayMessage::new_text(Role::System, handoff_prompt));
                    self.status = "Handoff complete".to_string();
                }
            }
            "/heal" | "/h" => {
                self.messages
                    .push(DisplayMessage::new_text(Role::User, "/heal"));
                self.processing = true;
                self.status = "Healing...".to_string();
                let _ = self.cmd_tx.send(AgentCommand::Execute(
                    "Run cargo check and cargo test. Fix any errors you find.".to_string(),
                ));
            }
            "/quit" | "/q" | "/exit" => {
                self.should_quit = true;
            }
            "/export" | "/e" => {
                let (path, format) = if arg.contains("--format") {
                    let parts: Vec<&str> = arg.splitn(3, ' ').collect();
                    let format_str = parts.get(2).unwrap_or(&"md");
                    let path = parts.get(1).unwrap_or(&"pawan-session");
                    (path.to_string(), ExportFormat::from_str(format_str))
                } else if arg.is_empty() {
                    ("pawan-session.md".to_string(), ExportFormat::Markdown)
                } else {
                    (arg.to_string(), ExportFormat::Markdown)
                };
                match self.export_conversation(&path, format) {
                    Ok(n) => self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        format!("Exported {} messages to {}", n, path),
                    )),
                    Err(e) => self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        format!("Export failed: {}", e),
                    )),
                }
            }
            "/diff" | "/d" => {
                // Show git diff for current directory
                use std::process::Command;
                // Parse optional '--cached' flag and optional path argument
                let mut diff_path = ".";
                let mut cached = false;
                if !arg.is_empty() {
                    for token in arg.split_whitespace() {
                        if token == "--cached" {
                            cached = true;
                        } else {
                            diff_path = token;
                        }
                    }
                }
                let diff_arg = diff_path;
                let mut git_args = vec!["diff"];
                if cached {
                    git_args.push("--cached");
                }
                git_args.push(diff_path);
                let output = Command::new("git").args(&git_args).output();
                match output {
                    Ok(out) => {
                        let diff_output = String::from_utf8_lossy(&out.stdout);
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        if diff_output.is_empty() && stderr.is_empty() {
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                "No changes detected".to_string(),
                            ));
                        } else if !diff_output.is_empty() {
                            let raw_lines: Vec<&str> = diff_output.lines().take(100).collect();
                            let colored_lines: Vec<String> = raw_lines
                                .iter()
                                .map(|line| {
                                    if line.starts_with('+') && !line.starts_with("+++") {
                                        format!("\x1b[32m{}\x1b[0m", line)
                                    } else if line.starts_with('-') && !line.starts_with("---") {
                                        format!("\x1b[31m{}\x1b[0m", line)
                                    } else {
                                        (*line).to_string()
                                    }
                                })
                                .collect();
                            let preview = colored_lines.join("\n");
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!("Git diff for {}:\n\n{}", diff_arg, preview),
                            ));
                            if colored_lines.len() >= 100 {
                                self.messages.push(DisplayMessage::new_text(
                                    Role::System,
                                    "... (truncated)".to_string(),
                                ));
                                self.messages.push(DisplayMessage::new_text(
                                    Role::System,
                                    "... (truncated)".to_string(),
                                ));
                            }
                        } else if !stderr.is_empty() {
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!("Git diff error: {}", stderr),
                            ));
                        }
                    }
                    Err(e) => self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        format!("Failed to run git diff: {}", e),
                    )),
                }
            }

            "/import" => {
                if arg.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Usage: /import <path> - import session from JSON file".to_string(),
                    ));
                } else {
                    match Session::from_json_file(arg) {
                        Ok(mut session) => {
                            // Capture message count before moving
                            let msg_count = session.messages.len();
                            let model_name = session.model.clone();
                            let session_id = session.id.clone();
                            // Load session properties into TUI
                            self.model_name = session.model.clone();
                            self.session_tags = session.tags.clone();
                            self.current_session_id = Some(session.id.clone());
                            self.status = format!("Imported session: {}", session_id);
                            // Save to session directory with new UUID
                            match session.save() {
                                Ok(_) => self.messages.push(DisplayMessage::new_text(
                                    Role::System,
                                    format!(
                                        "Imported session from {} as {} (model: {}, {} messages)",
                                        arg, session_id, model_name, msg_count
                                    ),
                                )),
                                Err(e) => self.messages.push(DisplayMessage::new_text(
                                    Role::System,
                                    format!("Failed to save imported session: {}", e),
                                )),
                            }
                            // Convert messages after save (since save() needs the full session)
                            self.messages = App::messages_from_session(session.messages);
                        }
                        Err(e) => self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Failed to import session: {}", e),
                        )),
                    }
                }
            }

            "/fork" => {
                // Fork: create a new session with current messages and switch to it
                if self.messages.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "No conversation to fork. Start chatting first.",
                    ));
                    self.status = "Nothing to fork".to_string();
                } else {
                    let mut new_session =
                        Session::new_with_tags(&self.model_name, self.session_tags.clone());
                    new_session.total_tokens = self.total_tokens;
                    new_session.iteration_count = self.iteration_count;
                    for dm in &self.messages {
                        let mut text_content = String::new();
                        let mut tool_calls = Vec::new();
                        for block in &dm.blocks {
                            match block {
                                ContentBlock::Text { content, .. } => {
                                    if !text_content.is_empty() {
                                        text_content.push('\n');
                                    }
                                    text_content.push_str(content);
                                }
                                ContentBlock::ToolCall { state, .. } => {
                                    if let ToolBlockState::Done { ref record, .. } = &**state {
                                        tool_calls.push(ToolCallRequest {
                                            id: record.id.clone(),
                                            name: record.name.clone(),
                                            arguments: record.arguments.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        let has_content = !text_content.trim().is_empty();
                        if has_content || !tool_calls.is_empty() {
                            new_session.messages.push(Message {
                                role: dm.role.clone(),
                                content: text_content,
                                tool_calls,
                                tool_result: None,
                            });
                        }
                    }
                    match new_session.save() {
                        Ok(path) => {
                            let fork_id = new_session.id.clone();
                            self.current_session_id = Some(fork_id.clone());
                            self.status = format!("Forked to session: {}", fork_id);
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!(
                                    "Forked to new session: {} (saved to {})",
                                    fork_id,
                                    path.display()
                                ),
                            ));
                        }
                        Err(e) => {
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!("Fork failed: {}", e),
                            ));
                        }
                    }
                }
            }

            "/dump" => {
                if self.messages.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Nothing to dump. Start chatting first.",
                    ));
                } else {
                    let mut markdown = String::new();
                    markdown.push_str("# Pawan Session\n\n");
                    markdown.push_str(&format!("**Model:** {}\n\n", self.model_name));
                    for msg in &self.messages {
                        let role = match msg.role {
                            Role::User => "**You**",
                            Role::Assistant => "**Pawan**",
                            _ => "**System**",
                        };
                        markdown.push_str(&format!("### {}\n\n", role));
                        markdown.push_str(&msg.text_content());
                        markdown.push_str("\n\n");
                        let tool_records = msg.tool_records();
                        if !tool_records.is_empty() {
                            markdown.push_str(&format!(
                                "<details><summary>Tool calls ({})</summary>\n\n",
                                tool_records.len()
                            ));
                            for tc in tool_records {
                                let status = if tc.success { "ok" } else { "err" };
                                markdown.push_str(&format!(
                                    "- `{}` ({}) — {}ms\n",
                                    tc.name, status, tc.duration_ms
                                ));
                            }
                            markdown.push_str("\n</details>\n\n");
                        }
                    }
                    match arboard::Clipboard::new() {
                        Ok(mut cb) => match cb.set_text(&markdown) {
                            Ok(_) => {
                                let char_count = markdown.len();
                                self.messages.push(DisplayMessage::new_text(
                                    Role::System,
                                    format!("Copied {} characters to clipboard", char_count),
                                ));
                                self.status = "Copied to clipboard".to_string();
                            }
                            Err(e) => self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!("Failed to copy: {}", e),
                            )),
                        },
                        Err(e) => self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Failed to access clipboard: {}", e),
                        )),
                    }
                }
            }

            "/save" => {
                if self.messages.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Nothing to save. Start chatting first.",
                    ));
                    self.status = "Nothing to save".to_string();
                } else {
                    let mut session = if let Some(ref sid) = self.current_session_id {
                        match Session::load(sid) {
                            Ok(mut s) => {
                                s.model = self.model_name.clone();
                                s.tags = self.session_tags.clone();
                                s.total_tokens = self.total_tokens;
                                s.iteration_count = self.iteration_count;
                                s
                            }
                            Err(_) => Session::new_with_id(
                                sid.clone(),
                                &self.model_name,
                                self.session_tags.clone(),
                            ),
                        }
                    } else {
                        let mut ns =
                            Session::new_with_tags(&self.model_name, self.session_tags.clone());
                        self.current_session_id = Some(ns.id.clone());
                        ns.total_tokens = self.total_tokens;
                        ns.iteration_count = self.iteration_count;
                        ns
                    };
                    session.messages.clear();
                    for dm in &self.messages {
                        let mut tc = String::new();
                        let mut calls = Vec::new();
                        for b in &dm.blocks {
                            match b {
                                ContentBlock::Text { content, .. } => {
                                    if !tc.is_empty() {
                                        tc.push('\n');
                                    }
                                    tc.push_str(content);
                                }
                                ContentBlock::ToolCall { state, .. } => {
                                    if let ToolBlockState::Done { ref record, .. } = &**state {
                                        calls.push(ToolCallRequest {
                                            id: record.id.clone(),
                                            name: record.name.clone(),
                                            arguments: record.arguments.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        if !tc.trim().is_empty() || !calls.is_empty() {
                            session.messages.push(Message {
                                role: dm.role.clone(),
                                content: tc,
                                tool_calls: calls,
                                tool_result: None,
                            });
                        }
                    }
                    match session.save() {
                        Ok(p) => {
                            let ps = p.to_string_lossy().to_string();
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!("Session saved: {}", ps),
                            ));
                            self.status = "Session saved".to_string();
                        }
                        Err(e) => self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Save failed: {}", e),
                        )),
                    }
                }
            }
            "/share" => {
                if self.messages.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Nothing to share. Start chatting first.",
                    ));
                    self.status = "Nothing to share".to_string();
                } else {
                    let mut session = if let Some(ref sid) = self.current_session_id {
                        match Session::load(sid) {
                            Ok(mut s) => {
                                s.model = self.model_name.clone();
                                s.tags = self.session_tags.clone();
                                s.total_tokens = self.total_tokens;
                                s.iteration_count = self.iteration_count;
                                s
                            }
                            Err(_) => Session::new_with_id(
                                sid.clone(),
                                &self.model_name,
                                self.session_tags.clone(),
                            ),
                        }
                    } else {
                        let mut ns =
                            Session::new_with_tags(&self.model_name, self.session_tags.clone());
                        self.current_session_id = Some(ns.id.clone());
                        ns.total_tokens = self.total_tokens;
                        ns.iteration_count = self.iteration_count;
                        ns
                    };
                    session.messages.clear();
                    for dm in &self.messages {
                        let mut tc = String::new();
                        let mut calls = Vec::new();
                        for b in &dm.blocks {
                            match b {
                                ContentBlock::Text { content, .. } => {
                                    if !tc.is_empty() {
                                        tc.push('\n');
                                    }
                                    tc.push_str(content);
                                }
                                ContentBlock::ToolCall { state, .. } => {
                                    if let ToolBlockState::Done { ref record, .. } = &**state {
                                        calls.push(ToolCallRequest {
                                            id: record.id.clone(),
                                            name: record.name.clone(),
                                            arguments: record.arguments.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        if !tc.trim().is_empty() || !calls.is_empty() {
                            session.messages.push(Message {
                                role: dm.role.clone(),
                                content: tc,
                                tool_calls: calls,
                                tool_result: None,
                            });
                        }
                    }
                    match session.save() {
                        Ok(p) => {
                            let ps = p.to_string_lossy().to_string();
                            let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(&ps));
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!("Session saved: {}", ps),
                            ));
                            self.status = "Session shared".to_string();
                        }
                        Err(e) => self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Share failed: {}", e),
                        )),
                    }
                }
            }
            "/sessions" => {
                self.session_browser_open = true;
                self.session_browser_query.clear();
                self.session_browser_selected = 0;
            }
            "/ss" | "/searchsessions" => {
                // Search saved sessions
                if arg.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Usage: /ss <query> - search saved sessions".to_string(),
                    ));
                } else {
                    let results: Vec<SearchResult> =
                        pawan::agent::session::search_sessions(arg).unwrap_or_default();
                    if results.is_empty() {
                        self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("No sessions found matching: {}", arg),
                        ));
                        self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("No sessions found matching: {}", arg),
                        ));
                    } else {
                        let mut output =
                            format!("Found {} session(s) matching '{}':\n", results.len(), arg);
                        for (i, r) in results.iter().take(10).enumerate() {
                            let id_short = r.id.chars().take(8).collect::<String>();
                            output.push_str(&format!(
                                "\n{}. [{}] {} ({} msgs)\n",
                                i + 1,
                                id_short,
                                r.model,
                                r.message_count
                            ));
                            if !r.tags.is_empty() {
                                output.push_str(&format!("   Tags: {}\n", r.tags.join(", ")));
                            }
                            for m in r.matches.iter().take(2) {
                                let preview = m.preview.chars().take(60).collect::<String>();
                                output.push_str(&format!("   [...] {}...\n", preview));
                            }
                        }
                        if results.len() > 10 {
                            output.push_str(&format!("\n... and {} more", results.len() - 10));
                        }
                        self.messages
                            .push(DisplayMessage::new_text(Role::System, output));
                    }
                }
            }
            "/prune" => {
                // Prune old sessions
                let mut max_days: Option<u32> = None;
                let mut max_sessions: Option<usize> = None;
                for part in arg.split_whitespace() {
                    if let Some(base) = part.strip_suffix('d') {
                        if let Ok(d) = base.parse::<u32>() {
                            max_days = Some(d);
                        }
                    } else if let Some(base) = part.strip_suffix('s') {
                        if let Ok(s) = base.parse::<usize>() {
                            max_sessions = Some(s);
                        }
                    }
                }
                let policy = RetentionPolicy {
                    max_age_days: max_days,
                    max_sessions,
                    keep_tags: vec![],
                };
                match pawan::agent::session::prune_sessions(&policy) {
                    Ok(count) => {
                        let msg = if count > 0 {
                            format!("Pruned {} session(s)", count)
                        } else {
                            "No sessions to prune".to_string()
                        };
                        self.messages
                            .push(DisplayMessage::new_text(Role::System, msg));
                    }
                    Err(e) => self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        format!("Prune error: {}", e),
                    )),
                }
            }
            "/tag" => {
                if arg.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Usage: /tag add <tags> | rm <tag> | list | clear".to_string(),
                    ));
                } else if let Some(tags_str) = arg.strip_prefix("add ") {
                    let tags_str = tags_str.trim();
                    let mut added = Vec::new();
                    for raw in tags_str.split_whitespace() {
                        let sanitized = raw.trim().to_string();
                        if !self.session_tags.contains(&sanitized) && !sanitized.is_empty() {
                            self.session_tags.push(sanitized.clone());
                            added.push(sanitized);
                        }
                    }
                    if !added.is_empty() {
                        self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Added tags: {}", added.join(", ")),
                        ));
                    } else {
                        self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            "No new tags added".to_string(),
                        ));
                    }
                } else if let Some(tag) = arg.strip_prefix("rm ") {
                    let tag = tag.trim();
                    if let Some(pos) = self.session_tags.iter().position(|t| t == tag) {
                        self.session_tags.remove(pos);
                        self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Removed tag: {}", tag),
                        ));
                    } else {
                        self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Tag not found: {}", tag),
                        ));
                    }
                } else if arg == "list" {
                    let list = if self.session_tags.is_empty() {
                        "No tags".to_string()
                    } else {
                        self.session_tags.join(", ")
                    };
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        format!("Current tags: {}", list),
                    ));
                } else if arg == "clear" {
                    self.session_tags.clear();
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "All tags cleared".to_string(),
                    ));
                } else {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Usage: /tag add <tags> | rm <tag> | list | clear".to_string(),
                    ));
                }
            }
            "/load" => {
                if arg.is_empty() {
                    // Open session browser when no ID provided
                    self.session_browser_open = true;
                    self.session_browser_query.clear();
                    self.session_browser_selected = 0;
                } else {
                    match Session::load(arg) {
                        Ok(session) => {
                            self.model_name = session.model.clone();
                            self.status = format!("Loaded session: {}", session.id);
                            self.session_tags = session.tags.clone();
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!(
                                    "Loaded session {} (model: {}, {} messages). Full message loading not yet implemented.",
                                    session.id,
                                    session.model,
                                    session.messages.len()
                                ),
                            ));
                        }
                        Err(e) => self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Failed to load session: {}", e),
                        )),
                    }
                }
            }
            "/resume" => {
                if arg.is_empty() {
                    // Open session browser when no ID provided
                    self.session_browser_open = true;
                    self.session_browser_query.clear();
                    self.session_browser_selected = 0;
                } else {
                    match Session::load(arg) {
                        Ok(session) => {
                            self.model_name = session.model.clone();
                            self.status = format!("Resumed session: {}", session.id);
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!(
                                    "Resumed session {} (model: {}, {} messages). Continue chatting with this context.",
                                    session.id,
                                    session.model,
                                    session.messages.len()
                                ),
                            ));
                        }
                        Err(e) => self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Failed to resume session: {}", e),
                        )),
                    }
                }
            }
            "/new" => {
                let had_user = self.messages.iter().any(|m| matches!(m.role, Role::User));
                let had_system = self.messages.iter().any(|m| matches!(m.role, Role::System));
                self.messages.clear();
                self.scroll = 0;
                self.processing = false;
                self.status = "New conversation started".to_string();
                // Keep current model, just clear conversation
                if had_user && !had_system {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Started new conversation",
                    ));
                }
            }
            "/compact" => {
                use pawan::compaction::{compact_messages, CompactionStrategy};

                if self.messages.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "No messages to compact. Start chatting first.".to_string(),
                    ));
                    self.status = "Nothing to compact".to_string();
                } else {
                    // Parse strategy from arg
                    let strategy = match arg {
                        "aggressive" => CompactionStrategy {
                            keep_recent: 5,
                            keep_keywords: vec![
                                "error".to_string(),
                                "fix".to_string(),
                                "bug".to_string(),
                            ],
                            keep_tool_results: false,
                            keep_system: false,
                        },
                        "conservative" => CompactionStrategy {
                            keep_recent: 20,
                            keep_keywords: vec![
                                "error".to_string(),
                                "fix".to_string(),
                                "bug".to_string(),
                                "issue".to_string(),
                                "problem".to_string(),
                                "solution".to_string(),
                                "important".to_string(),
                                "note".to_string(),
                                "warning".to_string(),
                                "decision".to_string(),
                                "todo".to_string(),
                            ],
                            keep_tool_results: true,
                            keep_system: true,
                        },
                        _ => CompactionStrategy::default(), // Default balanced strategy
                    };

                    // Convert DisplayMessages to Messages for compaction
                    let original_messages: Vec<Message> = self
                        .messages
                        .iter()
                        .filter_map(|dm| {
                            let text_content = dm.text_content();
                            if text_content.trim().is_empty() {
                                return None;
                            }
                            Some(Message {
                                role: dm.role.clone(),
                                content: text_content,
                                tool_calls: vec![], // Tool calls not preserved in simple compaction
                                tool_result: None,
                            })
                        })
                        .collect();

                    let original_count = original_messages.len();

                    // Apply compaction
                    let result = compact_messages(original_messages, &strategy);

                    // Convert compacted messages back to DisplayMessages
                    self.messages = result
                        .messages
                        .into_iter()
                        .map(|m| DisplayMessage::new_text(m.role, m.content))
                        .collect();

                    let reduction_pct = if original_count > 0 {
                        ((original_count - result.compacted_count) as f64 / original_count as f64
                            * 100.0) as u32
                    } else {
                        0
                    };

                    self.status = format!(
                        "Compacted: {} → {} messages ({}% reduction, ~{} tokens saved)",
                        original_count, result.compacted_count, reduction_pct, result.tokens_saved
                    );
                    self.messages
                        .push(DisplayMessage::new_text(Role::System, self.status.clone()));
                }
            }
            "/help" | "/?" => {
                let help_text = self.slash_registry.help_text();
                self.messages
                    .push(DisplayMessage::new_text(Role::System, help_text));
            }
            "/session" => {
                if arg.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Usage: /session <id> — switch to a saved session. Use /sessions to browse."
                            .to_string(),
                    ));
                } else {
                    match Session::load(arg) {
                        Ok(s) => {
                            let sid = s.id.clone();
                            let msg_count = s.messages.len();
                            self.model_name = s.model.clone();
                            self.current_session_id = Some(sid.clone());
                            self.session_tags = s.tags.clone();
                            self.messages = App::messages_from_session(s.messages);
                            self.scroll = 0;
                            self.status = format!("Switched to session: {}", sid);
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!("Switched to session: {} ({} messages)", sid, msg_count),
                            ));
                        }
                        Err(e) => {
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                format!("Failed to load session: {}", e),
                            ));
                        }
                    }
                }
            }
            "/retry" => {
                if self.processing {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Cannot retry while a response is in progress.".to_string(),
                    ));
                } else if self
                    .messages
                    .iter()
                    .rposition(|m| m.role == Role::Assistant)
                    .is_none()
                {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "No assistant message to retry.".to_string(),
                    ));
                } else {
                    let assistant_i = self
                        .messages
                        .iter()
                        .rposition(|m| m.role == Role::Assistant)
                        .unwrap();
                    self.messages.truncate(assistant_i);
                    if let Some(user) = self.messages.iter().rfind(|m| m.role == Role::User) {
                        let user_text = user.text_content();
                        if user_text.trim().is_empty() {
                            self.messages.push(DisplayMessage::new_text(
                                Role::System,
                                "Cannot retry: last user message is empty.".to_string(),
                            ));
                        } else {
                            self.processing = true;
                            self.status = "Retrying...".to_string();
                            let _ = self.cmd_tx.send(AgentCommand::Execute(user_text));
                        }
                    } else {
                        self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            "No user message to retry.".to_string(),
                        ));
                    }
                }
            }
            "/theme" => {
                let args = arg.trim();
                if args.is_empty() {
                    let themes = super::theme::available_themes();
                    self.status = format!("Available: {}", themes.join(", "));
                } else {
                    match super::theme::set_theme(args) {
                        Ok(_) => {
                            let t = super::theme::current();
                            self.current_theme = t.name.to_string();
                            // Animate from current accent to new accent (captures in-flight color)
                            self.accent_transition.set(t.accent);
                            self.status_bar
                                .flash(format!("Switched to {} theme", t.name));
                            self.status = format!("Theme: {} — accent transition", t.name);
                        }
                        Err(_) => {
                            let themes = super::theme::available_themes();
                            self.status = format!(
                                "Unknown theme '{}'; available: {}",
                                args,
                                themes.join(", ")
                            );
                        }
                    }
                }
            }

            _ => {
                debug_assert!(
                    false,
                    "unregistered slash command in slash_route match: {command}"
                );
            }
        }
    }
}

pub(crate) fn default_slash_fuzzy_lines() -> Vec<String> {
    let r = SlashCommandRegistry::built_in();
    let mut out: Vec<String> = r
        .all()
        .iter()
        .map(|c| format!("{} — {}", c.name, c.description))
        .collect();
    out.sort();
    // Model shortcut lines (kept from the old static palette)
    out.extend(
        [
            (
                "/model qwen/qwen3.5-122b-a10b",
                "Qwen 3.5 122B (S tier, fast)",
            ),
            ("/model minimaxai/minimax-m2.5", "MiniMax M2.5 (SWE 80.2%)"),
            (
                "/model stepfun-ai/step-3.5-flash",
                "Step 3.5 Flash (S+ tier)",
            ),
            (
                "/model mistralai/mistral-small-4-119b-2603",
                "Mistral Small 4 119B",
            ),
        ]
        .into_iter()
        .map(|(c, d)| format!("{c} — {d}")),
    );
    out
}
