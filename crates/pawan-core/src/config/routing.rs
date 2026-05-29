use serde::{Deserialize, Serialize};

use super::provider::LlmProvider;

/// Task-type model routing — use different models for different task categories.
///
/// # Example (pawan.toml)
/// ```toml
/// [models]
/// code = "qwen/qwen3.5-122b-a10b"                  # best for code generation
/// orchestrate = "minimaxai/minimax-m2.5"            # best for tool calling
/// execute = "mlx-community/Qwen3.5-9B-OptiQ-4bit"  # fast local execution
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRouting {
    /// Model for code generation tasks (implement, refactor, write tests)
    pub code: Option<String>,
    /// Model for orchestration tasks (multi-step tool chains, analysis)
    pub orchestrate: Option<String>,
    /// Model for simple execution tasks (bash, write_file, cargo test)
    pub execute: Option<String>,
}

impl ModelRouting {
    /// Select the best model for a given task based on keyword analysis.
    /// Returns None if no routing matches (use default model).
    pub fn route(&self, query: &str) -> Option<&str> {
        let q = query.to_lowercase();

        // Code generation patterns
        if self.code.is_some() {
            let code_signals = [
                "implement",
                "write",
                "create",
                "refactor",
                "fix",
                "add test",
                "add function",
                "struct",
                "enum",
                "trait",
                "algorithm",
                "data structure",
            ];
            if code_signals.iter().any(|s| q.contains(s)) {
                return self.code.as_deref();
            }
        }

        // Orchestration patterns
        if self.orchestrate.is_some() {
            let orch_signals = [
                "search", "find", "analyze", "review", "explain", "compare", "list", "check",
                "verify", "diagnose", "audit",
            ];
            if orch_signals.iter().any(|s| q.contains(s)) {
                return self.orchestrate.as_deref();
            }
        }

        // Execution patterns
        if self.execute.is_some() {
            let exec_signals = [
                "run", "execute", "bash", "cargo", "test", "build", "deploy", "install", "commit",
            ];
            if exec_signals.iter().any(|s| q.contains(s)) {
                return self.execute.as_deref();
            }
        }

        None
    }
}

/// Cloud fallback configuration for hybrid local+cloud model routing.
///
/// When the primary provider (typically a local model via OpenAI-compatible API)
/// fails or is unavailable, pawan automatically falls back to this cloud provider.
/// This enables zero-cost local inference with cloud reliability as a safety net.
///
/// # Example (pawan.toml)
/// ```toml
/// provider = "openai"
/// model = "Qwen3.5-9B-Q4_K_M"
///
/// [cloud]
/// provider = "nvidia"
/// model = "mistralai/devstral-2-123b-instruct-2512"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    /// Cloud LLM provider to fall back to (nvidia or openai)
    pub provider: LlmProvider,
    /// Primary cloud model to try first on fallback
    pub model: String,
    /// Additional cloud models to try if the primary cloud model also fails
    #[serde(default)]
    pub fallback_models: Vec<String>,
}
