//! Reusable skill workflows via thulp-skills
//!
//! Provides a bridge from pawan's `ToolRegistry` to thulp-skills' `Transport`
//! trait, allowing pawan to define and execute multi-step skill workflows
//! with timeout, retry, and context propagation.
//!
//! # Architecture
//!
//! - [`PawanTransport`] — implements [`thulp_core::Transport`] by delegating
//!   to a pawan [`ToolRegistry`]. This allows any thulp-skills executor to
//!   call pawan's native tools (bash, file ops, git, search, etc.).
//! - [`built_in_skills`] — curated collection of reusable skills like
//!   "format-and-commit", "test-and-fix", "review-diff".
//!
//! # Example
//!
//! ```ignore
//! use pawan::skills::{built_in_skills, PawanTransport};
//! use pawan::tools::ToolRegistry;
//! use std::sync::Arc;
//! use thulp_skills::{DefaultSkillExecutor, ExecutionContext, SkillExecutor};
//!
//! let registry = Arc::new(ToolRegistry::with_defaults(".".into()));
//! let transport = PawanTransport::new(registry);
//! let skill = built_in_skills::format_and_commit();
//!
//! let executor = DefaultSkillExecutor::new(Box::new(transport));
//! let mut ctx = ExecutionContext::new()
//!     .with_input("message", serde_json::json!("update docs"));
//! let result = executor.execute(&skill, &mut ctx).await?;
//! ```

use crate::tools::ToolRegistry;
use async_trait::async_trait;
use std::sync::Arc;
use thulp_core::{Result as ThulpResult, ToolCall, ToolDefinition, ToolResult, Transport};

/// Transport bridge that delegates thulp-skills tool calls to a pawan
/// [`ToolRegistry`]. Stateless, always "connected" — pawan tools run in-process.
pub struct PawanTransport {
    registry: Arc<ToolRegistry>,
}

impl PawanTransport {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Transport for PawanTransport {
    async fn connect(&mut self) -> ThulpResult<()> {
        // No-op — pawan tools are always available in-process
        Ok(())
    }

    async fn disconnect(&mut self) -> ThulpResult<()> {
        Ok(())
    }

    fn is_connected(&self) -> bool {
        true
    }

    async fn list_tools(&self) -> ThulpResult<Vec<ToolDefinition>> {
        // Gather thulp_definition() from every registered tool (not just
        // the visible set — skill workflows may need access to all tools).
        let defs: Vec<ToolDefinition> = self
            .registry
            .get_all_definitions()
            .iter()
            .filter_map(|pawan_def| {
                // Look up the actual tool to get its rich thulp definition
                self.registry
                    .get(&pawan_def.name)
                    .map(|tool| tool.thulp_definition())
            })
            .collect();
        Ok(defs)
    }

    async fn call(&self, call: &ToolCall) -> ThulpResult<ToolResult> {
        match self.registry.execute(&call.tool, call.arguments.clone()).await {
            Ok(value) => Ok(ToolResult::success(value)),
            Err(e) => Ok(ToolResult::failure(format!("{}", e))),
        }
    }
}

/// Curated collection of reusable pawan skills.
///
/// Each function returns a fresh [`thulp_skills::Skill`] that can be passed
/// to a [`thulp_skills::SkillExecutor`] for execution. Skills are designed
/// to be composable — the output of one step can be referenced in another
/// via `{{step_name}}` template variables.
pub mod built_in_skills {
    use serde_json::json;
    use thulp_skills::{Skill, SkillStep};

    /// Format code with cargo fmt, then stage and commit with a user-provided message.
    ///
    /// Input variables:
    /// - `message` (required) — commit message
    pub fn format_and_commit() -> Skill {
        Skill::new("format_and_commit", "Format code, stage, and commit")
            .with_input("message")
            .with_step(SkillStep {
                name: "format".to_string(),
                tool: "bash".to_string(),
                arguments: json!({ "command": "cargo fmt --all" }),
                ..Default::default()
            })
            .with_step(SkillStep {
                name: "stage".to_string(),
                tool: "git_add".to_string(),
                arguments: json!({ "paths": ["."] }),
                ..Default::default()
            })
            .with_step(SkillStep {
                name: "commit".to_string(),
                tool: "git_commit".to_string(),
                arguments: json!({ "message": "{{message}}" }),
                ..Default::default()
            })
    }

    /// Run the test suite, then report the outcome.
    ///
    /// Input variables: none
    pub fn test_and_report() -> Skill {
        Skill::new("test_and_report", "Run cargo test and capture output")
            .with_step(SkillStep {
                name: "test".to_string(),
                tool: "bash".to_string(),
                arguments: json!({ "command": "cargo test --workspace 2>&1 | tail -20" }),
                ..Default::default()
            })
    }

    /// Deagle map → stats → search pipeline for codebase exploration.
    ///
    /// Input variables:
    /// - `symbol` (required) — symbol name to search for after indexing
    pub fn deagle_explore() -> Skill {
        Skill::new("deagle_explore", "Index codebase and search for a symbol")
            .with_input("symbol")
            .with_step(SkillStep {
                name: "index".to_string(),
                tool: "deagle_map".to_string(),
                arguments: json!({ "path": "." }),
                ..Default::default()
            })
            .with_step(SkillStep {
                name: "stats".to_string(),
                tool: "deagle_stats".to_string(),
                arguments: json!({}),
                ..Default::default()
            })
            .with_step(SkillStep {
                name: "search".to_string(),
                tool: "deagle_search".to_string(),
                arguments: json!({ "query": "{{symbol}}", "fuzzy": true }),
                ..Default::default()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use std::path::PathBuf;

    #[test]
    fn test_pawan_transport_always_connected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ToolRegistry::with_defaults(tmp.path().to_path_buf()));
        let transport = PawanTransport::new(registry);
        assert!(transport.is_connected());
    }

    #[tokio::test]
    async fn test_pawan_transport_connect_disconnect_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ToolRegistry::with_defaults(tmp.path().to_path_buf()));
        let mut transport = PawanTransport::new(registry);
        // Both should be no-ops and succeed
        assert!(transport.connect().await.is_ok());
        assert!(transport.disconnect().await.is_ok());
    }

    #[tokio::test]
    async fn test_pawan_transport_list_tools_returns_all() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ToolRegistry::with_defaults(tmp.path().to_path_buf()));
        let transport = PawanTransport::new(registry);
        let tools = transport.list_tools().await.unwrap();
        // Should return all 34 tools (7 core + 15 standard + 12 extended)
        assert!(tools.len() >= 30, "Expected at least 30 tools, got {}", tools.len());
    }

    #[test]
    fn test_built_in_format_and_commit_skill() {
        let skill = built_in_skills::format_and_commit();
        assert_eq!(skill.name, "format_and_commit");
    }

    #[test]
    fn test_built_in_test_and_report_skill() {
        let skill = built_in_skills::test_and_report();
        assert_eq!(skill.name, "test_and_report");
    }

    #[test]
    fn test_built_in_deagle_explore_skill() {
        let skill = built_in_skills::deagle_explore();
        assert_eq!(skill.name, "deagle_explore");
    }

    #[tokio::test]
    async fn test_pawan_transport_call_unknown_tool_returns_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ToolRegistry::with_defaults(tmp.path().to_path_buf()));
        let transport = PawanTransport::new(registry);

        let call = ToolCall {
            tool: "nonexistent_tool_xyz".to_string(),
            arguments: serde_json::json!({}),
        };
        // call() always returns Ok(ToolResult) — failures are encoded in the
        // ToolResult, not as Err (so skill executors can decide policy).
        let result = transport.call(&call).await.unwrap();
        assert!(!result.success, "unknown tool should produce a failure result");
    }

    #[tokio::test]
    async fn test_pawan_transport_call_dispatches_to_read_file() {
        // Integration: write a tempfile, call read_file via transport, verify
        // the round-trip works end-to-end through the registry.
        let tmp = tempfile::TempDir::new().unwrap();
        let file_path = tmp.path().join("hello.txt");
        std::fs::write(&file_path, "hello from transport test\n").unwrap();

        let registry = Arc::new(ToolRegistry::with_defaults(tmp.path().to_path_buf()));
        let transport = PawanTransport::new(registry);

        let call = ToolCall {
            tool: "read_file".to_string(),
            arguments: serde_json::json!({ "path": "hello.txt" }),
        };
        let result = transport.call(&call).await.unwrap();
        assert!(result.success, "read_file via transport should succeed");
        // The result content should contain the file body somewhere
        let result_str = format!("{:?}", result);
        assert!(
            result_str.contains("hello from transport test"),
            "result should include file contents, got: {}",
            result_str
        );
    }

    #[tokio::test]
    async fn test_pawan_transport_list_tools_names_match_registry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ToolRegistry::with_defaults(tmp.path().to_path_buf()));
        let transport = PawanTransport::new(Arc::clone(&registry));

        let transport_names: std::collections::HashSet<String> = transport
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        let registry_names: std::collections::HashSet<String> = registry
            .get_all_definitions()
            .into_iter()
            .map(|d| d.name)
            .collect();

        assert_eq!(
            transport_names, registry_names,
            "transport.list_tools() must match registry.get_all_definitions() 1:1"
        );
    }

    #[test]
    fn test_built_in_skills_are_distinct() {
        // Pin the invariant that the 3 built-in skills have unique names
        // so accidentally renaming one to an existing name fails the test.
        let names: std::collections::HashSet<String> = [
            built_in_skills::format_and_commit().name,
            built_in_skills::test_and_report().name,
            built_in_skills::deagle_explore().name,
        ]
        .into_iter()
        .collect();
        assert_eq!(names.len(), 3, "all 3 built-in skills must have unique names");
    }

    // Suppress unused imports warning for PathBuf when tests compile but
    // don't exercise it directly.
    #[allow(dead_code)]
    fn _unused_pathbuf() -> PathBuf {
        PathBuf::new()
    }

    #[test]
    fn test_format_and_commit_skill_has_3_steps_with_expected_tools() {
        // Pins the format → stage → commit pipeline. If someone reorders
        // these steps (or drops one), the workflow breaks because git_add
        // needs to run after cargo fmt edits and git_commit needs staged
        // files.
        let skill = built_in_skills::format_and_commit();
        assert_eq!(skill.steps.len(), 3);
        assert_eq!(skill.steps[0].name, "format");
        assert_eq!(skill.steps[0].tool, "bash");
        assert_eq!(skill.steps[1].name, "stage");
        assert_eq!(skill.steps[1].tool, "git_add");
        assert_eq!(skill.steps[2].name, "commit");
        assert_eq!(skill.steps[2].tool, "git_commit");
    }

    #[test]
    fn test_format_and_commit_requires_message_input() {
        // The commit step references `{{message}}` which the skill
        // executor substitutes from inputs. If inputs drops "message",
        // the template stays literal and the commit message is wrong.
        let skill = built_in_skills::format_and_commit();
        assert!(skill.inputs.contains(&"message".to_string()));
        // The commit arguments must still reference the template variable
        let commit_args = &skill.steps[2].arguments;
        let msg = commit_args["message"].as_str().unwrap();
        assert!(msg.contains("{{message}}"), "commit should use {{{{message}}}} template, got: {}", msg);
    }

    #[test]
    fn test_test_and_report_skill_has_exactly_one_step() {
        // Structure pin: this skill is intentionally minimal — a single
        // `cargo test` step. Anything more should live in a different skill.
        let skill = built_in_skills::test_and_report();
        assert_eq!(skill.steps.len(), 1);
        assert_eq!(skill.steps[0].name, "test");
        assert_eq!(skill.steps[0].tool, "bash");
        assert!(skill.inputs.is_empty(), "test_and_report takes no inputs");
    }

    #[test]
    fn test_deagle_explore_pipeline_order() {
        // deagle_map MUST run before deagle_stats and deagle_search —
        // otherwise the graph is empty and the search returns nothing.
        // If someone swaps the order, this catches it.
        let skill = built_in_skills::deagle_explore();
        assert_eq!(skill.steps.len(), 3);
        assert_eq!(skill.steps[0].tool, "deagle_map", "index must run first");
        assert_eq!(skill.steps[1].tool, "deagle_stats", "stats runs after map");
        assert_eq!(skill.steps[2].tool, "deagle_search", "search last");
        // symbol input must be declared + referenced in search step
        assert!(skill.inputs.contains(&"symbol".to_string()));
        let search_args = &skill.steps[2].arguments;
        assert!(search_args["query"].as_str().unwrap().contains("{{symbol}}"));
    }

    #[test]
    fn test_all_built_in_skill_tools_are_in_default_registry() {
        // Every tool referenced by every built-in skill MUST exist in the
        // default ToolRegistry — otherwise the skill can never run. This
        // test catches the "skill added but tool rename happened" drift.
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = ToolRegistry::with_defaults(tmp.path().to_path_buf());

        let all_skills = [
            built_in_skills::format_and_commit(),
            built_in_skills::test_and_report(),
            built_in_skills::deagle_explore(),
        ];

        for skill in &all_skills {
            for step in &skill.steps {
                assert!(
                    registry.has_tool(&step.tool),
                    "skill {:?} step {:?} references unregistered tool {:?}",
                    skill.name, step.name, step.tool,
                );
            }
        }
    }

    #[tokio::test]
    async fn test_pawan_transport_list_tools_all_have_descriptions() {
        // Integration quality check: every thulp_definition exposed via
        // list_tools must have a non-empty description. Empty descriptions
        // confuse downstream skill authors and LLMs choosing tools.
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(ToolRegistry::with_defaults(tmp.path().to_path_buf()));
        let transport = PawanTransport::new(registry);

        let tools = transport.list_tools().await.unwrap();
        assert!(!tools.is_empty());
        for tool in &tools {
            assert!(
                !tool.description.is_empty(),
                "tool {} has empty description",
                tool.name,
            );
        }
    }
}
