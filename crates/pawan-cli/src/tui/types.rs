//! Shared types, enums, and helpers for the TUI.

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

use serde_json;

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
}

/// Commands sent from the TUI to the agent task
pub(crate) enum AgentCommand {
    Execute(String),
    SwitchModel(String),
    Quit,
}

/// A single content block within a message, preserving event ordering.
#[derive(Clone, Debug)]
pub(crate) enum ContentBlock {
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
pub(crate) enum ToolBlockState {
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

/// Export format options

impl ExportFormat {
    /// Parse format from string, defaulting to Markdown

    pub fn from_str(s: &str) -> Self {
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
pub(crate) fn summarize_args(args: &serde_json::Value) -> String {
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

/// One-line preview of a tool result for collapsed view.
pub(crate) fn one_line_preview(result: &serde_json::Value, max_len: usize) -> String {
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
pub(crate) fn format_tool_result(result: &serde_json::Value) -> String {
    match result {
        serde_json::Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

static REASONING_STRIP: OnceLock<Regex> = OnceLock::new();

/// Strip model "thinking" / reasoning tag regions from assistant text before display.
pub(crate) fn strip_reasoning_tags(s: &str) -> String {
    let re = REASONING_STRIP
        .get_or_init(|| Regex::new(r"(?s)<think>.*?</think>|\[/think\]").expect("static regex"));
    re.replace_all(s, "").to_string()
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

/// One keybinding row for documentation / the status bar.
pub struct KeyAction {
    /// Which UI mode this row applies to (used for docs / key maps).
    #[allow(dead_code)]
    pub context: KeybindContext,
    pub key: &'static str,
    pub description: &'static str,
}

/// Model picker: list, selection, filter query, and visibility.
pub struct ModelPickerState {
    pub models: Vec<ModelInfo>,
    pub selected: usize,
    pub visible: bool,
    pub query: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Which panel is focused in the TUI
///
/// Represents the currently active input focus in the terminal UI.
pub enum Panel {
    Input,
    Messages,
}
