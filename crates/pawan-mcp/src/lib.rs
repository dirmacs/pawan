//! Pawan MCP Integration — Client + Server
//!
//! **Client:** Connects to MCP servers and bridges their tools into pawan's ToolRegistry.
//! **Server:** Exposes pawan's agent as MCP tools (pawan_run, pawan_task, pawan_heal).

mod bridge;
mod manager;
pub mod server;
mod thulp_bridge;

pub use bridge::McpToolBridge;
pub use manager::{McpManager, McpServerConfig};
pub use server::PawanServer;
pub use thulp_bridge::ThulpMcpBridge;
