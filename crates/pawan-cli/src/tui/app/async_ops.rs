//! Async TUI loop, agent task, and entrypoints.

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pawan::agent::{AgentResponse, IrcHub, PawanAgent, Role, ToolCallRecord};
use pawan::config::TuiConfig;
use pawan::{PawanError, Result};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io::{self, Stdout};
use std::time::Instant;
use tokio::sync::mpsc;

use super::state::{App, PermissionDialog};
use super::types::*;

impl<'a> App<'a> {
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
            // Poll for live model catalog result
            if let Some(mut rx) = self.model_fetch_rx.take() {
                match rx.try_recv() {
                    Ok(models) => {
                        self.model_picker.models = models;
                        self.status = format!("Loaded {} live models", self.model_picker.models.len());
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                        self.model_fetch_rx = Some(rx); // not ready yet, put back
                    }
                    Err(_) => {} // sender dropped = fetch failed, keep fallback
                }
            }

            self.poll_irc_inbox();

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
                    AgentEvent::IrcReceived(msg) => {
                        self.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("[IRC] {} → {}: {}", msg.from, msg.to, msg.body),
                        ));
                        self.status = format!("IRC from {}", msg.from);
                    }
                    AgentEvent::IrcSent(msg) => {
                        self.status = format!("IRC sent to {}", msg.to);
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
                                if self.goal_mode {
                                    let hint = self
                                        .goal_objective
                                        .as_deref()
                                        .map(|o| format!("Goal mode: checking objective — {o}"))
                                        .unwrap_or_else(|| {
                                            "Goal mode: checking if objective achieved..."
                                                .to_string()
                                        });
                                    self.messages.push(DisplayMessage::new_text(
                                        Role::System,
                                        hint.clone(),
                                    ));
                                    self.status = hint;
                                }
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

}

async fn agent_task(
    mut agent: PawanAgent,
    mut cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    irc_relay: std::sync::Arc<std::sync::Mutex<pawan::agent::IrcRelay>>,
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
            AgentCommand::IrcSend { to, body } => {
                let result = {
                    let relay = irc_relay.lock().expect("irc relay lock");
                    relay.send(&to, body)
                };
                match result {
                    Ok(msg) => {
                        let _ = event_tx.send(AgentEvent::IrcSent(msg));
                    }
                    Err(err) => {
                        let _ = event_tx.send(AgentEvent::Complete(Err(PawanError::Agent(
                            format!("IRC send failed: {err}"),
                        ))));
                    }
                }
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

    let irc_hub = IrcHub::new();
    let irc_relay = std::sync::Arc::new(std::sync::Mutex::new(irc_hub.join("main")));
    let irc_for_task = std::sync::Arc::clone(&irc_relay);

    // Spawn agent on a separate task
    tokio::spawn(agent_task(agent, cmd_rx, event_tx, irc_for_task));

    // Run the TUI on the current task
    let mut app = App::new(config, model_name, cmd_tx, event_rx, irc_relay);
    // Spawn live model catalog fetch
    let (fetch_tx, fetch_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        if let Some(models) = super::model_catalog::fetch_live_models().await {
            let _ = fetch_tx.send(models);
        }
    });
    app.model_fetch_rx = Some(fetch_rx);

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
