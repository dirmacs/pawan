//! Module wiring: keep this file small; implementation lives in submodules.

//! Root module for the interactive terminal UI.
//!
//! Non-blocking TUI: agent runs on a spawned tokio task,
//! events stream back to the UI via mpsc channel.

mod fuzzy_search;
mod theme;
mod splash;

mod layout;
mod highlight;

pub mod status_bar;
pub mod scrollbar;
pub mod activity_panel;
pub mod queue_panel;
pub mod tool_display;

mod app;
mod events;
mod input;
mod render;
mod session_panel;
mod slash_commands;
mod types;

pub(crate) use slash_commands::default_slash_fuzzy_lines;

pub use app::{run_simple, run_tui};
