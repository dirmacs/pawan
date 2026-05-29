//! Model picker modal key handling.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::app::App;

pub(crate) fn handle_model_picker_key(app: &mut App<'_>, key: &KeyEvent) -> bool {
    if !app.model_picker.visible {
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            app.model_picker.visible = false;
            app.model_picker.query.clear();
            app.model_picker.selected = 0;
        }
        KeyCode::Backspace => {
            app.model_picker.query.pop();
            app.model_picker.selected = 0;
        }
        KeyCode::Char(c) => {
            app.model_picker.query.push(c);
            app.model_picker.selected = 0;
        }
        KeyCode::Up => {
            let filtered = app.filtered_models().len();
            if filtered > 0 {
                app.model_picker.selected = app.model_picker.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            let filtered = app.filtered_models().len();
            if filtered > 0 {
                app.model_picker.selected = (app.model_picker.selected + 1).min(filtered - 1);
            }
        }
        KeyCode::PageUp => {
            let filtered = app.filtered_models().len();
            if filtered > 0 {
                app.model_picker.selected = app.model_picker.selected.saturating_sub(10);
            }
        }
        KeyCode::PageDown => {
            let filtered = app.filtered_models().len();
            if filtered > 0 {
                app.model_picker.selected = (app.model_picker.selected + 10).min(filtered - 1);
            }
        }
        KeyCode::Home => {
            app.model_picker.selected = 0;
        }
        KeyCode::End => {
            let filtered = app.filtered_models().len();
            if filtered > 0 {
                app.model_picker.selected = filtered - 1;
            }
        }
        KeyCode::Enter => {
            let model_id = {
                let models = app.filtered_models();
                models.get(app.model_picker.selected).map(|m| m.id.clone())
            };
            if let Some(model_id) = model_id {
                app.switch_model(model_id);
            }
            app.model_picker.visible = false;
            app.model_picker.query.clear();
            app.model_picker.selected = 0;
        }
        _ => {}
    }
    true
}
