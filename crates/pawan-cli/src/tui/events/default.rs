//! Default key routing: in-buffer search, session browser, and panel navigation.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pawan::agent::session::{Session, SessionSummary};
use pawan::agent::Role;
use ratatui_textarea::Input;

use super::super::app::App;
use super::super::fuzzy_search::{default_command_item_lines, FuzzySearchState};
use super::super::types::{DisplayMessage, Panel};

pub(crate) fn handle_search_mode_key(app: &mut App<'_>, key: &KeyEvent) -> bool {
    if !app.search_mode {
        return false;
    }

    match key.code {
        KeyCode::Enter | KeyCode::Esc => {
            app.search_mode = false;
            if key.code == KeyCode::Esc {
                app.search_query.clear();
            }
        }
        KeyCode::Backspace => {
            app.search_query.pop();
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
        }
        _ => {}
    }
    true
}

pub(crate) fn handle_session_browser_key(app: &mut App<'_>, key: &KeyEvent) -> bool {
    if !app.session_browser_open {
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            app.session_browser_open = false;
            app.session_browser_query.clear();
            app.session_browser_selected = 0;
        }
        KeyCode::Backspace => {
            app.session_browser_query.pop();
            app.session_browser_selected = 0;
        }
        KeyCode::Char(c) => {
            app.session_browser_query.push(c);
            app.session_browser_selected = 0;
        }
        KeyCode::Up => {
            let sessions = app.filtered_sessions().len();
            if sessions > 0 {
                app.session_browser_selected = app.session_browser_selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            let sessions = app.filtered_sessions().len();
            if sessions > 0 {
                app.session_browser_selected =
                    (app.session_browser_selected + 1).min(sessions - 1);
            }
        }
        KeyCode::PageUp => {
            let sessions = app.filtered_sessions().len();
            if sessions > 0 {
                app.session_browser_selected = app.session_browser_selected.saturating_sub(10);
            }
        }
        KeyCode::PageDown => {
            let sessions = app.filtered_sessions().len();
            if sessions > 0 {
                app.session_browser_selected =
                    (app.session_browser_selected + 10).min(sessions - 1);
            }
        }
        KeyCode::Home => {
            app.session_browser_selected = 0;
        }
        KeyCode::End => {
            let sessions = app.filtered_sessions().len();
            if sessions > 0 {
                app.session_browser_selected = sessions - 1;
            }
        }
        KeyCode::Enter => {
            let sessions: Vec<SessionSummary> = app.filtered_sessions();
            if let Some(session) = sessions.get(app.session_browser_selected) {
                match Session::load(&session.id) {
                    Ok(s) => {
                        app.model_name = s.model.clone();
                        app.current_session_id = Some(s.id.clone());
                        app.session_tags = s.tags.clone();
                        app.messages = App::messages_from_session(s.messages);
                        app.scroll = 0;
                        app.status = format!("Loaded session: {}", session.id);
                        app.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Loaded session: {}", session.id),
                        ));
                    }
                    Err(e) => {
                        app.messages.push(DisplayMessage::new_text(
                            Role::System,
                            format!("Failed to load session: {}", e),
                        ));
                    }
                }
            }
            app.session_browser_open = false;
            app.session_browser_query.clear();
            app.session_browser_selected = 0;
        }
        _ => {}
    }
    true
}

pub(crate) fn handle_panel_key(app: &mut App<'_>, key: &KeyEvent) {
    match app.focus {
        Panel::Input => handle_input_panel_key(app, key),
        Panel::Messages => handle_messages_panel_key(app, key),
    }
}

fn handle_input_slash_popup_keys(app: &mut App<'_>, key: &KeyEvent) -> bool {
    if !app.is_slash_popup_active() {
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            app.reset_input();
            app.slash_popup_selected = 0;
        }
        KeyCode::Up => {
            app.slash_popup_selected = app.slash_popup_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            let items = app.slash_items();
            if !items.is_empty() {
                app.slash_popup_selected =
                    (app.slash_popup_selected + 1).min(items.len() - 1);
            }
        }
        KeyCode::PageUp => {
            let items = app.slash_items();
            if !items.is_empty() {
                app.slash_popup_selected = app.slash_popup_selected.saturating_sub(10);
            }
        }
        KeyCode::PageDown => {
            let items = app.slash_items();
            if !items.is_empty() {
                app.slash_popup_selected =
                    (app.slash_popup_selected + 10).min(items.len() - 1);
            }
        }
        KeyCode::Char('g') | KeyCode::Home => {
            app.slash_popup_selected = 0;
        }
        KeyCode::Char('G') | KeyCode::End => {
            let items = app.slash_items();
            if !items.is_empty() {
                app.slash_popup_selected = items.len() - 1;
            }
        }
        KeyCode::Tab => {
            let items = app.slash_items();
            if !items.is_empty() {
                app.slash_popup_selected = (app.slash_popup_selected + 1) % items.len();
            }
        }
        KeyCode::Enter => {
            let items = app.slash_items();
            if let Some((cmd, _)) = items.get(app.slash_popup_selected) {
                let cmd = cmd.to_string();
                app.reset_input();
                app.slash_popup_selected = 0;
                app.handle_slash_command(&cmd);
            }
        }
        _ => {
            app.input.input(Input::from(*key));
            app.slash_popup_selected = 0;
        }
    }
    true
}

fn handle_input_submit_keys(app: &mut App<'_>, key: &KeyEvent) -> bool {
    if key.code == KeyCode::Enter {
        app.submit_input();
        true
    } else {
        false
    }
}

fn handle_input_history_keys(app: &mut App<'_>, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Up => {
            if !app.history.is_empty() {
                let new_pos = match app.history_position {
                    None => Some(app.history.len() - 1),
                    Some(pos) if pos > 0 => Some(pos - 1),
                    _ => app.history_position,
                };
                if let Some(pos) = new_pos {
                    app.history_position = new_pos;
                    app.reset_input();
                    app.input.insert_str(&app.history[pos]);
                }
            }
            true
        }
        KeyCode::Down => {
            if let Some(pos) = app.history_position {
                if pos + 1 < app.history.len() {
                    app.history_position = Some(pos + 1);
                    app.reset_input();
                    app.input.insert_str(&app.history[pos + 1]);
                } else {
                    app.history_position = None;
                    app.reset_input();
                }
            }
            true
        }
        _ => false,
    }
}

fn handle_input_control_keys(app: &mut App<'_>, key: &KeyEvent) -> bool {
    if key.code == KeyCode::Char(':') && key.modifiers.is_empty() {
        let text: String = app.input.lines().join("\n");
        if text.trim().is_empty() {
            app.fuzzy_search = Some(FuzzySearchState::new(default_command_item_lines()));
        } else {
            app.input.input(Input::from(*key));
        }
        true
    } else if key.code == KeyCode::Tab {
        app.focus = Panel::Messages;
        true
    } else {
        false
    }
}

fn handle_input_editing_keys(app: &mut App<'_>, key: &KeyEvent) {
    app.input.input(Input::from(*key));
}

fn handle_input_panel_key(app: &mut App<'_>, key: &KeyEvent) {
    if handle_input_slash_popup_keys(app, key) {
        return;
    }
    if handle_input_submit_keys(app, key) {
        return;
    }
    if handle_input_history_keys(app, key) {
        return;
    }
    if handle_input_control_keys(app, key) {
        return;
    }
    handle_input_editing_keys(app, key);
}

fn handle_messages_panel_key(app: &mut App<'_>, key: &KeyEvent) {
    match key.code {
        KeyCode::Tab | KeyCode::Char('i') => app.focus = Panel::Input,
        KeyCode::Char('e') => {
            app.toggle_nearest_tool_expansion();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.scroll = app.scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.scroll = app.scroll.saturating_add(1);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll = app.scroll.saturating_sub(20);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.scroll = app.scroll.saturating_add(20);
        }
        KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10),
        KeyCode::Char('g') | KeyCode::Home => app.scroll = 0,
        KeyCode::Char('G') | KeyCode::End => {
            app.scroll = app.messages.len().saturating_sub(1);
        }
        KeyCode::Char('/') => {
            app.search_mode = true;
            app.search_query.clear();
        }
        KeyCode::Char('n') if !app.search_query.is_empty() => {
            let query = app.search_query.to_lowercase();
            for (i, msg) in app.messages.iter().enumerate() {
                if i > app.scroll && msg.text_content().to_lowercase().contains(&query) {
                    app.scroll = i;
                    break;
                }
            }
        }
        KeyCode::Char('N') if !app.search_query.is_empty() => {
            let query = app.search_query.to_lowercase();
            for i in (0..app.scroll).rev() {
                if app.messages[i]
                    .text_content()
                    .to_lowercase()
                    .contains(&query)
                {
                    app.scroll = i;
                    break;
                }
            }
        }
        _ => {}
    }
}
