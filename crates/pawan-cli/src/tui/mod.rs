//! Module wiring: keep this file small; implementation lives in submodules.

//! Root module for the interactive terminal UI.
//!
//! Non-blocking TUI: agent runs on a spawned tokio task,
//! events stream back to the UI via mpsc channel.

mod fuzzy_search;

mod types;
mod app;
mod events;
mod input;
mod session_panel;
mod render;
mod slash_commands;

pub(crate) use slash_commands::default_slash_fuzzy_lines;

pub use app::{run_simple, run_tui};
