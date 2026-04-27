//! Input submission (`submit_input`) for `App`.

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
use regex::Regex;
use std::io::{self, Stdout};
use std::sync::OnceLock;
use std::time::Instant;
use ratatui_textarea::{Input, TextArea};
use tokio::sync::mpsc;

use super::app::App;
use super::types::*;

impl<'a> App<'a> {
    /// Submit input — handles slash commands or sends to agent
    pub(crate) fn submit_input(&mut self) {
        let content: String = self.input.lines().join("\n");
        if content.trim().is_empty() {
            return;
        }

        // Save to history (only for non-slash commands)
        let trimmed = content.trim();
        if !trimmed.starts_with('/') && !trimmed.starts_with(':') {
            self.history.push(content.clone());
            self.history_position = None; // Reset history position when submitting new message
        }

        // Reset input
        self.input = TextArea::default();
        self.input.set_cursor_line_style(Style::default());
        self.input.set_placeholder_text(
            "Type your message... (Enter to send, ↑↓ for history, Ctrl+C to clear, Ctrl+Q to quit)",
        );

        let trimmed = content.trim();

        // Handle slash commands
        if trimmed.starts_with('/') || (trimmed.starts_with(':') && trimmed != ":") {
            self.handle_slash_command(trimmed);
            return;
        }

        // Normal message — send to agent
        self.messages
            .push(DisplayMessage::new_text(Role::User, content.clone()));

        self.processing = true;
        self.status = "Processing...".to_string();

        let _ = self.cmd_tx.send(AgentCommand::Execute(content));
    }
}
