//! IRC compose modal key handling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::app::App;
use super::is_submit_key;

pub(crate) fn handle_irc_compose_key(app: &mut App<'_>, key: &KeyEvent) -> bool {
    if !app.irc_compose_open {
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            app.irc_compose_open = false;
            app.irc_compose_input.clear();
        }
        KeyCode::Backspace => {
            app.irc_compose_input.pop();
        }
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            app.irc_compose_input.push(c);
        }
        _ if is_submit_key(key) => {
            app.submit_irc_compose();
        }
        _ => {}
    }
    true
}
