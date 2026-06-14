//! Fuzzy search modal key handling.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::app::App;
use super::super::fuzzy_search::command_prefix;
use super::is_submit_key;

pub(crate) fn handle_fuzzy_search_key(app: &mut App<'_>, key: &KeyEvent) -> bool {
    let Some(fs) = app.fuzzy_search.as_mut() else {
        return false;
    };

    match key.code {
        KeyCode::Esc => {
            app.fuzzy_search = None;
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
        KeyCode::Char(c)
            if key.modifiers.is_empty()
                || key.modifiers == crossterm::event::KeyModifiers::SHIFT =>
        {
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
        _ if is_submit_key(key) => {
            let cmd = fs
                .results
                .get(fs.selected)
                .map(|s| command_prefix(s).to_string());
            app.fuzzy_search = None;
            if let Some(cmd) = cmd {
                if !cmd.is_empty() {
                    app.handle_slash_command(&cmd);
                }
            }
        }
        _ => {}
    }
    true
}
