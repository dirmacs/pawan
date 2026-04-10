//! Terminal User Interface for Pawan
//!
//! Non-blocking TUI: agent runs on a spawned tokio task,
//! events stream back to the UI via mpsc channel.

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pawan::agent::{AgentResponse, PawanAgent, Role, ToolCallRecord};
use pawan::config::TuiConfig;
use pawan::{PawanError, Result};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use tokio::sync::mpsc;
use tui_textarea::{Input, TextArea};

/// Events sent from the agent task back to the TUI
enum AgentEvent {
    /// Streaming token from LLM
    Token(String),
    /// A tool call started
    ToolStart(String),
    /// A tool call completed
    ToolComplete(ToolCallRecord),
    /// Agent requests permission to run a tool
    PermissionRequest {
        tool_name: String,
        args_summary: String,
        respond: tokio::sync::oneshot::Sender<bool>,
    },
    /// Agent finished
    Complete(std::result::Result<AgentResponse, PawanError>),
}

/// Commands sent from the TUI to the agent task
enum AgentCommand {
    Execute(String),
    SwitchModel(String),
    Quit,
}

/// A single content block within a message, preserving event ordering.
#[derive(Clone, Debug)]
enum ContentBlock {
    /// Text emitted by the model. May be built incrementally during streaming.
    Text { content: String, streaming: bool },
    /// A tool call with optional result. Transitions: Running -> Done.
    ToolCall { name: String, args_summary: String, state: Box<ToolBlockState> },
}

/// State of a tool call block.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
enum ToolBlockState {
    Running,
    Done { record: ToolCallRecord, expanded: bool },
}

/// Streaming state for the assistant message currently being assembled.
struct StreamingAssistantState {
    blocks: Vec<ContentBlock>,
}

#[derive(Clone)]
/// A message for display in the TUI
pub struct DisplayMessage {
    pub role: Role,
    blocks: Vec<ContentBlock>,
    pub timestamp: std::time::Instant,
    /// Cached rendered lines for content blocks (excludes header). None = needs rebuild.
    cached_block_lines: Option<Vec<Line<'static>>>,
}

impl DisplayMessage {
    fn new_text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            blocks: vec![ContentBlock::Text { content: content.into(), streaming: false }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        }
    }

    fn from_agent_response(resp: &AgentResponse) -> Self {
        let mut blocks = Vec::new();
        if !resp.content.is_empty() {
            blocks.push(ContentBlock::Text { content: resp.content.clone(), streaming: false });
        }
        for tc in &resp.tool_calls {
            blocks.push(ContentBlock::ToolCall {
                name: tc.name.clone(),
                args_summary: summarize_args(&tc.arguments),
                state: Box::new(ToolBlockState::Done { record: tc.clone(), expanded: !tc.success }),
            });
        }
        Self { role: Role::Assistant, blocks, timestamp: std::time::Instant::now(), cached_block_lines: None }
    }

    /// Invalidate the render cache (call when blocks change).
    fn invalidate_cache(&mut self) {
        self.cached_block_lines = None;
    }

    /// Get or build cached block lines. Returns cached lines if available.
    fn block_lines_cached(&mut self) -> &[Line<'static>] {
        if self.cached_block_lines.is_none() {
            let mut lines = Vec::new();
            let is_assistant = self.role == Role::Assistant;
            for block in &self.blocks {
                App::render_block_to_lines(block, is_assistant, &mut lines);
            }
            self.cached_block_lines = Some(lines);
        }
        self.cached_block_lines.as_ref().unwrap()
    }

    /// Flat text content for search and export.
    fn text_content(&self) -> String {
        self.blocks.iter().filter_map(|b| match b {
            ContentBlock::Text { content, .. } => Some(content.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join("\n")
    }

    /// All completed tool call records.
    fn tool_records(&self) -> Vec<&ToolCallRecord> {
        self.blocks.iter().filter_map(|b| match b {
            ContentBlock::ToolCall { state, .. } => if let ToolBlockState::Done { record, .. } = state.as_ref() { Some(record) } else { None },
            _ => None,
        }).collect()
    }
}

/// Summarize JSON arguments to a compact display string.
fn summarize_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => {
            map.iter()
                .filter(|(_, v)| !matches!(v, serde_json::Value::String(s) if s.len() > 100))
                .take(3)
                .map(|(k, v)| {
                    let val = match v {
                        serde_json::Value::String(s) if s.len() > 40 => {
                            format!("\"{}...\"", &s[..37])
                        }
                        v => v.to_string(),
                    };
                    format!("{}={}", k, val)
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
        _ => String::new(),
    }
}

/// One-line preview of a tool result for collapsed view.
fn one_line_preview(result: &serde_json::Value, max_len: usize) -> String {
    let s = match result {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            if let Some(content) = map.get("content").and_then(|v| v.as_str()) {
                content.to_string()
            } else {
                serde_json::to_string(result).unwrap_or_default()
            }
        }
        v => v.to_string(),
    };
    let first_line = s.lines().next().unwrap_or("");
    if first_line.len() > max_len {
        format!("{}...", &first_line[..max_len.saturating_sub(3)])
    } else {
        first_line.to_string()
    }
}

/// Format tool result for expanded display.
fn format_tool_result(result: &serde_json::Value) -> String {
    match result {
        serde_json::Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Which panel is focused in the TUI
///
/// Represents the currently active input focus in the terminal UI.
pub enum Panel {
    Input,
    Messages,
}
/// Application state
struct App<'a> {
    config: TuiConfig,
    model_name: String,
    messages: Vec<DisplayMessage>,
    input: TextArea<'a>,
    scroll: usize,
    processing: bool,
    should_quit: bool,
    status: String,
    focus: Panel,
    /// Cumulative token usage across all requests
    total_tokens: u64,
    total_prompt_tokens: u64,
    total_completion_tokens: u64,
    /// Cumulative thinking vs action token split
    total_reasoning_tokens: u64,
    total_action_tokens: u64,
    /// Streaming assistant state: builds interleaved content blocks as events arrive
    streaming: Option<StreamingAssistantState>,
    /// Iteration count (increments on each tool completion)
    iteration_count: u32,
    /// Context tokens estimate
    context_estimate: usize,
    /// Search mode state
    search_mode: bool,
    search_query: String,
    /// Command palette state (Ctrl+P)
    palette_open: bool,
    palette_query: String,
    palette_selected: usize,
    /// Keyboard shortcuts overlay (F1)
    help_overlay: bool,
    /// Session stats
    session_tool_calls: u32,
    session_files_edited: u32,
    /// Inline slash command popup (triggered by typing /)
    slash_popup_selected: usize,
    /// Welcome screen shown on first launch
    show_welcome: bool,
    /// Permission dialog state — when Some, the agent is waiting for y/n
    permission_dialog: Option<PermissionDialog>,
    /// Channel to send commands to the agent task
    cmd_tx: mpsc::UnboundedSender<AgentCommand>,
    /// Channel to receive events from the agent task
    event_rx: mpsc::UnboundedReceiver<AgentEvent>,
}

/// State for an active permission prompt dialog
struct PermissionDialog {
    tool_name: String,
    args_summary: String,
    respond: Option<tokio::sync::oneshot::Sender<bool>>,
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
        input.set_placeholder_text("Type your message... (Enter to send, Ctrl+C to quit)");

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
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            help_overlay: false,
            session_tool_calls: 0,
            session_files_edited: 0,
            slash_popup_selected: 0,
            show_welcome: true,
            permission_dialog: None,
            cmd_tx,
            event_rx,
        }
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

    async fn main_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|f| self.ui(f)).map_err(PawanError::Io)?;

            // Non-blocking: check for agent events first
            while let Ok(event) = self.event_rx.try_recv() {
                match event {
                    AgentEvent::Token(token) => {
                        let state = self.streaming.get_or_insert_with(|| StreamingAssistantState {
                            blocks: Vec::new(),
                        });
                        // Append to last streaming text block, or start a new one
                        match state.blocks.last_mut() {
                            Some(ContentBlock::Text { content, streaming: true }) => {
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
                        let state = self.streaming.get_or_insert_with(|| StreamingAssistantState {
                            blocks: Vec::new(),
                        });
                        // Freeze current text block
                        if let Some(ContentBlock::Text { streaming, .. }) = state.blocks.last_mut() {
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
                                if let ContentBlock::ToolCall { name, args_summary, state: tool_state } = block {
                                    if matches!(tool_state.as_ref(), ToolBlockState::Running) && *name == record.name {
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
                        self.status = format!("{} {} ({}ms)", icon, record.name, record.duration_ms);
                    }
                    AgentEvent::PermissionRequest { tool_name, args_summary, respond } => {
                        self.permission_dialog = Some(PermissionDialog {
                            tool_name: tool_name.clone(),
                            args_summary: args_summary.clone(),
                            respond: Some(respond),
                        });
                        self.status = format!("Permission required: {} — y/n", tool_name);
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
                                    DisplayMessage { role: Role::Assistant, blocks, timestamp: std::time::Instant::now(), cached_block_lines: None }
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
                                self.context_estimate = (self.total_prompt_tokens + self.total_completion_tokens) as usize;
                                self.status = format!("Done ({} iterations)", resp.iterations);
                                self.scroll = self.messages.len().saturating_sub(1);
                            }
                            Err(e) => {
                                self.streaming = None;
                                self.status = format!("Error: {}", e);
                                self.messages.push(DisplayMessage::new_text(Role::Assistant, format!("Error: {}", e)));
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

            if self.should_quit {
                let _ = self.cmd_tx.send(AgentCommand::Quit);
                break;
            }
        }

        Ok(())
    }

    fn handle_event(&mut self, event: Event) {
        // Permission dialog intercepts y/n/Esc before anything else
        if self.permission_dialog.is_some() {
            if let Event::Key(key) = &event {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        if let Some(mut dialog) = self.permission_dialog.take() {
                            if let Some(tx) = dialog.respond.take() {
                                let _ = tx.send(true);
                            }
                            self.status = format!("Allowed: {}", dialog.tool_name);
                        }
                        return;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        if let Some(mut dialog) = self.permission_dialog.take() {
                            if let Some(tx) = dialog.respond.take() {
                                let _ = tx.send(false);
                            }
                            self.status = format!("Denied: {}", dialog.tool_name);
                        }
                        return;
                    }
                    _ => return, // Ignore other keys while dialog is open
                }
            }
            return;
        }

        match event {
            Event::Key(key) => {
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                        self.should_quit = true;
                        return;
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                        self.messages.clear();
                        self.status = "Cleared".to_string();
                        return;
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                        self.palette_open = !self.palette_open;
                        self.palette_query.clear();
                        self.palette_selected = 0;
                        return;
                    }
                    (_, KeyCode::F(1)) => {
                        self.help_overlay = !self.help_overlay;
                        return;
                    }
                    _ => {}
                }

                // Dismiss welcome on any key
                if self.show_welcome {
                    self.show_welcome = false;
                    return;
                }

                // Help overlay — dismiss with any key
                if self.help_overlay {
                    self.help_overlay = false;
                    return;
                }

                // Command palette intercepts all keys when open
                if self.palette_open {
                    match key.code {
                        KeyCode::Esc => {
                            self.palette_open = false;
                        }
                        KeyCode::Backspace => {
                            self.palette_query.pop();
                            self.palette_selected = 0;
                        }
                        KeyCode::Char(c) => {
                            self.palette_query.push(c);
                            self.palette_selected = 0;
                        }
                        KeyCode::Up => {
                            self.palette_selected = self.palette_selected.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            self.palette_selected += 1;
                        }
                        KeyCode::Enter => {
                            let items = self.palette_items();
                            if let Some(item) = items.get(self.palette_selected) {
                                let cmd = item.0.to_string();
                                self.palette_open = false;
                                self.handle_slash_command(&cmd);
                            }
                        }
                        _ => {}
                    }
                    return;
                }

                // Search mode intercepts all keys
                if self.search_mode {
                    match key.code {
                        KeyCode::Enter | KeyCode::Esc => {
                            self.search_mode = false;
                            if key.code == KeyCode::Esc {
                                self.search_query.clear();
                            }
                        }
                        KeyCode::Backspace => {
                            self.search_query.pop();
                        }
                        KeyCode::Char(c) => {
                            self.search_query.push(c);
                        }
                        _ => {}
                    }
                    return;
                }

                match self.focus {
                    Panel::Input => {
                        let slash_active = self.is_slash_popup_active();
                        if slash_active {
                            match key.code {
                                KeyCode::Esc => {
                                    // Close popup, clear input
                                    self.input = TextArea::default();
                                    self.input.set_cursor_line_style(Style::default());
                                    self.input.set_placeholder_text("Type your message... (Enter to send, Ctrl+C to quit)");
                                    self.slash_popup_selected = 0;
                                }
                                KeyCode::Up => {
                                    self.slash_popup_selected = self.slash_popup_selected.saturating_sub(1);
                                }
                                KeyCode::Down => {
                                    let items = self.slash_items();
                                    if !items.is_empty() {
                                        self.slash_popup_selected = (self.slash_popup_selected + 1).min(items.len() - 1);
                                    }
                                }
                                KeyCode::Tab => {
                                    let items = self.slash_items();
                                    if !items.is_empty() {
                                        self.slash_popup_selected = (self.slash_popup_selected + 1) % items.len();
                                    }
                                }
                                KeyCode::Enter => {
                                    let items = self.slash_items();
                                    if let Some((cmd, _)) = items.get(self.slash_popup_selected) {
                                        let cmd = cmd.to_string();
                                        // Replace input with selected command
                                        self.input = TextArea::default();
                                        self.input.set_cursor_line_style(Style::default());
                                        self.input.set_placeholder_text("Type your message... (Enter to send, Ctrl+C to quit)");
                                        self.input.insert_str(&cmd);
                                        self.slash_popup_selected = 0;
                                        // If it's a simple command (no args needed), submit immediately
                                        let simple = ["/help", "/tools", "/heal", "/clear", "/quit", "/?"];
                                        if simple.contains(&cmd.as_str()) {
                                            self.submit_input();
                                        }
                                    }
                                }
                                _ => {
                                    // Pass through to input, then reset selection
                                    self.input.input(Input::from(key));
                                    self.slash_popup_selected = 0;
                                }
                            }
                        } else if key.code == KeyCode::Enter {
                            self.submit_input();
                        } else if key.code == KeyCode::Tab {
                            self.focus = Panel::Messages;
                        } else {
                            self.input.input(Input::from(key));
                        }
                    }
                    Panel::Messages => match key.code {
                        KeyCode::Tab | KeyCode::Char('i') => self.focus = Panel::Input,
                        KeyCode::Char('e') => {
                            self.toggle_nearest_tool_expansion();
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.scroll = self.scroll.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            self.scroll = self.scroll.saturating_add(1);
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.scroll = self.scroll.saturating_sub(20);
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.scroll = self.scroll.saturating_add(20);
                        }
                        KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
                        KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
                        KeyCode::Char('g') | KeyCode::Home => self.scroll = 0,
                        KeyCode::Char('G') | KeyCode::End => {
                            self.scroll = self.messages.len().saturating_sub(1);
                        }
                        KeyCode::Char('/') => {
                            self.search_mode = true;
                            self.search_query.clear();
                        }
                        KeyCode::Char('n') => {
                            // Jump to next search match
                            if !self.search_query.is_empty() {
                                let query = self.search_query.to_lowercase();
                                for (i, msg) in self.messages.iter().enumerate() {
                                    if i > self.scroll
                                        && msg.text_content().to_lowercase().contains(&query)
                                    {
                                        self.scroll = i;
                                        break;
                                    }
                                }
                            }
                        }
                        KeyCode::Char('N') => {
                            // Jump to previous search match
                            if !self.search_query.is_empty() {
                                let query = self.search_query.to_lowercase();
                                for i in (0..self.scroll).rev() {
                                    if self.messages[i].text_content().to_lowercase().contains(&query) {
                                        self.scroll = i;
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
            Event::Mouse(mouse) => {
                if self.config.mouse_support {
                    match mouse.kind {
                        event::MouseEventKind::ScrollUp => {
                            self.scroll = self.scroll.saturating_sub(self.config.scroll_speed);
                        }
                        event::MouseEventKind::ScrollDown => {
                            self.scroll = self.scroll.saturating_add(self.config.scroll_speed);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Submit input — handles slash commands or sends to agent
    fn submit_input(&mut self) {
        let content: String = self.input.lines().join("\n");
        if content.trim().is_empty() {
            return;
        }

        // Reset input
        self.input = TextArea::default();
        self.input.set_cursor_line_style(Style::default());
        self.input
            .set_placeholder_text("Type your message... (Enter to send, Ctrl+C to quit)");

        let trimmed = content.trim();

        // Handle slash commands
        if trimmed.starts_with('/') {
            self.handle_slash_command(trimmed);
            return;
        }

        // Normal message — send to agent
        self.messages.push(DisplayMessage::new_text(Role::User, content.clone()));

        self.processing = true;
        self.status = "Processing...".to_string();

        let _ = self.cmd_tx.send(AgentCommand::Execute(content));
    }

    /// Handle slash commands locally without sending to the agent
    fn handle_slash_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        let command = parts[0];
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        match command {
            "/clear" | "/c" => {
                self.messages.clear();
                self.status = "Cleared".to_string();
            }
            "/model" | "/m" => {
                if arg.is_empty() {
                    self.messages.push(DisplayMessage::new_text(Role::System, format!("Current model: {}", self.model_name)));
                } else {
                    self.model_name = arg.to_string();
                    self.status = format!("Model → {}", arg);
                    self.messages.push(DisplayMessage::new_text(Role::System, format!("Switching model to: {}", arg)));
                    let _ = self.cmd_tx.send(AgentCommand::SwitchModel(arg.to_string()));
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
                    self.messages.push(DisplayMessage::new_text(Role::System, "Usage: /search <query>"));
                } else {
                    let search_prompt = format!(
                        "Use mcp_daedra_web_search to search for '{}' and report the results", arg
                    );
                    self.messages.push(DisplayMessage::new_text(Role::User, format!("/search {}", arg)));
                    self.processing = true;
                    self.status = format!("Searching: {}", arg);
                    let _ = self.cmd_tx.send(AgentCommand::Execute(search_prompt));
                }
            }
            "/heal" | "/h" => {
                self.messages.push(DisplayMessage::new_text(Role::User, "/heal"));
                self.processing = true;
                self.status = "Healing...".to_string();
                let _ = self.cmd_tx.send(AgentCommand::Execute(
                    "Run cargo check and cargo test. Fix any errors you find.".to_string()
                ));
            }
            "/quit" | "/q" => {
                self.should_quit = true;
            }
            "/export" | "/e" => {
                let path = if arg.is_empty() { "pawan-session.md" } else { arg };
                match self.export_conversation(path) {
                    Ok(n) => self.messages.push(DisplayMessage::new_text(Role::System, format!("Exported {} messages to {}", n, path))),
                    Err(e) => self.messages.push(DisplayMessage::new_text(Role::System, format!("Export failed: {}", e))),
                }
            }
            "/help" | "/?" => {
                self.messages.push(DisplayMessage::new_text(Role::System,
                    "/model [name]  — show or switch LLM model\n\
                     /search <query> — web search via Daedra\n\
                     /tools         — list available tools\n\
                     /heal          — auto-fix build errors\n\
                     /export [path] — export conversation to markdown\n\
                     /clear         — clear chat history\n\
                     /quit          — exit pawan\n\
                     /help          — show this help"));
            }
            _ => {
                self.messages.push(DisplayMessage::new_text(Role::System, format!("Unknown command: {}. Type /help for available commands.", command)));
            }
        }
    }

    /// Export conversation to a markdown file
    fn export_conversation(&self, path: &str) -> std::result::Result<usize, String> {
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
                writeln!(f, "<details><summary>Tool calls ({})</summary>\n", tool_records.len()).map_err(|e| e.to_string())?;
                for tc in tool_records {
                    let status = if tc.success { "ok" } else { "err" };
                    writeln!(f, "- `{}` ({}) — {}ms", tc.name, status, tc.duration_ms).map_err(|e| e.to_string())?;
                }
                writeln!(f, "\n</details>\n").map_err(|e| e.to_string())?;
            }
        }
        writeln!(f, "---\n*Tokens: {} total ({} prompt, {} completion)*",
            self.total_tokens, self.total_prompt_tokens, self.total_completion_tokens).map_err(|e| e.to_string())?;
        Ok(self.messages.len())
    }

    fn ui(&self, f: &mut Frame) {
        // Dynamic input height: 3 lines default, grows with content up to 10
        let input_lines = self.input.lines().len();
        let input_height = (input_lines + 2).clamp(3, 10) as u16; // +2 for border

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Min(3),         // messages: takes all remaining space
                Constraint::Length(input_height), // input: auto-resizes
                Constraint::Length(1),       // status bar
            ])
            .split(f.area());

        // Split layout: messages + activity panel when processing
        if self.processing {
            let horiz = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
                .split(chunks[0]);
            self.render_messages(f, horiz[0]);
            self.render_activity(f, horiz[1]);
        } else {
            self.render_messages(f, chunks[0]);
        }
        self.render_input(f, chunks[1]);
        self.render_status(f, chunks[2]);

        // Inline slash popup (above input, below other overlays)
        if self.is_slash_popup_active() && !self.show_welcome && !self.help_overlay && !self.palette_open {
            self.render_slash_popup(f, chunks[1]);
        }

        // Overlays (on top of everything)
        if self.permission_dialog.is_some() {
            self.render_permission_dialog(f);
        } else if self.show_welcome {
            self.render_welcome(f);
        } else if self.help_overlay {
            self.render_help_overlay(f);
        } else if self.palette_open {
            self.render_palette(f);
        }
    }

    fn render_permission_dialog(&self, f: &mut Frame) {
        let dialog = match &self.permission_dialog {
            Some(d) => d,
            None => return,
        };

        let area = f.area();
        let width = 60u16.min(area.width.saturating_sub(4));
        let height = 7u16;
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        // Clear background
        f.render_widget(ratatui::widgets::Clear, popup_area);

        let text = vec![
            Line::from(vec![
                Span::styled("Tool: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(&dialog.tool_name),
            ]),
            Line::from(vec![
                Span::styled("Args: ", Style::default().fg(Color::DarkGray)),
                Span::raw(if dialog.args_summary.len() > 45 {
                    format!("{}...", &dialog.args_summary[..42])
                } else {
                    dialog.args_summary.clone()
                }),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(" y ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw(" Allow  "),
                Span::styled(" n ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw(" Deny"),
            ]),
        ];

        let block = Block::default()
            .title(" Permission Required ")
            .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let paragraph = Paragraph::new(text).block(block);
        f.render_widget(paragraph, popup_area);
    }

    /// Get filtered palette items based on query
    fn palette_items(&self) -> Vec<(&str, &str)> {
        let all_items: Vec<(&str, &str)> = vec![
            ("/help", "Show available commands"),
            ("/model", "Show or switch LLM model"),
            ("/model qwen/qwen3.5-122b-a10b", "Qwen 3.5 122B (S tier, fast)"),
            ("/model minimaxai/minimax-m2.5", "MiniMax M2.5 (SWE 80.2%)"),
            ("/model stepfun-ai/step-3.5-flash", "Step 3.5 Flash (S+ tier)"),
            ("/model mistralai/mistral-small-4-119b-2603", "Mistral Small 4 119B"),
            ("/search", "Web search via Daedra"),
            ("/tools", "List available tools"),
            ("/heal", "Auto-fix build errors"),
            ("/export", "Export conversation to markdown"),
            ("/clear", "Clear chat history"),
            ("/quit", "Exit pawan"),
        ];
        if self.palette_query.is_empty() {
            return all_items;
        }
        let q = self.palette_query.to_lowercase();
        all_items.into_iter()
            .filter(|(cmd, desc)| cmd.to_lowercase().contains(&q) || desc.to_lowercase().contains(&q))
            .collect()
    }

    /// Check if the inline slash popup should be shown.
    fn is_slash_popup_active(&self) -> bool {
        let text: String = self.input.lines().join("\n");
        let trimmed = text.trim();
        trimmed.starts_with('/') && !trimmed.contains(' ')
    }

    /// Get filtered slash command items based on current input.
    fn slash_items(&self) -> Vec<(&str, &str)> {
        let all_items: Vec<(&str, &str)> = vec![
            ("/help", "Show available commands"),
            ("/model", "Show or switch LLM model"),
            ("/search", "Web search via Daedra"),
            ("/tools", "List available tools"),
            ("/heal", "Auto-fix build errors"),
            ("/export", "Export conversation to markdown"),
            ("/clear", "Clear chat history"),
            ("/quit", "Exit pawan"),
        ];
        let text: String = self.input.lines().join("\n");
        let q = text.trim().to_lowercase();
        if q == "/" {
            return all_items;
        }
        all_items.into_iter()
            .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&q))
            .collect()
    }

    /// Render inline slash command popup above the input area.
    fn render_slash_popup(&self, f: &mut Frame, input_area: Rect) {
        let items = self.slash_items();
        if items.is_empty() {
            return;
        }

        let h = (items.len() as u16 + 2).min(10); // +2 for borders
        let w = 45u16.min(input_area.width);
        let y = input_area.y.saturating_sub(h);
        let popup_area = Rect::new(input_area.x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" / Commands ");

        f.render_widget(ratatui::widgets::Clear, popup_area);

        let selected = self.slash_popup_selected.min(items.len().saturating_sub(1));
        let list_items: Vec<ListItem> = items.iter().enumerate().map(|(i, (cmd, desc))| {
            let style = if i == selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", cmd), style.add_modifier(Modifier::BOLD)),
                Span::styled(format!("— {}", desc), if i == selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
            ]))
        }).collect();
        f.render_widget(List::new(list_items).block(block), popup_area);
    }

    /// Render command palette overlay
    fn render_welcome(&self, f: &mut Frame) {
        let area = f.area();
        let w = 52u16.min(area.width.saturating_sub(4));
        let h = 12u16.min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(w)) / 2;
        let y = (area.height.saturating_sub(h)) / 2;
        let welcome_area = Rect::new(x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" पवन — pawan ");

        f.render_widget(ratatui::widgets::Clear, welcome_area);
        f.render_widget(block.clone(), welcome_area);

        let inner = block.inner(welcome_area);
        let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
        let text = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Self-healing CLI coding agent", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(format!("  v{}", env!("CARGO_PKG_VERSION")), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Model: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&self.model_name, Style::default().fg(Color::Cyan)),
            ]),
            Line::from(vec![
                Span::styled("  Path:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(cwd, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(Span::styled("  Type a task, or explore:", Style::default().fg(Color::DarkGray))),
            Line::from(vec![
                Span::styled("  Ctrl+P", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("  command palette", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled("  F1    ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("  keyboard shortcuts", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(Span::styled("  Press any key to start...", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC))),
        ];
        f.render_widget(Paragraph::new(text), inner);
    }

    fn render_help_overlay(&self, f: &mut Frame) {
        let area = f.area();
        let w = 48u16.min(area.width.saturating_sub(4));
        let h = 16u16.min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(w)) / 2;
        let y = (area.height.saturating_sub(h)) / 2;
        let help_area = Rect::new(x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Keyboard Shortcuts (F1) ");

        f.render_widget(ratatui::widgets::Clear, help_area);
        f.render_widget(block.clone(), help_area);

        let inner = block.inner(help_area);
        let shortcuts = vec![
            Line::from(""),
            Line::from(Span::styled("  Navigation", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(vec![Span::styled("  Tab     ", Style::default().fg(Color::Yellow)), Span::raw("Switch focus (input/messages)")]),
            Line::from(vec![Span::styled("  j/k     ", Style::default().fg(Color::Yellow)), Span::raw("Scroll up/down")]),
            Line::from(vec![Span::styled("  g/G     ", Style::default().fg(Color::Yellow)), Span::raw("Jump to top/bottom")]),
            Line::from(vec![Span::styled("  /       ", Style::default().fg(Color::Yellow)), Span::raw("Search in messages")]),
            Line::from(""),
            Line::from(Span::styled("  Commands", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(vec![Span::styled("  Ctrl+P  ", Style::default().fg(Color::Yellow)), Span::raw("Command palette")]),
            Line::from(vec![Span::styled("  Ctrl+L  ", Style::default().fg(Color::Yellow)), Span::raw("Clear chat")]),
            Line::from(vec![Span::styled("  Ctrl+C  ", Style::default().fg(Color::Yellow)), Span::raw("Quit")]),
            Line::from(""),
            Line::from(Span::styled("  Slash Commands", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(vec![Span::styled("  /model  ", Style::default().fg(Color::Yellow)), Span::raw("Switch model at runtime")]),
            Line::from(vec![Span::styled("  /search ", Style::default().fg(Color::Yellow)), Span::raw("Web search via Daedra")]),
            Line::from(vec![Span::styled("  /tools  ", Style::default().fg(Color::Yellow)), Span::raw("List all tools")]),
        ];
        f.render_widget(Paragraph::new(shortcuts), inner);
    }

    fn render_palette(&self, f: &mut Frame) {
        let area = f.area();
        // Center the palette: 50% width, up to 14 lines tall
        let w = (area.width * 50 / 100).max(30);
        let items = self.palette_items();
        let h = (items.len() as u16 + 4).min(14);
        let x = (area.width.saturating_sub(w)) / 2;
        let y = area.height / 4;
        let palette_area = Rect::new(x, y, w, h);

        // Background
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Command Palette (Ctrl+P) ");

        let inner = block.inner(palette_area);
        f.render_widget(ratatui::widgets::Clear, palette_area);
        f.render_widget(block, palette_area);

        // Search input
        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::styled(&self.palette_query, Style::default().fg(Color::White)),
            Span::styled("▌", Style::default().fg(Color::Cyan).add_modifier(Modifier::SLOW_BLINK)),
        ]);
        if inner.height > 0 {
            f.render_widget(Paragraph::new(search_line), Rect::new(inner.x, inner.y, inner.width, 1));
        }

        // Items
        let list_area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1));
        let selected = self.palette_selected.min(items.len().saturating_sub(1));
        let list_items: Vec<ListItem> = items.iter().enumerate().map(|(i, (cmd, desc))| {
            let style = if i == selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", cmd), style.add_modifier(Modifier::BOLD)),
                Span::styled(format!("— {}", desc), if i == selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
            ]))
        }).collect();
        f.render_widget(List::new(list_items), list_area);
    }

    fn render_activity(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta))
            .title(" Activity ");
        let mut items: Vec<ListItem> = Vec::new();
        for msg in self.messages.iter().rev().take(5) {
            for tc in msg.tool_records() {
                let icon = if tc.success { "✓" } else { "✗" };
                let color = if tc.success { Color::Green } else { Color::Red };
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                    Span::styled(tc.name.clone(), Style::default().fg(Color::White)),
                    Span::styled(format!(" {}ms", tc.duration_ms), Style::default().fg(Color::DarkGray)),
                ])));
            }
        }
        // Show running tools from streaming state
        if let Some(ref state) = self.streaming {
            for block in &state.blocks {
                if let ContentBlock::ToolCall { name, state, .. } = block {
                    if matches!(state.as_ref(), ToolBlockState::Running) {
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled(" ⚙ ", Style::default().fg(Color::Yellow)),
                            Span::styled(name.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                        ])));
                    }
                }
            }
        }
        if items.is_empty() {
            items.push(ListItem::new(Span::styled(" Waiting...", Style::default().fg(Color::DarkGray))));
        }
        f.render_widget(List::new(items).block(block), area);
    }

    fn render_messages(&self, f: &mut Frame, area: Rect) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let now = std::time::Instant::now();

        for msg in &self.messages {
            self.render_message_to_lines(msg, now, &mut lines);
            lines.push(Line::from(""));
        }

        // Streaming state: render the in-progress assistant message
        if self.processing {
            if let Some(ref state) = self.streaming {
                if !state.blocks.is_empty() {
                    lines.push(Line::from(vec![Span::styled(
                        "Pawan: ",
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                    )]));
                    for block in &state.blocks {
                        Self::render_block_to_lines(block, true, &mut lines);
                    }
                } else {
                    lines.push(Line::from(vec![Span::styled(
                        "  Pawan is thinking...",
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::ITALIC),
                    )]));
                }
            } else {
                lines.push(Line::from(vec![Span::styled(
                    "  Pawan is thinking...",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::ITALIC),
                )]));
            }
        }

        let border_style = if self.focus == Panel::Messages {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let total_lines = lines.len();
        let visible_height = area.height.saturating_sub(2) as usize; // minus borders
        let max_offset = total_lines.saturating_sub(visible_height);
        let scroll_offset = if self.scroll == usize::MAX {
            max_offset // auto-scroll to bottom
        } else {
            self.scroll.min(max_offset)
        };

        let scroll_indicator = if total_lines > visible_height {
            let pct = if max_offset > 0 { scroll_offset * 100 / max_offset } else { 100 };
            format!(" [{}%]", pct)
        } else {
            String::new()
        };

        let title = if self.search_mode {
            format!(" Search: {}▌ ", self.search_query)
        } else if !self.search_query.is_empty() {
            format!(" Messages{} [/{}] (n/N next/prev) ", scroll_indicator, self.search_query)
        } else {
            format!(" Messages{} (Tab, j/k, /, g/G, e) ", scroll_indicator)
        };

        let messages_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let paragraph = Paragraph::new(lines)
            .block(messages_block)
            .scroll((scroll_offset as u16, 0));
        f.render_widget(paragraph, area);
    }

    /// Render a single DisplayMessage into Lines.
    /// Uses cached block lines when available (populated by `block_lines_cached()`).
    fn render_message_to_lines(&self, msg: &DisplayMessage, now: std::time::Instant, lines: &mut Vec<Line<'static>>) {
        let (prefix, style) = match msg.role {
            Role::User => ("You", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Role::Assistant => ("Pawan", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Role::System => ("System", Style::default().fg(Color::Yellow)),
            Role::Tool => ("Tool", Style::default().fg(Color::Magenta)),
        };

        let elapsed = now.duration_since(msg.timestamp);
        let time_str = if elapsed.as_secs() < 5 {
            "now".to_string()
        } else if elapsed.as_secs() < 60 {
            format!("{}s", elapsed.as_secs())
        } else if elapsed.as_secs() < 3600 {
            format!("{}m", elapsed.as_secs() / 60)
        } else {
            format!("{}h", elapsed.as_secs() / 3600)
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{}: ", prefix), style),
            Span::styled(format!("({})", time_str), Style::default().fg(Color::DarkGray)),
        ]));

        // Use cached block lines if available; otherwise render fresh
        if let Some(ref cached) = msg.cached_block_lines {
            lines.extend(cached.iter().cloned());
        } else {
            let is_assistant = msg.role == Role::Assistant;
            for block in &msg.blocks {
                Self::render_block_to_lines(block, is_assistant, lines);
            }
        }
    }

    /// Render a single ContentBlock into Lines.
    fn render_block_to_lines(block: &ContentBlock, use_markdown: bool, lines: &mut Vec<Line<'static>>) {
        match block {
            ContentBlock::Text { content, streaming } => {
                if use_markdown {
                    for line in markdown_to_lines(content) {
                        let mut spans: Vec<Span<'static>> = vec![Span::raw("  ".to_string())];
                        spans.extend(line.spans);
                        lines.push(Line::from(spans));
                    }
                } else {
                    for line_str in content.lines() {
                        lines.push(Line::from(Span::raw(format!("  {}", line_str))));
                    }
                }
                if *streaming {
                    lines.push(Line::from(vec![Span::styled(
                        "  ▌",
                        Style::default().fg(Color::Green).add_modifier(Modifier::SLOW_BLINK),
                    )]));
                }
            }
            ContentBlock::ToolCall { name, args_summary, state } => {
                match state.as_ref() {
                    ToolBlockState::Running => {
                        lines.push(Line::from(vec![
                            Span::styled("  ⚙ ", Style::default().fg(Color::Yellow)),
                            Span::styled(
                                format!("Running {}...", name),
                                Style::default().fg(Color::Yellow).add_modifier(Modifier::ITALIC),
                            ),
                        ]));
                    }
                    ToolBlockState::Done { record, expanded } => {
                        let icon = if record.success { "✓" } else { "✗" };
                        let color = if record.success { Color::Green } else { Color::Red };
                        let mut spans = vec![
                            Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                            Span::styled(
                                name.clone(),
                                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                            ),
                        ];
                        if !args_summary.is_empty() {
                            spans.push(Span::styled(
                                format!("({})", args_summary),
                                Style::default().fg(Color::DarkGray),
                            ));
                        }
                        spans.push(Span::styled(
                            format!(" {}ms", record.duration_ms),
                            Style::default().fg(Color::DarkGray),
                        ));
                        lines.push(Line::from(spans));

                        if *expanded {
                            let result_str = format_tool_result(&record.result);
                            for result_line in result_str.lines().take(20) {
                                lines.push(Line::from(Span::styled(
                                    format!("    {}", result_line),
                                    Style::default().fg(Color::DarkGray),
                                )));
                            }
                            let total = result_str.lines().count();
                            if total > 20 {
                                lines.push(Line::from(Span::styled(
                                    format!("    ... ({} more lines)", total - 20),
                                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                                )));
                            }
                        } else {
                            let preview = one_line_preview(&record.result, 60);
                            if !preview.is_empty() {
                                lines.push(Line::from(Span::styled(
                                    format!("    {}", preview),
                                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                                )));
                            }
                        }
                    }
                }
            }
        }
    }

    /// Toggle expand/collapse on the nearest tool block to the current scroll position.
    fn toggle_nearest_tool_expansion(&mut self) {
        let mut line_offset = 0usize;
        let mut best: Option<(usize, usize, usize)> = None; // (msg_idx, block_idx, distance)

        for (mi, msg) in self.messages.iter().enumerate() {
            line_offset += 1; // header line
            for (bi, block) in msg.blocks.iter().enumerate() {
                if let ContentBlock::ToolCall { state, .. } = block {
                    if matches!(state.as_ref(), ToolBlockState::Done { .. }) {
                        let dist = line_offset.abs_diff(self.scroll);
                        if best.is_none() || dist < best.unwrap().2 {
                            best = Some((mi, bi, dist));
                        }
                    }
                }
                // Estimate lines this block takes
                match block {
                    ContentBlock::Text { content, .. } => {
                        line_offset += content.lines().count().max(1);
                    }
                    ContentBlock::ToolCall { state, .. } => {
                        if let ToolBlockState::Done { expanded, record } = state.as_ref() {
                            line_offset += 1; // summary line
                            if *expanded {
                                line_offset += format_tool_result(&record.result).lines().count().min(21);
                            } else {
                                line_offset += 1; // preview line
                            }
                        } else {
                            line_offset += 1;
                        }
                    }
                }
            }
            line_offset += 1; // spacer
        }

        if let Some((mi, bi, _)) = best {
            if let ContentBlock::ToolCall { state, .. } = &mut self.messages[mi].blocks[bi] {
                if let ToolBlockState::Done { expanded, .. } = state.as_mut() {
                    *expanded = !*expanded;
                }
            }
            // Invalidate cache since expanded state changed
            self.messages[mi].invalidate_cache();
        }
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        let border_style = if self.focus == Panel::Input {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let title = if self.processing {
            " Input (processing...) "
        } else {
            " Input (Enter to send, /help for commands) "
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(&self.input, inner);
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let status_style = if self.processing {
            Style::default().fg(Color::Yellow)
        } else if self.status.starts_with("Error") {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let mut spans = vec![
            Span::styled("Model: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&self.model_name, Style::default().fg(Color::Cyan)),
            Span::raw(" | "),
            Span::styled(&self.status, status_style),
        ];

        if self.total_tokens > 0 {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("{}tok", self.total_tokens),
                Style::default().fg(Color::Yellow),
            ));
            // Show thinking/action split if reasoning tokens were tracked
            if self.total_reasoning_tokens > 0 {
                spans.push(Span::styled(
                    format!(" (think:{} act:{})", self.total_reasoning_tokens, self.total_action_tokens),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::styled(
                    format!(" ({}↑ {}↓)", self.total_prompt_tokens, self.total_completion_tokens),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        if self.iteration_count > 0 {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(format!("iter:{}", self.iteration_count), Style::default().fg(Color::Magenta)));
        }
        if self.context_estimate > 0 {
            let ctx_k = self.context_estimate / 1000;
            let ctx_style = if ctx_k > 80 { Style::default().fg(Color::Red) } else if ctx_k > 60 { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) };
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(format!("~{}k ctx", ctx_k), ctx_style));
        }

        // Session stats
        if self.session_tool_calls > 0 {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("{}⚡", self.session_tool_calls),
                Style::default().fg(Color::Magenta),
            ));
            if self.session_files_edited > 0 {
                spans.push(Span::styled(
                    format!(" {}📝", self.session_files_edited),
                    Style::default().fg(Color::Green),
                ));
            }
        }

        if !self.messages.is_empty() {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("{}msg", self.messages.len()),
                Style::default().fg(Color::DarkGray),
            ));
        }

        spans.extend([
            Span::raw(" | "),
            Span::styled("Ctrl+P".to_string(), Style::default().fg(Color::Cyan)),
            Span::styled(" palette".to_string(), Style::default().fg(Color::DarkGray)),
        ]);

        let status = Paragraph::new(Line::from(spans));

        f.render_widget(status, area);
    }
}

/// Spawn the agent task that listens for commands and sends back events
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
                let on_token: pawan::agent::TokenCallback =
                    Box::new(move |token: &str| {
                        let _ = token_tx.send(AgentEvent::Token(token.to_string()));
                    });

                // Create tool start callback
                let tool_start_tx = event_tx.clone();
                let on_tool_start: pawan::agent::ToolStartCallback =
                    Box::new(move |name: &str| {
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
                agent.switch_model(&model);
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

/// Parse markdown text into styled ratatui Lines
fn markdown_to_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines_out = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("```") {
            if !in_code_block {
                in_code_block = true;
                code_lang = rest.trim().to_string();
                let label = if code_lang.is_empty() {
                    "─── code ───".to_string()
                } else {
                    format!("─── {} ───", code_lang)
                };
                lines_out.push(Line::from(Span::styled(
                    label,
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                in_code_block = false;
                code_lang.clear();
                lines_out.push(Line::from(Span::styled(
                    "────────────".to_string(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            continue;
        }

        if in_code_block {
            // Code lines: monospace with distinct background color
            lines_out.push(Line::from(Span::styled(
                format!("  {}", line),
                Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 46)),
            )));
            continue;
        }

        if let Some(rest) = line.strip_prefix("### ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(rest) = line.strip_prefix("## ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
        } else if let Some(rest) = line.strip_prefix("# ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
        } else if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            let mut spans = vec![Span::raw("• ".to_string())];
            spans.extend(parse_inline_markdown(rest));
            lines_out.push(Line::from(spans));
        } else if line.len() > 2
            && line.as_bytes()[0].is_ascii_digit()
            && (line.contains(". ") || line.contains(") "))
        {
            // Numbered list: "1. text" or "1) text"
            if let Some(pos) = line.find(". ").or_else(|| line.find(") ")) {
                let num = &line[..=pos];
                let rest = &line[pos + 2..];
                let mut spans = vec![Span::styled(
                    num.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                )];
                spans.push(Span::raw(" ".to_string()));
                spans.extend(parse_inline_markdown(rest));
                lines_out.push(Line::from(spans));
            } else {
                lines_out.push(Line::from(parse_inline_markdown(line)));
            }
        } else if let Some(rest) = line.strip_prefix("> ") {
            lines_out.push(Line::from(Span::styled(
                format!("│ {}", rest),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else if line.chars().all(|c| c == '-' || c == '=') && line.len() >= 3 {
            // Horizontal rule
            lines_out.push(Line::from(Span::styled(
                "─".repeat(40),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines_out.push(Line::from(parse_inline_markdown(line)));
        }
    }

    lines_out
}

/// Parse inline markdown: **bold**, `code`, *italic*
fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Find the next markdown marker
        let next_bold = remaining.find("**");
        let next_code = remaining.find('`');
        let next_italic = remaining.find('*').filter(|&pos| {
            // Not a ** (bold) marker
            next_bold != Some(pos)
        });

        // Find earliest marker
        let earliest = [next_bold, next_code, next_italic]
            .into_iter()
            .flatten()
            .min();

        match earliest {
            None => {
                spans.push(Span::raw(remaining.to_string()));
                break;
            }
            Some(pos) => {
                if pos > 0 {
                    spans.push(Span::raw(remaining[..pos].to_string()));
                }

                if Some(pos) == next_bold {
                    let after = &remaining[pos + 2..];
                    if let Some(end) = after.find("**") {
                        spans.push(Span::styled(
                            after[..end].to_string(),
                            Style::default().add_modifier(Modifier::BOLD),
                        ));
                        remaining = &after[end + 2..];
                    } else {
                        spans.push(Span::raw("**".to_string()));
                        remaining = after;
                    }
                } else if Some(pos) == next_code {
                    let after = &remaining[pos + 1..];
                    if let Some(end) = after.find('`') {
                        spans.push(Span::styled(
                            after[..end].to_string(),
                            Style::default().fg(Color::Yellow),
                        ));
                        remaining = &after[end + 1..];
                    } else {
                        spans.push(Span::raw("`".to_string()));
                        remaining = after;
                    }
                } else {
                    // italic *...*
                    let after = &remaining[pos + 1..];
                    if let Some(end) = after.find('*') {
                        spans.push(Span::styled(
                            after[..end].to_string(),
                            Style::default().add_modifier(Modifier::ITALIC),
                        ));
                        remaining = &after[end + 1..];
                    } else {
                        spans.push(Span::raw("*".to_string()));
                        remaining = after;
                    }
                }
            }
        }
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }

    spans
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    /// Create a test App with dummy channels (for state/render tests, not event loops)
    fn test_app<'a>() -> App<'a> {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        // Disable welcome screen in tests so keypresses reach normal handlers
        app.show_welcome = false;
        app
    }

    // ===== Rendering tests using TestBackend =====

    #[test]
    fn test_render_empty_state() {
        let app = test_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let buf = terminal.backend().buffer().clone();
        // Should contain model name in status bar
        let content = buffer_to_string(&buf);
        assert!(content.contains("test-model"), "Status bar should show model name");
        assert!(content.contains("Ready"), "Status bar should show Ready");
        assert!(content.contains("Messages"), "Messages panel title should render");
        assert!(content.contains("Input"), "Input panel title should render");
    }

    #[test]
    fn test_render_with_messages() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(Role::User, "Hello pawan"));
        app.messages.push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("You:"), "Should render user prefix");
        assert!(content.contains("Pawan:"), "Should render assistant prefix");
        assert!(content.contains("Hello pawan"), "Should render user message");
        assert!(content.contains("Hi there!"), "Should render assistant message");
    }

    #[test]
    fn test_render_processing_thinking() {
        let mut app = test_app();
        app.processing = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("thinking"), "Should show thinking indicator");
    }

    #[test]
    fn test_render_streaming_content() {
        let mut app = test_app();
        app.processing = true;
        app.streaming = Some(StreamingAssistantState {
            blocks: vec![ContentBlock::Text {
                content: "partial response so far".to_string(),
                streaming: true,
            }],
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("partial response"), "Should render streaming content");
        assert!(content.contains("▌"), "Should show blinking cursor");
    }

    #[test]
    fn test_render_active_tool() {
        let mut app = test_app();
        app.processing = true;
        app.streaming = Some(StreamingAssistantState {
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Running),
            }],
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("bash"), "Should show active tool name");
    }

    #[test]
    fn test_render_token_stats() {
        let mut app = test_app();
        app.total_tokens = 1500;
        app.total_prompt_tokens = 1000;
        app.total_completion_tokens = 500;

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("1500tok"), "Should show total token count");
    }

    #[test]
    fn test_render_tool_call_results() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![
                ContentBlock::Text { content: "Done".into(), streaming: false },
                ContentBlock::ToolCall {
                    name: "write_file".into(),
                    args_summary: String::new(),
                    state: Box::new(ToolBlockState::Done {
                        record: ToolCallRecord {
                            id: "1".into(),
                            name: "write_file".into(),
                            arguments: serde_json::json!({}),
                            result: serde_json::json!({"success": true}),
                            success: true,
                            duration_ms: 42,
                        },
                        expanded: false,
                    }),
                },
                ContentBlock::ToolCall {
                    name: "bash".into(),
                    args_summary: String::new(),
                    state: Box::new(ToolBlockState::Done {
                        record: ToolCallRecord {
                            id: "2".into(),
                            name: "bash".into(),
                            arguments: serde_json::json!({}),
                            result: serde_json::json!({"error": "timeout"}),
                            success: false,
                            duration_ms: 30000,
                        },
                        expanded: true,
                    }),
                },
            ],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("write_file"), "Should show successful tool name");
        assert!(content.contains("bash"), "Should show failed tool name");
        assert!(content.contains("42ms") || content.contains("✓"), "Should show success indicator");
    }

    #[test]
    fn test_render_context_estimate() {
        let mut app = test_app();
        app.context_estimate = 85000; // 85k — should be red

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("85k ctx"), "Should show context estimate");
    }

    #[test]
    fn test_render_search_mode() {
        let mut app = test_app();
        app.search_mode = true;
        app.search_query = "hello".to_string();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("Search: hello"), "Should show search query in panel title");
    }

    #[test]
    fn test_render_focus_input() {
        let app = test_app();
        assert_eq!(app.focus, Panel::Input, "Default focus should be Input");
    }

    // ===== Event handling tests =====

    #[test]
    fn test_ctrl_c_quits() {
        let mut app = test_app();
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.should_quit, "Ctrl+C should set should_quit");
    }

    #[test]
    fn test_ctrl_l_clears() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(Role::User, "test"));
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.messages.is_empty(), "Ctrl+L should clear messages");
        assert_eq!(app.status, "Cleared");
    }

    #[test]
    fn test_tab_switches_focus() {
        let mut app = test_app();
        assert_eq!(app.focus, Panel::Input);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.focus, Panel::Messages, "Tab from Input goes to Messages");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.focus, Panel::Input, "Tab from Messages goes to Input");
    }

    #[test]
    fn test_scroll_keys_in_messages_panel() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 5;

        // j scrolls down
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 6);

        // k scrolls up
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 5);

        // g goes to top
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn test_scroll_saturates_at_zero() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 0;
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0, "Scroll should not go below 0");
    }

    #[test]
    fn test_search_mode_entry_and_exit() {
        let mut app = test_app();
        app.focus = Panel::Messages;

        // Enter search mode with /
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        )));
        assert!(app.search_mode, "/ should enter search mode");

        // Type search query
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.search_query, "hi");

        // Backspace deletes
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.search_query, "h");

        // Enter exits search mode, keeps query
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(!app.search_mode);
        assert_eq!(app.search_query, "h");
    }

    #[test]
    fn test_search_esc_clears_query() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_mode = true;
        app.search_query = "findme".to_string();

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.search_mode);
        assert!(app.search_query.is_empty(), "Esc should clear search query");
    }

    #[test]
    fn test_search_n_jumps_forward() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_query = "target".to_string();
        app.messages.push(DisplayMessage::new_text(Role::User, "no match"));
        app.messages.push(DisplayMessage::new_text(Role::User, "has target word"));
        app.messages.push(DisplayMessage::new_text(Role::User, "another target"));
        app.scroll = 0;

        // n should jump to first match after current scroll
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 1, "n should jump to first match at index 1");

        // n again should jump to next match
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 2, "n should jump to next match at index 2");
    }

    #[test]
    fn test_search_n_reverse() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_query = "target".to_string();
        app.messages.push(DisplayMessage::new_text(Role::User, "first target"));
        app.messages.push(DisplayMessage::new_text(Role::User, "no match"));
        app.messages.push(DisplayMessage::new_text(Role::User, "second target"));
        app.scroll = 2;

        // N should jump to previous match
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('N'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0, "N should jump to previous match at index 0");
    }

    #[test]
    fn test_mouse_scroll() {
        let mut app = test_app();
        app.scroll = 5;
        app.config.mouse_support = true;
        app.config.scroll_speed = 3;

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.scroll, 2, "Mouse scroll up should decrease by scroll_speed");

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.scroll, 5, "Mouse scroll down should increase by scroll_speed");
    }

    #[test]
    fn test_mouse_scroll_disabled() {
        let mut app = test_app();
        app.scroll = 5;
        app.config.mouse_support = false;

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.scroll, 5, "Mouse scroll should be ignored when disabled");
    }

    // ===== State transition tests =====

    #[test]
    fn test_submit_input_creates_message() {
        let mut app = test_app();
        app.input = TextArea::from(vec!["hello pawan"]);

        app.submit_input();

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].text_content(), "hello pawan");
        assert_eq!(app.messages[0].role, Role::User);
        assert!(app.processing, "Should be processing after submit");
        assert_eq!(app.status, "Processing...");
    }

    #[test]
    fn test_submit_empty_input_ignored() {
        let mut app = test_app();
        app.input = TextArea::from(vec!["   "]);

        app.submit_input();

        assert!(app.messages.is_empty(), "Empty input should not create message");
        assert!(!app.processing, "Should not be processing for empty input");
    }

    #[test]
    fn test_processing_input_title() {
        let mut app = test_app();
        app.processing = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("processing"), "Input panel should show processing state");
    }

    #[test]
    fn test_error_status_renders() {
        let mut app = test_app();
        app.status = "Error: connection refused".to_string();

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("Error: connection refused"), "Error status should render");
    }

    #[test]
    fn test_page_up_down_scroll() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 15;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageUp,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 5, "PageUp should scroll up by 10");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 15, "PageDown should scroll down by 10");
    }

    #[test]
    fn test_ctrl_u_d_half_page() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 25;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.scroll, 5, "Ctrl+U should scroll up by 20");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.scroll, 25, "Ctrl+D should scroll down by 20");
    }

    #[test]
    fn test_i_returns_to_input() {
        let mut app = test_app();
        app.focus = Panel::Messages;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.focus, Panel::Input, "'i' in Messages panel should return to Input");
    }

    // ===== Helper =====

    /// Convert a ratatui Buffer to a plain string for assertion matching
    fn buffer_to_string(buf: &Buffer) -> String {
        let area = buf.area;
        let mut result = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let cell = &buf[(x, y)];
                result.push_str(cell.symbol());
            }
            result.push('\n');
        }
        result
    }

    // ===== Slash command tests =====

    #[test]
    fn test_slash_help() {
        let mut app = test_app();
        app.handle_slash_command("/help");
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0].text_content().contains("/model"));
        assert!(app.messages[0].text_content().contains("/search"));
        assert!(app.messages[0].text_content().contains("/quit"));
    }

    #[test]
    fn test_slash_clear() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(Role::User, "test"));
        app.messages.push(DisplayMessage::new_text(Role::Assistant, "reply"));
        app.handle_slash_command("/clear");
        assert!(app.messages.is_empty());
        assert_eq!(app.status, "Cleared");
    }

    #[test]
    fn test_slash_model_show() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("test-model"));
    }

    #[test]
    fn test_slash_model_switch() {
        let mut app = test_app();
        app.handle_slash_command("/model mistral-small-4");
        assert_eq!(app.model_name, "mistral-small-4");
        assert!(app.messages[0].text_content().contains("mistral-small-4"));
    }

    #[test]
    fn test_slash_tools() {
        let mut app = test_app();
        app.handle_slash_command("/tools");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("bash"));
        assert!(app.messages[0].text_content().contains("ast_grep"));
        assert!(app.messages[0].text_content().contains("mcp_daedra"));
    }

    #[test]
    fn test_slash_quit() {
        let mut app = test_app();
        app.handle_slash_command("/quit");
        assert!(app.should_quit);
    }

    #[test]
    fn test_slash_unknown() {
        let mut app = test_app();
        app.handle_slash_command("/bogus");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("Unknown command"));
    }

    #[test]
    fn test_slash_shorthand() {
        let mut app = test_app();
        app.handle_slash_command("/c");
        // /c is alias for /clear
        assert!(app.messages.is_empty());
    }

    #[test]
    fn test_dynamic_input_height() {
        let app = test_app();
        // Default: 1 line of input → height should be 3 (1 + 2 for border)
        let input_lines = app.input.lines().len();
        let height = (input_lines + 2).clamp(3, 10);
        assert_eq!(height, 3);
    }

    // ===== Command palette tests =====

    #[test]
    fn test_ctrl_p_toggles_palette() {
        let mut app = test_app();
        assert!(!app.palette_open);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('p'), KeyModifiers::CONTROL,
        )));
        assert!(app.palette_open);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('p'), KeyModifiers::CONTROL,
        )));
        assert!(!app.palette_open);
    }

    #[test]
    fn test_palette_filter() {
        let mut app = test_app();
        app.palette_open = true;
        app.palette_query = "model".to_string();
        let items = app.palette_items();
        assert!(items.iter().all(|(cmd, _)| cmd.contains("model") || cmd.contains("Model")));
        assert!(!items.is_empty());
    }

    #[test]
    fn test_palette_esc_closes() {
        let mut app = test_app();
        app.palette_open = true;
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc, KeyModifiers::NONE,
        )));
        assert!(!app.palette_open);
    }

    #[test]
    fn test_session_stats_increment() {
        let app = test_app();
        assert_eq!(app.session_tool_calls, 0);
        assert_eq!(app.session_files_edited, 0);
    }

    // ===== Markdown rendering tests (existing) =====

    #[test]
    fn test_header_h1() {
        let lines = markdown_to_lines("# Hello");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Hello");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn test_header_h2() {
        let lines = markdown_to_lines("## Subtitle");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Subtitle");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn test_header_h3() {
        let lines = markdown_to_lines("### Section");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Section");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_bullet_list() {
        let lines = markdown_to_lines("- item one");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('•'));
    }

    #[test]
    fn test_star_bullet() {
        let lines = markdown_to_lines("* star item");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('•'));
    }

    #[test]
    fn test_code_block() {
        let lines = markdown_to_lines("```rust\nlet x = 1;\n```");
        assert_eq!(lines.len(), 3);
        // First line is separator with language
        assert!(lines[0].spans[0].content.contains("rust"));
        // Middle line is code with dark background
        assert_eq!(lines[1].spans[0].style.bg, Some(Color::Rgb(30, 30, 46)));
        // Last line is closing separator
        assert!(lines[2].spans[0].content.contains('─'));
    }

    #[test]
    fn test_code_block_no_lang() {
        let lines = markdown_to_lines("```\nhello\n```");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].spans[0].content.contains("code"));
    }

    #[test]
    fn test_blockquote() {
        let lines = markdown_to_lines("> quoted text");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('│'));
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::ITALIC));
    }

    #[test]
    fn test_horizontal_rule() {
        let lines = markdown_to_lines("---");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('─'));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn test_numbered_list() {
        let lines = markdown_to_lines("1. first item");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans.len() >= 2);
        // First span is the bold number
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_inline_bold() {
        let spans = parse_inline_markdown("hello **world**");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "hello ");
        assert_eq!(spans[1].content, "world");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_inline_code() {
        let spans = parse_inline_markdown("use `cargo test` here");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "use ");
        assert_eq!(spans[1].content, "cargo test");
        assert_eq!(spans[1].style.fg, Some(Color::Yellow));
        assert_eq!(spans[2].content, " here");
    }

    #[test]
    fn test_inline_italic() {
        let spans = parse_inline_markdown("this is *important*");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "this is ");
        assert_eq!(spans[1].content, "important");
        assert!(spans[1].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_inline_mixed() {
        let spans = parse_inline_markdown("**bold** and `code`");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, " and ");
        assert_eq!(spans[2].content, "code");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_plain_text() {
        let spans = parse_inline_markdown("just plain text");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "just plain text");
    }

    #[test]
    fn test_unclosed_bold() {
        // Unclosed ** should render as literal **
        let spans = parse_inline_markdown("hello **unclosed");
        assert!(spans.len() >= 2);
    }

    #[test]
    fn test_multiline_markdown() {
        let text = "# Title\n\nSome **bold** text\n\n- bullet\n- another";
        let lines = markdown_to_lines(text);
        assert!(lines.len() >= 5);
        // First line is H1
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_empty_input() {
        let lines = markdown_to_lines("");
        // Empty string produces no lines (str::lines() returns empty iterator)
        assert_eq!(lines.len(), 0);
    }

    // ===== Welcome screen tests =====

    #[test]
    fn test_welcome_shown_on_fresh_app() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        assert!(app.show_welcome, "Fresh app should show welcome");
    }

    #[test]
    fn test_welcome_dismissed_on_keypress() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        assert!(app.show_welcome);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'), KeyModifiers::NONE,
        )));
        assert!(!app.show_welcome, "Any keypress should dismiss welcome");
    }

    #[test]
    fn test_welcome_swallows_keypress() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        // Type 'a' while welcome is showing — should NOT reach input
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'), KeyModifiers::NONE,
        )));
        assert!(app.input.lines().iter().all(|l| l.is_empty()), "Welcome should swallow the keypress");
    }

    #[test]
    fn test_welcome_renders() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(text.contains("pawan"), "Welcome overlay should show 'pawan'");
    }

    // ===== F1 Help overlay tests =====

    #[test]
    fn test_f1_toggles_help_overlay() {
        let mut app = test_app();
        assert!(!app.help_overlay);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::F(1), KeyModifiers::NONE,
        )));
        assert!(app.help_overlay, "F1 should open help overlay");
    }

    #[test]
    fn test_help_overlay_dismissed_on_keypress() {
        let mut app = test_app();
        app.help_overlay = true;
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('q'), KeyModifiers::NONE,
        )));
        assert!(!app.help_overlay, "Any keypress should dismiss help overlay");
    }

    #[test]
    fn test_help_overlay_swallows_keypress() {
        let mut app = test_app();
        app.help_overlay = true;
        // Type 'a' while help is showing — should NOT reach input
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'), KeyModifiers::NONE,
        )));
        assert!(app.input.lines().iter().all(|l| l.is_empty()), "Help overlay should swallow the keypress");
    }

    #[test]
    fn test_help_overlay_renders() {
        let mut app = test_app();
        app.help_overlay = true;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(text.contains("Keyboard"), "Help overlay should show keyboard shortcuts");
    }

    // ===== Export tests =====

    #[test]
    fn test_export_conversation() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(Role::User, "Hello"));
        app.messages.push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));
        let path = "/tmp/pawan_test_export.md";
        let result = app.export_conversation(path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("**You**"));
        assert!(content.contains("**Pawan**"));
        assert!(content.contains("Hello"));
        assert!(content.contains("Hi there!"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_slash_export() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(Role::User, "test msg"));
        app.handle_slash_command("/export /tmp/pawan_test_slash_export.md");
        // Should have added a system message about export
        assert!(app.messages.len() >= 2);
        let last = app.messages.last().unwrap();
        assert_eq!(last.role, Role::System);
        assert!(last.text_content().contains("Exported"), "Should confirm export: {}", last.text_content());
        std::fs::remove_file("/tmp/pawan_test_slash_export.md").ok();
    }

    // ===== Timestamp tests =====

    #[test]
    fn test_message_has_timestamp() {
        let before = std::time::Instant::now();
        let msg = DisplayMessage::new_text(Role::User, "test");
        let after = std::time::Instant::now();
        assert!(msg.timestamp >= before);
        assert!(msg.timestamp <= after);
    }

    // ===== Scroll indicator tests =====

    #[test]
    fn test_scroll_indicator_in_title() {
        let mut app = test_app();
        // Add enough messages to exceed the visible area so scroll indicator appears
        for i in 0..20 {
            app.messages.push(DisplayMessage::new_text(Role::User, format!("message line {}", i)));
        }
        app.scroll = 5;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        // Scroll indicator now shows percentage when content exceeds visible area
        assert!(text.contains("[") && text.contains("%]"),
            "Should show scroll percentage indicator, got:\n{}", &text[..300.min(text.len())]);
    }

    // ===== Message count in status bar =====

    #[test]
    fn test_status_bar_shows_message_count() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(Role::User, "hi"));
        app.messages.push(DisplayMessage::new_text(Role::Assistant, "hello"));
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(text.contains("2msg"), "Status bar should show '2msg'");
    }

    #[test]
    fn test_palette_includes_export() {
        let app = test_app();
        let items = app.palette_items();
        assert!(items.iter().any(|(cmd, _)| *cmd == "/export"), "Palette should include /export");
    }
}

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
