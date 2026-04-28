//! Crossterm event handling for `App`.

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

use super::app::App;
use super::fuzzy_search::{command_prefix, default_command_item_lines, FuzzySearchState};
use super::types::*;

impl<'a> App<'a> {
    pub(crate) fn handle_event(&mut self, event: Event) {
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
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        if let Some(mut dialog) = self.permission_dialog.take() {
                            if let Some(tx) = dialog.respond.take() {
                                let _ = tx.send(true);
                            }
                            self.status = format!("Allowed (all): {}", dialog.tool_name);
                            self.auto_approve_tools = true;
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
                        // Always clear the input
                        self.reset_input();
                        self.history_position = None; // Reset history position
                        self.status = "Input cleared".to_string();
                        return;
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
                        // Quit the application
                        self.should_quit = true;
                        return;
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                        self.messages.clear();
                        self.status = "Cleared".to_string();
                        return;
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                        self.toggle_fuzzy_search();
                        return;
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('f'))
                    | (KeyModifiers::CONTROL, KeyCode::Char('F')) => {
                        self.toggle_fuzzy_search();
                        return;
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('m'))
                    | (KeyModifiers::CONTROL, KeyCode::Char('M')) => {
                        if self.model_picker.models.is_empty() {
                            self.load_available_models();
                        }
                        self.model_picker.visible = !self.model_picker.visible;
                        if !self.model_picker.visible {
                            self.model_picker.query.clear();
                            self.model_picker.selected = 0;
                        }
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

                // Fuzzy search modal
                if let Some(fs) = self.fuzzy_search.as_mut() {
                    match key.code {
                        KeyCode::Esc => {
                            self.fuzzy_search = None;
                        }
                        KeyCode::Backspace => {
                            fs.query.pop();
                            let q = fs.query.clone();
                            fs.filter(&q);
                        }
                        KeyCode::Char('g') | KeyCode::Home => {
                            fs.selected = 0;
                        }
                        KeyCode::Char('G') | KeyCode::End => {
                            fs.selected = fs.results.len().saturating_sub(1);
                        }
                        KeyCode::Char(c) => {
                            fs.query.push(c);
                            let q = fs.query.clone();
                            fs.filter(&q);
                        }
                        KeyCode::Up => {
                            fs.prev();
                        }
                        KeyCode::Down => {
                            fs.next();
                        }
                        KeyCode::PageUp => {
                            for _ in 0..10 {
                                fs.prev();
                            }
                        }
                        KeyCode::PageDown => {
                            for _ in 0..10 {
                                fs.next();
                            }
                        }
                        KeyCode::Enter => {
                            let cmd = fs
                                .results
                                .get(fs.selected)
                                .map(|s| command_prefix(s).to_string());
                            self.fuzzy_search = None;
                            if let Some(cmd) = cmd {
                                if !cmd.is_empty() {
                                    self.handle_slash_command(&cmd);
                                }
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

                // Model selector modal - intercept all keys when open
                if self.model_picker.visible {
                    match key.code {
                        KeyCode::Esc => {
                            self.model_picker.visible = false;
                            self.model_picker.query.clear();
                            self.model_picker.selected = 0;
                        }
                        KeyCode::Backspace => {
                            self.model_picker.query.pop();
                            self.model_picker.selected = 0;
                        }
                        KeyCode::Char(c) => {
                            self.model_picker.query.push(c);
                            self.model_picker.selected = 0;
                        }
                        KeyCode::Up => {
                            let filtered = self.filtered_models().len();
                            if filtered > 0 {
                                self.model_picker.selected =
                                    self.model_picker.selected.saturating_sub(1);
                            }
                        }
                        KeyCode::Down => {
                            let filtered = self.filtered_models().len();
                            if filtered > 0 {
                                self.model_picker.selected =
                                    (self.model_picker.selected + 1).min(filtered - 1);
                            }
                        }
                        KeyCode::PageUp => {
                            let filtered = self.filtered_models().len();
                            if filtered > 0 {
                                self.model_picker.selected =
                                    self.model_picker.selected.saturating_sub(10);
                            }
                        }
                        KeyCode::PageDown => {
                            let filtered = self.filtered_models().len();
                            if filtered > 0 {
                                self.model_picker.selected =
                                    (self.model_picker.selected + 10).min(filtered - 1);
                            }
                        }
                        KeyCode::Home => {
                            self.model_picker.selected = 0;
                        }
                        KeyCode::End => {
                            let filtered = self.filtered_models().len();
                            if filtered > 0 {
                                self.model_picker.selected = filtered - 1;
                            }
                        }
                        KeyCode::Enter => {
                            let model_id = {
                                let models = self.filtered_models();
                                models.get(self.model_picker.selected).map(|m| m.id.clone())
                            };
                            if let Some(model_id) = model_id {
                                self.switch_model(model_id);
                            }
                            self.model_picker.visible = false;
                            self.model_picker.query.clear();
                            self.model_picker.selected = 0;
                        }
                        _ => {}
                    }
                    return;
                }

                // Session browser modal - intercept all keys when open
                if self.session_browser_open {
                    match key.code {
                        KeyCode::Esc => {
                            self.session_browser_open = false;
                            self.session_browser_query.clear();
                            self.session_browser_selected = 0;
                        }
                        KeyCode::Backspace => {
                            self.session_browser_query.pop();
                            self.session_browser_selected = 0;
                        }
                        KeyCode::Char(c) => {
                            self.session_browser_query.push(c);
                            self.session_browser_selected = 0;
                        }
                        KeyCode::Up => {
                            let sessions = self.filtered_sessions().len();
                            if sessions > 0 {
                                self.session_browser_selected =
                                    self.session_browser_selected.saturating_sub(1);
                            }
                        }
                        KeyCode::Down => {
                            let sessions = self.filtered_sessions().len();
                            if sessions > 0 {
                                self.session_browser_selected =
                                    (self.session_browser_selected + 1).min(sessions - 1);
                            }
                        }
                        KeyCode::PageUp => {
                            let sessions = self.filtered_sessions().len();
                            if sessions > 0 {
                                self.session_browser_selected =
                                    self.session_browser_selected.saturating_sub(10);
                            }
                        }
                        KeyCode::PageDown => {
                            let sessions = self.filtered_sessions().len();
                            if sessions > 0 {
                                self.session_browser_selected =
                                    (self.session_browser_selected + 10).min(sessions - 1);
                            }
                        }
                        KeyCode::Home => {
                            self.session_browser_selected = 0;
                        }
                        KeyCode::End => {
                            let sessions = self.filtered_sessions().len();
                            if sessions > 0 {
                                self.session_browser_selected = sessions - 1;
                            }
                        }
                        KeyCode::Enter => {
                            let sessions: Vec<SessionSummary> = self.filtered_sessions();
                            if let Some(session) = sessions.get(self.session_browser_selected) {
                                match Session::load(&session.id) {
                                    Ok(s) => {
                                        self.model_name = s.model.clone();
                                        self.current_session_id = Some(s.id.clone());
                                        self.session_tags = s.tags.clone();
                                        self.messages = App::messages_from_session(s.messages);
                                        self.scroll = 0;
                                        self.status = format!("Loaded session: {}", session.id);
                                        self.messages.push(DisplayMessage::new_text(
                                            Role::System,
                                            format!("Loaded session: {}", session.id),
                                        ));
                                    }
                                    Err(e) => {
                                        self.messages.push(DisplayMessage::new_text(
                                            Role::System,
                                            format!("Failed to load session: {}", e),
                                        ));
                                    }
                                }
                            }
                            self.session_browser_open = false;
                            self.session_browser_query.clear();
                            self.session_browser_selected = 0;
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
                                    self.reset_input();
                                    self.slash_popup_selected = 0;
                                }
                                KeyCode::Up => {
                                    self.slash_popup_selected =
                                        self.slash_popup_selected.saturating_sub(1);
                                }
                                KeyCode::Down => {
                                    let items = self.slash_items();
                                    if !items.is_empty() {
                                        self.slash_popup_selected =
                                            (self.slash_popup_selected + 1).min(items.len() - 1);
                                    }
                                }
                                KeyCode::PageUp => {
                                    let items = self.slash_items();
                                    if !items.is_empty() {
                                        self.slash_popup_selected =
                                            self.slash_popup_selected.saturating_sub(10);
                                    }
                                }
                                KeyCode::PageDown => {
                                    let items = self.slash_items();
                                    if !items.is_empty() {
                                        self.slash_popup_selected =
                                            (self.slash_popup_selected + 10).min(items.len() - 1);
                                    }
                                }
                                KeyCode::Char('g') | KeyCode::Home => {
                                    self.slash_popup_selected = 0;
                                }
                                KeyCode::Char('G') | KeyCode::End => {
                                    let items = self.slash_items();
                                    if !items.is_empty() {
                                        self.slash_popup_selected = items.len() - 1;
                                    }
                                }
                                KeyCode::Tab => {
                                    let items = self.slash_items();
                                    if !items.is_empty() {
                                        self.slash_popup_selected =
                                            (self.slash_popup_selected + 1) % items.len();
                                    }
                                }
                                KeyCode::Enter => {
                                    let items = self.slash_items();
                                    if let Some((cmd, _)) = items.get(self.slash_popup_selected) {
                                        let cmd = cmd.to_string();
                                        self.reset_input();
                                        self.slash_popup_selected = 0;
                                        self.handle_slash_command(&cmd);
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
                        } else if key.code == KeyCode::Up {
                            // Navigate history backwards
                            if !self.history.is_empty() {
                                let new_pos = match self.history_position {
                                    None => Some(self.history.len() - 1),
                                    Some(pos) if pos > 0 => Some(pos - 1),
                                    _ => self.history_position,
                                };
                                if let Some(pos) = new_pos {
                                    self.history_position = new_pos;
                                    self.reset_input();
                                    self.input.insert_str(&self.history[pos]);
                                }
                            }
                        } else if key.code == KeyCode::Down {
                            // Navigate history forwards
                            if let Some(pos) = self.history_position {
                                if pos + 1 < self.history.len() {
                                    // Move to next history item
                                    self.history_position = Some(pos + 1);
                                    self.reset_input();
                                    self.input.insert_str(&self.history[pos + 1]);
                                } else {
                                    // Exit history mode, clear input
                                    self.history_position = None;
                                    self.reset_input();
                                }
                            }
                        } else if key.code == KeyCode::Char(':') && key.modifiers.is_empty() {
                            let text: String = self.input.lines().join("\n");
                            if text.trim().is_empty() {
                                self.fuzzy_search =
                                    Some(FuzzySearchState::new(default_command_item_lines()));
                            } else {
                                self.input.input(Input::from(key));
                            }
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
                        KeyCode::Char('n') if !self.search_query.is_empty() => {
                            // Jump to next search match
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
                        KeyCode::Char('N') if !self.search_query.is_empty() => {
                            // Jump to previous search match
                            let query = self.search_query.to_lowercase();
                            for i in (0..self.scroll).rev() {
                                if self.messages[i]
                                    .text_content()
                                    .to_lowercase()
                                    .contains(&query)
                                {
                                    self.scroll = i;
                                    break;
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
            Event::Mouse(mouse) if self.config.mouse_support => {
                match mouse.kind {
                    event::MouseEventKind::ScrollUp => {
                        // Handle popups first
                        if self.model_picker.visible {
                            self.model_picker.selected = self
                                .model_picker
                                .selected
                                .saturating_sub(self.config.scroll_speed);
                        } else if let Some(fs) = self.fuzzy_search.as_mut() {
                            let n = fs.results.len();
                            if n > 0 {
                                fs.selected = fs.selected.saturating_sub(self.config.scroll_speed);
                            }
                        } else if self.session_browser_open {
                            let sessions = self.filtered_sessions().len();
                            if sessions > 0 {
                                self.session_browser_selected = self
                                    .session_browser_selected
                                    .saturating_sub(self.config.scroll_speed);
                            }
                        } else if self.is_slash_popup_active() {
                            let items = self.slash_items();
                            if !items.is_empty() {
                                self.slash_popup_selected = self
                                    .slash_popup_selected
                                    .saturating_sub(self.config.scroll_speed);
                            }
                        } else {
                            // Default to messages panel
                            self.scroll = self.scroll.saturating_sub(self.config.scroll_speed);
                        }
                    }
                    event::MouseEventKind::ScrollDown => {
                        // Handle popups first
                        if self.model_picker.visible {
                            let filtered = self.filtered_models().len();
                            if filtered > 0 {
                                self.model_picker.selected = (self.model_picker.selected
                                    + self.config.scroll_speed)
                                    .min(filtered - 1);
                            }
                        } else if let Some(fs) = self.fuzzy_search.as_mut() {
                            let n = fs.results.len();
                            if n > 0 {
                                fs.selected = (fs.selected + self.config.scroll_speed).min(n - 1);
                            }
                        } else if self.session_browser_open {
                            let sessions = self.filtered_sessions().len();
                            if sessions > 0 {
                                self.session_browser_selected = (self.session_browser_selected
                                    + self.config.scroll_speed)
                                    .min(sessions - 1);
                            }
                        } else if self.is_slash_popup_active() {
                            let items = self.slash_items();
                            if !items.is_empty() {
                                self.slash_popup_selected = (self.slash_popup_selected
                                    + self.config.scroll_speed)
                                    .min(items.len() - 1);
                            }
                        } else {
                            // Default to messages panel
                            self.scroll = self.scroll.saturating_add(self.config.scroll_speed);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
