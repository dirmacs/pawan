//! Bridge MCP tools into pawan's Tool trait

use async_trait::async_trait;
use pawan::tools::Tool;
use pawan::{PawanError, Result};
use rmcp::model::{CallToolRequestParam, Content, Tool as McpTool};
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
        Self {
            tool_name: namespaced_name(server_name, &mcp_tool.name),
            mcp_tool_name: mcp_tool.name.to_string(),
            description: description_or_default(mcp_tool),
            schema: schema_as_value(mcp_tool),
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

        if result.is_error.unwrap_or(false) {
            let texts = extract_text_content(&result.content);
            return Err(PawanError::Tool(format!(
                "MCP tool error: {}",
                texts.join("\n")
            )));
        }

        let texts = extract_text_content(&result.content);
        Ok(format_text_results(texts))
    }
}

// Namespace an MCP tool under its server so tools from different servers don't collide.
fn namespaced_name(server_name: &str, mcp_tool_name: &str) -> String {
    format!("mcp_{}_{}", server_name, mcp_tool_name)
}

// Pull the description out of an McpTool, falling back to "MCP tool" if absent.
fn description_or_default(mcp_tool: &McpTool) -> String {
    mcp_tool
        .description
        .as_deref()
        .unwrap_or("MCP tool")
        .to_string()
}

// Serialize the input_schema as a JSON Value, falling back to {} on failure.
fn schema_as_value(mcp_tool: &McpTool) -> Value {
    serde_json::to_value(&*mcp_tool.input_schema).unwrap_or(Value::Object(Default::default()))
}

// Collect all text fragments from an MCP result's content array, preserving order.
fn extract_text_content(content: &[Content]) -> Vec<String> {
    content
        .iter()
        .filter_map(|c| {
            if let rmcp::model::RawContent::Text(t) = &c.raw {
                Some(t.text.clone())
            } else {
                None
            }
        })
        .collect()
}

// Format a "data" array from a search-style MCP response as human-readable plain text.
fn format_search_results(data: &[Value]) -> String {
    let mut output = format!("Found {} search results:\n\n", data.len());
    for (i, r) in data.iter().enumerate() {
        let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("?");
        let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("?");
        let desc = r.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let source = r
            .get("metadata")
            .and_then(|m| m.get("source"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        output.push_str(&format!("{}. {}\n", i + 1, title));
        output.push_str(&format!("   URL: {}\n", url));
        output.push_str(&format!("   Source: {}\n", source));
        if !desc.is_empty() && desc.len() < 200 {
            output.push_str(&format!("   {}\n", desc));
        }
        output.push('\n');
    }
    output
}

// Decide how to shape a list of text fragments returned by an MCP tool:
// - single text parseable as JSON with a `data` array -> flattened search-results string
// - single text parseable as other JSON -> TOON-encoded wrapper for token savings
// - single text not parseable as JSON -> raw string
// - multiple texts -> `{ "results": [...] }` wrapper
fn format_text_results(mut texts: Vec<String>) -> Value {
    if texts.len() == 1 {
        let text = texts.pop().unwrap();
        if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
            if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
                return Value::String(format_search_results(data));
            }
            return match toon_format::encode_default(&parsed) {
                Ok(toon_str) => serde_json::json!({
                    "format": "toon",
                    "content": toon_str,
                }),
                Err(_) => parsed,
            };
        }
        Value::String(text)
    } else {
        serde_json::json!({ "results": texts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{Annotated, JsonObject, RawContent, RawTextContent};
    use serde_json::json;

    fn make_mcp_tool(name: &str, description: Option<&'static str>, schema: JsonObject) -> McpTool {
        McpTool {
            name: name.to_string().into(),
            title: None,
            description: description.map(Into::into),
            input_schema: Arc::new(schema),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        }
    }

    fn text_content(text: &str) -> Content {
        Annotated::new(
            RawContent::Text(RawTextContent {
                text: text.to_string(),
                meta: None,
            }),
            None,
        )
    }

    fn image_content() -> Content {
        Annotated::new(
            RawContent::Image(rmcp::model::RawImageContent {
                data: "fake-base64".to_string(),
                mime_type: "image/png".to_string(),
                meta: None,
            }),
            None,
        )
    }

    #[test]
    fn namespaced_name_joins_server_and_tool() {
        assert_eq!(
            namespaced_name("daedra", "web_search"),
            "mcp_daedra_web_search"
        );
        assert_eq!(namespaced_name("eruka", "context.get"), "mcp_eruka_context.get");
    }

    #[test]
    fn description_or_default_returns_description_when_present() {
        let tool = make_mcp_tool("t", Some("a search tool"), JsonObject::new());
        assert_eq!(description_or_default(&tool), "a search tool");
    }

    #[test]
    fn description_or_default_falls_back_when_none() {
        let tool = make_mcp_tool("t", None, JsonObject::new());
        assert_eq!(description_or_default(&tool), "MCP tool");
    }

    #[test]
    fn schema_as_value_serializes_object_fields() {
        let mut schema = JsonObject::new();
        schema.insert("type".to_string(), json!("object"));
        schema.insert(
            "properties".to_string(),
            json!({ "query": { "type": "string" } }),
        );
        let tool = make_mcp_tool("t", None, schema);

        let value = schema_as_value(&tool);
        assert_eq!(value["type"], json!("object"));
        assert_eq!(value["properties"]["query"]["type"], json!("string"));
    }

    #[test]
    fn schema_as_value_returns_empty_object_for_empty_schema() {
        let tool = make_mcp_tool("t", None, JsonObject::new());
        let value = schema_as_value(&tool);
        assert!(value.is_object());
        assert_eq!(value.as_object().unwrap().len(), 0);
    }

    #[test]
    fn extract_text_content_returns_all_text_variants() {
        let content = vec![text_content("first"), text_content("second")];
        assert_eq!(
            extract_text_content(&content),
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn extract_text_content_skips_non_text_variants() {
        let content = vec![text_content("keep"), image_content(), text_content("me")];
        assert_eq!(
            extract_text_content(&content),
            vec!["keep".to_string(), "me".to_string()]
        );
    }

    #[test]
    fn extract_text_content_handles_empty_input() {
        assert!(extract_text_content(&[]).is_empty());
    }

    #[test]
    fn format_search_results_renders_title_url_source_and_index() {
        let data = vec![
            json!({
                "title": "Rust Book",
                "url": "https://doc.rust-lang.org/book",
                "description": "The canonical guide",
                "metadata": { "source": "rust-lang.org" }
            }),
            json!({
                "title": "Cargo Guide",
                "url": "https://doc.rust-lang.org/cargo",
                "metadata": { "source": "rust-lang.org" }
            }),
        ];
        let out = format_search_results(&data);

        assert!(out.starts_with("Found 2 search results:\n\n"));
        assert!(out.contains("1. Rust Book"));
        assert!(out.contains("   URL: https://doc.rust-lang.org/book"));
        assert!(out.contains("   Source: rust-lang.org"));
        assert!(out.contains("   The canonical guide"));
        assert!(out.contains("2. Cargo Guide"));
    }

    #[test]
    fn format_search_results_omits_overlong_description() {
        let long_desc = "x".repeat(250);
        let data = vec![json!({
            "title": "Big",
            "url": "https://ex.com",
            "description": long_desc,
            "metadata": { "source": "ex.com" }
        })];
        let out = format_search_results(&data);
        assert!(out.contains("1. Big"));
        assert!(!out.contains(&"x".repeat(250)));
    }

    #[test]
    fn format_search_results_handles_missing_fields_with_placeholders() {
        let data = vec![json!({})];
        let out = format_search_results(&data);
        assert!(out.contains("1. ?"));
        assert!(out.contains("   URL: ?"));
        assert!(out.contains("   Source: unknown"));
    }

    #[test]
    fn format_text_results_formats_search_data_as_plain_text() {
        let body = json!({
            "data": [
                { "title": "A", "url": "https://a.com", "metadata": { "source": "a.com" } }
            ]
        })
        .to_string();

        let out = format_text_results(vec![body]);
        let s = out.as_str().expect("expected plain string output");
        assert!(s.contains("Found 1 search results:"));
        assert!(s.contains("1. A"));
    }

    #[test]
    fn format_text_results_encodes_non_search_json_as_toon() {
        let body = json!({ "name": "pawan", "version": "0.3.1" }).to_string();
        let out = format_text_results(vec![body]);
        assert_eq!(out["format"], json!("toon"));
        let content = out["content"].as_str().expect("toon content should be a string");
        assert!(content.contains("pawan"));
        assert!(content.contains("0.3.1"));
    }

    #[test]
    fn format_text_results_wraps_single_non_json_text_as_string() {
        let out = format_text_results(vec!["hello world".to_string()]);
        assert_eq!(out, Value::String("hello world".to_string()));
    }

    #[test]
    fn format_text_results_joins_multiple_texts_in_results_object() {
        let out = format_text_results(vec!["one".to_string(), "two".to_string()]);
        assert_eq!(out["results"], json!(["one", "two"]));
    }

    #[test]
    fn format_text_results_handles_empty_input_as_empty_results() {
        let out = format_text_results(Vec::new());
        assert_eq!(out["results"], json!([]));
    }
}
