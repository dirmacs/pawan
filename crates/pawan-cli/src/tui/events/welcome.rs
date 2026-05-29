//! Welcome screen and help overlay key handling.

use crossterm::event::KeyEvent;

use super::super::app::App;

/// Dismisses the welcome screen on any key. Returns `true` when the welcome was showing.
pub(crate) fn handle_welcome_key(app: &mut App<'_>, _key: &KeyEvent) -> bool {
    if app.show_welcome {
        app.show_welcome = false;
        return true;
    }
    false
}

/// Dismisses the help overlay on any key. Returns `true` when the overlay was showing.
pub(crate) fn handle_help_overlay_key(app: &mut App<'_>, _key: &KeyEvent) -> bool {
    if app.help_overlay {
        app.help_overlay = false;
        return true;
    }
    false
}
