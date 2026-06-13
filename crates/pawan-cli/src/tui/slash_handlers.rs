//! Slash command handlers grouped by category.

#![allow(unused_imports)]

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pawan::agent::session::{
    prune_sessions, search_sessions, RetentionPolicy, SearchResult, Session, SessionSummary,
};
use pawan::agent::{AgentResponse, Message, PawanAgent, Role, ToolCallRecord, ToolCallRequest};
use pawan::compaction::compact_messages;
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

const RMUX_USAGE: &str = "Usage: /rmux <terminal task> or /rmux session|send|key|wait|snapshot ...";
const RMUX_SESSION_USAGE: &str =
    "Usage: /rmux session <name> [--cwd <path>] [--size <cols>x<rows>] [--cmd <command>]";
const RMUX_SEND_USAGE: &str = "Usage: /rmux send <session> <text to send and press Enter>";
const RMUX_KEY_USAGE: &str = "Usage: /rmux key <session> <key>";
const RMUX_WAIT_USAGE: &str = "Usage: /rmux wait <session> <text>";
const RMUX_SNAPSHOT_USAGE: &str = "Usage: /rmux snapshot <session>";

fn build_rmux_slash_prompt(request: &str) -> std::result::Result<String, &'static str> {
    let request = request.trim();
    if request.is_empty() {
        return Err(RMUX_USAGE);
    }

    let parts: Vec<&str> = request.split_whitespace().collect();
    match parts.first().copied() {
        Some("session") => build_rmux_session_prompt(&parts[1..]),
        Some("send") => build_rmux_send_prompt(&parts[1..]),
        Some("key") => build_rmux_key_prompt(&parts[1..]),
        Some("wait") => build_rmux_wait_prompt(&parts[1..]),
        Some("snapshot") => build_rmux_snapshot_prompt(&parts[1..]),
        Some(_) => Ok(format!(
            "Use the rmux tool to complete this terminal-multiplexer task. Prefer durable named sessions, wait_for_text, and snapshot evidence before reporting results. Task: {request}"
        )),
        None => Err(RMUX_USAGE),
    }
}

fn build_rmux_session_prompt(args: &[&str]) -> std::result::Result<String, &'static str> {
    let Some(session) = args.first().copied() else {
        return Err(RMUX_SESSION_USAGE);
    };
    let mut cwd: Option<&str> = None;
    let mut cols: Option<&str> = None;
    let mut rows: Option<&str> = None;
    let mut command: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i] {
            "--cwd" => {
                i += 1;
                cwd = args.get(i).copied();
                if cwd.is_none() {
                    return Err(RMUX_SESSION_USAGE);
                }
            }
            "--size" => {
                i += 1;
                let Some(size) = args.get(i).copied() else {
                    return Err(RMUX_SESSION_USAGE);
                };
                let Some((c, r)) = size.split_once('x') else {
                    return Err(RMUX_SESSION_USAGE);
                };
                if c.parse::<u16>().is_err() || r.parse::<u16>().is_err() {
                    return Err(RMUX_SESSION_USAGE);
                }
                cols = Some(c);
                rows = Some(r);
            }
            "--cmd" => {
                let rest = args.get(i + 1..).unwrap_or_default();
                if rest.is_empty() {
                    return Err(RMUX_SESSION_USAGE);
                }
                command = Some(rest.join(" "));
                break;
            }
            _ => return Err(RMUX_SESSION_USAGE),
        }
        i += 1;
    }

    let mut lines = vec![
        "Use the rmux tool with these exact typed steps, then report the final pane snapshot."
            .to_string(),
        "1. Call rmux with action: ensure_session".to_string(),
        format!("   session: {session}"),
    ];
    if let Some(cwd) = cwd {
        lines.push(format!("   cwd: {cwd}"));
    }
    if let (Some(cols), Some(rows)) = (cols, rows) {
        lines.push(format!("   cols: {cols}"));
        lines.push(format!("   rows: {rows}"));
    }
    if let Some(command) = command {
        lines.push(format!("   command: {command}"));
    }
    lines.push("2. Call rmux with action: snapshot".to_string());
    lines.push(format!("   session: {session}"));
    lines.push(
        "3. Report the snapshot visible_text and whether the session was created or reused."
            .to_string(),
    );
    Ok(lines.join("\n"))
}

fn build_rmux_send_prompt(args: &[&str]) -> std::result::Result<String, &'static str> {
    let Some(session) = args.first().copied() else {
        return Err(RMUX_SEND_USAGE);
    };
    let text_parts = args.get(1..).unwrap_or_default();
    if text_parts.is_empty() {
        return Err(RMUX_SEND_USAGE);
    }
    let text = text_parts.join(" ");
    Ok(format!(
        "Use the rmux tool with these exact typed steps, then report the resulting pane snapshot.\n1. Call rmux with action: send_text\n   session: {session}\n   text: {text}\n2. Call rmux with action: send_key\n   session: {session}\n   key: Enter\n3. Call rmux with action: snapshot\n   session: {session}\n4. Report the snapshot visible_text."
    ))
}

fn build_rmux_key_prompt(args: &[&str]) -> std::result::Result<String, &'static str> {
    let [session, key] = args else {
        return Err(RMUX_KEY_USAGE);
    };
    Ok(format!(
        "Use the rmux tool with these exact typed steps, then report the resulting pane snapshot.\n1. Call rmux with action: send_key\n   session: {session}\n   key: {key}\n2. Call rmux with action: snapshot\n   session: {session}\n3. Report the snapshot visible_text."
    ))
}

fn build_rmux_wait_prompt(args: &[&str]) -> std::result::Result<String, &'static str> {
    let Some(session) = args.first().copied() else {
        return Err(RMUX_WAIT_USAGE);
    };
    let text_parts = args.get(1..).unwrap_or_default();
    if text_parts.is_empty() {
        return Err(RMUX_WAIT_USAGE);
    }
    let text = text_parts.join(" ");
    Ok(format!(
        "Use the rmux tool with these exact typed steps, then report the matching pane snapshot.\n1. Call rmux with action: wait_for_text\n   session: {session}\n   text: {text}\n2. Call rmux with action: snapshot\n   session: {session}\n3. Report the snapshot visible_text."
    ))
}

fn build_rmux_snapshot_prompt(args: &[&str]) -> std::result::Result<String, &'static str> {
    let [session] = args else {
        return Err(RMUX_SNAPSHOT_USAGE);
    };
    Ok(format!(
        "Use the rmux tool with action: snapshot\nsession: {session}\nReport cols, rows, revision, and visible_text."
    ))
}

// ── shared mode helpers ────────────────────────────────────────────────

impl<'a> App<'a> {
    pub(crate) fn apply_goal_command(&mut self, arg: &str) {
        let arg = arg.trim();
        if arg.is_empty() {
            if self.goal_mode {
                self.goal_mode = false;
                self.goal_objective = None;
                self.status_bar.flash("Goal mode off".to_string());
            } else {
                self.goal_mode = true;
                self.status_bar.flash("Goal mode on".to_string());
            }
        } else {
            self.goal_mode = true;
            self.goal_objective = Some(arg.to_string());
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Goal set: {arg}"),
            ));
            self.status_bar
                .flash(format!("Goal: {}", Self::truncate_status(arg, 48)));
        }
    }

    pub(crate) fn apply_loop_command(&mut self) {
        self.loop_mode = !self.loop_mode;
        if self.loop_mode {
            let hint = if self.iteration_count > 0 {
                format!(
                    "Loop mode on — auto-continue after each response (iteration {})",
                    self.iteration_count
                )
            } else {
                "Loop mode on — auto-continue after each response".to_string()
            };
            self.messages
                .push(DisplayMessage::new_text(Role::System, hint.clone()));
            self.status_bar.flash(hint);
        } else {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Loop mode disabled".to_string(),
            ));
            self.status_bar.flash("Loop mode off".to_string());
        }
    }

    pub(crate) fn apply_orchestrate_command(&mut self, arg: &str) {
        let task = arg.trim();
        if task.is_empty() {
            if self.orchestrate_mode {
                self.orchestrate_mode = false;
                self.orchestrate_task = None;
                self.status_bar.flash("Orchestration mode off".to_string());
            } else {
                self.orchestrate_mode = true;
                self.status_bar.flash("Orchestration mode on".to_string());
            }
        } else {
            self.orchestrate_mode = true;
            self.orchestrate_task = Some(task.to_string());
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Orchestration task: {task}"),
            ));
            self.status_bar
                .flash(format!("Orchestrate: {}", Self::truncate_status(task, 48)));
        }
    }

    fn truncate_status(s: &str, max: usize) -> String {
        if s.chars().count() <= max {
            s.to_string()
        } else {
            let mut end = 0usize;
            for (count, (i, _)) in s.char_indices().enumerate() {
                if count == max {
                    break;
                }
                end = i + 1;
            }
            format!("{}…", &s[..end])
        }
    }
}

// ── misc ─────────────────────────────────────────────────────────────

impl<'a> App<'a> {
    pub(crate) fn slash_clear(&mut self) {
        self.messages.clear();
        self.status = "Cleared".to_string();
    }

    pub(crate) fn slash_model(&mut self, arg: &str) {
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

    pub(crate) fn slash_tools(&mut self) {
        self.messages.push(DisplayMessage::new_text(Role::System,
            "Core: bash, read_file, write_file, edit_file, ast_grep, glob_search, grep_search\n\
             Standard: git (status/diff/add/commit/log/blame/branch/checkout/stash), agents, edit modes\n\
             Extended: rg, fd, sd, tree, mise, zoxide, lsp, rmux\n\
             MCP: mcp_daedra_web_search, mcp_daedra_visit_page"));
    }

    pub(crate) fn slash_rmux(&mut self, arg: &str) {
        let request = arg.trim();
        let prompt = match build_rmux_slash_prompt(request) {
            Ok(prompt) => prompt,
            Err(usage) => {
                self.messages
                    .push(DisplayMessage::new_text(Role::System, usage));
                self.status_bar.flash(usage.to_string());
                return;
            }
        };

        self.messages.push(DisplayMessage::new_text(
            Role::User,
            format!("/rmux {request}"),
        ));
        self.processing = true;
        self.status = format!("RMUX: {}", Self::truncate_status(request, 48));
        self.status_bar
            .flash(format!("RMUX: {}", Self::truncate_status(request, 48)));
        let _ = self.cmd_tx.send(AgentCommand::Execute(prompt));
    }

    pub(crate) fn slash_search(&mut self, arg: &str) {
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

    pub(crate) fn slash_handoff(&mut self) {
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

    pub(crate) fn slash_heal(&mut self) {
        self.messages
            .push(DisplayMessage::new_text(Role::User, "/heal"));
        self.processing = true;
        self.status = "Healing...".to_string();
        let _ = self.cmd_tx.send(AgentCommand::Execute(
            "Run cargo check and cargo test. Fix any errors you find.".to_string(),
        ));
    }

    pub(crate) fn slash_quit(&mut self) {
        self.should_quit = true;
    }

    pub(crate) fn slash_export(&mut self, arg: &str) {
        let (path, format) = if arg.contains("--format") {
            let parts: Vec<&str> = arg.splitn(3, ' ').collect();
            let format_str = parts.get(2).unwrap_or(&"md");
            let path = parts.get(1).unwrap_or(&"pawan-session");
            (path.to_string(), ExportFormat::parse(format_str))
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

    pub(crate) fn slash_diff(&mut self, arg: &str) {
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
                    }
                } else if !stderr.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        stderr.trim().to_string(),
                    ));
                }
            }
            Err(e) => {
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Failed to run git diff: {e}"),
                ));
            }
        }
    }

    pub(crate) fn slash_import(&mut self, arg: &str) {
        let path = arg.trim();
        if path.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Usage: /import <path>".to_string(),
            ));
            return;
        }
        match Session::from_json_file(path) {
            Ok(session) => {
                self.model_name = session.model.clone();
                self.messages = Self::messages_from_session(session.messages);
                self.session_tags = session.tags.clone();
                self.current_session_id = Some(session.id.clone());
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Imported session {}", session.id),
                ));
            }
            Err(e) => {
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Import failed: {e}"),
                ));
            }
        }
    }

    pub(crate) fn slash_fork(&mut self) {
        if self.messages.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "No conversation to fork".to_string(),
            ));
            return;
        }
        let mut session = Session::new_with_tags(&self.model_name, self.session_tags.clone());
        session.messages = self.display_messages_as_session_messages();
        match session.save() {
            Ok(path) => {
                self.current_session_id = Some(session.id.clone());
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Forked to new session {} ({})", session.id, path.display()),
                ));
            }
            Err(e) => {
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Fork failed: {e}"),
                ));
            }
        }
    }

    pub(crate) fn slash_dump(&mut self) {
        if self.messages.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Nothing to dump".to_string(),
            ));
            return;
        }
        let path = std::env::temp_dir().join(format!("pawan-dump-{}.md", uuid::Uuid::new_v4()));
        let path_str = path.to_string_lossy().to_string();
        match self.export_as_markdown(&path_str) {
            Ok(n) => match std::fs::read_to_string(&path_str) {
                Ok(content) => {
                    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(content)) {
                        Ok(()) => self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Copied {n} messages to clipboard"),
                        )),
                        Err(e) => self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Failed to copy to clipboard: {e}"),
                        )),
                    }
                }
                Err(e) => self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Failed to read dump file: {e}"),
                )),
            },
            Err(e) => self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Dump failed: {e}"),
            )),
        }
        let _ = std::fs::remove_file(&path);
    }

    pub(crate) fn slash_save(&mut self) {
        if self.messages.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Nothing to save".to_string(),
            ));
            return;
        }
        match self.save_current_session() {
            Ok(path) => self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Session saved to {}", path.display()),
            )),
            Err(e) => self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Save failed: {e}"),
            )),
        }
    }

    pub(crate) fn slash_share(&mut self) {
        if self.messages.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Nothing to share".to_string(),
            ));
            return;
        }
        match self.save_current_session() {
            Ok(path) => {
                let path_str = path.to_string_lossy().to_string();
                match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(path_str.clone())) {
                    Ok(()) => self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        format!("Session saved to {path_str} (path copied to clipboard)"),
                    )),
                    Err(e) => self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        format!("Session saved to {path_str} but clipboard failed: {e}"),
                    )),
                }
            }
            Err(e) => self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Share failed: {e}"),
            )),
        }
    }

    pub(crate) fn slash_sessions(&mut self) {
        self.session_browser_open = true;
        self.session_browser_query.clear();
        self.session_browser_selected = 0;
    }

    pub(crate) fn slash_search_sessions(&mut self, arg: &str) {
        let query = arg.trim();
        if query.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Usage: /ss <query>".to_string(),
            ));
            return;
        }
        match search_sessions(query) {
            Ok(results) if results.is_empty() => {
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("No sessions matched '{query}'"),
                ));
            }
            Ok(results) => {
                let mut lines = vec![format!("Found {} session(s) for '{query}':", results.len())];
                for hit in results.iter().take(20) {
                    lines.push(format!("- {} ({} match(es))", hit.id, hit.matches.len()));
                }
                if results.len() > 20 {
                    lines.push("... (truncated)".to_string());
                }
                self.messages
                    .push(DisplayMessage::new_text(Role::System, lines.join("\n")));
            }
            Err(e) => {
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Session search failed: {e}"),
                ));
            }
        }
    }

    pub(crate) fn slash_prune(&mut self, arg: &str) {
        let policy = Self::parse_prune_policy(arg);
        match prune_sessions(&policy) {
            Ok(n) => self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Pruned {n} session(s)"),
            )),
            Err(e) => self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Prune failed: {e}"),
            )),
        }
    }

    pub(crate) fn slash_tag(&mut self, arg: &str) {
        let arg = arg.trim();
        if arg.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Usage: /tag <add|rm|list|clear> [tags...]".to_string(),
            ));
            return;
        }
        let mut parts = arg.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        match cmd {
            "add" => {
                let tags: Vec<String> = parts.map(|t| t.to_string()).collect();
                if tags.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Usage: /tag add <tag> [more tags...]".to_string(),
                    ));
                    return;
                }
                for tag in tags {
                    if !self.session_tags.iter().any(|t| t == &tag) {
                        self.session_tags.push(tag);
                    }
                }
            }
            "rm" | "remove" => {
                let tags: Vec<&str> = parts.collect();
                if tags.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "Usage: /tag rm <tag> [more tags...]".to_string(),
                    ));
                    return;
                }
                self.session_tags
                    .retain(|t| !tags.iter().any(|rm| rm == &t.as_str()));
            }
            "list" => {
                if self.session_tags.is_empty() {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        "No tags on current session".to_string(),
                    ));
                } else {
                    self.messages.push(DisplayMessage::new_text(
                        Role::System,
                        format!("Tags: {}", self.session_tags.join(", ")),
                    ));
                }
            }
            "clear" => {
                self.session_tags.clear();
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    "Cleared session tags".to_string(),
                ));
            }
            _ => {
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    "Usage: /tag <add|rm|list|clear> [tags...]".to_string(),
                ));
            }
        }
    }

    pub(crate) fn slash_new(&mut self) {
        let had_chat = self
            .messages
            .iter()
            .any(|m| matches!(m.role, Role::User | Role::Assistant));
        let had_session = self.current_session_id.is_some();
        self.messages.clear();
        self.scroll = 0;
        self.current_session_id = None;
        self.session_tags.clear();
        self.processing = false;
        if had_chat && !had_session {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Started new conversation".to_string(),
            ));
        }
    }

    pub(crate) fn slash_compact(&mut self, arg: &str) {
        if self.messages.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Nothing to compact".to_string(),
            ));
            return;
        }
        let strategy = Self::compaction_strategy_for(arg);
        let original = self.messages.len();
        let session_messages = self.display_messages_as_session_messages();
        let result = compact_messages(session_messages, &strategy);
        self.messages = Self::messages_from_session(result.messages);
        let compacted = self.messages.len();
        let pct = if original > 0 {
            ((original.saturating_sub(compacted)) * 100)
                .checked_div(original)
                .unwrap_or(0)
        } else {
            0
        };
        self.status = format!(
            "Compacted: {original} → {compacted} messages ({pct}% reduction, ~{} tokens saved)",
            result.tokens_saved
        );
        self.messages
            .push(DisplayMessage::new_text(Role::System, self.status.clone()));
    }

    pub(crate) fn slash_help(&mut self) {
        let mut lines: Vec<String> = self
            .slash_registry
            .all()
            .iter()
            .map(|c| format!("{} — {}", c.name, c.description))
            .collect();
        lines.sort_by_key(|a| a.to_ascii_lowercase());
        self.messages
            .push(DisplayMessage::new_text(Role::System, lines.join("\n")));
    }

    pub(crate) fn slash_session(&mut self, arg: &str) {
        let id = arg.trim();
        if id.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Usage: /session <id>".to_string(),
            ));
            return;
        }
        self.load_session_by_id(id);
    }

    pub(crate) fn slash_retry(&mut self) {
        let idx = self.messages.iter().rposition(|m| m.role == Role::User);
        let Some(idx) = idx else {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Nothing to retry".to_string(),
            ));
            return;
        };
        let prompt = self.messages[idx].text_content();
        if prompt.trim().is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "Nothing to retry".to_string(),
            ));
            return;
        }
        self.messages.truncate(idx + 1);
        self.processing = true;
        self.status = "Retrying...".to_string();
        let _ = self.cmd_tx.send(AgentCommand::Execute(prompt));
    }

    pub(crate) fn slash_theme(&mut self, arg: &str) {
        let name = arg.trim();
        if name.is_empty() {
            let themes = super::theme::available_themes().join(", ");
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("Available themes: {themes}"),
            ));
            self.status = "Theme list shown".to_string();
            return;
        }
        match super::theme::set_theme(name) {
            Ok(()) => {
                self.current_theme = name.to_string();
                // Fade the accent toward the new palette's colour.
                animate_core::Animate::set(&mut self.accent_tween, super::theme::current().accent);
                self.restyle_input();
                self.status = format!("Theme: {name}");
            }
            Err(_) => {
                let themes = super::theme::available_themes().join(", ");
                self.status = format!("Unknown theme '{name}' — available: {themes}");
            }
        }
    }

    pub(crate) fn slash_resume(&mut self, arg: &str) {
        if arg.trim().is_empty() {
            self.slash_sessions();
        } else {
            self.load_session_by_id(arg.trim());
        }
    }

    pub(crate) fn slash_load(&mut self, arg: &str) {
        if arg.trim().is_empty() {
            self.slash_sessions();
        } else {
            self.load_session_by_id(arg.trim());
        }
    }

    pub(crate) fn slash_irc(&mut self) {
        self.open_irc_compose();
    }

    fn load_session_by_id(&mut self, id: &str) {
        match Session::load(id) {
            Ok(session) => {
                self.model_name = session.model.clone();
                self.messages = Self::messages_from_session(session.messages);
                self.session_tags = session.tags.clone();
                self.current_session_id = Some(session.id.clone());
                self.session_browser_open = false;
                self.session_browser_query.clear();
                self.session_browser_selected = 0;
                self.scroll = 0;
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Loaded session {}", session.id),
                ));
            }
            Err(e) => {
                self.messages.push(DisplayMessage::new_text(
                    Role::System,
                    format!("Failed to load session: {e}"),
                ));
            }
        }
    }

    fn display_messages_as_session_messages(&self) -> Vec<Message> {
        let mut out = Vec::new();
        for dm in &self.messages {
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
            if !text_content.trim().is_empty() || !tool_calls.is_empty() {
                out.push(Message {
                    role: dm.role.clone(),
                    content: text_content,
                    tool_calls,
                    tool_result: None,
                });
            }
        }
        out
    }

    fn save_current_session(&mut self) -> std::result::Result<std::path::PathBuf, String> {
        let mut session = if let Some(ref session_id) = self.current_session_id {
            match Session::load(session_id) {
                Ok(mut s) => {
                    s.model = self.model_name.clone();
                    s.tags = self.session_tags.clone();
                    s
                }
                Err(_) => Session::new_with_id(
                    session_id.clone(),
                    &self.model_name,
                    self.session_tags.clone(),
                ),
            }
        } else {
            let new_session = Session::new_with_tags(&self.model_name, self.session_tags.clone());
            self.current_session_id = Some(new_session.id.clone());
            new_session
        };
        session.messages = self.display_messages_as_session_messages();
        session.save().map_err(|e| e.to_string())
    }

    fn parse_prune_policy(arg: &str) -> RetentionPolicy {
        let mut policy = RetentionPolicy::default();
        for token in arg.split_whitespace() {
            if let Some(days) = token.strip_suffix('d').and_then(|n| n.parse::<u32>().ok()) {
                policy.max_age_days = Some(days);
            } else if let Some(max) = token
                .strip_suffix('s')
                .and_then(|n| n.parse::<usize>().ok())
            {
                policy.max_sessions = Some(max);
            }
        }
        policy
    }

    fn compaction_strategy_for(arg: &str) -> pawan::compaction::CompactionStrategy {
        use pawan::compaction::CompactionStrategy;
        match arg.trim() {
            "aggressive" => CompactionStrategy {
                keep_recent: 5,
                keep_keywords: vec!["error".into(), "fix".into(), "bug".into()],
                keep_tool_results: false,
                keep_system: false,
            },
            "conservative" => CompactionStrategy {
                keep_recent: 20,
                keep_keywords: vec![
                    "error".into(),
                    "fix".into(),
                    "bug".into(),
                    "issue".into(),
                    "problem".into(),
                    "solution".into(),
                    "important".into(),
                    "note".into(),
                    "warning".into(),
                    "decision".into(),
                    "todo".into(),
                ],
                keep_tool_results: true,
                keep_system: true,
            },
            _ => CompactionStrategy::default(),
        }
    }
}
