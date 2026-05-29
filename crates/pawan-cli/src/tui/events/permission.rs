//! Permission dialog key handling.

use crossterm::event::{Event, KeyCode};

use super::super::app::App;

/// Handles keys while the permission dialog is open. Returns `true` when the dialog is active.
pub(crate) fn handle_permission_dialog(app: &mut App<'_>, event: &Event) -> bool {
    if app.permission_dialog.is_none() {
        return false;
    }

    if let Event::Key(key) = event {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(mut dialog) = app.permission_dialog.take() {
                    if let Some(tx) = dialog.respond.take() {
                        let _ = tx.send(true);
                    }
                    app.status = format!("Allowed: {}", dialog.tool_name);
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if let Some(mut dialog) = app.permission_dialog.take() {
                    if let Some(tx) = dialog.respond.take() {
                        let _ = tx.send(true);
                    }
                    app.status = format!("Allowed (all): {}", dialog.tool_name);
                    app.auto_approve_tools = true;
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                if let Some(mut dialog) = app.permission_dialog.take() {
                    if let Some(tx) = dialog.respond.take() {
                        let _ = tx.send(false);
                    }
                    app.status = format!("Denied: {}", dialog.tool_name);
                }
            }
            _ => {}
        }
    }
    true
}
