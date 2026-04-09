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

/// Tool tier — controls which tools are sent to the LLM in the prompt.
/// All tools remain executable regardless of tier; tier only affects
/// which tool definitions appear in the LLM system prompt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolTier {
    /// Always sent to LLM — core file ops, bash, ast-grep
    Core,
    /// Sent to LLM by default — git, search, agent
    Standard,
    /// Only sent when explicitly requested or after first use — mise, tree, zoxide, sd, ripgrep, fd
    Extended,
}

/// Registry for managing tools with tiered visibility.
///
/// All tools are always executable. Tier controls which definitions
/// are sent to the LLM to save prompt tokens on simple tasks.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    tiers: HashMap<String, ToolTier>,
    /// Extended tools that have been activated (promoted to visible)
    activated: std::sync::Mutex<std::collections::HashSet<String>>,
    /// Precomputed lowercased "name description" for each tool (avoids per-query allocation)
    tool_text_cache: HashMap<String, String>,
}

impl ToolRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            tiers: HashMap::new(),
            activated: std::sync::Mutex::new(std::collections::HashSet::new()),
            tool_text_cache: HashMap::new(),
        }
    }

    /// Create a registry with all default tools, assigned to tiers.
    ///
    /// Core (always in LLM prompt): bash, read/write/edit, ast_grep, glob/grep
    /// Standard (in prompt by default): git, agents
    /// Extended (in prompt after first use): ripgrep, fd, sd, erd, mise, zoxide
    pub fn with_defaults(workspace_root: std::path::PathBuf) -> Self {
        let mut registry = Self::new();
        use ToolTier::*;

        // ── Core tier: always visible to LLM ──
        registry.register_with_tier(Arc::new(bash::BashTool::new(workspace_root.clone())), Core);
        registry.register_with_tier(Arc::new(file::ReadFileTool::new(workspace_root.clone())), Core);
        registry.register_with_tier(Arc::new(file::WriteFileTool::new(workspace_root.clone())), Core);
        registry.register_with_tier(Arc::new(edit::EditFileTool::new(workspace_root.clone())), Core);
        registry.register_with_tier(Arc::new(native::AstGrepTool::new(workspace_root.clone())), Core);
        registry.register_with_tier(Arc::new(native::GlobSearchTool::new(workspace_root.clone())), Core);
        registry.register_with_tier(Arc::new(native::GrepSearchTool::new(workspace_root.clone())), Core);

        // ── Standard tier: visible by default ──
        registry.register_with_tier(Arc::new(file::ListDirectoryTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(edit::EditFileLinesTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(edit::InsertAfterTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(edit::AppendFileTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitStatusTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitDiffTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitAddTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitCommitTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitLogTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitBlameTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitBranchTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitCheckoutTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(git::GitStashTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(agent::SpawnAgentsTool::new(workspace_root.clone())), Standard);
        registry.register_with_tier(Arc::new(agent::SpawnAgentTool::new(workspace_root.clone())), Standard);

        // ── Extended tier: hidden until first use ──
        registry.register_with_tier(Arc::new(native::RipgrepTool::new(workspace_root.clone())), Extended);
        registry.register_with_tier(Arc::new(native::FdTool::new(workspace_root.clone())), Extended);
        registry.register_with_tier(Arc::new(native::SdTool::new(workspace_root.clone())), Extended);
        registry.register_with_tier(Arc::new(native::ErdTool::new(workspace_root.clone())), Extended);
        registry.register_with_tier(Arc::new(native::MiseTool::new(workspace_root.clone())), Extended);
        registry.register_with_tier(Arc::new(native::ZoxideTool::new(workspace_root.clone())), Extended);
        registry.register_with_tier(Arc::new(native::LspTool::new(workspace_root)), Extended);

        registry
    }

    /// Register a tool at Standard tier (default)
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.register_with_tier(tool, ToolTier::Standard);
    }

    /// Register a tool at a specific tier
    pub fn register_with_tier(&mut self, tool: Arc<dyn Tool>, tier: ToolTier) {
        let name = tool.name().to_string();
        let cached_text = format!("{} {}", name, tool.description()).to_lowercase();
        self.tool_text_cache.insert(name.clone(), cached_text);
        self.tiers.insert(name.clone(), tier);
        self.tools.insert(name, tool);
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

    /// Get tool definitions visible to the LLM (Core + Standard + activated Extended).
    /// Extended tools become visible after first use or explicit activation.
    pub fn get_definitions(&self) -> Vec<ToolDefinition> {
        let activated = self.activated.lock().unwrap_or_else(|e| e.into_inner());
        self.tools.iter()
            .filter(|(name, _)| {
                match self.tiers.get(name.as_str()).copied().unwrap_or(ToolTier::Standard) {
                    ToolTier::Core | ToolTier::Standard => true,
                    ToolTier::Extended => activated.contains(name.as_str()),
                }
            })
            .map(|(_, tool)| tool.to_definition())
            .collect()
    }

    /// Dynamic tool selection — pick the most relevant tools for a given query.
    ///
    /// Returns Core tools (always) + top-K scored Standard/Extended tools based
    /// on keyword matching between the query and tool names/descriptions.
    /// This reduces 22+ tools to ~8-10, making MCP and extended tools visible.
    pub fn select_for_query(&self, query: &str, max_tools: usize) -> Vec<ToolDefinition> {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(i32, String)> = Vec::new();

        for name in self.tools.keys() {
            let tier = self.tiers.get(name.as_str()).copied().unwrap_or(ToolTier::Standard);

            // Core tools always included — skip scoring
            if tier == ToolTier::Core { continue; }

            // Score based on keyword overlap — use precomputed cache
            let tool_text = self.tool_text_cache.get(name.as_str())
                .map(|s| s.as_str())
                .unwrap_or("");
            let mut score: i32 = 0;

            for word in &query_words {
                if word.len() < 3 { continue; } // skip short words
                if tool_text.contains(word) { score += 2; }
            }

            // Bonus for keyword categories
            let search_words = ["search", "find", "web", "query", "look", "google", "bing", "wikipedia"];
            let git_words = ["git", "commit", "branch", "diff", "status", "log", "stash", "checkout", "blame"];
            let file_words = ["file", "read", "write", "edit", "append", "insert", "directory", "list"];
            let code_words = ["refactor", "rename", "replace", "ast", "lsp", "symbol", "function", "struct"];
            let tool_words = ["install", "mise", "tool", "runtime", "build", "test", "cargo"];

            for word in &query_words {
                if search_words.contains(word) && tool_text.contains("search") { score += 3; }
                if git_words.contains(word) && tool_text.contains("git") { score += 3; }
                if file_words.contains(word) && (tool_text.contains("file") || tool_text.contains("edit")) { score += 3; }
                if code_words.contains(word) && (tool_text.contains("ast") || tool_text.contains("lsp")) { score += 3; }
                if tool_words.contains(word) && tool_text.contains("mise") { score += 3; }
            }

            // MCP tools get a boost — especially web search when query mentions web/internet/online
            if name.starts_with("mcp_") {
                score += 1;
                if name.contains("search") || name.contains("web") {
                    let web_words = ["web", "search", "internet", "online", "find", "look up", "google"];
                    if web_words.iter().any(|w| query_lower.contains(w)) {
                        score += 10; // Strong boost — this is what the user wants
                    }
                }
            }

            // Activated extended tools get a boost (user has used them before)
            let activated = self.activated.lock().unwrap_or_else(|e| e.into_inner());
            if tier == ToolTier::Extended && activated.contains(name.as_str()) { score += 2; }

            if score > 0 || tier == ToolTier::Standard {
                scored.push((score, name.clone()));
            }
        }

        // Sort by score descending
        scored.sort_by(|a, b| b.0.cmp(&a.0));

        // Collect: all Core tools + top-K scored tools
        let mut result: Vec<ToolDefinition> = self.tools.iter()
            .filter(|(name, _)| {
                self.tiers.get(name.as_str()).copied().unwrap_or(ToolTier::Standard) == ToolTier::Core
            })
            .map(|(_, tool)| tool.to_definition())
            .collect();

        let remaining_slots = max_tools.saturating_sub(result.len());
        for (_, name) in scored.into_iter().take(remaining_slots) {
            if let Some(tool) = self.tools.get(&name) {
                result.push(tool.to_definition());
            }
        }

        result
    }

    /// Get ALL tool definitions regardless of tier (for tests and introspection)
    pub fn get_all_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.to_definition()).collect()
    }

    /// Activate an extended tool (makes it visible to the LLM)
    pub fn activate(&self, name: &str) {
        if self.tools.contains_key(name) {
            self.activated.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string());
        }
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
