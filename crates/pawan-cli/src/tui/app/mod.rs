//! `App` state, slash registry, and async entrypoints.

#![allow(unused_imports)]

mod async_ops;
mod constructors;
mod model_ops;
mod session_ops;
mod state;

// Re-export parent `tui` modules so submodules can keep using `super::types`, etc.
pub(crate) use super::fuzzy_search;
pub(crate) use super::model_catalog;
pub(crate) use super::queue_panel;
pub(crate) use super::status_bar;
pub(crate) use super::theme;
pub(crate) use super::types;

pub use async_ops::{run_simple, run_tui};
pub(crate) use state::{App, PermissionDialog, SlashCommand, SlashCommandRegistry};
