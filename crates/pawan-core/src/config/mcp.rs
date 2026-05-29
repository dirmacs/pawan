use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::defaults::default_true;

/// Configuration for an MCP server in pawan.toml
///
/// This struct represents configuration for an MCP (Multi-Cursor Protocol) server
/// that can be managed by Pawan. It includes the command to run, arguments,
/// environment variables, and whether the server is enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// Command to run
    pub command: String,
    /// Command arguments
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether this server is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
}
