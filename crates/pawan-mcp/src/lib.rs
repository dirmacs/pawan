//! Pawan MCP Client Integration
//!
//! Connects to MCP servers (stdio or TCP) and bridges their tools
//! into pawan's ToolRegistry as dynamically-discovered tools.

mod bridge;
mod manager;

pub use bridge::McpToolBridge;
pub use manager::{McpManager, McpServerConfig};
