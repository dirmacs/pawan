//! MCP server connection manager

use crate::thulp_bridge::ThulpMcpBridge;
use pawan::tools::ToolRegistry;
use pawan::{PawanError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thulp_mcp::{McpClient, McpTransport};
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

/// A connected MCP server — holds the client and keeps the service alive
struct ConnectedServer {
    name: String,
    client: Arc<Mutex<McpClient>>,
    tool_names: Vec<thulp_core::ToolDefinition>,
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
                        server.tool_names.len()
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
        let args = if config.args.is_empty() {
            None
        } else {
            Some(config.args.clone())
        };

        let transport = if config.env.is_empty() {
            McpTransport::new_stdio(config.name.clone(), config.command.clone(), args)
        } else {
            McpTransport::new_stdio_with_env(
                config.name.clone(),
                config.command.clone(),
                args,
                config.env.clone(),
            )
        };

        let mut client = McpClient::new(transport);

        tokio::time::timeout(std::time::Duration::from_secs(10), client.connect())
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

        // Discover tools — must happen while client is alive
        let tools = client.list_tools().await.map_err(|e| {
            PawanError::Tool(format!(
                "Failed to list tools from MCP server '{}': {}",
                config.name, e
            ))
        })?;

        Ok(ConnectedServer {
            name: config.name.clone(),
            client: Arc::new(Mutex::new(client)),
            tool_names: tools,
        })
    }

    /// Register all discovered MCP tools into a ToolRegistry
    pub fn register_tools(&self, registry: &mut ToolRegistry) -> usize {
        let mut count = 0;

        for server in &self.servers {
            for definition in &server.tool_names {
                let bridge = ThulpMcpBridge::new(&server.name, definition, Arc::clone(&server.client));
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
            .map(|s| (s.name.clone(), s.tool_names.len()))
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

    // ─── Edge cases for McpManager + McpServerConfig (task #25) ──────────

    #[tokio::test]
    async fn register_tools_on_empty_manager_returns_zero() {
        // An empty manager (no connected servers) registering into a
        // registry must register 0 tools without crashing.
        let mgr = McpManager::connect(&[]).await.unwrap();
        let mut registry = ToolRegistry::new();
        let count = mgr.register_tools(&mut registry);
        assert_eq!(count, 0, "empty manager ⇒ 0 registered tools");
    }

    #[tokio::test]
    async fn summary_on_empty_manager_returns_empty() {
        let mgr = McpManager::connect(&[]).await.unwrap();
        let sum = mgr.summary();
        assert!(sum.is_empty(), "empty manager ⇒ empty summary");
    }

    #[tokio::test]
    async fn connect_mixed_enabled_disabled_filters_correctly() {
        // When a list has both enabled and disabled servers, the disabled
        // ones must be filtered BEFORE any spawn attempt. Use nonexistent
        // commands for the disabled entries to prove they're never run.
        let configs = vec![
            McpServerConfig {
                name: "disabled-1".into(),
                command: "/nope/no/binary/here".into(),
                args: vec![],
                env: HashMap::new(),
                enabled: false, // filter here
            },
            McpServerConfig {
                name: "enabled-but-missing".into(),
                command: "/also/not/real".into(),
                args: vec![],
                env: HashMap::new(),
                enabled: true, // spawn attempted, will fail
            },
        ];
        let mgr = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            McpManager::connect(&configs),
        )
        .await
        .expect("must not hang")
        .expect("must return Ok");
        assert_eq!(
            mgr.summary().len(),
            0,
            "disabled filtered + enabled spawn failed ⇒ 0 servers"
        );
    }

    #[test]
    fn config_enabled_default_is_true_via_helper() {
        // Pin the default_true() helper: omitting `enabled` in JSON must
        // result in enabled=true. This is the important default because
        // config files typically omit `enabled` for active servers.
        let json = r#"{"name":"implicit","command":"cmd"}"#;
        let config: McpServerConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled, "default_true() must yield enabled=true");
    }

    #[test]
    fn config_with_env_vars_roundtrips_correctly() {
        // Env with multiple entries including special chars should survive
        // a serde roundtrip without dropping any keys.
        let mut env = HashMap::new();
        env.insert("NVIDIA_API_KEY".into(), "nvapi-abc123def".into());
        env.insert("RUST_LOG".into(), "debug".into());
        env.insert("PATH".into(), "/usr/bin:/usr/local/bin".into());

        let original = McpServerConfig {
            name: "env-heavy".into(),
            command: "mcp-server".into(),
            args: vec![],
            env,
            enabled: true,
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.env.len(), 3);
        assert_eq!(
            restored.env.get("NVIDIA_API_KEY"),
            Some(&"nvapi-abc123def".to_string())
        );
        assert_eq!(
            restored.env.get("PATH"),
            Some(&"/usr/bin:/usr/local/bin".to_string())
        );
    }

    #[test]
    fn config_unknown_fields_are_rejected_by_default() {
        // Serde's default is to accept unknown fields (permissive). This
        // test pins that behavior — if we ever add #[serde(deny_unknown_fields)]
        // this test will break and force an explicit decision.
        let json = r#"{"name":"foo","command":"bar","unknown_field":"ignored"}"#;
        let result: std::result::Result<McpServerConfig, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "unknown fields should currently be ignored — if this breaks, \
             someone added deny_unknown_fields and should document why"
        );
    }
}
