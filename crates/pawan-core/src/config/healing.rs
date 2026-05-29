use serde::{Deserialize, Serialize};

/// Configuration for self-healing behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HealingConfig {
    /// Automatically commit fixes
    pub auto_commit: bool,

    /// Fix compilation errors
    pub fix_errors: bool,

    /// Fix clippy warnings
    pub fix_warnings: bool,

    /// Fix failing tests
    pub fix_tests: bool,

    /// Generate missing documentation
    pub generate_docs: bool,

    /// Run `cargo audit` and surface security advisories as diagnostics.
    /// Off by default — `cargo audit` requires the binary to be installed
    /// and has occasional network dependencies for the advisory database.
    #[serde(default)]
    pub fix_security: bool,

    /// Maximum fix attempts per issue
    pub max_attempts: usize,

    /// Optional shell command to run after `cargo check` passes (stage 2 gate).
    /// If this command exits non-zero the heal loop treats the output as a
    /// remaining failure and retries.  Useful values:
    ///   - `"cargo test --workspace"` — run full test suite
    ///   - `"cargo clippy -- -D warnings"` — enforce zero warnings
    ///
    ///     Leave unset (default) to skip the second stage.
    #[serde(default)]
    pub verify_cmd: Option<String>,
}

impl Default for HealingConfig {
    fn default() -> Self {
        Self {
            auto_commit: false,
            fix_errors: true,
            fix_warnings: true,
            fix_tests: true,
            generate_docs: false,
            fix_security: false,
            max_attempts: 3,
            verify_cmd: None,
        }
    }
}
