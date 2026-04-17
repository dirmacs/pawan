//! # Pawan (पवन) - Self-Healing CLI Coding Agent
//!
//! Core library for the Pawan coding agent. Provides:
//!
//! - **Agent**: Tool-calling loop engine with multi-provider LLM support
//! - **Tools**: File operations, bash execution, git, search
//! - **Healing**: Automatic detection and repair of compilation errors, test failures, warnings
//! - **Config**: TOML-based configuration with provider/model management
//!
//! ## Architecture
//!
//! - `pawan-core` (this crate): Library with zero dirmacs dependencies
//! - `pawan-cli`: Binary with TUI and CLI interface
//!
//! ## Quick Start
//!
//! ```bash
//! pawan                    # Interactive TUI mode
//! pawan heal               # Auto-fix compilation issues
//! pawan task "description" # Execute a coding task
//! pawan run "prompt"       # Headless single-prompt execution
//! ```

pub mod agent;
pub mod bootstrap;
pub mod compaction;
pub mod config;
pub mod coordinator;
pub mod credentials;
pub mod eruka_bridge;
pub mod handoff;
pub mod init;
pub mod healing;
pub mod skill_distillation;
pub mod skills;
pub mod tasks;
pub mod tools;

pub use agent::PawanAgent;
pub use agent::{AgentEvent, FinishReason, TokenUsageInfo};
pub use config::PawanConfig;

/// Error types for Pawan
///
/// Represents all possible error conditions that can occur in Pawan operations.
#[derive(Debug, thiserror::Error)]
pub enum PawanError {
    /// I/O error
    ///
    /// Represents errors related to file operations, network operations, or other I/O.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Configuration error
    ///
    /// Represents errors in Pawan configuration, such as invalid settings or missing required values.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Tool execution error
    ///
    /// Represents errors that occur when executing tools (file operations, bash commands, etc.).
    #[error("Tool execution error: {0}")]
    Tool(String),

    /// Agent error
    ///
    /// Represents errors in the agent's logic or execution flow.
    #[error("Agent error: {0}")]
    Agent(String),

    /// LLM error
    ///
    /// Represents errors from the language model backend or API.
    #[error("LLM error: {0}")]
    Llm(String),

    /// Git error
    ///
    /// Represents errors from Git operations.
    #[error("Git error: {0}")]
    Git(String),

    /// Parse error
    ///
    /// Represents errors in parsing JSON, TOML, or other structured data.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Timeout error
    ///
    /// Represents errors when an operation exceeds its time limit.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Not found error
    ///
    /// Represents errors when a requested resource (file, tool, etc.) is not found.
    #[error("Not found: {0}")]
    NotFound(String),
}

/// Result type alias for Pawan operations
///
/// A convenience type alias that represents the result of Pawan operations,
/// where success contains the desired value and failure contains a PawanError.
pub type Result<T> = std::result::Result<T, PawanError>;

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default model for coding tasks (Mistral Small 4 119B MoE via NVIDIA NIM)
/// Default model — Qwen3.5 122B (S tier, 383ms, solid tool calling).
/// Override via pawan.toml or PAWAN_MODEL env var.
pub const DEFAULT_MODEL: &str = "qwen/qwen3.5-122b-a10b";

/// Default NVIDIA API URL
pub const DEFAULT_NVIDIA_API_URL: &str = "https://integrate.api.nvidia.com/v1";

/// Maximum iterations for tool calling loops
pub const MAX_TOOL_ITERATIONS: usize = 50;

/// Default timeout for bash commands (in seconds)
pub const DEFAULT_BASH_TIMEOUT: u64 = 120;
