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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_serde_roundtrip_preserves_fields() {
        let original = McpServerConfig {
            name: "daedra".into(),
            command: "daedra-mcp".into(),
            args: vec!["--port".into(), "9999".into()],
            env: {
                let mut m = HashMap::new();
                m.insert("API_KEY".into(), "secret".into());
                m
            },
            enabled: true,
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "daedra");
        assert_eq!(restored.command, "daedra-mcp");
        assert_eq!(restored.args, vec!["--port", "9999"]);
        assert_eq!(restored.env.get("API_KEY"), Some(&"secret".to_string()));
        assert!(restored.enabled);
    }

    #[test]
    fn config_json_minimal_fills_defaults() {
        // A minimal config (name + command only) should get defaults for
        // args=[], env={}, enabled=true. This pins the #[serde(default)]
        // and default_true behavior. Using JSON since pawan-mcp does not
        // depend on the toml crate.
        let json = r#"{"name":"minimal","command":"mcp-server"}"#;
        let config: McpServerConfig = serde_json::from_str(json).expect("should parse");
        assert_eq!(config.name, "minimal");
        assert_eq!(config.command, "mcp-server");
        assert!(config.args.is_empty(), "args default is empty vec");
        assert!(config.env.is_empty(), "env default is empty map");
        assert!(config.enabled, "enabled default is true (default_true helper)");
    }

    #[test]
    fn config_enabled_explicit_false_is_respected() {
        let json = r#"{"name":"offline","command":"some-cmd","enabled":false}"#;
        let config: McpServerConfig = serde_json::from_str(json).unwrap();
        assert!(!config.enabled);
    }

    #[tokio::test]
    async fn connect_empty_list_returns_empty_manager() {
        let mgr = McpManager::connect(&[]).await.unwrap();
        assert_eq!(mgr.summary().len(), 0, "no configs ⇒ no servers");
    }

    #[tokio::test]
    async fn connect_all_disabled_skips_them_without_spawning() {
        // An all-disabled list must never spawn subprocesses — use a
        // nonexistent command to prove the disabled filter kicks in first.
        let configs = vec![
            McpServerConfig {
                name: "a".into(),
                command: "/definitely/does/not/exist/binary_xyz".into(),
                args: vec![],
                env: HashMap::new(),
                enabled: false,
            },
            McpServerConfig {
                name: "b".into(),
                command: "/also/not/real".into(),
                args: vec![],
                env: HashMap::new(),
                enabled: false,
            },
        ];
        let mgr = McpManager::connect(&configs).await.unwrap();
        assert_eq!(mgr.summary().len(), 0);
    }

    #[tokio::test]
    async fn connect_failed_server_is_non_fatal() {
        // Spawning a nonexistent command must not crash McpManager::connect —
        // the failure should be logged and the server filtered out.
        let configs = vec![McpServerConfig {
            name: "bogus".into(),
            command: "/definitely/does/not/exist/nope_nope".into(),
            args: vec![],
            env: HashMap::new(),
            enabled: true,
        }];
        // Cap at 15s so a hanging spawn attempt would fail the test.
        let mgr = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            McpManager::connect(&configs),
        )
        .await
        .expect("connect must not hang on bad command")
        .expect("connect must return Ok even when individual spawns fail");
        assert_eq!(mgr.summary().len(), 0, "failed spawn ⇒ no registered server");
    }
}
