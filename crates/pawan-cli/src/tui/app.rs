//! `App` state, slash registry, and async entrypoints.

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

use super::fuzzy_search::{command_prefix, default_command_item_lines, FuzzySearchState};
use super::types::*;

pub(crate) struct App<'a> {
    pub(crate) config: TuiConfig,
    pub(crate) model_name: String,
    pub(crate) messages: Vec<DisplayMessage>,
    pub(crate) input: TextArea<'a>,
    pub(crate) scroll: usize,
    pub(crate) processing: bool,
    pub(crate) should_quit: bool,
    pub(crate) status: String,
    pub(crate) focus: Panel,
    /// Cumulative token usage across all requests
    pub(crate) total_tokens: u64,
    pub(crate) total_prompt_tokens: u64,
    pub(crate) total_completion_tokens: u64,
    /// Cumulative thinking vs action token split
    pub(crate) total_reasoning_tokens: u64,
    pub(crate) total_action_tokens: u64,
    /// Streaming assistant state: builds interleaved content blocks as events arrive
    pub(crate) streaming: Option<StreamingAssistantState>,
    /// Iteration count (increments on each tool completion)
    pub(crate) iteration_count: u32,
    /// Context tokens estimate
    pub(crate) context_estimate: usize,
    /// Search mode state
    pub(crate) search_mode: bool,
    pub(crate) search_query: String,
    /// Fuzzy search over slash commands (Ctrl+P / Ctrl+F)
    pub(crate) fuzzy_search: Option<FuzzySearchState>,
    /// Keyboard shortcuts overlay (F1)
    pub(crate) help_overlay: bool,
    /// Session stats
    pub(crate) session_tool_calls: u32,
    pub(crate) session_files_edited: u32,
    /// Inline slash command popup (triggered by typing /)
    pub(crate) slash_popup_selected: usize,
    /// File completion popup (triggered by typing @)
    #[allow(dead_code)]
    pub(crate) file_completion_open: bool,
    #[allow(dead_code)]
    pub(crate) file_completion_query: String,
    #[allow(dead_code)]
    pub(crate) file_completion_selected: usize,
    /// Welcome screen shown on first launch
    pub(crate) show_welcome: bool,
    /// Permission dialog state — when Some, the agent is waiting for y/n
    pub(crate) permission_dialog: Option<PermissionDialog>,
    /// Auto-approve all tool calls for this session (set when user selects "yes to all")
    pub(crate) auto_approve_tools: bool,
    /// Channel to send commands to the agent task
    pub(crate) cmd_tx: mpsc::UnboundedSender<AgentCommand>,
    /// Channel to receive events from the agent task
    pub(crate) event_rx: mpsc::UnboundedReceiver<AgentEvent>,
    /// Keybinding context (refreshed each frame from UI state)
    pub(crate) current_context: KeybindContext,
    /// Model picker modal
    pub(crate) model_picker: ModelPickerState,
    /// Session browser state
    pub(crate) session_browser_open: bool,
    pub(crate) session_browser_query: String,
    pub(crate) session_browser_selected: usize,
    pub(crate) session_sort_mode: SessionSortMode,
    /// Tags for the current session
    pub(crate) session_tags: Vec<String>,
    /// Current session ID (for autosave)
    pub(crate) current_session_id: Option<String>,
    /// Last autosave time
    pub(crate) last_autosave: Instant,
    /// Command history for up/down arrow navigation
    pub(crate) history: Vec<String>,
    /// Current position in history (None means not browsing history)
    pub(crate) history_position: Option<usize>,
    /// Set while a slash command is being dispatched
    pub(crate) slash_inflight: Option<String>,
    /// Slash command registry (metadata + shared handler)
    pub(crate) slash_registry: SlashCommandRegistry,
}

/// Registered TUI `/command` (names are explicit, including short aliases)
#[allow(private_interfaces, dead_code)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    /// All built-ins share a single entrypoint that defers to `App::slash_route`
    pub handler: fn(&mut App<'_>, &[&str]) -> Result<()>,
    /// Extra tab-completion options (e.g. model id hints) — optional
    pub completion: Vec<String>,
}

/// Registry of slash commands shown in /help, completion, and dispatch allow-list
pub struct SlashCommandRegistry {
    commands: Vec<SlashCommand>,
}

impl SlashCommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn register(&mut self, cmd: SlashCommand) {
        self.commands.push(cmd);
    }

    pub fn get(&self, name: &str) -> Option<&SlashCommand> {
        self.commands.iter().find(|c| c.name == name)
    }

    /// Prefix match on the command name (e.g. `/m` returns `/model`, `/m`, ...).
    pub fn complete(&self, prefix: &str) -> Vec<&SlashCommand> {
        let p = prefix.to_lowercase();
        self.commands
            .iter()
            .filter(|c| c.name.to_lowercase().starts_with(&p))
            .collect()
    }

    pub fn all(&self) -> &[SlashCommand] {
        &self.commands
    }

    /// Help string for /help, derived from the registry
    pub(crate) fn help_text(&self) -> String {
        let mut cmds: Vec<&SlashCommand> = self.commands.iter().collect();
        cmds.sort_by(|a, b| a.name.cmp(&b.name));
        let mut out = String::new();
        for c in cmds {
            out.push_str(&format!("{:<18} - {}\n", c.name, c.description));
        }
        out
    }

    pub fn built_in() -> Self {
        const H: fn(&mut App<'_>, &[&str]) -> Result<()> = App::universal_slash_entry;
        let mut r = Self::new();
        for (name, desc) in BUILTIN_SLASH_COMMANDS {
            r.register(SlashCommand {
                name: (*name).to_string(),
                description: (*desc).to_string(),
                handler: H,
                completion: vec![],
            });
        }
        r
    }
}
const BUILTIN_SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/clear", "Clear chat history"),
    ("/c", "Clear chat history (shorthand)"),
    ("/model", "Show or switch LLM model"),
    ("/m", "Show or switch model (shorthand)"),
    ("/tools", "List available tools"),
    ("/t", "List tools (shorthand)"),
    ("/search", "Web search via Daedra"),
    ("/s", "Web search (shorthand)"),
    ("/handoff", "Hand off conversation to a new session"),
    ("/heal", "Auto-fix build errors"),
    ("/h", "Heal (shorthand)"),
    ("/quit", "Exit pawan"),
    ("/q", "Exit (shorthand)"),
    ("/exit", "Exit pawan (alias)"),
    ("/export", "Export conversation to a file"),
    ("/e", "Export (shorthand)"),
    ("/diff", "Show git diff"),
    ("/d", "Show git diff (shorthand)"),
    ("/import", "Import conversation from JSON"),
    ("/fork", "Clone current session to a new one"),
    ("/dump", "Copy conversation to clipboard"),
    ("/share", "Export session and print a shareable path"),
    ("/save", "Save current conversation as a session"),
    ("/sessions", "Browse and manage saved sessions"),
    ("/ss", "Search saved sessions"),
    ("/searchsessions", "Search saved sessions (alias)"),
    ("/prune", "Prune old saved sessions"),
    ("/tag", "Manage session tags (add/rm/list/clear)"),
    ("/load", "Load a saved session"),
    ("/resume", "Resume a saved session"),
    ("/new", "Start a fresh conversation"),
    ("/session", "Switch to a session by id"),
    ("/retry", "Retry the last assistant response"),
    ("/compact", "Compact the conversation context"),
    ("/help", "Show this help list"),
    ("/?", "Show help (shorthand)"),
];

/// State for an active permission prompt dialog
pub(crate) struct PermissionDialog {
    pub(crate) tool_name: String,
    pub(crate) args_summary: String,
    pub(crate) respond: Option<tokio::sync::oneshot::Sender<bool>>,
}

impl<'a> App<'a> {
    pub fn new(
        config: TuiConfig,
        model_name: String,
        cmd_tx: mpsc::UnboundedSender<AgentCommand>,
        event_rx: mpsc::UnboundedReceiver<AgentEvent>,
    ) -> Self {
        let mut input = TextArea::default();
        input.set_cursor_line_style(Style::default());
        input.set_placeholder_text(
            "Type your message... (Enter to send, ↑↓ for history, Ctrl+C to clear, Ctrl+Q to quit)",
        );

        Self {
            config,
            model_name,
            messages: Vec::new(),
            input,
            scroll: 0,
            processing: false,
            should_quit: false,
            status: "Ready".to_string(),
            focus: Panel::Input,
            total_tokens: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_reasoning_tokens: 0,
            total_action_tokens: 0,
            streaming: None,
            iteration_count: 0,
            context_estimate: 0,
            search_mode: false,
            search_query: String::new(),
            fuzzy_search: None,
            help_overlay: false,
            session_tool_calls: 0,
            session_files_edited: 0,
            session_tags: Vec::new(),
            current_session_id: None,
            slash_popup_selected: 0,
            file_completion_open: false,
            file_completion_query: String::new(),
            file_completion_selected: 0,
            show_welcome: true,
            permission_dialog: None,
            auto_approve_tools: false,
            cmd_tx,
            event_rx,
            current_context: KeybindContext::Input,
            model_picker: ModelPickerState {
                models: Vec::new(),
                selected: 0,
                visible: false,
                query: String::new(),
            },
            session_browser_open: false,
            session_browser_query: String::new(),
            session_browser_selected: 0,
            session_sort_mode: SessionSortMode::NewestFirst,
            last_autosave: Instant::now(),
            history: Vec::new(),
            history_position: None,
            slash_inflight: None,
            slash_registry: SlashCommandRegistry::built_in(),
        }
    }

    /// Derive keybinding context from modal / focus state.
    pub(crate) fn determine_keybind_context(&self) -> KeybindContext {
        if self.help_overlay {
            KeybindContext::Help
        } else if self.fuzzy_search.is_some() {
            KeybindContext::Command
        } else if self.model_picker.visible {
            KeybindContext::ModelPicker
        } else if self.focus == Panel::Messages {
            KeybindContext::Normal
        } else {
            KeybindContext::Input
        }
    }

    pub(crate) fn refresh_keybind_context(&mut self) {
        self.current_context = self.determine_keybind_context();
    }

    pub(crate) fn toggle_fuzzy_search(&mut self) {
        if self.fuzzy_search.is_some() {
            self.fuzzy_search = None;
        } else {
            self.fuzzy_search = Some(FuzzySearchState::new(default_command_item_lines()));
        }
    }

    pub(crate) fn keybind_status_hint(&self) -> String {
        if self.is_slash_popup_active() {
            let text: String = self.input.lines().join("\n");
            let mut q = text.trim().to_lowercase();
            if q.starts_with(':') {
                if q == ":" {
                    q = "/".to_string();
                } else {
                    q = format!("/{}", &q[1..]);
                }
            }
            if let Some(cmd) = self.slash_registry.get(&q) {
                return format!("cmd {} — {}", cmd.name, cmd.description);
            }
            let matches = self.slash_registry.complete(&q);
            if matches.len() == 1 {
                let c = matches[0];
                return format!("cmd {} — {}", c.name, c.description);
            }
            if matches.is_empty() {
                return "cmd: (no matches) — keep typing or Esc".to_string();
            }
            return format!("cmd: {} matches — ↑↓ pick", matches.len());
        }
        fn row(actions: &[KeyAction]) -> String {
            actions
                .iter()
                .map(|a| format!("{} {}", a.key, a.description))
                .collect::<Vec<_>>()
                .join(" · ")
        }
        match self.current_context {
            KeybindContext::Input => row(&[
                KeyAction {
                    context: KeybindContext::Input,
                    key: "^Q",
                    description: "quit",
                },
                KeyAction {
                    context: KeybindContext::Input,
                    key: "^P^F/:",
                    description: "search",
                },
                KeyAction {
                    context: KeybindContext::Input,
                    key: "^M",
                    description: "models",
                },
                KeyAction {
                    context: KeybindContext::Input,
                    key: "F1",
                    description: "help",
                },
                KeyAction {
                    context: KeybindContext::Input,
                    key: "Tab",
                    description: "msgs",
                },
            ]),
            KeybindContext::Normal => row(&[
                KeyAction {
                    context: KeybindContext::Normal,
                    key: "j/k",
                    description: "scroll",
                },
                KeyAction {
                    context: KeybindContext::Normal,
                    key: "/",
                    description: "search",
                },
                KeyAction {
                    context: KeybindContext::Normal,
                    key: "i",
                    description: "input",
                },
                KeyAction {
                    context: KeybindContext::Normal,
                    key: "^M",
                    description: "models",
                },
            ]),
            KeybindContext::Command => row(&[
                KeyAction {
                    context: KeybindContext::Command,
                    key: "Esc",
                    description: "close",
                },
                KeyAction {
                    context: KeybindContext::Command,
                    key: "Enter",
                    description: "run",
                },
            ]),
            KeybindContext::Help => row(&[KeyAction {
                context: KeybindContext::Help,
                key: "any",
                description: "close",
            }]),
            KeybindContext::ModelPicker => row(&[
                KeyAction {
                    context: KeybindContext::ModelPicker,
                    key: "Esc",
                    description: "cancel",
                },
                KeyAction {
                    context: KeybindContext::ModelPicker,
                    key: "Enter",
                    description: "pick",
                },
            ]),
        }
    }

    /// Switch the active model (UI + agent task).
    pub(crate) fn switch_model(&mut self, model_id: String) {
        self.model_name = model_id.clone();
        self.status = format!("Model → {}", model_id);
        self.messages.push(DisplayMessage::new_text(
            Role::System,
            format!("Switched to model: {}", model_id),
        ));
        let _ = self.cmd_tx.send(AgentCommand::SwitchModel(model_id));
    }

    /// Convert persisted core session messages into TUI display messages.
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

    pub async fn run(&mut self) -> Result<()> {
        enable_raw_mode().map_err(PawanError::Io)?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(PawanError::Io)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(PawanError::Io)?;

        let result = self.main_loop(&mut terminal).await;

        disable_raw_mode().ok();
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .ok();
        terminal.show_cursor().ok();

        result
    }

    pub(crate) async fn main_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        loop {
            self.refresh_keybind_context();
            terminal.draw(|f| self.ui(f)).map_err(PawanError::Io)?;

            // Non-blocking: check for agent events first
            while let Ok(event) = self.event_rx.try_recv() {
                match event {
                    AgentEvent::Token(token) => {
                        let state = self
                            .streaming
                            .get_or_insert_with(|| StreamingAssistantState { blocks: Vec::new() });
                        // Append to last streaming text block, or start a new one
                        match state.blocks.last_mut() {
                            Some(ContentBlock::Text {
                                content,
                                streaming: true,
                            }) => {
                                content.push_str(&token);
                            }
                            _ => {
                                state.blocks.push(ContentBlock::Text {
                                    content: token,
                                    streaming: true,
                                });
                            }
                        }
                        self.scroll = usize::MAX;
                    }
                    AgentEvent::ToolStart(name) => {
                        let state = self
                            .streaming
                            .get_or_insert_with(|| StreamingAssistantState { blocks: Vec::new() });
                        // Freeze current text block
                        if let Some(ContentBlock::Text { streaming, .. }) = state.blocks.last_mut()
                        {
                            *streaming = false;
                        }
                        state.blocks.push(ContentBlock::ToolCall {
                            name: name.clone(),
                            args_summary: String::new(),
                            state: Box::new(ToolBlockState::Running),
                        });
                        self.status = format!("Running tool: {}", name);
                    }
                    AgentEvent::ToolComplete(record) => {
                        if let Some(state) = &mut self.streaming {
                            for block in state.blocks.iter_mut().rev() {
                                if let ContentBlock::ToolCall {
                                    name,
                                    args_summary,
                                    state: tool_state,
                                } = block
                                {
                                    if matches!(tool_state.as_ref(), ToolBlockState::Running)
                                        && *name == record.name
                                    {
                                        *args_summary = summarize_args(&record.arguments);
                                        **tool_state = ToolBlockState::Done {
                                            record: record.clone(),
                                            expanded: !record.success,
                                        };
                                        break;
                                    }
                                }
                            }
                        }
                        self.session_tool_calls += 1;
                        if record.name.contains("write_file") || record.name.contains("edit_file") {
                            self.session_files_edited += 1;
                        }
                        let icon = if record.success { "✓" } else { "✗" };
                        self.status =
                            format!("{} {} ({}ms)", icon, record.name, record.duration_ms);
                    }
                    AgentEvent::PermissionRequest {
                        tool_name,
                        args_summary,
                        respond,
                    } => {
                        if self.auto_approve_tools {
                            // Auto-approve all tool calls
                            let _ = respond.send(true);
                            self.status = format!("Auto-approved: {}", tool_name);
                        } else {
                            self.permission_dialog = Some(PermissionDialog {
                                tool_name: tool_name.clone(),
                                args_summary: args_summary.clone(),
                                respond: Some(respond),
                            });
                            self.status = format!("Permission required: {} — y/n/a", tool_name);
                        }
                    }
                    AgentEvent::Complete(result) => {
                        self.processing = false;
                        match result {
                            Ok(resp) => {
                                let msg = if let Some(state) = self.streaming.take() {
                                    let mut blocks = state.blocks;
                                    for block in &mut blocks {
                                        if let ContentBlock::Text { streaming, .. } = block {
                                            *streaming = false;
                                        }
                                    }
                                    DisplayMessage {
                                        role: Role::Assistant,
                                        blocks,
                                        timestamp: std::time::Instant::now(),
                                        cached_block_lines: None,
                                    }
                                } else {
                                    DisplayMessage::from_agent_response(&resp)
                                };
                                self.messages.push(msg);
                                // Pre-populate render cache for the finalized message
                                if let Some(last) = self.messages.last_mut() {
                                    last.block_lines_cached();
                                }
                                self.total_tokens += resp.usage.total_tokens;
                                self.total_prompt_tokens += resp.usage.prompt_tokens;
                                self.total_completion_tokens += resp.usage.completion_tokens;
                                self.total_reasoning_tokens += resp.usage.reasoning_tokens;
                                self.total_action_tokens += resp.usage.action_tokens;
                                self.context_estimate = (self.total_prompt_tokens
                                    + self.total_completion_tokens)
                                    as usize;
                                self.status = format!("Done ({} iterations)", resp.iterations);
                                self.scroll = self.messages.len().saturating_sub(1);
                            }
                            Err(e) => {
                                self.streaming = None;
                                self.status = format!("Error: {}", e);
                                self.messages.push(DisplayMessage::new_text(
                                    Role::Assistant,
                                    format!("Error: {}", e),
                                ));
                                self.scroll = self.messages.len().saturating_sub(1);
                            }
                        }
                    }
                }
            }

            // Handle terminal events with short poll timeout
            if event::poll(std::time::Duration::from_millis(50)).map_err(PawanError::Io)? {
                let event = event::read().map_err(PawanError::Io)?;
                self.handle_event(event);
            }

            // Periodic autosave
            if self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
                self.autosave();
                self.last_autosave = Instant::now();
            }

            if self.should_quit {
                // Final autosave before exit
                self.autosave();
                let _ = self.cmd_tx.send(AgentCommand::Quit);
                break;
            }
        }

        Ok(())
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
        writeln!(f, "<!DOCTYPE html>\n").map_err(|e| e.to_string())?;
        writeln!(f, "<html lang='en'>\n").map_err(|e| e.to_string())?;
        writeln!(f, "<head>\n").map_err(|e| e.to_string())?;
        writeln!(f, "  <meta charset='UTF-8'>\n").map_err(|e| e.to_string())?;
        writeln!(
            f,
            "  <meta name='viewport' content='width=device-width, initial-scale=1.0'>\n"
        )
        .map_err(|e| e.to_string())?;
        writeln!(f, "  <title>Pawan Session</title>\n").map_err(|e| e.to_string())?;
        writeln!(f, "  <style>\n").map_err(|e| e.to_string())?;
        writeln!(f, "    body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; line-height: 1.6; }}\n").map_err(|e| e.to_string())?;
        writeln!(
            f,
            "    .message {{ margin: 20px 0; padding: 15px; border-radius: 8px; }}\n"
        )
        .map_err(|e| e.to_string())?;
        writeln!(f, "    .user {{ background-color: #e3f2fd; }}\n").map_err(|e| e.to_string())?;
        writeln!(f, "    .assistant {{ background-color: #f3e5f5; }}\n")
            .map_err(|e| e.to_string())?;
        writeln!(f, "    .system {{ background-color: #f5f5f5; }}\n").map_err(|e| e.to_string())?;
        writeln!(
            f,
            "    .role {{ font-weight: bold; margin-bottom: 10px; }}\n"
        )
        .map_err(|e| e.to_string())?;
        writeln!(f, "    .content {{ white-space: pre-wrap; }}\n").map_err(|e| e.to_string())?;
        writeln!(f, "    .tool-calls {{ margin-top: 10px; padding: 10px; background-color: #fff3cd; border-radius: 4px; }}\n").map_err(|e| e.to_string())?;
        writeln!(f, "    .footer {{ margin-top: 30px; padding-top: 20px; border-top: 1px solid #ddd; color: #666; }}\n").map_err(|e| e.to_string())?;
        writeln!(f, "  </style>\n").map_err(|e| e.to_string())?;
        writeln!(f, "</head>\n").map_err(|e| e.to_string())?;
        writeln!(f, "<body>\n").map_err(|e| e.to_string())?;
        writeln!(f, "  <h1>Pawan Session</h1>\n").map_err(|e| e.to_string())?;
        writeln!(f, "  <p><strong>Model:</strong> {}</p>\n", self.model_name)
            .map_err(|e| e.to_string())?;
        for msg in &self.messages {
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
            writeln!(f, "  <div class='message {}'>\n", class).map_err(|e| e.to_string())?;
            writeln!(f, "    <div class='role'>{}</div>\n", role_name)
                .map_err(|e| e.to_string())?;
            writeln!(
                f,
                "    <div class='content'>{}</div>\n",
                Self::html_escape(&msg.text_content())
            )
            .map_err(|e| e.to_string())?;
            let tool_records = msg.tool_records();
            if !tool_records.is_empty() {
                writeln!(f, "    <div class='tool-calls'>\n").map_err(|e| e.to_string())?;
                writeln!(
                    f,
                    "      <strong>Tool calls ({}):</strong>\n",
                    tool_records.len()
                )
                .map_err(|e| e.to_string())?;
                for tc in tool_records {
                    let status = if tc.success { "✓" } else { "✗" };
                    writeln!(f, "      {} `{}` — {}ms\n", status, tc.name, tc.duration_ms)
                        .map_err(|e| e.to_string())?;
                }
                writeln!(f, "    </div>\n").map_err(|e| e.to_string())?;
            }
            writeln!(f, "  </div>\n").map_err(|e| e.to_string())?;
        }
        writeln!(f, "  <div class='footer'>\n").map_err(|e| e.to_string())?;
        writeln!(
            f,
            "    Tokens: {} total ({} prompt, {} completion)\n",
            self.total_tokens, self.total_prompt_tokens, self.total_completion_tokens
        )
        .map_err(|e| e.to_string())?;
        writeln!(f, "  </div>\n").map_err(|e| e.to_string())?;
        writeln!(f, "</body>\n").map_err(|e| e.to_string())?;
        writeln!(f, "</html>\n").map_err(|e| e.to_string())?;
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
    /// This strips noise while preserving file paths, constraints, and key context
    pub(crate) fn generate_handoff_prompt(&self) -> String {
        use std::collections::HashSet;

        if self.messages.is_empty() {
            return "No conversation context available.".to_string();
        }

        let mut context_parts = Vec::new();
        let mut file_paths: HashSet<String> = HashSet::new();
        let mut constraints = Vec::new();
        let mut key_tasks = Vec::new();

        // Extract key information from messages
        for msg in &self.messages {
            let content = msg.text_content();

            // Extract file paths (common patterns)
            for line in content.lines() {
                // Match file paths like src/main.rs, /path/to/file, etc.
                if line.contains(".rs")
                    || line.contains(".ts")
                    || line.contains(".js")
                    || line.contains(".py")
                    || line.contains(".go")
                    || line.contains(".java")
                    || line.contains("/")
                        && (line.contains("src") || line.contains("lib") || line.contains("test"))
                {
                    // Extract potential file paths
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

                // Extract constraints (MUST, MUST NOT, should, etc.)
                if line.contains("MUST")
                    || line.contains("MUST NOT")
                    || line.contains("should")
                    || line.contains("constraint")
                    || line.contains("requirement")
                {
                    constraints.push(line.trim().to_string());
                }

                // Extract key tasks (imperative statements, TODO, etc.)
                if line.starts_with("-")
                    || line.starts_with("*")
                    || line.contains("TODO")
                    || line.contains("implement")
                    || line.contains("fix")
                    || line.contains("add")
                    || line.contains("create")
                {
                    key_tasks.push(line.trim().to_string());
                }
            }
        }

        // Build the handoff prompt
        context_parts.push("# Session Handoff".to_string());
        context_parts.push(String::new());
        context_parts.push(format!("**Model:** {}", self.model_name));
        context_parts.push(format!("**Messages:** {}", self.messages.len()));
        context_parts.push(format!("**Tool calls:** {}", self.session_tool_calls));
        context_parts.push(format!("**Files edited:** {}", self.session_files_edited));
        context_parts.push(String::new());

        // Add file paths if any
        if !file_paths.is_empty() {
            context_parts.push("## Files Referenced".to_string());
            let mut paths: Vec<_> = file_paths.into_iter().collect();
            paths.sort();
            for path in paths {
                context_parts.push(format!("- {}", path));
            }
            context_parts.push(String::new());
        }

        // Add constraints if any
        if !constraints.is_empty() {
            context_parts.push("## Constraints".to_string());
            for constraint in constraints.iter().take(10) {
                // Limit to 10 constraints
                context_parts.push(format!("- {}", constraint));
            }
            if constraints.len() > 10 {
                context_parts.push(format!("- ... and {} more", constraints.len() - 10));
            }
            context_parts.push(String::new());
        }

        // Add key tasks if any
        if !key_tasks.is_empty() {
            context_parts.push("## Key Tasks".to_string());
            for task in key_tasks.iter().take(15) {
                // Limit to 15 tasks
                context_parts.push(format!("- {}", task));
            }
            if key_tasks.len() > 15 {
                context_parts.push(format!("- ... and {} more", key_tasks.len() - 15));
            }
            context_parts.push(String::new());
        }

        // Add summary of last few messages for context
        context_parts.push("## Recent Context".to_string());
        let recent_count = self.messages.len().min(3);
        for msg in self.messages.iter().rev().take(recent_count).rev() {
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
            context_parts.push(format!("**{}:** {}", role, preview));
        }

        context_parts.join("\n")
    }
    /// Load available models (synchronous version)
    pub(crate) fn load_available_models(&mut self) {
        let default_models = vec![
            // 01-ai models (1)
            ModelInfo {
                id: "01-ai/yi-large".to_string(),
                provider: "01-ai".to_string(),
                quality_score: 75,
            },
            // Abacusai models (1)
            ModelInfo {
                id: "abacusai/dracarys-llama-3.1-70b-instruct".to_string(),
                provider: "Abacusai".to_string(),
                quality_score: 93,
            },
            // Ai21labs models (1)
            ModelInfo {
                id: "ai21labs/jamba-1.5-large-instruct".to_string(),
                provider: "Ai21labs".to_string(),
                quality_score: 75,
            },
            // Aisingapore models (1)
            ModelInfo {
                id: "aisingapore/sea-lion-7b-instruct".to_string(),
                provider: "Aisingapore".to_string(),
                quality_score: 79,
            },
            // Bigcode models (1)
            ModelInfo {
                id: "bigcode/starcoder2-15b".to_string(),
                provider: "Bigcode".to_string(),
                quality_score: 75,
            },
            // Bytedance models (1)
            ModelInfo {
                id: "bytedance/seed-oss-36b-instruct".to_string(),
                provider: "Bytedance".to_string(),
                quality_score: 75,
            },
            // Databricks models (1)
            ModelInfo {
                id: "databricks/dbrx-instruct".to_string(),
                provider: "Databricks".to_string(),
                quality_score: 75,
            },
            // DeepSeek models (3)
            ModelInfo {
                id: "deepseek-ai/deepseek-coder-6.7b-instruct".to_string(),
                provider: "Deepseek-ai".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "deepseek-ai/deepseek-v3.1-terminus".to_string(),
                provider: "Deepseek-ai".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "deepseek-ai/deepseek-v3.2".to_string(),
                provider: "Deepseek-ai".to_string(),
                quality_score: 93,
            },
            // Google models (10)
            ModelInfo {
                id: "google/codegemma-1.1-7b".to_string(),
                provider: "Google".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "google/codegemma-7b".to_string(),
                provider: "Google".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "google/gemma-2-2b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "google/gemma-2b".to_string(),
                provider: "Google".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "google/gemma-3-12b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "google/gemma-3-27b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "google/gemma-3-4b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "google/gemma-3n-e2b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "google/gemma-3n-e4b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "google/gemma-4-31b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 87,
            },
            // IBM models (4)
            ModelInfo {
                id: "ibm/granite-3.0-3b-a800m-instruct".to_string(),
                provider: "Ibm".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "ibm/granite-3.0-8b-instruct".to_string(),
                provider: "Ibm".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "ibm/granite-34b-code-instruct".to_string(),
                provider: "Ibm".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "ibm/granite-8b-code-instruct".to_string(),
                provider: "Ibm".to_string(),
                quality_score: 81,
            },
            // Meta models (8)
            ModelInfo {
                id: "meta/llama-3.1-405b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "meta/llama-3.1-70b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 93,
            },
            ModelInfo {
                id: "meta/llama-3.1-8b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "meta/llama-3.2-11b-vision-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "meta/llama-3.2-1b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "meta/llama-3.2-3b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "meta/llama-3.3-70b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "meta/llama-4-maverick-17b-128e-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 95,
            },
            // Microsoft models (4)
            ModelInfo {
                id: "microsoft/phi-3-vision-128k-instruct".to_string(),
                provider: "Microsoft".to_string(),
                quality_score: 83,
            },
            ModelInfo {
                id: "microsoft/phi-3.5-moe-instruct".to_string(),
                provider: "Microsoft".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "microsoft/phi-4-mini-instruct".to_string(),
                provider: "Microsoft".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "microsoft/phi-4-multimodal-instruct".to_string(),
                provider: "Microsoft".to_string(),
                quality_score: 91,
            },
            // MiniMax models (2)
            ModelInfo {
                id: "minimaxai/minimax-m2.5".to_string(),
                provider: "Minimaxai".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "minimaxai/minimax-m2.7".to_string(),
                provider: "Minimaxai".to_string(),
                quality_score: 89,
            },
            // Mistral models (14)
            ModelInfo {
                id: "mistralai/codestral-22b-instruct-v0.1".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "mistralai/devstral-2-123b-instruct-2512".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "mistralai/magistral-small-2506".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 75,
            },
            ModelInfo {
                id: "mistralai/ministral-14b-instruct-2512".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "mistralai/mistral-7b-instruct-v0.3".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "mistralai/mistral-large".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "mistralai/mistral-large-2-instruct".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 93,
            },
            ModelInfo {
                id: "mistralai/mistral-large-3-675b-instruct-2512".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "mistralai/mistral-medium-3-instruct".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "mistralai/mistral-nemotron".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "mistralai/mistral-small-4-119b-2603".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "mistralai/mixtral-8x22b-instruct-v0.1".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "mistralai/mixtral-8x22b-v0.1".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "mistralai/mixtral-8x7b-instruct-v0.1".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 83,
            },
            // Moonshot models (4)
            ModelInfo {
                id: "moonshotai/kimi-k2-instruct".to_string(),
                provider: "Moonshotai".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "moonshotai/kimi-k2-instruct-0905".to_string(),
                provider: "Moonshotai".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "moonshotai/kimi-k2-thinking".to_string(),
                provider: "Moonshotai".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "moonshotai/kimi-k2.5".to_string(),
                provider: "Moonshotai".to_string(),
                quality_score: 93,
            },
            // NV-Mistral models (1)
            ModelInfo {
                id: "nv-mistralai/mistral-nemo-12b-instruct".to_string(),
                provider: "Nv-mistralai".to_string(),
                quality_score: 81,
            },
            // NVIDIA models (15)
            ModelInfo {
                id: "nvidia/ising-calibration-1-35b-a3b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-51b-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-70b-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 93,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-nano-8b-v1".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-nano-vl-8b-v1".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-ultra-253b-v1".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 93,
            },
            ModelInfo {
                id: "nvidia/llama-3.3-nemotron-super-49b-v1".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 75,
            },
            ModelInfo {
                id: "nvidia/llama-3.3-nemotron-super-49b-v1.5".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 75,
            },
            ModelInfo {
                id: "nvidia/llama3-chatqa-1.5-70b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "nvidia/mistral-nemo-minitron-8b-8k-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "nvidia/nemotron-3-nano-30b-a3b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "nvidia/nemotron-3-super-120b-a12b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "nvidia/nemotron-4-340b-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "nvidia/nemotron-4-340b-reward".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "nvidia/nemotron-mini-4b-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "nvidia/nemotron-nano-12b-v2-vl".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "nvidia/nemotron-nano-3-30b-a3b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "nvidia/nvidia-nemotron-nano-9b-v2".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 75,
            },
            // OpenAI models (4)
            ModelInfo {
                id: "openai/gpt-oss-120b".to_string(),
                provider: "OpenAI".to_string(),
                quality_score: 75,
            },
            ModelInfo {
                id: "openai/gpt-oss-20b".to_string(),
                provider: "OpenAI".to_string(),
                quality_score: 75,
            },
            // Qwen models (6)
            ModelInfo {
                id: "qwen/qwen2.5-coder-32b-instruct".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "qwen/qwen3-coder-480b-a35b-instruct".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "qwen/qwen3-next-80b-a3b-instruct".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "qwen/qwen3-next-80b-a3b-thinking".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "qwen/qwen3.5-122b-a10b".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "qwen/qwen3.5-397b-a17b".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 95,
            },
            // Sarvamai models (1)
            ModelInfo {
                id: "sarvamai/sarvam-m".to_string(),
                provider: "Sarvamai".to_string(),
                quality_score: 75,
            },
            // StepFun models (1)
            ModelInfo {
                id: "stepfun-ai/step-3.5-flash".to_string(),
                provider: "Stepfun-ai".to_string(),
                quality_score: 85,
            },
            // Stockmark models (1)
            ModelInfo {
                id: "stockmark/stockmark-2-100b-instruct".to_string(),
                provider: "Stockmark".to_string(),
                quality_score: 75,
            },
            // Upstage models (1)
            ModelInfo {
                id: "upstage/solar-10.7b-instruct".to_string(),
                provider: "Upstage".to_string(),
                quality_score: 79,
            },
            // Writer models (4)
            ModelInfo {
                id: "writer/palmyra-creative-122b".to_string(),
                provider: "Writer".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "writer/palmyra-fin-70b-32k".to_string(),
                provider: "Writer".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "writer/palmyra-med-70b".to_string(),
                provider: "Writer".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "writer/palmyra-med-70b-32k".to_string(),
                provider: "Writer".to_string(),
                quality_score: 87,
            },
            // Z-AI models (3)
            ModelInfo {
                id: "z-ai/glm-5.1".to_string(),
                provider: "Z-ai".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "z-ai/glm4.7".to_string(),
                provider: "Z-ai".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "z-ai/glm5".to_string(),
                provider: "Z-ai".to_string(),
                quality_score: 95,
            },
            // Zyphra models (1)
            ModelInfo {
                id: "zyphra/zamba2-7b-instruct".to_string(),
                provider: "Zyphra".to_string(),
                quality_score: 79,
            },
        ];
        self.model_picker.models = default_models;
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

async fn agent_task(
    mut agent: PawanAgent,
    mut cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
) {
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AgentCommand::Execute(prompt) => {
                // Create token streaming callback
                let token_tx = event_tx.clone();
                let on_token: pawan::agent::TokenCallback = Box::new(move |token: &str| {
                    let _ = token_tx.send(AgentEvent::Token(token.to_string()));
                });

                // Create tool start callback
                let tool_start_tx = event_tx.clone();
                let on_tool_start: pawan::agent::ToolStartCallback = Box::new(move |name: &str| {
                    let _ = tool_start_tx.send(AgentEvent::ToolStart(name.to_string()));
                });

                // Create tool complete callback
                let tool_tx = event_tx.clone();
                let on_tool: pawan::agent::ToolCallback =
                    Box::new(move |record: &ToolCallRecord| {
                        let _ = tool_tx.send(AgentEvent::ToolComplete(record.clone()));
                    });

                // Create permission callback — sends request to TUI, returns oneshot receiver
                let perm_tx = event_tx.clone();
                let on_permission: pawan::agent::PermissionCallback =
                    Box::new(move |req: pawan::agent::PermissionRequest| {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let _ = perm_tx.send(AgentEvent::PermissionRequest {
                            tool_name: req.tool_name,
                            args_summary: req.args_summary,
                            respond: tx,
                        });
                        rx
                    });

                let result = agent
                    .execute_with_all_callbacks(
                        &prompt,
                        Some(on_token),
                        Some(on_tool),
                        Some(on_tool_start),
                        Some(on_permission),
                    )
                    .await;
                let _ = event_tx.send(AgentEvent::Complete(result));
            }
            AgentCommand::SwitchModel(model) => {
                let _ = agent.switch_model(&model);
                let _ = event_tx.send(AgentEvent::Complete(Ok(AgentResponse {
                    content: format!("Model switched to: {}", model),
                    tool_calls: vec![],
                    iterations: 0,
                    usage: pawan::agent::TokenUsage::default(),
                })));
            }
            AgentCommand::Quit => break,
        }
    }
}

/// Run the TUI with the given agent
pub async fn run_tui(agent: PawanAgent, config: TuiConfig) -> Result<()> {
    let model_name = agent.config().model.clone();

    // Create channels
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Spawn agent on a separate task
    tokio::spawn(agent_task(agent, cmd_rx, event_tx));

    // Run the TUI on the current task
    let mut app = App::new(config, model_name, cmd_tx, event_rx);
    app.run().await
}

/// Simple non-TUI interactive mode (fallback)
/// Simple non-TUI interactive mode (fallback)
pub async fn run_simple(mut agent: PawanAgent) -> Result<()> {
    use std::io::{BufRead, Write};

    println!("Pawan - Self-Healing CLI Coding Agent");
    println!("Type 'quit' or 'exit' to quit, 'clear' to clear history");
    println!("---");

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        print!("> ");
        stdout.flush().ok();

        let mut line = String::new();
        stdin.lock().read_line(&mut line).ok();

        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "quit" || line == "exit" {
            break;
        }
        if line == "clear" {
            agent.clear_history();
            println!("History cleared.");
            continue;
        }

        println!("\nProcessing...\n");

        match agent.execute(line).await {
            Ok(response) => {
                println!("{}\n", response.content);
                if !response.tool_calls.is_empty() {
                    println!("Tool calls made:");
                    for tc in &response.tool_calls {
                        let status = if tc.success { "✓" } else { "✗" };
                        println!("  {} {} ({}ms)", status, tc.name, tc.duration_ms);
                    }
                    println!();
                }
            }
            Err(e) => println!("Error: {}\n", e),
        }
    }

    Ok(())
}
