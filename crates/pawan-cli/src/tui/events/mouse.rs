//! Mouse scroll handling when enabled in config.

use crossterm::event::{self, MouseEvent};

use super::super::app::App;

pub(crate) fn handle_mouse_event(app: &mut App<'_>, mouse: MouseEvent) -> bool {
    if !app.config.mouse_support {
        return false;
    }

    match mouse.kind {
        event::MouseEventKind::ScrollUp => {
            if app.model_picker.visible {
                app.model_picker.selected = app
                    .model_picker
                    .selected
                    .saturating_sub(app.config.scroll_speed);
            } else if let Some(fs) = app.fuzzy_search.as_mut() {
                let n = fs.results.len();
                if n > 0 {
                    fs.selected = fs.selected.saturating_sub(app.config.scroll_speed);
                }
            } else if app.session_browser_open {
                let sessions = app.filtered_sessions().len();
                if sessions > 0 {
                    app.session_browser_selected = app
                        .session_browser_selected
                        .saturating_sub(app.config.scroll_speed);
                }
            } else if app.is_slash_popup_active() {
                let items = app.slash_items();
                if !items.is_empty() {
                    app.slash_popup_selected = app
                        .slash_popup_selected
                        .saturating_sub(app.config.scroll_speed);
                }
            } else {
                app.scroll = app.scroll.saturating_sub(app.config.scroll_speed);
            }
        }
        event::MouseEventKind::ScrollDown => {
            if app.model_picker.visible {
                let filtered = app.filtered_models().len();
                if filtered > 0 {
                    app.model_picker.selected =
                        (app.model_picker.selected + app.config.scroll_speed).min(filtered - 1);
                }
            } else if let Some(fs) = app.fuzzy_search.as_mut() {
                let n = fs.results.len();
                if n > 0 {
                    fs.selected = (fs.selected + app.config.scroll_speed).min(n - 1);
                }
            } else if app.session_browser_open {
                let sessions = app.filtered_sessions().len();
                if sessions > 0 {
                    app.session_browser_selected =
                        (app.session_browser_selected + app.config.scroll_speed).min(sessions - 1);
                }
            } else if app.is_slash_popup_active() {
                let items = app.slash_items();
                if !items.is_empty() {
                    app.slash_popup_selected =
                        (app.slash_popup_selected + app.config.scroll_speed).min(items.len() - 1);
                }
            } else {
                app.scroll = app.scroll.saturating_add(app.config.scroll_speed);
            }
        }
        _ => return false,
    }
    true
}
