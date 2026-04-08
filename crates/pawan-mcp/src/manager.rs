//! MCP server connection manager

use crate::bridge::McpToolBridge;
use pawan::tools::ToolRegistry;
use pawan::{PawanError, Result};
use rmcp::model::Tool as McpTool;
use rmcp::service::{Peer, RoleClient, ServiceExt};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;

/// Configuration for an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Server name (used for tool namespacing)
    pub name: String,
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

fn default_true() -> bool {
    true
}

/// A connected MCP server — holds the peer and keeps the service alive
struct ConnectedServer {
    name: String,
    peer: Arc<Mutex<Peer<RoleClient>>>,
    tools: Vec<McpTool>,
    /// Keep-alive handle — dropping this kills the connection
    _keepalive: tokio::task::JoinHandle<()>,
}

/// Manages connections to MCP servers
pub struct McpManager {
    servers: Vec<ConnectedServer>,
}

impl McpManager {
    /// Connect to all configured MCP servers in parallel and discover their tools
    pub async fn connect(configs: &[McpServerConfig]) -> Result<Self> {
        let enabled: Vec<_> = configs.iter().filter(|c| {
            if !c.enabled {
                tracing::debug!("Skipping disabled MCP server: {}", c.name);
            }
            c.enabled
        }).collect();

        // Connect to all servers concurrently
        let results = futures::future::join_all(
            enabled.iter().map(|config| Self::connect_one(config))
        ).await;

        let mut servers = Vec::new();
        for result in results {
            match result {
                Ok(server) => {
                    tracing::info!(
                        "Connected to MCP server '{}': {} tools",
                        server.name,
                        server.tools.len()
                    );
                    servers.push(server);
                }
                Err(e) => {
                    tracing::warn!("Failed to connect to MCP server: {}", e);
                }
            }
        }

        Ok(Self { servers })
    }

    async fn connect_one(config: &McpServerConfig) -> Result<ConnectedServer> {
        let args = config.args.clone();
        let env = config.env.clone();

        let transport = TokioChildProcess::new(Command::new(&config.command).configure(|cmd| {
            cmd.args(&args);
            for (k, v) in &env {
                cmd.env(k, v);
            }
        }))
        .map_err(|e| {
            PawanError::Tool(format!(
                "Failed to spawn MCP server '{}': {}",
                config.name, e
            ))
        })?;

        let service = tokio::time::timeout(std::time::Duration::from_secs(10), ().serve(transport))
            .await
            .map_err(|_| {
                PawanError::Tool(format!(
                    "Timeout connecting to MCP server '{}'",
                    config.name
                ))
            })?
            .map_err(|e| {
                PawanError::Tool(format!(
                    "Failed to connect to MCP server '{}': {}",
                    config.name, e
                ))
            })?;

        // Discover tools — must happen while service is alive
        let tools = service.list_all_tools().await.map_err(|e| {
            PawanError::Tool(format!(
                "Failed to list tools from MCP server '{}': {}",
                config.name, e
            ))
        })?;

        let peer = service.peer().clone();

        // Keep service alive in background
        let keepalive = tokio::spawn(async move {
            let _ = service.waiting().await;
        });

        Ok(ConnectedServer {
            name: config.name.clone(),
            peer: Arc::new(Mutex::new(peer)),
            tools,
            _keepalive: keepalive,
        })
    }

    /// Register all discovered MCP tools into a ToolRegistry
    pub fn register_tools(&self, registry: &mut ToolRegistry) -> usize {
        let mut count = 0;

        for server in &self.servers {
            for tool in &server.tools {
                let bridge = McpToolBridge::new(&server.name, tool, Arc::clone(&server.peer));
                registry.register(Arc::new(bridge));
                count += 1;
            }
        }

        count
    }

    /// Get summary of connected servers and their tool counts
    pub fn summary(&self) -> Vec<(String, usize)> {
        self.servers
            .iter()
            .map(|s| (s.name.clone(), s.tools.len()))
            .collect()
    }
}
