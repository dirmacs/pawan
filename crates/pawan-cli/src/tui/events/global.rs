//! Global key chords (Ctrl+*) handled before overlay-specific routing.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::app::App;

pub(crate) fn handle_global_key(app: &mut App<'_>, key: &KeyEvent) -> bool {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            app.reset_input();
            app.history_position = None;
            app.status = "Input cleared".to_string();
            true
        }
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
            app.should_quit = true;
            true
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            app.messages.clear();
            app.status = "Cleared".to_string();
            true
        }
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            app.apply_goal_command("");
            true
        }
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.toggle_fuzzy_search();
            true
        }
        (KeyModifiers::CONTROL, KeyCode::Char('f')) | (KeyModifiers::CONTROL, KeyCode::Char('F')) => {
            app.toggle_fuzzy_search();
            true
        }
        (KeyModifiers::CONTROL, KeyCode::Char('m')) | (KeyModifiers::CONTROL, KeyCode::Char('M')) => {
            if app.model_picker.models.is_empty() {
                app.load_available_models();
            }
            app.model_picker.visible = !app.model_picker.visible;
            if !app.model_picker.visible {
                app.model_picker.query.clear();
                app.model_picker.selected = 0;
            }
            true
        }
        (_, KeyCode::F(1)) => {
            app.help_overlay = !app.help_overlay;
            true
        }
        _ => false,
    }
}
