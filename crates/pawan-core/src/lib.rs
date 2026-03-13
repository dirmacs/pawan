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
pub mod config;
pub mod healing;
pub mod tools;

pub use agent::PawanAgent;
pub use config::PawanConfig;

/// Error types for Pawan
#[derive(Debug, thiserror::Error)]
pub enum PawanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Tool execution error: {0}")]
    Tool(String),

    #[error("Agent error: {0}")]
    Agent(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Git error: {0}")]
    Git(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Not found: {0}")]
    NotFound(String),
}

/// Result type alias for Pawan operations
pub type Result<T> = std::result::Result<T, PawanError>;

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default model for coding tasks (DeepSeek v3.2 via NVIDIA API)
pub const DEFAULT_MODEL: &str = "deepseek-ai/deepseek-v3.2";

/// Default NVIDIA API URL
pub const DEFAULT_NVIDIA_API_URL: &str = "https://integrate.api.nvidia.com/v1";

/// Maximum iterations for tool calling loops
pub const MAX_TOOL_ITERATIONS: usize = 50;

/// Default timeout for bash commands (in seconds)
pub const DEFAULT_BASH_TIMEOUT: u64 = 120;
