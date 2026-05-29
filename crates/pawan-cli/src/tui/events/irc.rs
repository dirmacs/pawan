//! IRC compose modal key handling.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::app::App;

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
        KeyCode::Char(c) => {
            app.irc_compose_input.push(c);
        }
        KeyCode::Enter => {
            app.submit_irc_compose();
        }
        _ => {}
    }
    true
}
