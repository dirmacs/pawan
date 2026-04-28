//! Bridge thulp-mcp tools into pawan's `Tool` trait.
//!
//! Current MCP bridge implementation backed by `thulp-mcp` / `rs-utcp`.
//! It exposes MCP server tools through the same pawan `Tool` trait surface used
//! by native tools, so the registry and agent loop can dispatch them uniformly.
//!
//! The bridge keeps namespacing, fallback descriptions, schema conversion, and
//! text/result formatting as pure helpers so they can be unit tested without a
//! live MCP peer.
//!
//! Construction takes a [`thulp_core::ToolDefinition`] — exactly the form
//! returned by [`thulp_mcp::McpClient::list_tools`]. After Phase 4 (#13)
//! pawan's `ToolDefinition` is also re-exported from `thulp-core`, so the
//! type flows end-to-end without conversion.

use async_trait::async_trait;
use pawan::tools::Tool;
use pawan::{PawanError, Result};
use serde_json::Value;
use std::sync::Arc;
use thulp_core::ToolDefinition as ThulpToolDefinition;
use thulp_mcp::McpClient;
use tokio::sync::Mutex;

/// Wraps a thulp-mcp tool as a pawan `Tool`, mirroring [`crate::McpToolBridge`]
/// but dispatching through `thulp_mcp::McpClient::call_tool`.
pub struct ThulpMcpBridge {
    /// Namespaced tool name (e.g. "mcp_daedra_web_search") — what pawan's
    /// registry and the LLM see.
    tool_name: String,
    /// Original tool name on the MCP server side, used for the actual call.
    mcp_tool_name: String,
    /// Human-readable description from the server's tool definition.
    description: String,
    /// JSON schema for parameters, computed once from the typed thulp form.
    schema: Value,
    /// Shared client handle. `Arc<Mutex<>>` because the same client may be
    /// used to dispatch multiple concurrent tool calls and the underlying
    /// transport needs serialized access.
    client: Arc<Mutex<McpClient>>,
}

impl ThulpMcpBridge {
    /// Construct a bridge from a server name + a thulp-core ToolDefinition
    /// (the form returned by [`thulp_mcp::McpClient::list_tools`]) + a shared
    /// client handle.
    pub fn new(
        server_name: &str,
        definition: &ThulpToolDefinition,
        client: Arc<Mutex<McpClient>>,
    ) -> Self {
        let description = if definition.description.is_empty() {
            "MCP tool".to_string()
        } else {
            definition.description.clone()
        };
        Self {
            tool_name: namespaced_name(server_name, &definition.name),
            mcp_tool_name: definition.name.clone(),
            description,
            schema: definition.to_mcp_input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ThulpMcpBridge {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let client = self.client.lock().await;
        let result = client
            .call_tool(&self.mcp_tool_name, args)
            .await
            .map_err(|e| PawanError::Tool(format!("thulp-mcp tool call failed: {}", e)))?;

        if !result.success {
            return Err(PawanError::Tool(format!(
                "thulp-mcp tool error: {}",
                result
                    .error
                    .unwrap_or_else(|| "(no error message)".to_string())
            )));
        }

        Ok(result.data.unwrap_or(Value::Null))
    }
}

// Namespace an MCP tool under its server so tools from different servers
// don't collide. Keep the stable `mcp_<server>_<tool>` naming scheme.
fn namespaced_name(server_name: &str, mcp_tool_name: &str) -> String {
    format!("mcp_{}_{}", server_name, mcp_tool_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use thulp_core::Parameter;
    use thulp_mcp::McpTransport;

    fn fake_definition() -> ThulpToolDefinition {
        ThulpToolDefinition::builder("web_search")
            .description("Search the web")
            .parameter(Parameter::required_string("query"))
            .build()
    }

    fn fake_client() -> Arc<Mutex<McpClient>> {
        // McpClient::new only configures transport — no I/O happens until
        // connect() is called, so this is safe in unit tests.
        let transport =
            McpTransport::new_http("test".to_string(), "http://localhost:0".to_string());
        Arc::new(Mutex::new(McpClient::new(transport)))
    }

    #[test]
    fn namespaced_name_joins_server_and_tool() {
        assert_eq!(
            namespaced_name("daedra", "web_search"),
            "mcp_daedra_web_search"
        );
        assert_eq!(
            namespaced_name("eruka", "context.get"),
            "mcp_eruka_context.get"
        );
    }

    #[test]
    fn bridge_namespaces_tool_under_server() {
        let bridge = ThulpMcpBridge::new("daedra", &fake_definition(), fake_client());
        assert_eq!(bridge.name(), "mcp_daedra_web_search");
    }

    #[test]
    fn bridge_carries_description_and_schema_from_definition() {
        let bridge = ThulpMcpBridge::new("daedra", &fake_definition(), fake_client());
        assert_eq!(bridge.description(), "Search the web");

        let schema = bridge.parameters_schema();
        assert_eq!(schema["type"], "object");
        // The required string parameter from the typed definition should
        // round-trip through to_mcp_input_schema().
        assert!(schema["properties"]["query"].is_object());
        assert_eq!(schema["properties"]["query"]["type"], "string");
        assert_eq!(schema["required"][0], "query");
    }

    #[test]
    fn bridge_falls_back_to_default_description_when_empty() {
        let def = ThulpToolDefinition::builder("ping").build();
        let bridge = ThulpMcpBridge::new("test", &def, fake_client());
        assert_eq!(bridge.description(), "MCP tool");
    }

    #[test]
    fn bridge_uses_inner_tool_name_not_namespaced_for_dispatch() {
        let bridge = ThulpMcpBridge::new("daedra", &fake_definition(), fake_client());
        // Public Tool::name returns the namespaced form (LLM-facing), but
        // the underlying mcp_tool_name field carries the un-namespaced name
        // that gets sent to the server. Verify they differ as expected.
        assert_eq!(bridge.name(), "mcp_daedra_web_search");
        assert_eq!(bridge.mcp_tool_name, "web_search");
    }
}
