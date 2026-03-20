//! Tools for Pawan agent
//!
//! This module provides all the tools that Pawan can use to interact with
//! the filesystem, execute commands, and perform coding operations.
//!
//! Native tools (rg, fd, sd, erd, mise) are thin wrappers over CLI binaries
//! that provide structured JSON output and auto-install hints.

pub mod agent;
pub mod bash;
pub mod edit;
#[cfg(test)]
mod edit_tests;
pub mod file;
pub mod git;
pub mod native;
pub mod search;

#[cfg(feature = "ares")]
pub mod ares_bridge;

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Tool definition for LLM
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// JSON Schema for parameters
    pub parameters: Value,
}

/// Trait for implementing tools
#[async_trait]
pub trait Tool: Send + Sync {
    /// Returns the unique name of this tool
    fn name(&self) -> &str;

    /// Returns a description of what this tool does
    fn description(&self) -> &str;

    /// Returns the JSON schema for this tool's parameters
    fn parameters_schema(&self) -> Value;

    /// Executes the tool with the given arguments
    async fn execute(&self, args: Value) -> crate::Result<Value>;

    /// Convert to ToolDefinition
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

/// Registry for managing tools
/// Registry for managing tools
///
/// This struct maintains a collection of tools that the Pawan agent can use.
/// It provides methods for registering tools, retrieving them by name,
/// and getting tool definitions for the LLM.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Create a registry with all default tools
    pub fn with_defaults(workspace_root: std::path::PathBuf) -> Self {
        let mut registry = Self::new();

        // File tools
        registry.register(Arc::new(file::ReadFileTool::new(workspace_root.clone())));
        registry.register(Arc::new(file::WriteFileTool::new(workspace_root.clone())));
        registry.register(Arc::new(file::ListDirectoryTool::new(
            workspace_root.clone(),
        )));

        // Edit tools (anchor mode is most reliable, then insert_after, then line numbers)
        registry.register(Arc::new(edit::EditFileLinesTool::new(workspace_root.clone())));
        registry.register(Arc::new(edit::EditFileTool::new(workspace_root.clone())));
        registry.register(Arc::new(edit::InsertAfterTool::new(workspace_root.clone())));
        registry.register(Arc::new(edit::AppendFileTool::new(workspace_root.clone())));

        // Search tools (native rg/fd wrappers override old search module)
        registry.register(Arc::new(native::GlobSearchTool::new(workspace_root.clone())));
        registry.register(Arc::new(native::GrepSearchTool::new(workspace_root.clone())));

        // Bash tool
        registry.register(Arc::new(bash::BashTool::new(workspace_root.clone())));

        // Git tools
        registry.register(Arc::new(git::GitStatusTool::new(workspace_root.clone())));
        registry.register(Arc::new(git::GitDiffTool::new(workspace_root.clone())));
        registry.register(Arc::new(git::GitAddTool::new(workspace_root.clone())));
        registry.register(Arc::new(git::GitCommitTool::new(workspace_root.clone())));
        registry.register(Arc::new(git::GitLogTool::new(workspace_root.clone())));
        registry.register(Arc::new(git::GitBlameTool::new(workspace_root.clone())));
        registry.register(Arc::new(git::GitBranchTool::new(workspace_root.clone())));
        registry.register(Arc::new(git::GitCheckoutTool::new(workspace_root.clone())));
        registry.register(Arc::new(git::GitStashTool::new(workspace_root.clone())));

        // Sub-agent tools
        registry.register(Arc::new(agent::SpawnAgentsTool::new(workspace_root.clone())));
        registry.register(Arc::new(agent::SpawnAgentTool::new(workspace_root.clone())));

        // Native CLI tools (rg, fd, sd, erd, mise)
        registry.register(Arc::new(native::RipgrepTool::new(workspace_root.clone())));
        registry.register(Arc::new(native::FdTool::new(workspace_root.clone())));
        registry.register(Arc::new(native::SdTool::new(workspace_root.clone())));
        registry.register(Arc::new(native::ErdTool::new(workspace_root.clone())));
        registry.register(Arc::new(native::MiseTool::new(workspace_root.clone())));
        registry.register(Arc::new(native::ZoxideTool::new(workspace_root.clone())));

        // AST-level code manipulation
        registry.register(Arc::new(native::AstGrepTool::new(workspace_root)));

        registry
    }

    /// Register a tool
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Check if a tool exists
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Execute a tool by name
    pub async fn execute(&self, name: &str, args: Value) -> crate::Result<Value> {
        match self.tools.get(name) {
            Some(tool) => tool.execute(args).await,
            None => Err(crate::PawanError::NotFound(format!(
                "Tool not found: {}",
                name
            ))),
        }
    }

    /// Get all tool definitions
    pub fn get_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.to_definition()).collect()
    }

    /// Get tool names
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
