//! `App` state, slash registry types, and permission dialog.

use pawan::config::TuiConfig;
use pawan::Result;
use ratatui::style::Style;
use ratatui_textarea::TextArea;
use std::time::Instant;
use tokio::sync::mpsc;

use super::fuzzy_search::FuzzySearchState;
use super::types::*;

pub(crate) const INPUT_PLACEHOLDER: &str =
    "Type your message... (Enter to send, ↑↓ for history, Ctrl+C to clear, Ctrl+Q to quit)";

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
    /// Goal mode — when active, agent works toward a user-specified objective
    pub(crate) goal_mode: bool,
    /// Objective text set via `/goal <objective>` (cleared when goal mode is turned off)
    pub(crate) goal_objective: Option<String>,
    /// Loop mode — when active, agent auto-continues after each response
    pub(crate) loop_mode: bool,
    /// Orchestration mode — when active, subagents coordinate on a task
    pub(crate) orchestrate_mode: bool,
    /// Task text set via `/orchestrate <task>` (cleared when orchestration mode is off)
    pub(crate) orchestrate_task: Option<String>,
    /// Receiver for live model catalog (oneshot from spawned fetch)
    pub(crate) model_fetch_rx: Option<tokio::sync::oneshot::Receiver<Vec<ModelInfo>>>,
    /// Centralized theme system (ArcSwap-backed, thread-safe)
    pub(crate) current_theme: String,
    /// Accent-colour fade across `/theme` switches (animate crate).
    pub(crate) accent_tween: super::effects::ValueTween<ratatui::style::Color>,
    /// Rolls the displayed cumulative token total toward its new value.
    pub(crate) token_tween: super::effects::ValueTween<f64>,
    /// Eases the context-usage fraction (0.0..=1.0) toward its new value.
    pub(crate) ctx_tween: super::effects::ValueTween<f32>,
    /// Sub-agent task queue display
    pub(crate) queue_panel: super::queue_panel::QueuePanel,
    /// Bottom status strip with flash-on-event support
    pub(crate) status_bar: super::status_bar::StatusBar,
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
    /// IRC compose modal (opened by /irc)
    pub(crate) irc_compose_open: bool,
    pub(crate) irc_compose_input: String,
    /// Main orchestrator IRC endpoint (shared with agent task)
    pub(crate) irc_relay: std::sync::Arc<std::sync::Mutex<pawan::agent::IrcRelay>>,
    /// Wall-clock timestamp of the previous rendered frame; drives effect timing.
    pub(crate) last_frame: Instant,
    /// Fade-in applied to the message area when a new assistant turn finalizes.
    pub(crate) content_fx: Option<tachyonfx::Effect>,
    /// Sweep-in applied when a modal overlay (dialog/picker) becomes active.
    pub(crate) popup_fx: Option<tachyonfx::Effect>,
    /// Accent pulse applied to the status strip on token/context updates.
    pub(crate) status_fx: Option<tachyonfx::Effect>,
    /// Tracks whether a modal overlay was active last frame (popup_fx trigger edge).
    pub(crate) overlay_was_active: bool,
    /// Animated "thinking" spinner shown while the agent is processing.
    pub(crate) spinner: ratatui_cheese::spinner::SpinnerState,
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    ("/model", "Show or switch LLM model"),
    ("/tools", "List available tools"),
    ("/search", "Web search via Daedra"),
    ("/handoff", "Hand off conversation to a new session"),
    ("/heal", "Auto-fix build errors"),
    ("/quit", "Exit pawan"),
    ("/exit", "Exit pawan (alias)"),
    ("/export", "Export conversation to a file"),
    ("/diff", "Show git diff"),
    ("/import", "Import conversation from JSON"),
    ("/fork", "Clone current session to a new one"),
    ("/dump", "Copy conversation to clipboard"),
    ("/share", "Export session and print a shareable path"),
    ("/save", "Save current conversation as a session"),
    ("/sessions", "Browse and manage saved sessions"),
    ("/searchsessions", "Search saved sessions"),
    ("/prune", "Prune old saved sessions"),
    ("/tag", "Manage session tags (add/rm/list/clear)"),
    ("/load", "Load a saved session"),
    ("/resume", "Resume a saved session"),
    ("/new", "Start a fresh conversation"),
    ("/session", "Switch to a session by id"),
    ("/retry", "Retry the last assistant response"),
    ("/compact", "Compact the conversation context"),
    ("/theme", "Switch color theme (e.g. /theme nord)"),
    ("/irc", "Send IRC message to a subagent"),
    ("/help", "Show this help list"),
    ("/goal", "Set a goal for the agent to work toward"),
    ("/orchestrate", "Orchestrate subagents for a task"),
    ("/loop", "Toggle auto-continue loop mode"),
];

/// State for an active permission prompt dialog
pub(crate) struct PermissionDialog {
    pub(crate) tool_name: String,
    pub(crate) args_summary: String,
    pub(crate) respond: Option<tokio::sync::oneshot::Sender<bool>>,
}
