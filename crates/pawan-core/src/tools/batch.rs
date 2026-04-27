//! Batch tool: execute multiple tool calls concurrently.
//!
//! Input: {"calls":[{"tool":"read_file","input":{...}}, ...]}
//!
//! Notes:
//! - Limits to 25 calls.
//! - Rejects nested batch calls.
//! - Accepts both nested (input/parameters) and flat parameter formats.

use crate::tools::{Tool, ToolRegistry};
use async_trait::async_trait;
use futures::future::join_all;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const MAX_BATCH_SIZE: usize = 25;

#[derive(Debug, Clone)]
struct BatchCall {
    tool: String,
    input: Value,
}

/// Flexible entry deserializer.
///
/// Accepts either:
/// - { tool: "read_file", input: { path: "..." } }
/// - { tool: "read_file", parameters: { path: "..." } }   (alias)
/// - { tool: "read_file", path: "..." }                  (flat)
/// - { tool: "read_file", input: {...}, path: "..." }    (merge; duplicates rejected)
impl<'de> Deserialize<'de> for BatchCall {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;

        impl<'de> Visitor<'de> for V {
            type Value = BatchCall;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a batch call with 'tool' and either 'input'/'parameters' or flat args")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<BatchCall, M::Error> {
                let mut tool: Option<String> = None;
                let mut nested: Option<Value> = None;
                let mut rest = serde_json::Map::new();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "tool" => tool = Some(map.next_value()?),
                        "input" | "parameters" => nested = Some(map.next_value()?),
                        _ => {
                            rest.insert(key, map.next_value()?);
                        }
                    }
                }

                let tool = tool.ok_or_else(|| de::Error::missing_field("tool"))?;
                let input = match nested {
                    Some(v) if rest.is_empty() => v,
                    Some(Value::Object(mut obj)) => {
                        for (k, v) in rest {
                            if obj.contains_key(&k) {
                                return Err(de::Error::custom(format_args!(
                                    "duplicate parameter '{k}' in both nested input and flat fields"
                                )));
                            }
                            obj.insert(k, v);
                        }
                        Value::Object(obj)
                    }
                    Some(_) => {
                        return Err(de::Error::custom(
                            "'input'/'parameters' must be an object when flat fields are also present",
                        ));
                    }
                    None if !rest.is_empty() => Value::Object(rest),
                    None => return Err(de::Error::missing_field("input")),
                };

                Ok(BatchCall { tool, input })
            }
        }

        deserializer.deserialize_map(V)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct BatchArgs {
    calls: Vec<BatchCall>,
}

#[derive(Clone)]
pub struct BatchTool {
    workspace_root: PathBuf,
}

impl BatchTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for BatchTool {
    fn name(&self) -> &str {
        "batch"
    }

    fn description(&self) -> &str {
        "Execute up to 25 tool calls concurrently and return an array of results."
    }

    fn mutating(&self) -> bool {
        false
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "calls": {
                    "type": "array",
                    "description": "Array of tool calls: {tool: string, input: object} or flat {tool: string, ...args}",
                    "items": { "type": "object" }
                }
            },
            "required": ["calls"]
        })
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let parsed: BatchArgs = serde_json::from_value(args)
            .map_err(|e| crate::PawanError::Tool(format!("invalid batch args: {e}")))?;

        if parsed.calls.is_empty() {
            return Ok(Value::Array(vec![]));
        }

        let active_len = parsed.calls.len().min(MAX_BATCH_SIZE);
        if parsed.calls.len() > MAX_BATCH_SIZE {
            tracing::warn!(
                total = parsed.calls.len(),
                used = active_len,
                limit = MAX_BATCH_SIZE,
                "batch: truncating calls over limit"
            );
        }

        let calls = parsed
            .calls
            .into_iter()
            .take(active_len)
            .collect::<Vec<_>>();
        let total = calls.len();
        let completed = Arc::new(AtomicUsize::new(0));

        let registry = Arc::new(ToolRegistry::with_defaults(self.workspace_root.clone()));

        let futs = calls.into_iter().map(|call| {
            let registry = Arc::clone(&registry);
            let completed = Arc::clone(&completed);
            async move {
                let out = if call.tool == "batch" {
                    json!({"error": "cannot nest batch inside batch"})
                } else {
                    match registry.execute(&call.tool, call.input).await {
                        Ok(v) => v,
                        Err(e) => json!({"error": e.to_string()}),
                    }
                };

                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::info!(completed = done, total, "BatchProgress");

                out
            }
        });

        let results = join_all(futs).await;
        Ok(Value::Array(results))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn batch_three_reads_returns_all_contents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "A").unwrap();
        std::fs::write(dir.path().join("b.txt"), "B").unwrap();
        std::fs::write(dir.path().join("c.txt"), "C").unwrap();

        let tool = BatchTool::new(dir.path().to_path_buf());
        let out = tool
            .execute(json!({
                "calls": [
                    {"tool": "read_file", "input": {"path": "a.txt"}},
                    {"tool": "read_file", "input": {"path": "b.txt"}},
                    {"tool": "read_file", "input": {"path": "c.txt"}}
                ]
            }))
            .await
            .unwrap();

        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert!(arr[0]
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("A"));
        assert!(arr[1]
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("B"));
        assert!(arr[2]
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("C"));
    }

    #[tokio::test]
    async fn batch_unknown_tool_is_partial_success() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.txt"), "OK").unwrap();

        let tool = BatchTool::new(dir.path().to_path_buf());
        let out = tool
            .execute(json!({
                "calls": [
                    {"tool": "read_file", "input": {"path": "ok.txt"}},
                    {"tool": "no_such_tool", "input": {}}
                ]
            }))
            .await
            .unwrap();

        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert!(arr[0].get("content").is_some());
        let err = arr[1].get("error").and_then(|v| v.as_str()).unwrap();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn nested_batch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let tool = BatchTool::new(dir.path().to_path_buf());

        let out = tool
            .execute(json!({
                "calls": [
                    {"tool": "batch", "input": {"calls": []}}
                ]
            }))
            .await
            .unwrap();

        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr[0].get("error").and_then(|v| v.as_str()).unwrap(),
            "cannot nest batch inside batch"
        );
    }

    #[tokio::test]
    async fn accepts_flat_call_format() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "X").unwrap();

        let tool = BatchTool::new(dir.path().to_path_buf());
        let out = tool
            .execute(json!({
                "calls": [
                    {"tool": "read_file", "path": "x.txt"}
                ]
            }))
            .await
            .unwrap();

        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert!(arr[0]
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("X"));
    }
}
