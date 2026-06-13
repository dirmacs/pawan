//! Crossterm event handling for `App`.

mod default;
mod fuzzy;
mod global;
mod irc;
mod model_picker;
mod mouse;
mod permission;
mod welcome;

use crossterm::event::{Event, KeyEvent, KeyEventKind};

use super::app::App;
use default::{handle_panel_key, handle_search_mode_key, handle_session_browser_key};
use fuzzy::handle_fuzzy_search_key;
use global::handle_global_key;
use irc::handle_irc_compose_key;
use model_picker::handle_model_picker_key;
use mouse::handle_mouse_event;
use permission::handle_permission_dialog;
use welcome::{handle_help_overlay_key, handle_welcome_key};

impl App<'_> {
    pub(crate) fn handle_event(&mut self, event: Event) {
        if handle_permission_dialog(self, &event) {
            return;
        }

        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key_event(key),
            Event::Key(_) => {}
            Event::Mouse(mouse) => {
                let _ = handle_mouse_event(self, mouse);
            }
            _ => {}
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        if handle_global_key(self, &key) {
            return;
        }
        if handle_welcome_key(self, &key) {
            return;
        }
        if handle_help_overlay_key(self, &key) {
            return;
        }
        if handle_fuzzy_search_key(self, &key) {
            return;
        }
        if handle_search_mode_key(self, &key) {
            return;
        }
        if handle_model_picker_key(self, &key) {
            return;
        }
        if handle_irc_compose_key(self, &key) {
            return;
        }
        if handle_session_browser_key(self, &key) {
            return;
        }
        handle_panel_key(self, &key);
    }
}
