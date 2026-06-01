//! `App` construction and input/keybind helpers.

use pawan::config::TuiConfig;
use ratatui::style::Style;
use ratatui_textarea::TextArea;
use std::time::Instant;
use tokio::sync::mpsc;

use super::fuzzy_search::{default_command_item_lines, FuzzySearchState};
use super::state::{App, SlashCommandRegistry, INPUT_PLACEHOLDER};
use super::types::*;

impl<'a> App<'a> {
    pub(crate) fn new_input() -> TextArea<'a> {
        let theme = super::theme::current();
        let mut input = TextArea::default();
        input.set_cursor_line_style(Style::default());
        input.set_style(Style::default().fg(theme.foreground).bg(theme.surface));
        input.set_placeholder_text(INPUT_PLACEHOLDER);
        input.set_placeholder_style(Style::default().fg(theme.muted).bg(theme.surface));
        input
    }

    pub(crate) fn reset_input(&mut self) {
        self.input = Self::new_input();
    }

    pub(crate) fn restyle_input(&mut self) {
        let theme = super::theme::current();
        self.input
            .set_style(Style::default().fg(theme.foreground).bg(theme.surface));
        self.input
            .set_placeholder_style(Style::default().fg(theme.muted).bg(theme.surface));
    }

    pub fn new(
        config: TuiConfig,
        model_name: String,
        cmd_tx: mpsc::UnboundedSender<AgentCommand>,
        event_rx: mpsc::UnboundedReceiver<AgentEvent>,
        irc_relay: std::sync::Arc<std::sync::Mutex<pawan::agent::IrcRelay>>,
    ) -> Self {
        let input = Self::new_input();

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
            goal_mode: false,
            goal_objective: None,
            loop_mode: false,
            orchestrate_mode: false,
            orchestrate_task: None,
            model_fetch_rx: None,
            current_theme: "default".to_string(),
            accent_tween: super::effects::accent_fade_tween(super::theme::current().accent),
            token_tween: super::effects::token_roll_tween(),
            ctx_tween: super::effects::ctx_glide_tween(),
            queue_panel: super::queue_panel::QueuePanel::new(),
            status_bar: super::status_bar::StatusBar::new(),
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
            irc_compose_open: false,
            irc_compose_input: String::new(),
            irc_relay,
            last_frame: Instant::now(),
            content_fx: None,
            popup_fx: None,
            status_fx: None,
            overlay_was_active: false,
            spinner: ratatui_cheese::spinner::SpinnerState::new(
                ratatui_cheese::spinner::SpinnerType::Dot,
            ),
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
}
