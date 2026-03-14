//! Bridge ares tools into pawan's ToolRegistry
//!
//! Feature-gated behind `ares` feature flag.

use super::Tool;
use crate::{PawanError, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Wraps an ares Tool as a pawan Tool
pub struct AresTool {
    inner: Arc<dyn ares::tools::registry::Tool>,
}

impl AresTool {
    pub fn new(tool: Arc<dyn ares::tools::registry::Tool>) -> Self {
        Self { inner: tool }
    }
}

#[async_trait]
impl Tool for AresTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        self.inner
            .execute(args)
            .await
            .map_err(|e| PawanError::Tool(format!("Ares tool error: {}", e)))
    }
}

/// Register all tools from an ares ToolRegistry into a pawan ToolRegistry
pub fn bridge_ares_tools(
    ares_registry: &ares::tools::registry::ToolRegistry,
    pawan_registry: &mut super::ToolRegistry,
) -> usize {
    let ares_defs = ares_registry.get_tool_definitions();
    let mut count = 0;

    for def in &ares_defs {
        if let Some(tool) = ares_registry.get(&def.name) {
            pawan_registry.register(Arc::new(AresTool::new(Arc::clone(tool))));
            count += 1;
        }
    }

    count
}
