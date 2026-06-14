//! Shared types, enums, and helpers for the TUI.

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
use std::sync::LazyLock;
use std::time::Instant;
use tokio::sync::mpsc;

/// Autosave interval (5 minutes)
pub(crate) const AUTOSAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

/// Events sent from the agent task back to the TUI
pub(crate) enum AgentEvent {
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
    /// IRC message delivered to a peer inbox
    IrcSent(pawan::agent::IrcMessage),
    /// IRC message received on this agent's inbox
    #[allow(dead_code)]
    IrcReceived(pawan::agent::IrcMessage),
}

/// Commands sent from the TUI to the agent task
pub(crate) enum AgentCommand {
    Execute(String),
    SwitchModel(String),
    /// Route an IRC-style message to another agent id (or "all")
    IrcSend {
        to: String,
        body: String,
    },
    Quit,
}

/// A single content block within a message, preserving event ordering.
#[derive(Clone, Debug)]
pub enum ContentBlock {
    /// Text emitted by the model. May be built incrementally during streaming.
    Text { content: String, streaming: bool },
    /// A tool call with optional result. Transitions: Running -> Done.
    ToolCall {
        name: String,
        args_summary: String,
        state: Box<ToolBlockState>,
    },
}

/// State of a tool call block.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ToolBlockState {
    Running,
    Done {
        record: ToolCallRecord,
        expanded: bool,
    },
}

/// Session sort modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSortMode {
    #[allow(dead_code)]
    NewestFirst,
    #[allow(dead_code)]
    Alphabetical,
    #[allow(dead_code)]
    MostUsed,
}

/// Export format options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    #[allow(dead_code)]
    Markdown,
    Html,
    Json,
    Txt,
}

impl ExportFormat {
    /// Parse format from string, defaulting to Markdown
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "html" => ExportFormat::Html,
            "json" => ExportFormat::Json,
            "txt" | "text" => ExportFormat::Txt,
            "md" | "markdown" => ExportFormat::Markdown,
            _ => ExportFormat::Markdown,
        }
    }

    /// Get file extension for this format
    #[allow(dead_code)]
    pub fn extension(&self) -> &str {
        match self {
            ExportFormat::Markdown => ".md",
            ExportFormat::Html => ".html",
            ExportFormat::Json => ".json",
            ExportFormat::Txt => ".txt",
        }
    }

    /// Get all valid format names for error messages
    #[allow(dead_code)]
    pub fn valid_formats() -> &'static [&'static str] {
        &["markdown", "html", "json", "txt"]
    }
}

/// Model info for visual model selector
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub quality_score: u8,
}

/// Streaming state for the assistant message currently being assembled.
pub(crate) struct StreamingAssistantState {
    pub(crate) blocks: Vec<ContentBlock>,
}

#[derive(Clone)]
/// A message for display in the TUI
pub struct DisplayMessage {
    pub role: Role,
    pub(crate) blocks: Vec<ContentBlock>,
    pub timestamp: std::time::Instant,
    /// Cached rendered lines for content blocks (excludes header). None = needs rebuild.
    pub(crate) cached_block_lines: Option<Vec<Line<'static>>>,
}

impl DisplayMessage {
    pub(crate) fn new_text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            blocks: vec![ContentBlock::Text {
                content: content.into(),
                streaming: false,
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        }
    }

    pub(crate) fn from_agent_response(resp: &AgentResponse) -> Self {
        let mut blocks = Vec::new();
        if !resp.content.is_empty() {
            blocks.push(ContentBlock::Text {
                content: resp.content.clone(),
                streaming: false,
            });
        }
        for tc in &resp.tool_calls {
            blocks.push(ContentBlock::ToolCall {
                name: tc.name.clone(),
                args_summary: summarize_args(&tc.arguments),
                state: Box::new(ToolBlockState::Done {
                    record: tc.clone(),
                    expanded: !tc.success,
                }),
            });
        }
        Self {
            role: Role::Assistant,
            blocks,
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        }
    }

    /// Invalidate the render cache (call when blocks change).
    pub(crate) fn invalidate_cache(&mut self) {
        self.cached_block_lines = None;
    }

    /// Flat text content for search and export.
    pub(crate) fn text_content(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// All completed tool call records.
    pub(crate) fn tool_records(&self) -> Vec<&ToolCallRecord> {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolCall { state, .. } => {
                    if let ToolBlockState::Done { record, .. } = state.as_ref() {
                        Some(record)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect()
    }
}

/// Summarize JSON arguments to a compact display string.
pub fn summarize_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => map
            .iter()
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
            .join(", "),
        _ => String::new(),
    }
}

/// Format a completed tool result for expanded display.
pub fn format_tool_record_result(record: &ToolCallRecord) -> String {
    format_rmux_snapshot_result(record)
        .or_else(|| format_rmux_list_sessions_result(record))
        .or_else(|| format_rmux_list_panes_result(record))
        .or_else(|| format_rmux_status_result(record))
        .unwrap_or_else(|| format_tool_result(&record.result))
}

/// Format tool result for expanded display.
pub fn format_tool_result(result: &serde_json::Value) -> String {
    match result {
        serde_json::Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn is_rmux_action(record: &ToolCallRecord, action: &str) -> bool {
    record.name == "rmux" && record.arguments.get("action").and_then(|v| v.as_str()) == Some(action)
}

fn format_rmux_snapshot_result(record: &ToolCallRecord) -> Option<String> {
    if !is_rmux_action(record, "snapshot") {
        return None;
    }

    let visible_text = record
        .result
        .get("visible_text")
        .or_else(|| record.result.get("text"))
        .and_then(|v| v.as_str())?;

    let session = rmux_session_argument(record);
    let window = record
        .arguments
        .get("window")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let pane = record
        .arguments
        .get("pane")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cols = record
        .result
        .get("cols")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let rows = record
        .result
        .get("rows")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let revision = record
        .result
        .get("revision")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let mut out = format!(
        "RMUX snapshot\nsession: {session}\npane: window {window} / pane {pane}\nsize: {cols}x{rows}\nrevision: {revision}\nvisible text:"
    );
    if visible_text.is_empty() {
        out.push_str("\n(empty)");
    } else {
        out.push('\n');
        out.push_str(visible_text.trim_end());
    }
    Some(out)
}

fn rmux_session_argument(record: &ToolCallRecord) -> &str {
    record
        .arguments
        .get("session")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>")
}

fn format_rmux_list_sessions_result(record: &ToolCallRecord) -> Option<String> {
    if !is_rmux_action(record, "list_sessions") {
        return None;
    }

    let sessions = record.result.get("sessions")?.as_array()?;
    let count = record
        .result
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(sessions.len() as u64);
    let mut out = format!("RMUX sessions ({count})");
    if sessions.is_empty() {
        out.push_str("\n(no sessions)");
        return Some(out);
    }

    for session in sessions.iter().filter_map(|value| value.as_str()).take(20) {
        out.push_str(&format!("\n- {session}"));
    }
    if sessions.len() > 20 {
        out.push_str(&format!("\n… {} more sessions", sessions.len() - 20));
    }
    Some(out)
}

fn format_rmux_status_result(record: &ToolCallRecord) -> Option<String> {
    let action = record.arguments.get("action")?.as_str()?;
    let session = rmux_session_argument(record);
    match action {
        "send_text" => {
            let ok = record
                .result
                .get("ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let text = record
                .arguments
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "RMUX send text\nsession: {session}\nstatus: {}\ntext: {text}",
                if ok { "ok" } else { "failed" }
            ))
        }
        "send_key" => {
            let ok = record
                .result
                .get("ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let key = record
                .arguments
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>");
            Some(format!(
                "RMUX send key\nsession: {session}\nstatus: {}\nkey: {key}",
                if ok { "ok" } else { "failed" }
            ))
        }
        "wait_for_text" => {
            let ok = record
                .result
                .get("ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let text = record
                .arguments
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "RMUX wait for text\nsession: {session}\nstatus: {}\nmatched: {text}",
                if ok { "matched" } else { "not matched" }
            ))
        }
        "kill_session" => {
            let killed = record
                .result
                .get("killed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(format!(
                "RMUX kill session\nsession: {session}\nstatus: {}",
                if killed { "killed" } else { "not found" }
            ))
        }
        _ => None,
    }
}

fn format_rmux_list_panes_result(record: &ToolCallRecord) -> Option<String> {
    if !is_rmux_action(record, "list_panes") {
        return None;
    }

    let panes = record.result.get("panes")?.as_array()?;
    let count = record
        .result
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(panes.len() as u64);
    let mut out = format!("RMUX panes ({count})");
    if panes.is_empty() {
        out.push_str("\n(no panes)");
        return Some(out);
    }

    for pane in panes.iter().take(12) {
        let session = pane
            .get("session")
            .and_then(|v| v.as_str())
            .unwrap_or("<unknown>");
        let window_index = pane
            .get("window_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let pane_index = pane.get("pane_index").and_then(|v| v.as_u64()).unwrap_or(0);
        let pane_id = pane.get("pane_id").and_then(|v| v.as_u64()).unwrap_or(0);
        let state = pane
            .get("process")
            .and_then(|v| v.get("state"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let title = pane
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("untitled");
        let cwd = pane
            .get("working_directory")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let command = pane
            .get("command")
            .and_then(|v| v.as_array())
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|command| !command.is_empty())
            .unwrap_or_else(|| "-".to_string());

        out.push_str(&format!(
            "\n- {session}:{window_index}.{pane_index} %{pane_id} [{state}] {title}\n  cwd: {cwd}\n  cmd: {command}"
        ));
    }
    if panes.len() > 12 {
        out.push_str(&format!("\n… {} more panes", panes.len() - 12));
    }
    Some(out)
}

static REASONING_STRIP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<think>.*?</think>|\[/think\]").expect("static regex"));

/// Strip model "thinking" / reasoning tag regions from assistant text before display.
pub fn strip_reasoning_tags(s: &str) -> String {
    REASONING_STRIP.replace_all(s, "").to_string()
}

/// Active keybinding context (drives the status bar hint and modal priority).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeybindContext {
    Input,
    Normal,
    Command,
    Help,
    ModelPicker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelCatalogSource {
    Empty,
    Fallback,
    Live,
}

impl ModelCatalogSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Fallback => "fallback",
            Self::Live => "live NVIDIA",
        }
    }
}

/// Model picker: list, selection, filter query, visibility, and catalog source.
pub struct ModelPickerState {
    pub models: Vec<ModelInfo>,
    pub selected: usize,
    pub visible: bool,
    pub query: String,
    pub source: ModelCatalogSource,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Which panel is focused in the TUI
///
/// Represents the currently active input focus in the terminal UI.
pub enum Panel {
    Input,
    Messages,
}
