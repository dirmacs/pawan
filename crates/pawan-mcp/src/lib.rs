//! Pawan MCP Integration — Client + Server
//!
//! **Client:** Connects to MCP servers and bridges their tools into pawan's ToolRegistry.
//! **Server:** Exposes pawan's agent as MCP tools (pawan_run, pawan_task, pawan_heal).

mod manager;
pub mod server;
mod thulp_bridge;

pub use manager::{McpManager, McpServerConfig};
pub use server::PawanServer;
pub use thulp_bridge::ThulpMcpBridge;

/// Backwards-compatible alias — was previously the rmcp-backed bridge.
/// Now points to ThulpMcpBridge (thulp-mcp backed).
pub use thulp_bridge::ThulpMcpBridge as McpToolBridge;
