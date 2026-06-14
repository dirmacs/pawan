//! Model picker modal key handling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::app::App;
use super::is_submit_key;

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
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
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
        _ if is_submit_key(key) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pawan::config::TuiConfig;
    use tokio::sync::mpsc;

    fn test_app<'a>() -> App<'a> {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let irc_hub = pawan::agent::IrcHub::new();
        let irc_relay = std::sync::Arc::new(std::sync::Mutex::new(irc_hub.join("main")));
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
            irc_relay,
        );
        app.show_welcome = false;
        app
    }

    fn visible_picker_app<'a>() -> App<'a> {
        let mut app = test_app();
        app.load_available_models();
        app.model_picker.visible = true;
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_model_picker_returns_false_when_hidden() {
        let mut app = test_app();
        app.model_picker.visible = false;
        assert!(!handle_model_picker_key(&mut app, &key(KeyCode::Esc)));
    }

    #[test]
    fn test_model_picker_esc_closes() {
        let mut app = visible_picker_app();
        app.model_picker.query.push('x');
        app.model_picker.selected = 3;

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::Esc)));
        assert!(!app.model_picker.visible);
        assert!(app.model_picker.query.is_empty());
        assert_eq!(app.model_picker.selected, 0);
    }

    #[test]
    fn test_model_picker_char_input() {
        let mut app = visible_picker_app();
        app.model_picker.selected = 2;

        assert!(handle_model_picker_key(
            &mut app,
            &KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
        ));
        assert_eq!(app.model_picker.query, "a");
        assert_eq!(app.model_picker.selected, 0);
    }

    #[test]
    fn test_model_picker_ignores_non_submit_control_character_input() {
        let mut app = visible_picker_app();

        assert!(handle_model_picker_key(
            &mut app,
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
        ));
        assert!(app.model_picker.query.is_empty());
    }

    #[test]
    fn test_model_picker_backspace() {
        let mut app = visible_picker_app();
        app.model_picker.query = "ab".to_string();
        app.model_picker.selected = 2;

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::Backspace)));
        assert_eq!(app.model_picker.query, "a");
        assert_eq!(app.model_picker.selected, 0);
    }

    #[test]
    fn test_model_picker_up_down_navigation() {
        let mut app = visible_picker_app();
        assert_eq!(app.model_picker.selected, 0);

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::Down)));
        assert_eq!(app.model_picker.selected, 1);

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::Up)));
        assert_eq!(app.model_picker.selected, 0);
    }

    #[test]
    fn test_model_picker_page_navigation() {
        let mut app = visible_picker_app();
        let last = app.filtered_models().len() - 1;

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::PageDown)));
        assert_eq!(app.model_picker.selected, 10.min(last));

        app.model_picker.selected = 5;
        assert!(handle_model_picker_key(&mut app, &key(KeyCode::PageDown)));
        assert_eq!(app.model_picker.selected, last);

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::PageUp)));
        assert_eq!(app.model_picker.selected, last.saturating_sub(10));
    }

    #[test]
    fn test_model_picker_home_end() {
        let mut app = visible_picker_app();
        let last = app.filtered_models().len() - 1;
        app.model_picker.selected = 4;

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::End)));
        assert_eq!(app.model_picker.selected, last);

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::Home)));
        assert_eq!(app.model_picker.selected, 0);
    }

    #[test]
    fn test_model_picker_lf_alias_closes() {
        let mut app = visible_picker_app();

        assert!(handle_model_picker_key(
            &mut app,
            &KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)
        ));
        assert!(!app.model_picker.visible);
    }

    #[test]
    fn test_model_picker_cr_alias_closes() {
        let mut app = visible_picker_app();

        assert!(handle_model_picker_key(
            &mut app,
            &KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL)
        ));
        assert!(!app.model_picker.visible);
    }

    #[test]
    fn test_model_picker_enter_closes() {
        let mut app = visible_picker_app();
        app.model_picker.query = "test".to_string();
        app.model_picker.selected = 1;

        assert!(handle_model_picker_key(&mut app, &key(KeyCode::Enter)));
        assert!(!app.model_picker.visible);
        assert!(app.model_picker.query.is_empty());
        assert_eq!(app.model_picker.selected, 0);
    }
}
