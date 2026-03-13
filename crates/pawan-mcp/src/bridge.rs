//! Bridge MCP tools into pawan's Tool trait

use async_trait::async_trait;
use pawan::tools::Tool;
use pawan::{PawanError, Result};
use rmcp::model::{CallToolRequestParam, Tool as McpTool};
use rmcp::service::Peer;
use rmcp::service::RoleClient;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Wraps an MCP tool as a pawan Tool
pub struct McpToolBridge {
    /// Namespaced tool name (e.g., "mcp_daedra_web_search")
    tool_name: String,
    /// Original MCP tool name
    mcp_tool_name: String,
    /// Tool description
    description: String,
    /// JSON schema for parameters
    schema: Value,
    /// Reference to the MCP server peer
    peer: Arc<Mutex<Peer<RoleClient>>>,
}

impl McpToolBridge {
    pub fn new(server_name: &str, mcp_tool: &McpTool, peer: Arc<Mutex<Peer<RoleClient>>>) -> Self {
        let tool_name = format!("mcp_{}_{}", server_name, mcp_tool.name);
        let description = mcp_tool
            .description
            .as_deref()
            .unwrap_or("MCP tool")
            .to_string();
        let schema = serde_json::to_value(&*mcp_tool.input_schema)
            .unwrap_or(Value::Object(Default::default()));

        Self {
            tool_name,
            mcp_tool_name: mcp_tool.name.to_string(),
            description,
            schema,
            peer,
        }
    }
}

#[async_trait]
impl Tool for McpToolBridge {
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
        let arguments = args.as_object().cloned();

        let peer = self.peer.lock().await;
        let result = peer
            .call_tool(CallToolRequestParam {
                name: self.mcp_tool_name.clone().into(),
                arguments,
            })
            .await
            .map_err(|e| PawanError::Tool(format!("MCP tool call failed: {}", e)))?;

        // Convert MCP result to JSON value
        if result.is_error.unwrap_or(false) {
            let error_text = result
                .content
                .iter()
                .filter_map(|c| {
                    if let rmcp::model::RawContent::Text(t) = &c.raw {
                        Some(t.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Err(PawanError::Tool(format!("MCP tool error: {}", error_text)));
        }

        // Extract text content
        let mut texts = Vec::new();
        for content in &result.content {
            if let rmcp::model::RawContent::Text(t) = &content.raw {
                texts.push(t.text.clone());
            }
        }

        if texts.len() == 1 {
            // Try to parse as JSON, fallback to string
            if let Ok(parsed) = serde_json::from_str::<Value>(&texts[0]) {
                Ok(parsed)
            } else {
                Ok(Value::String(texts[0].clone()))
            }
        } else {
            Ok(serde_json::json!({
                "results": texts,
            }))
        }
    }
}
