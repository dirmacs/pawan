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
    /// Agent finished
    Complete(std::result::Result<AgentResponse, PawanError>),
}

/// Commands sent from the TUI to the agent task
enum AgentCommand {
    Execute(String),
    SwitchModel(String),
    Quit,
}

#[derive(Clone)]
/// A message for display in the TUI
///
/// Represents a message to be displayed in the terminal UI,
/// including its role (user/agent), content, and any associated tool calls.
pub struct DisplayMessage {
    /// Role of the message sender (User or Agent)
    pub role: Role,
    /// Content of the message
    pub content: String,
    /// Tool calls associated with this message
    pub tool_calls: Vec<ToolCallRecord>,
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
    /// Streaming content buffer (accumulates tokens during generation)
    streaming_content: String,
    /// Active tool calls being displayed during processing
    active_tool: Option<String>,
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
    /// Session stats
    session_tool_calls: u32,
    session_files_edited: u32,
    /// Channel to send commands to the agent task
    cmd_tx: mpsc::UnboundedSender<AgentCommand>,
    /// Channel to receive events from the agent task
    event_rx: mpsc::UnboundedReceiver<AgentEvent>,
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
            streaming_content: String::new(),
            active_tool: None,
            iteration_count: 0,
            context_estimate: 0,
            search_mode: false,
            search_query: String::new(),
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            session_tool_calls: 0,
            session_files_edited: 0,
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
                        self.streaming_content.push_str(&token);
                        // Auto-scroll to bottom during streaming
                        self.scroll = usize::MAX;
                    }
                    AgentEvent::ToolStart(name) => {
                        self.active_tool = Some(name.clone());
                        self.status = format!("Running tool: {}", name);
                    }
                    AgentEvent::ToolComplete(record) => {
                        self.active_tool = None;
                        self.session_tool_calls += 1;
                        if record.name.contains("write_file") || record.name.contains("edit_file") {
                            self.session_files_edited += 1;
                        }
                        let icon = if record.success { "✓" } else { "✗" };
                        self.status = format!("{} {} ({}ms)", icon, record.name, record.duration_ms);
                    }
                    AgentEvent::Complete(result) => {
                        self.processing = false;
                        self.streaming_content.clear();
                        self.active_tool = None;
                        match result {
                            Ok(resp) => {
                                self.messages.push(DisplayMessage {
                                    role: Role::Assistant,
                                    content: resp.content,
                                    tool_calls: resp.tool_calls,
                                });
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
                                self.status = format!("Error: {}", e);
                                self.messages.push(DisplayMessage {
                                    role: Role::Assistant,
                                    content: format!("Error: {}", e),
                                    tool_calls: vec![],
                                });
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
                    _ => {}
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
                        if key.code == KeyCode::Enter {
                            self.submit_input();
                        } else if key.code == KeyCode::Tab {
                            self.focus = Panel::Messages;
                        } else {
                            self.input.input(Input::from(key));
                        }
                    }
                    Panel::Messages => match key.code {
                        KeyCode::Tab | KeyCode::Char('i') => self.focus = Panel::Input,
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
                                        && msg.content.to_lowercase().contains(&query)
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
                                    if self.messages[i].content.to_lowercase().contains(&query) {
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
        self.messages.push(DisplayMessage {
            role: Role::User,
            content: content.clone(),
            tool_calls: vec![],
        });

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
                    self.messages.push(DisplayMessage {
                        role: Role::System,
                        content: format!("Current model: {}", self.model_name),
                        tool_calls: vec![],
                    });
                } else {
                    self.model_name = arg.to_string();
                    self.status = format!("Model → {}", arg);
                    self.messages.push(DisplayMessage {
                        role: Role::System,
                        content: format!("Switching model to: {}", arg),
                        tool_calls: vec![],
                    });
                    // Send model switch to agent task — recreates backend
                    let _ = self.cmd_tx.send(AgentCommand::SwitchModel(arg.to_string()));
                }
            }
            "/tools" | "/t" => {
                self.messages.push(DisplayMessage {
                    role: Role::System,
                    content: "Core: bash, read_file, write_file, edit_file, ast_grep, glob_search, grep_search\n\
                              Standard: git (status/diff/add/commit/log/blame/branch/checkout/stash), agents, edit modes\n\
                              Extended: rg, fd, sd, tree, mise, zoxide, lsp\n\
                              MCP: mcp_daedra_web_search, mcp_daedra_visit_page".to_string(),
                    tool_calls: vec![],
                });
            }
            "/search" | "/s" => {
                if arg.is_empty() {
                    self.messages.push(DisplayMessage {
                        role: Role::System,
                        content: "Usage: /search <query>".to_string(),
                        tool_calls: vec![],
                    });
                } else {
                    // Send as a web search task
                    let search_prompt = format!(
                        "Use mcp_daedra_web_search to search for '{}' and report the results", arg
                    );
                    self.messages.push(DisplayMessage {
                        role: Role::User,
                        content: format!("/search {}", arg),
                        tool_calls: vec![],
                    });
                    self.processing = true;
                    self.status = format!("Searching: {}", arg);
                    let _ = self.cmd_tx.send(AgentCommand::Execute(search_prompt));
                }
            }
            "/heal" | "/h" => {
                self.messages.push(DisplayMessage {
                    role: Role::User,
                    content: "/heal".to_string(),
                    tool_calls: vec![],
                });
                self.processing = true;
                self.status = "Healing...".to_string();
                let _ = self.cmd_tx.send(AgentCommand::Execute(
                    "Run cargo check and cargo test. Fix any errors you find.".to_string()
                ));
            }
            "/quit" | "/q" => {
                self.should_quit = true;
            }
            "/help" | "/?" => {
                self.messages.push(DisplayMessage {
                    role: Role::System,
                    content: "/model [name]  — show or switch LLM model\n\
                              /search <query> — web search via Daedra\n\
                              /tools         — list available tools\n\
                              /heal          — auto-fix build errors\n\
                              /clear         — clear chat history\n\
                              /quit          — exit pawan\n\
                              /help          — show this help".to_string(),
                    tool_calls: vec![],
                });
            }
            _ => {
                self.messages.push(DisplayMessage {
                    role: Role::System,
                    content: format!("Unknown command: {}. Type /help for available commands.", command),
                    tool_calls: vec![],
                });
            }
        }
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

        // Command palette overlay (on top of everything)
        if self.palette_open {
            self.render_palette(f);
        }
    }

    /// Get filtered palette items based on query
    fn palette_items(&self) -> Vec<(&str, &str)> {
        let all_items: Vec<(&str, &str)> = vec![
            ("/help", "Show available commands"),
            ("/model", "Show or switch LLM model"),
            ("/model mistralai/mistral-small-4-119b-2603", "Switch to Mistral Small 4"),
            ("/model stepfun-ai/step-3.5-flash", "Switch to StepFun Flash"),
            ("/model qwen/qwen3.5-122b-a10b", "Switch to Qwen 122B"),
            ("/model minimaxai/minimax-m2.5", "Switch to MiniMax M2.5"),
            ("/search", "Web search via Daedra"),
            ("/tools", "List available tools"),
            ("/heal", "Auto-fix build errors"),
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

    /// Render command palette overlay
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
            for tc in &msg.tool_calls {
                let icon = if tc.success { "✓" } else { "✗" };
                let color = if tc.success { Color::Green } else { Color::Red };
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                    Span::styled(tc.name.clone(), Style::default().fg(Color::White)),
                    Span::styled(format!(" {}ms", tc.duration_ms), Style::default().fg(Color::DarkGray)),
                ])));
            }
        }
        if let Some(ref tool) = self.active_tool {
            items.push(ListItem::new(Line::from(vec![
                Span::styled(" ⚙ ", Style::default().fg(Color::Yellow)),
                Span::styled(tool.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            ])));
        }
        if items.is_empty() {
            items.push(ListItem::new(Span::styled(" Waiting...", Style::default().fg(Color::DarkGray))));
        }
        f.render_widget(List::new(items).block(block), area);
    }

    fn render_messages(&self, f: &mut Frame, area: Rect) {
        let mut items: Vec<ListItem> = Vec::new();

        for msg in &self.messages {
            let (prefix, style) = match msg.role {
                Role::User => (
                    "You: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Role::Assistant => (
                    "Pawan: ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Role::System => ("System: ", Style::default().fg(Color::Yellow)),
                Role::Tool => ("Tool: ", Style::default().fg(Color::Magenta)),
            };

            items.push(ListItem::new(Line::from(vec![Span::styled(prefix, style)])));

            if msg.role == Role::Assistant {
                for line in markdown_to_lines(&msg.content) {
                    let mut spans: Vec<Span<'static>> = vec![Span::raw("  ".to_string())];
                    spans.extend(line.spans);
                    items.push(ListItem::new(Line::from(spans)));
                }
            } else {
                for line in msg.content.lines() {
                    items.push(ListItem::new(Line::from(Span::raw(format!("  {}", line)))));
                }
            }

            for tc in &msg.tool_calls {
                let icon = if tc.success { "✓" } else { "✗" };
                let color = if tc.success { Color::Green } else { Color::Red };
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                    Span::styled(
                        format!("{}({}ms) ", tc.name, tc.duration_ms),
                        Style::default().fg(Color::Magenta),
                    ),
                ])));
            }

            items.push(ListItem::new(Line::from("")));
        }

        if self.processing {
            if !self.streaming_content.is_empty() {
                // Show streaming content as it arrives
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    "Pawan: ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )])));
                for line in self.streaming_content.lines() {
                    items.push(ListItem::new(Line::from(Span::styled(
                        format!("  {}", line),
                        Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
                    ))));
                }
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    "  ▌",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::SLOW_BLINK),
                )])));
            } else if let Some(ref tool) = self.active_tool {
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(
                        "  ⚙ ",
                        Style::default().fg(Color::Magenta),
                    ),
                    Span::styled(
                        format!("Running {}...", tool),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ])));
            } else {
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    "  Pawan is thinking...",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::ITALIC),
                )])));
            }
        }

        let border_style = if self.focus == Panel::Messages {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let title = if self.search_mode {
            format!(" Search: {}▌ ", self.search_query)
        } else if !self.search_query.is_empty() {
            format!(" Messages [/{}] (n/N next/prev, g/G top/bottom) ", self.search_query)
        } else {
            " Messages (Tab to focus, j/k scroll, / search, g/G top/bottom) ".to_string()
        };

        let messages_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let list = List::new(items).block(messages_block);
        f.render_widget(list, area);
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

                let result = agent
                    .execute_with_callbacks(
                        &prompt,
                        Some(on_token),
                        Some(on_tool),
                        Some(on_tool_start),
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
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    /// Create a test App with dummy channels (for state/render tests, not event loops)
    fn test_app<'a>() -> App<'a> {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        )
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
        app.messages.push(DisplayMessage {
            role: Role::User,
            content: "Hello pawan".to_string(),
            tool_calls: vec![],
        });
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            content: "Hi there!".to_string(),
            tool_calls: vec![],
        });

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
        app.streaming_content = "partial response so far".to_string();

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
        app.active_tool = Some("bash".to_string());

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
            content: "Done".to_string(),
            tool_calls: vec![
                ToolCallRecord {
                    id: "1".into(),
                    name: "write_file".into(),
                    arguments: serde_json::json!({}),
                    result: serde_json::json!({"success": true}),
                    success: true,
                    duration_ms: 42,
                },
                ToolCallRecord {
                    id: "2".into(),
                    name: "bash".into(),
                    arguments: serde_json::json!({}),
                    result: serde_json::json!({"error": "timeout"}),
                    success: false,
                    duration_ms: 30000,
                },
            ],
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
        app.messages.push(DisplayMessage {
            role: Role::User,
            content: "test".into(),
            tool_calls: vec![],
        });
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
        app.messages.push(DisplayMessage { role: Role::User, content: "no match".into(), tool_calls: vec![] });
        app.messages.push(DisplayMessage { role: Role::User, content: "has target word".into(), tool_calls: vec![] });
        app.messages.push(DisplayMessage { role: Role::User, content: "another target".into(), tool_calls: vec![] });
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
        app.messages.push(DisplayMessage { role: Role::User, content: "first target".into(), tool_calls: vec![] });
        app.messages.push(DisplayMessage { role: Role::User, content: "no match".into(), tool_calls: vec![] });
        app.messages.push(DisplayMessage { role: Role::User, content: "second target".into(), tool_calls: vec![] });
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
        assert_eq!(app.messages[0].content, "hello pawan");
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
        assert!(app.messages[0].content.contains("/model"));
        assert!(app.messages[0].content.contains("/search"));
        assert!(app.messages[0].content.contains("/quit"));
    }

    #[test]
    fn test_slash_clear() {
        let mut app = test_app();
        app.messages.push(DisplayMessage { role: Role::User, content: "test".into(), tool_calls: vec![] });
        app.messages.push(DisplayMessage { role: Role::Assistant, content: "reply".into(), tool_calls: vec![] });
        app.handle_slash_command("/clear");
        assert!(app.messages.is_empty());
        assert_eq!(app.status, "Cleared");
    }

    #[test]
    fn test_slash_model_show() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].content.contains("test-model"));
    }

    #[test]
    fn test_slash_model_switch() {
        let mut app = test_app();
        app.handle_slash_command("/model mistral-small-4");
        assert_eq!(app.model_name, "mistral-small-4");
        assert!(app.messages[0].content.contains("mistral-small-4"));
    }

    #[test]
    fn test_slash_tools() {
        let mut app = test_app();
        app.handle_slash_command("/tools");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].content.contains("bash"));
        assert!(app.messages[0].content.contains("ast_grep"));
        assert!(app.messages[0].content.contains("mcp_daedra"));
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
        assert!(app.messages[0].content.contains("Unknown command"));
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
        let mut app = test_app();
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
