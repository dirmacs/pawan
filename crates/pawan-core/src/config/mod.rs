//! Configuration for Pawan
//!
//! Pawan can be configured via:
//! - `pawan.toml` in the current directory
//! - `[pawan]` section in `ares.toml`
//! - Environment variables
//! - Command line arguments

mod defaults;
mod healing;
mod mcp;
mod pawan;
mod permissions;
mod prompt;
mod provider;
mod routing;
mod target;
mod tui;

pub mod migration;

#[cfg(test)]
mod tests;

#[cfg(test)]
use defaults::{default_config_version, default_tool_idle_timeout};
pub use healing::HealingConfig;
pub use mcp::McpServerEntry;
pub use migration::{migrate_to_latest, save_config, MigrationResult};
pub use pawan::PawanConfig;
pub use permissions::ToolPermission;
pub use prompt::DEFAULT_SYSTEM_PROMPT;
pub use provider::LlmProvider;
pub use routing::{CloudConfig, ModelRouting};
pub use target::TargetConfig;
pub use tui::TuiConfig;
