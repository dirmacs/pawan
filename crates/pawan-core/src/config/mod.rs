//! Configuration for Pawan
//!
//! Pawan can be configured via:
//! - `pawan.toml` in the current directory
//! - `[pawan]` section in `ares.toml`
//! - Environment variables
//! - Command line arguments

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing;

/// LLM Provider type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    /// NVIDIA API (build.nvidia.com) - default
    #[default]
    Nvidia,
    /// Local Ollama instance
    Ollama,
    /// OpenAI-compatible API
    OpenAI,
    /// MLX LM server (Apple Silicon native, mlx_lm.server) — auto-routes to localhost:8080
    Mlx,
}

/// Main configuration for Pawan
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PawanConfig {
    /// LLM provider to use
    pub provider: LlmProvider,

    /// LLM model to use for coding tasks
    pub model: String,

    /// Override the API base URL (e.g. "http://localhost:8080/v1" for llama.cpp).
    /// Takes priority over OPENAI_API_URL / NVIDIA_API_URL env vars.
    pub base_url: Option<String>,

    /// Enable dry-run mode (show changes without applying)
    pub dry_run: bool,

    /// Create backups before editing files
    pub auto_backup: bool,

    /// Require clean git working directory
    pub require_git_clean: bool,

    /// Timeout for bash commands (seconds)
    pub bash_timeout_secs: u64,

    /// Maximum file size to read (KB)
    pub max_file_size_kb: usize,

    /// Maximum tool iterations per request
    pub max_tool_iterations: usize,
    /// Maximum context tokens before pruning
    pub max_context_tokens: usize,

    /// System prompt override
    pub system_prompt: Option<String>,

    /// Temperature for LLM responses
    pub temperature: f32,

    /// Top-p sampling parameter
    pub top_p: f32,

    /// Maximum tokens in response
    pub max_tokens: usize,

    /// Maximum retries for LLM API calls (429 or 5xx errors)
    pub max_retries: usize,

    /// Fallback models to try when primary model fails
    pub fallback_models: Vec<String>,
    /// Maximum characters in tool result before truncation
    pub max_result_chars: usize,

    /// Enable reasoning/thinking mode (for DeepSeek/Nemotron models)
    pub reasoning_mode: bool,

    /// Healing configuration
    pub healing: HealingConfig,

    /// Target projects
    pub targets: HashMap<String, TargetConfig>,

    /// TUI configuration
    pub tui: TuiConfig,

    /// MCP server configurations
    #[serde(default)]
    pub mcp: HashMap<String, McpServerEntry>,

    /// Tool permission overrides (tool_name -> permission)
    #[serde(default)]
    pub permissions: HashMap<String, ToolPermission>,

    /// Cloud fallback: when primary model fails, fall back to cloud provider.
    /// Enables hybrid local+cloud routing.
    pub cloud: Option<CloudConfig>,

    /// Eruka context engine integration (3-tier memory injection)
    #[serde(default)]
    pub eruka: crate::eruka_bridge::ErukaConfig,
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

/// Permission level for a tool
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolPermission {
    /// Always allow (default for most tools)
    Allow,
    /// Deny — tool is disabled
    Deny,
}

impl Default for PawanConfig {
    fn default() -> Self {
        let mut targets = HashMap::new();
        targets.insert(
            "ares".to_string(),
            TargetConfig {
                path: PathBuf::from("../.."),
                description: "A.R.E.S server codebase".to_string(),
            },
        );
        targets.insert(
            "self".to_string(),
            TargetConfig {
                path: PathBuf::from("."),
                description: "Pawan's own codebase".to_string(),
            },
        );

        Self {
            provider: LlmProvider::Nvidia,
            model: crate::DEFAULT_MODEL.to_string(),
            base_url: None,
            dry_run: false,
            auto_backup: true,
            require_git_clean: false,
            bash_timeout_secs: crate::DEFAULT_BASH_TIMEOUT,
            max_file_size_kb: 1024,
            max_tool_iterations: crate::MAX_TOOL_ITERATIONS,
            max_context_tokens: 100000,
            system_prompt: None,
            temperature: 1.0,
            top_p: 0.95,
            max_tokens: 8192,
            reasoning_mode: true,
            max_retries: 3,
            fallback_models: Vec::new(),
            max_result_chars: 8000,
            healing: HealingConfig::default(),
            targets,
            tui: TuiConfig::default(),
            mcp: HashMap::new(),
            permissions: HashMap::new(),
            cloud: None,
            eruka: crate::eruka_bridge::ErukaConfig::default(),
        }
    }
}

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

    /// Maximum fix attempts per issue
    pub max_attempts: usize,
}

impl Default for HealingConfig {
    fn default() -> Self {
        Self {
            auto_commit: false,
            fix_errors: true,
            fix_warnings: true,
            fix_tests: true,
            generate_docs: false,
            max_attempts: 3,
        }
    }
}

/// Configuration for a target project
#[derive(Debug, Clone, Serialize, Deserialize)]
/// Configuration for a target project
///
/// This struct represents configuration for a specific target project that Pawan
/// can work with. It includes the project path and description.
pub struct TargetConfig {
    /// Path to the project root
    pub path: PathBuf,

    /// Description of the project
    pub description: String,
}

/// Configuration for the TUI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Enable syntax highlighting
    pub syntax_highlighting: bool,

    /// Theme for syntax highlighting
    pub theme: String,

    /// Show line numbers in code blocks
    pub line_numbers: bool,

    /// Enable mouse support
    pub mouse_support: bool,

    /// Scroll speed (lines per scroll event)
    pub scroll_speed: usize,

    /// Maximum history entries to keep
    pub max_history: usize,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            syntax_highlighting: true,
            theme: "base16-ocean.dark".to_string(),
            line_numbers: true,
            mouse_support: true,
            scroll_speed: 3,
            max_history: 1000,
        }
    }
}

/// Configuration for an MCP server in pawan.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
/// Configuration for an MCP server in pawan.toml
///
/// This struct represents configuration for an MCP (Multi-Cursor Protocol) server
/// that can be managed by Pawan. It includes the command to run, arguments,
/// environment variables, and whether the server is enabled.
pub struct McpServerEntry {
    /// Command to run
    pub command: String,
    /// Command arguments
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether this server is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl PawanConfig {
    /// Load configuration from file
    pub fn load(path: Option<&PathBuf>) -> crate::Result<Self> {
        let config_path = path.cloned().or_else(|| {
            // 1. pawan.toml in CWD
            let pawan_toml = PathBuf::from("pawan.toml");
            if pawan_toml.exists() {
                return Some(pawan_toml);
            }

            // 2. ares.toml in CWD
            let ares_toml = PathBuf::from("ares.toml");
            if ares_toml.exists() {
                return Some(ares_toml);
            }

            // 3. Global user config: ~/.config/pawan/pawan.toml
            if let Some(home) = dirs::home_dir() {
                let global = home.join(".config/pawan/pawan.toml");
                if global.exists() {
                    return Some(global);
                }
            }

            None
        });

        match config_path {
            Some(path) => {
                let content = std::fs::read_to_string(&path).map_err(|e| {
                    crate::PawanError::Config(format!("Failed to read {}: {}", path.display(), e))
                })?;

                // Check if this is ares.toml (look for [pawan] section)
                if path.file_name().map(|n| n == "ares.toml").unwrap_or(false) {
                    // Parse as TOML and extract [pawan] section
                    let value: toml::Value = toml::from_str(&content).map_err(|e| {
                        crate::PawanError::Config(format!(
                            "Failed to parse {}: {}",
                            path.display(),
                            e
                        ))
                    })?;

                    if let Some(pawan_section) = value.get("pawan") {
                        let config: PawanConfig =
                            pawan_section.clone().try_into().map_err(|e| {
                                crate::PawanError::Config(format!(
                                    "Failed to parse [pawan] section: {}",
                                    e
                                ))
                            })?;
                        return Ok(config);
                    }

                    // No [pawan] section, use defaults
                    Ok(Self::default())
                } else {
                    // Parse as pawan.toml
                    toml::from_str(&content).map_err(|e| {
                        crate::PawanError::Config(format!(
                            "Failed to parse {}: {}",
                            path.display(),
                            e
                        ))
                    })
                }
            }
            None => Ok(Self::default()),
        }
    }

    /// Apply environment variable overrides (PAWAN_MODEL, PAWAN_PROVIDER, etc.)
    pub fn apply_env_overrides(&mut self) {
        if let Ok(model) = std::env::var("PAWAN_MODEL") {
            self.model = model;
        }
        if let Ok(provider) = std::env::var("PAWAN_PROVIDER") {
            match provider.to_lowercase().as_str() {
                "nvidia" | "nim" => self.provider = LlmProvider::Nvidia,
                "ollama" => self.provider = LlmProvider::Ollama,
                "openai" => self.provider = LlmProvider::OpenAI,
                "mlx" | "mlx-lm" => self.provider = LlmProvider::Mlx,
                _ => tracing::warn!(provider = provider.as_str(), "Unknown PAWAN_PROVIDER, ignoring"),
            }
        }
        if let Ok(temp) = std::env::var("PAWAN_TEMPERATURE") {
            if let Ok(t) = temp.parse::<f32>() {
                self.temperature = t;
            }
        }
        if let Ok(tokens) = std::env::var("PAWAN_MAX_TOKENS") {
            if let Ok(t) = tokens.parse::<usize>() {
                self.max_tokens = t;
            }
        }
        if let Ok(iters) = std::env::var("PAWAN_MAX_ITERATIONS") {
            if let Ok(i) = iters.parse::<usize>() {
                self.max_tool_iterations = i;
            }
        }
        if let Ok(ctx) = std::env::var("PAWAN_MAX_CONTEXT_TOKENS") {
            if let Ok(c) = ctx.parse::<usize>() {
                self.max_context_tokens = c;
            }
        }
        if let Ok(models) = std::env::var("PAWAN_FALLBACK_MODELS") {
            self.fallback_models = models.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
        if let Ok(chars) = std::env::var("PAWAN_MAX_RESULT_CHARS") {
            if let Ok(c) = chars.parse::<usize>() {
                self.max_result_chars = c;
            }
        }
    }

    /// Get target by name
    pub fn get_target(&self, name: &str) -> Option<&TargetConfig> {
        self.targets.get(name)
    }

    /// Get the system prompt, with optional PAWAN.md context injection
    pub fn get_system_prompt(&self) -> String {
        let base = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        // Try to load PAWAN.md from current directory for project context
        let context = Self::load_context_file();
        if let Some(ctx) = context {
            format!("{}\n\n## Project Context (from PAWAN.md)\n\n{}", base, ctx)
        } else {
            base
        }
    }

    /// Load PAWAN.md context file from current directory (if it exists)
    fn load_context_file() -> Option<String> {
        // Check PAWAN.md first, then .pawan/context.md
        for path in &["PAWAN.md", ".pawan/context.md"] {
            let p = PathBuf::from(path);
            if p.exists() {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    if !content.trim().is_empty() {
                        return Some(content);
                    }
                }
            }
        }
        None
    }

    /// Check if thinking mode should be enabled.
    /// Only applicable to DeepSeek models (other NIM models don't support <think> tokens).
    pub fn use_thinking_mode(&self) -> bool {
        self.reasoning_mode && self.model.contains("deepseek")
    }
}

/// Default system prompt for coding tasks
pub const DEFAULT_SYSTEM_PROMPT: &str = r#"You are Pawan, an expert coding assistant. You have tools for file ops, search, shell, git, and agent spawning.

CRITICAL — Efficiency rules (you have limited tool iterations):
- Do NOT explore before acting. The user prompt tells you what to do — do it immediately.
- Do NOT check if files/directories exist before writing. write_file creates parents automatically.
- Do NOT read a file before writing it unless you need its existing content for an edit.
- Write code FIRST, then verify with cargo check or tests. Never plan without acting.
- If cargo check fails after you write, fix the errors immediately — the error output is in your context.
- Use relative paths from the workspace root whenever possible.

Available tools:
- File: read_file, write_file, edit_file, edit_file_lines, insert_after, append_file, list_directory
- Search: glob_search, grep_search
- Shell: bash
- Git: git_status, git_diff, git_add, git_commit, git_log, git_blame, git_branch, git_checkout, git_stash
- Agent: spawn_agent

When making changes:
1. Make minimal, focused changes
2. Follow existing code style and patterns
3. After writing .rs files, cargo check runs automatically — if it fails, fix immediately
4. Run tests when the task calls for it (cargo test -p <crate>)

When fixing issues:
1. Read the error carefully — fix the root cause, not symptoms
2. Make one fix at a time
3. If a fix doesn't work, try a different approach

Be concise. Act first, explain briefly after.

Git commits: always use author `bkataru <baalateja.k@gmail.com>`. Pass -c user.name="bkataru" -c user.email="baalateja.k@gmail.com" on every git commit."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_mlx_parsing() {
        // "mlx" string parses to LlmProvider::Mlx via serde rename_all = "lowercase"
        let toml = r#"
provider = "mlx"
model = "mlx-community/Qwen3.5-9B-4bit"
"#;
        let config: PawanConfig = toml::from_str(toml).expect("should parse without error");
        assert_eq!(config.provider, LlmProvider::Mlx);
        assert_eq!(config.model, "mlx-community/Qwen3.5-9B-4bit");
    }

    #[test]
    fn test_provider_mlx_lm_alias() {
        // "mlx-lm" is an alias for mlx via apply_env_overrides (env var path)
        let mut config = PawanConfig::default();
        std::env::set_var("PAWAN_PROVIDER", "mlx-lm");
        config.apply_env_overrides();
        std::env::remove_var("PAWAN_PROVIDER");
        assert_eq!(config.provider, LlmProvider::Mlx);
    }

    #[test]
    fn test_mlx_base_url_override() {
        // When provider=mlx and base_url is set, base_url is preserved in config
        let toml = r#"
provider = "mlx"
model = "test-model"
base_url = "http://192.168.1.100:8080/v1"
"#;
        let config: PawanConfig = toml::from_str(toml).expect("should parse without error");
        assert_eq!(config.provider, LlmProvider::Mlx);
        assert_eq!(
            config.base_url.as_deref(),
            Some("http://192.168.1.100:8080/v1")
        );
    }
}
