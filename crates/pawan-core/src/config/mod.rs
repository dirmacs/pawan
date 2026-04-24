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

/// Default config version
const fn default_config_version() -> u32 {
    1
}

/// Default tool idle timeout (5 minutes)
const fn default_tool_idle_timeout() -> u64 {
    300
}

/// Config migration result
#[derive(Debug)]
pub struct MigrationResult {
    /// Whether migration was performed
    pub migrated: bool,
    /// Original config version
    pub from_version: u32,
    /// Target config version
    pub to_version: u32,
    /// Path to backup if created
    pub backup_path: Option<std::path::PathBuf>,
}

impl MigrationResult {
    /// Create a new migration result
    pub fn new(from_version: u32, to_version: u32, backup_path: Option<std::path::PathBuf>) -> Self {
        Self {
            migrated: from_version != to_version,
            from_version,
            to_version,
            backup_path,
        }
    }

    /// Create a result indicating no migration was needed
    pub fn no_migration(version: u32) -> Self {
        Self {
            migrated: false,
            from_version: version,
            to_version: version,
            backup_path: None,
        }
    }
}

/// Latest config version
const LATEST_CONFIG_VERSION: u32 = 1;

/// Migrate config to the latest version
///
/// This function handles version upgrades by applying migration steps
/// sequentially from the current version to the latest version.
///
/// # Arguments
/// * `config` - The config to migrate (will be modified in place)
/// * `config_path` - Optional path to the config file (for backup)
///
/// # Returns
/// Migration result indicating whether migration occurred and details
pub fn migrate_to_latest(config: &mut PawanConfig, config_path: Option<&PathBuf>) -> MigrationResult {
    let current_version = config.config_version;

    if current_version >= LATEST_CONFIG_VERSION {
        return MigrationResult::no_migration(current_version);
    }

    // Create backup if we have a path
    let backup_path = config_path.and_then(|path| create_backup(path).ok());

    // Apply migrations sequentially
    let mut version = current_version;
    while version < LATEST_CONFIG_VERSION {
        version = match migrate_to_version(config, version + 1) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(
                    from_version = version,
                    to_version = LATEST_CONFIG_VERSION,
                    error = %e,
                    "Config migration failed"
                );
                return MigrationResult::new(current_version, version, backup_path);
            }
        };
    }

    config.config_version = LATEST_CONFIG_VERSION;
    MigrationResult::new(current_version, LATEST_CONFIG_VERSION, backup_path)
}

/// Migrate config to a specific version
///
/// # Arguments
/// * `config` - The config to migrate (will be modified in place)
/// * `target_version` - The target version to migrate to
///
/// # Returns
/// Ok with the new version, or Err if migration failed
fn migrate_to_version(config: &mut PawanConfig, target_version: u32) -> Result<u32, String> {
    match target_version {
        1 => migrate_to_v1(config),
        _ => Err(format!("Unknown target version: {}", target_version)),
    }
}

/// Migrate config to version 1
///
/// Version 1 adds:
/// - config_version field
/// - tool_call_idle_timeout_secs field (default: 300)
/// - skills_repo field
/// - local_first field
/// - local_endpoint field
fn migrate_to_v1(config: &mut PawanConfig) -> Result<u32, String> {
    // Set config version
    config.config_version = 1;

    // Set default tool idle timeout if not present
    if config.tool_call_idle_timeout_secs == 0 {
        config.tool_call_idle_timeout_secs = default_tool_idle_timeout();
    }

    // Note: skills_repo, local_first, and local_endpoint are Option fields
    // so they'll be None by default if not present in the config

    tracing::info!("Config migrated to version 1");
    Ok(1)
}

/// Create a backup of the config file
///
/// # Arguments
/// * `config_path` - Path to the config file
///
/// # Returns
/// Ok with the backup path, or Err if backup failed
fn create_backup(config_path: &PathBuf) -> Result<PathBuf, String> {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = config_path.with_extension(format!("toml.backup.{}", timestamp));

    std::fs::copy(config_path, &backup_path).map_err(|e| {
        format!("Failed to create backup at {}: {}", backup_path.display(), e)
    })?;

    tracing::info!(backup = %backup_path.display(), "Config backup created");
    Ok(backup_path)
}

/// Save config to a file
///
/// # Arguments
/// * `config` - The config to save
/// * `path` - Path to save the config to
///
/// # Returns
/// Ok if save succeeded, Err if save failed
pub fn save_config(config: &PawanConfig, path: &PathBuf) -> Result<(), String> {
    let toml_string = toml::to_string_pretty(config).map_err(|e| {
        format!("Failed to serialize config to TOML: {}", e)
    })?;

    std::fs::write(path, toml_string).map_err(|e| {
        format!("Failed to write config to {}: {}", path.display(), e)
    })?;

    tracing::info!(path = %path.display(), "Config saved");
    Ok(())
}

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
    /// Config version for migration tracking (default: 1)
    #[serde(default = "default_config_version")]
    pub config_version: u32,

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

    /// Timeout for tool calls that remain idle (seconds)
    /// Default: 300 (5 minutes)
    #[serde(default = "default_tool_idle_timeout")]
    pub tool_call_idle_timeout_secs: u64,

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

    /// Maximum tokens allowed for reasoning/thinking (0 = unlimited).
    /// When set, pawan tracks thinking vs action token usage per call.
    /// If thinking exceeds this budget, a warning is logged.
    pub thinking_budget: usize,

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

    /// Task-type model routing: use different models for different task categories.
    /// If not set, all tasks use the primary model.
    #[serde(default)]
    pub models: ModelRouting,

    /// Eruka context engine integration (3-tier memory injection)
    #[serde(default)]
    pub eruka: crate::eruka_bridge::ErukaConfig,

    /// Use ares-server's LLMClient + ToolCoordinator primitives instead of
    /// pawan's built-in OpenAI-compatible backend. Requires the `ares` feature
    /// flag to be enabled when building pawan-core. When true, pawan delegates
    /// LLM generation to ares which provides connection pooling, loop detection,
    /// and unified multi-provider support. Default: false (backwards compatible).
    #[serde(default)]
    pub use_ares_backend: bool,
    /// Use the ToolCoordinator for tool-calling loops instead of pawan's
    /// built-in implementation. When true, delegates tool execution to the
    /// coordinator which provides parallel execution, timeouts, and consistent
    /// error handling. Default: false (backwards compatible).
    #[serde(default)]
    pub use_coordinator: bool,

    /// Optional path to a skills repository (directory of SKILL.md files).
    ///
    /// Mirrors the dstack pattern: public repo + private skills linked by
    /// config. When set, pawan discovers all SKILL.md files under this path
    /// at runtime via thulp-skill-files SkillLoader. Useful for linking
    /// private skill libraries without embedding them in the public repo.
    ///
    /// Resolution order:
    ///   1. `PAWAN_SKILLS_REPO` environment variable
    ///   2. `skills_repo` field in pawan.toml
    ///   3. `~/.config/pawan/skills` if it exists
    ///   4. None (no skill discovery beyond the project SKILL.md)
    #[serde(default)]
    pub skills_repo: Option<PathBuf>,

    /// Prefer local inference over cloud when a local model server is reachable.
    /// Before each session pawan probes `local_endpoint` (or the Ollama default
    /// `http://localhost:11434/v1`) with a 100 ms TCP timeout.
    /// If the server responds, it is used instead of the configured cloud provider.
    /// If the server is unreachable the configured provider is used as normal.
    /// Default: false (always use configured provider).
    #[serde(default)]
    pub local_first: bool,

    /// Local inference endpoint URL for the `local_first` probe.
    /// Must be an OpenAI-compatible `/v1` endpoint (Ollama, llama.cpp, LM Studio, …).
    /// Defaults to `http://localhost:11434/v1` (Ollama) when not set.
    #[serde(default)]
    pub local_endpoint: Option<String>,
}

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
            let code_signals = ["implement", "write", "create", "refactor", "fix", "add test",
                "add function", "struct", "enum", "trait", "algorithm", "data structure"];
            if code_signals.iter().any(|s| q.contains(s)) {
                return self.code.as_deref();
            }
        }

        // Orchestration patterns
        if self.orchestrate.is_some() {
            let orch_signals = ["search", "find", "analyze", "review", "explain", "compare",
                "list", "check", "verify", "diagnose", "audit"];
            if orch_signals.iter().any(|s| q.contains(s)) {
                return self.orchestrate.as_deref();
            }
        }

        // Execution patterns
        if self.execute.is_some() {
            let exec_signals = ["run", "execute", "bash", "cargo", "test", "build",
                "deploy", "install", "commit"];
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

/// Permission level for a tool
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolPermission {
    /// Always allow (default for most tools)
    Allow,
    /// Deny — tool is disabled
    Deny,
    /// Prompt — ask user before executing (TUI shows confirmation, headless denies)
    Prompt,
}

impl ToolPermission {
    /// Resolve permission for a tool name.
    /// Checks explicit config first, then falls back to default rules:
    /// - bash, git_commit, write_file, edit_file: Prompt if not explicitly configured
    /// - Everything else: Allow
    pub fn resolve(name: &str, permissions: &HashMap<String, ToolPermission>) -> Self {
        if let Some(p) = permissions.get(name) {
            return p.clone();
        }
        // Default: sensitive tools prompt, others allow
        match name {
            "bash" | "git_commit" | "write_file" | "edit_file_lines"
            | "insert_after" | "append_file" => ToolPermission::Allow, // default allow for now; users can override to Prompt
            _ => ToolPermission::Allow,
        }
    }
}

impl Default for PawanConfig {
    fn default() -> Self {
        let mut targets = HashMap::new();
        targets.insert(
            "self".to_string(),
            TargetConfig {
                path: PathBuf::from("."),
                description: "Current project codebase".to_string(),
            },
        );

        Self {
            provider: LlmProvider::Nvidia,
            config_version: default_config_version(),
            model: crate::DEFAULT_MODEL.to_string(),
            base_url: None,
            dry_run: false,
            auto_backup: true,
            require_git_clean: false,
            bash_timeout_secs: crate::DEFAULT_BASH_TIMEOUT,
            tool_call_idle_timeout_secs: default_tool_idle_timeout(),
            max_file_size_kb: 1024,
            max_tool_iterations: crate::MAX_TOOL_ITERATIONS,
            max_context_tokens: 100000,
            system_prompt: None,
            temperature: 1.0,
            top_p: 0.95,
            max_tokens: 8192,
            thinking_budget: 0, // 0 = unlimited
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
            models: ModelRouting::default(),
            eruka: crate::eruka_bridge::ErukaConfig::default(),
            use_ares_backend: false,
            use_coordinator: false,
            skills_repo: None,
            local_first: false,
            local_endpoint: None,
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
    /// Leave unset (default) to skip the second stage.
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
    
    /// Auto-save enabled (default: true)
    pub auto_save_enabled: bool,
    /// Auto-save interval in minutes
    pub auto_save_interval_minutes: u32,
    /// Custom save directory for auto-saves (defaults to ~/.pawan/sessions/)
    pub auto_save_dir: Option<std::path::PathBuf>,
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
            auto_save_enabled: true,
            auto_save_interval_minutes: 5,
            auto_save_dir: None,
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
                    let mut config: PawanConfig = toml::from_str(&content).map_err(|e| {
                        crate::PawanError::Config(format!(
                            "Failed to parse {}: {}",
                            path.display(),
                            e
                        ))
                    })?;

                    // Migrate config to latest version
                    let migration_result = migrate_to_latest(&mut config, Some(&path));
                    if migration_result.migrated {
                        tracing::info!(
                            from_version = migration_result.from_version,
                            to_version = migration_result.to_version,
                            backup = ?migration_result.backup_path,
                            "Config migrated"
                        );

                        // Save migrated config
                        if let Err(e) = save_config(&config, &path) {
                            tracing::warn!(error = %e, "Failed to save migrated config");
                        }
                    }

                    Ok(config)
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

    /// Get the system prompt, with optional project context injection.
    /// Loads from PAWAN.md, AGENTS.md, CLAUDE.md, or .pawan/context.md.
    pub fn get_system_prompt(&self) -> String {
        let base = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        let mut prompt = base;

        if let Some((filename, ctx)) = Self::load_context_file() {
            prompt = format!("{}\n\n## Project Context (from {})\n\n{}", prompt, filename, ctx);
        }

        if let Some(skill_ctx) = Self::load_skill_context() {
            prompt = format!("{}\n\n## Active Skill (from SKILL.md)\n\n{}", prompt, skill_ctx);
        }

        prompt
    }

    /// Load project context file from current directory (if it exists).
    /// Checks PAWAN.md, AGENTS.md (cross-tool standard), CLAUDE.md, then .pawan/context.md.
    /// Returns (filename, content) of the first found file.
    fn load_context_file() -> Option<(String, String)> {
        for path in &["PAWAN.md", "AGENTS.md", "CLAUDE.md", ".pawan/context.md"] {
            let p = PathBuf::from(path);
            if p.exists() {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    if !content.trim().is_empty() {
                        return Some((path.to_string(), content));
                    }
                }
            }
        }
        None
    }

    /// Load SKILL.md files from the project using thulp-skill-files.
    /// Returns a summary of discovered skills for context injection.
    /// Sibling of `load_context_file`; only called from `get_system_prompt`.
    fn load_skill_context() -> Option<String> {
        use thulp_skill_files::SkillFile;

        let skill_path = std::path::Path::new("SKILL.md");
        if !skill_path.exists() {
            return None;
        }

        match SkillFile::parse(skill_path) {
            Ok(skill) => {
                let name = skill.effective_name();
                let desc = skill.frontmatter.description.as_deref().unwrap_or("no description");
                let tools_str = match &skill.frontmatter.allowed_tools {
                    Some(tools) => tools.join(", "),
                    None => "all".to_string(),
                };
                Some(format!(
                    "[Skill: {}] {}\nAllowed tools: {}\n---\n{}",
                    name, desc, tools_str, skill.content
                ))
            }
            Err(e) => {
                tracing::warn!("Failed to parse SKILL.md: {}", e);
                None
            }
        }
    }

    /// Resolve the effective skills repository path using the dstack pattern:
    /// env var > config field > default `~/.config/pawan/skills` > None.
    ///
    /// Returns `Some(path)` only if the resolved path exists as a directory.
    /// This allows public pawan repos to link to private skill libraries
    /// without embedding them — the path is configured per-machine.
    pub fn resolve_skills_repo(&self) -> Option<PathBuf> {
        // 1. Environment variable has highest priority
        if let Ok(env_path) = std::env::var("PAWAN_SKILLS_REPO") {
            let p = PathBuf::from(env_path);
            if p.is_dir() {
                return Some(p);
            }
            tracing::warn!(path = %p.display(), "PAWAN_SKILLS_REPO set but directory does not exist");
        }

        // 2. Config field
        if let Some(ref p) = self.skills_repo {
            if p.is_dir() {
                return Some(p.clone());
            }
            tracing::warn!(path = %p.display(), "config.skills_repo set but directory does not exist");
        }

        // 3. Default: ~/.config/pawan/skills
        if let Some(home) = dirs::home_dir() {
            let default = home.join(".config").join("pawan").join("skills");
            if default.is_dir() {
                return Some(default);
            }
        }

        None
    }

    /// Auto-discover MCP server binaries in PATH and register any that aren't
    /// already configured. Returns the names of newly-discovered servers.
    ///
    /// Supported auto-discovery targets:
    /// - `eruka-mcp` — Eruka context memory (anti-hallucination knowledge store)
    /// - `daedra` — web search across 9 backends
    /// - `deagle-mcp` — deagle code intelligence graph
    ///
    /// Existing entries in the `mcp` HashMap are never overwritten. This makes
    /// the auto-discovery idempotent and safe to call at every agent startup.
    pub fn auto_discover_mcp_servers(&mut self) -> Vec<String> {
        let mut discovered = Vec::new();

        // eruka-mcp: context memory for anti-hallucination
        if !self.mcp.contains_key("eruka") && which::which("eruka-mcp").is_ok() {
            self.mcp.insert(
                "eruka".to_string(),
                McpServerEntry {
                    command: "eruka-mcp".to_string(),
                    args: vec!["--transport".to_string(), "stdio".to_string()],
                    env: HashMap::new(),
                    enabled: true,
                },
            );
            discovered.push("eruka".to_string());
            tracing::info!("auto-discovered eruka-mcp");
        }

        // daedra: web search with 9 backends + fallback
        if !self.mcp.contains_key("daedra") && which::which("daedra").is_ok() {
            self.mcp.insert(
                "daedra".to_string(),
                McpServerEntry {
                    command: "daedra".to_string(),
                    args: vec![
                        "serve".to_string(),
                        "--transport".to_string(),
                        "stdio".to_string(),
                        "--quiet".to_string(),
                    ],
                    env: HashMap::new(),
                    enabled: true,
                },
            );
            discovered.push("daedra".to_string());
            tracing::info!("auto-discovered daedra");
        }

        // deagle-mcp: graph-backed code intelligence
        if !self.mcp.contains_key("deagle") && which::which("deagle-mcp").is_ok() {
            self.mcp.insert(
                "deagle".to_string(),
                McpServerEntry {
                    command: "deagle-mcp".to_string(),
                    args: vec!["--transport".to_string(), "stdio".to_string()],
                    env: HashMap::new(),
                    enabled: true,
                },
            );
            discovered.push("deagle".to_string());
            tracing::info!("auto-discovered deagle-mcp");
        }

        discovered
    }

    /// Discover all SKILL.md files in the configured skills repository using
    /// thulp-skill-files SkillLoader.
    ///
    /// Returns a list of `(skill_name, description, file_path)` tuples. The
    /// caller is responsible for deciding which skills to inject into the
    /// system prompt or present to the user.
    ///
    /// The skills repository is never compiled into the pawan binary — this
    /// enables the "public repo links to private skills via config" pattern
    /// used by dstack for `dirmacs/skills`.
    pub fn discover_skills_from_repo(&self) -> Vec<(String, String, PathBuf)> {
        use thulp_skill_files::SkillFile;

        let repo = match self.resolve_skills_repo() {
            Some(r) => r,
            None => return Vec::new(),
        };

        let mut results = Vec::new();
        let walker = match std::fs::read_dir(&repo) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(path = %repo.display(), error = %e, "failed to read skills repo");
                return Vec::new();
            }
        };

        for entry in walker.flatten() {
            let path = entry.path();
            // Each skill is a directory containing SKILL.md
            let skill_file = path.join("SKILL.md");
            if !skill_file.is_file() {
                continue;
            }
            match SkillFile::parse(&skill_file) {
                Ok(skill) => {
                    let name = skill.effective_name();
                    let desc = skill
                        .frontmatter
                        .description
                        .clone()
                        .unwrap_or_else(|| "(no description)".to_string());
                    results.push((name, desc, skill_file));
                }
                Err(e) => {
                    tracing::debug!(path = %skill_file.display(), error = %e, "skip unparseable skill");
                }
            }
        }

        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }

    /// Check if thinking mode should be enabled.
    /// Applicable to DeepSeek, Gemma-4, GLM, Qwen, and Mistral Small 4+ models on NIM.
    pub fn use_thinking_mode(&self) -> bool {
        self.reasoning_mode
            && (self.model.contains("deepseek")
                || self.model.contains("gemma")
                || self.model.contains("glm")
                || self.model.contains("qwen")
                || self.model.contains("mistral-small-4"))
    }
}

/// Default system prompt for coding tasks
pub const DEFAULT_SYSTEM_PROMPT: &str = r#"You are Pawan, an expert coding assistant.

# Efficiency
- Act immediately. Do NOT explore or plan before writing. Write code FIRST, then verify.
- write_file creates parents automatically. No mkdir needed.
- cargo check runs automatically after .rs writes — fix errors immediately.
- Use relative paths from workspace root.
- Missing tools are auto-installed via mise. Don't check dependencies.
- You have limited tool iterations. Be direct. No preamble.

# Tool Selection
Use the BEST tool for the job — do NOT use bash for things dedicated tools handle:
- File ops: read_file, write_file, edit_file, edit_file_lines, insert_after, append_file, list_directory
- Code intelligence: ast_grep (AST search + rewrite via tree-sitter — prefer for structural changes)
- Search: glob_search (files by pattern), grep_search (content by regex), ripgrep (native rg), fd (native find)
- Shell: bash (commands), sd (find-replace in files), mise (tool/task/env manager), zoxide (smart cd)
- Git: git_status, git_diff, git_add, git_commit, git_log, git_blame, git_branch, git_checkout, git_stash
- Agent: spawn_agent (delegate subtask), spawn_agents (parallel sub-agents)
- Web: mcp_daedra_web_search (ALWAYS use for web queries — never bash+curl)

Prefer ast_grep over edit_file for code refactors. Prefer grep_search over bash grep.
Prefer fd over bash find. Prefer sd over bash sed.

# Parallel Execution
Call multiple tools in a single response when they are independent.
If tool B depends on tool A's result, call them sequentially.
Never parallelize destructive operations (writes, deletes, commits).

# Read Before Modifying
Do NOT propose changes to code you haven't read. If asked to modify a file, read it first.
Understand existing code, patterns, and style before suggesting changes.

# Scope Discipline
Make minimal, focused changes. Follow existing code style.
- Don't add features, refactor, or "improve" code beyond what was asked.
- Don't add docstrings, comments, or type annotations to code you didn't change.
- A bug fix doesn't need surrounding code cleaned up.
- Don't add error handling for scenarios that can't happen.

# Executing Actions with Care
Consider reversibility and blast radius before acting:
- Freely take local, reversible actions (editing files, running tests).
- For hard-to-reverse actions (force-push, rm -rf, dropping tables), ask first.
- Match the scope of your actions to what was requested.
- Investigate before deleting — unfamiliar files may be the user's in-progress work.
- Don't use destructive shortcuts to bypass safety checks.

# Git Safety
- NEVER skip hooks (--no-verify) unless explicitly asked.
- ALWAYS create NEW commits rather than amending (amend after hook failure destroys work).
- NEVER force-push to main/master. Warn if requested.
- Prefer staging specific files over `git add -A` (avoids committing secrets).
- Only commit when explicitly asked. Don't be over-eager.
- Commit messages: focus on WHY, not WHAT. Use HEREDOC for multi-line messages.
- Use the git author from `git config user.name` / `git config user.email`.

# Output Style
Be concise. Lead with the answer, not the reasoning.
Focus text output on: decisions needing input, status updates, errors/blockers.
If you can say it in one sentence, don't use three.
After .rs writes, cargo check auto-runs — fix errors immediately if it fails.
Run tests when the task calls for it (cargo test -p <crate>).
One fix at a time. If it doesn't work, try a different approach."#;

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

    // --- ModelRouting tests ---

    #[test]
    fn test_route_code_signals() {
        let routing = ModelRouting {
            code: Some("code-model".into()),
            orchestrate: Some("orch-model".into()),
            execute: Some("exec-model".into()),
        };
        assert_eq!(routing.route("implement a linked list"), Some("code-model"));
        assert_eq!(routing.route("refactor the parser"), Some("code-model"));
        assert_eq!(routing.route("add test for config"), Some("code-model"));
        assert_eq!(routing.route("Write a new struct"), Some("code-model"));
    }

    #[test]
    fn test_route_orchestration_signals() {
        let routing = ModelRouting {
            code: Some("code-model".into()),
            orchestrate: Some("orch-model".into()),
            execute: Some("exec-model".into()),
        };
        assert_eq!(routing.route("analyze the error logs"), Some("orch-model"));
        assert_eq!(routing.route("review this PR"), Some("orch-model"));
        assert_eq!(routing.route("explain how the agent works"), Some("orch-model"));
        assert_eq!(routing.route("search for uses of foo"), Some("orch-model"));
    }

    #[test]
    fn test_route_execution_signals() {
        let routing = ModelRouting {
            code: Some("code-model".into()),
            orchestrate: Some("orch-model".into()),
            execute: Some("exec-model".into()),
        };
        assert_eq!(routing.route("run cargo test"), Some("exec-model"));
        assert_eq!(routing.route("execute the deploy script"), Some("exec-model"));
        assert_eq!(routing.route("build the project"), Some("exec-model"));
        assert_eq!(routing.route("commit these changes"), Some("exec-model"));
    }

    #[test]
    fn test_route_no_match_returns_none() {
        let routing = ModelRouting {
            code: Some("code-model".into()),
            orchestrate: Some("orch-model".into()),
            execute: Some("exec-model".into()),
        };
        assert_eq!(routing.route("hello world"), None);
    }

    #[test]
    fn test_route_empty_routing_returns_none() {
        let routing = ModelRouting::default();
        assert_eq!(routing.route("implement something"), None);
        assert_eq!(routing.route("search for bugs"), None);
    }

    #[test]
    fn test_route_case_insensitive() {
        let routing = ModelRouting {
            code: Some("code-model".into()),
            orchestrate: None,
            execute: None,
        };
        assert_eq!(routing.route("IMPLEMENT a FUNCTION"), Some("code-model"));
    }

    #[test]
    fn test_route_partial_routing() {
        // Only code model configured, orch/exec queries return None
        let routing = ModelRouting {
            code: Some("code-model".into()),
            orchestrate: None,
            execute: None,
        };
        assert_eq!(routing.route("implement x"), Some("code-model"));
        assert_eq!(routing.route("search for y"), None);
        assert_eq!(routing.route("run tests"), None);
    }

    // --- apply_env_overrides tests ---

    #[test]
    fn test_env_override_model() {
        let mut config = PawanConfig::default();
        std::env::set_var("PAWAN_MODEL", "custom/model-123");
        config.apply_env_overrides();
        std::env::remove_var("PAWAN_MODEL");
        assert_eq!(config.model, "custom/model-123");
    }

    #[test]
    fn test_env_override_temperature() {
        let mut config = PawanConfig::default();
        std::env::set_var("PAWAN_TEMPERATURE", "0.9");
        config.apply_env_overrides();
        std::env::remove_var("PAWAN_TEMPERATURE");
        assert!((config.temperature - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_env_override_invalid_temperature_ignored() {
        let mut config = PawanConfig::default();
        let original = config.temperature;
        std::env::set_var("PAWAN_TEMPERATURE", "not_a_number");
        config.apply_env_overrides();
        std::env::remove_var("PAWAN_TEMPERATURE");
        assert!((config.temperature - original).abs() < f32::EPSILON);
    }

    #[test]
    fn test_env_override_max_tokens() {
        let mut config = PawanConfig::default();
        std::env::set_var("PAWAN_MAX_TOKENS", "16384");
        config.apply_env_overrides();
        std::env::remove_var("PAWAN_MAX_TOKENS");
        assert_eq!(config.max_tokens, 16384);
    }

    #[test]
    fn test_env_override_fallback_models() {
        std::env::remove_var("PAWAN_FALLBACK_MODELS"); // Clean up before test
        let mut config = PawanConfig::default();
        std::env::set_var("PAWAN_FALLBACK_MODELS", "model-a, model-b, model-c");
        config.apply_env_overrides();
        std::env::remove_var("PAWAN_FALLBACK_MODELS");
        assert_eq!(config.fallback_models, vec!["model-a", "model-b", "model-c"]);
    }

    #[test]
    fn test_env_override_fallback_models_filters_empty() {
        std::env::remove_var("PAWAN_FALLBACK_MODELS"); // Clean up before test
        let mut config = PawanConfig::default();
        std::env::set_var("PAWAN_FALLBACK_MODELS", "model-a,,, model-b,");
        config.apply_env_overrides();
        std::env::remove_var("PAWAN_FALLBACK_MODELS");
        assert_eq!(config.fallback_models, vec!["model-a", "model-b"]);
    }

    #[test]
    fn test_env_override_provider_variants() {
        for (env_val, expected) in [
            ("nvidia", LlmProvider::Nvidia),
            ("nim", LlmProvider::Nvidia),
            ("ollama", LlmProvider::Ollama),
            ("openai", LlmProvider::OpenAI),
            ("mlx", LlmProvider::Mlx),
        ] {
            let mut config = PawanConfig::default();
            std::env::set_var("PAWAN_PROVIDER", env_val);
            config.apply_env_overrides();
            std::env::remove_var("PAWAN_PROVIDER");
            assert_eq!(config.provider, expected, "PAWAN_PROVIDER={} should map to {:?}", env_val, expected);
        }
    }

    // --- use_thinking_mode tests ---

    #[test]
    fn test_thinking_mode_supported_models() {
        for model in ["deepseek-ai/deepseek-r1", "google/gemma-4-31b-it", "z-ai/glm5",
                       "qwen/qwen3.5-122b", "mistralai/mistral-small-4-119b"] {
            let config = PawanConfig { model: model.into(), reasoning_mode: true, ..Default::default() };
            assert!(config.use_thinking_mode(), "thinking mode should be on for {}", model);
        }
    }

    #[test]
    fn test_thinking_mode_disabled_when_reasoning_off() {
        let config = PawanConfig { model: "deepseek-ai/deepseek-r1".into(), reasoning_mode: false, ..Default::default() };
        assert!(!config.use_thinking_mode());
    }

    #[test]
    fn test_thinking_mode_unsupported_models() {
        for model in ["meta/llama-3.1-70b", "minimaxai/minimax-m2.5", "stepfun-ai/step-3.5-flash"] {
            let config = PawanConfig { model: model.into(), reasoning_mode: true, ..Default::default() };
            assert!(!config.use_thinking_mode(), "thinking mode should be off for {}", model);
        }
    }

    // --- get_system_prompt tests ---

    #[test]
    fn test_system_prompt_default() {
        let config = PawanConfig::default();
        let prompt = config.get_system_prompt();
        assert!(prompt.contains("Pawan"), "default prompt should mention Pawan");
        assert!(prompt.contains("coding"), "default prompt should mention coding");
    }

    #[test]
    fn test_system_prompt_custom_override() {
        let config = PawanConfig { system_prompt: Some("Custom system prompt.".into()), ..Default::default() };
        let prompt = config.get_system_prompt();
        assert!(prompt.starts_with("Custom system prompt."));
    }

    // --- Config TOML parsing tests ---

    #[test]
    fn test_config_with_cloud_fallback() {
        let toml = r#"
model = "qwen/qwen3.5-122b-a10b"
[cloud]
provider = "nvidia"
model = "minimaxai/minimax-m2.5"
"#;
        let config: PawanConfig = toml::from_str(toml).expect("should parse");
        assert_eq!(config.model, "qwen/qwen3.5-122b-a10b");
        let cloud = config.cloud.unwrap();
        assert_eq!(cloud.model, "minimaxai/minimax-m2.5");
    }

    #[test]
    fn test_config_with_healing() {
        let toml = r#"
model = "test"
[healing]
fix_errors = true
fix_warnings = false
fix_tests = true
"#;
        let config: PawanConfig = toml::from_str(toml).expect("should parse");
        assert!(config.healing.fix_errors);
        assert!(!config.healing.fix_warnings);
        assert!(config.healing.fix_tests);
    }

    #[test]
    fn test_config_defaults_sensible() {
        let config = PawanConfig::default();
        assert_eq!(config.provider, LlmProvider::Nvidia);
        assert!(config.temperature > 0.0 && config.temperature <= 1.0);
        assert!(config.max_tokens > 0);
        assert!(config.max_tool_iterations > 0);
    }

    #[test]
    fn test_context_file_search_order() {
        // Verify the search list includes all expected files
        // (We test the behavior via get_system_prompt since load_context_file is private
        // and changing cwd is unsafe in parallel tests)
        let config = PawanConfig::default();
        let prompt = config.get_system_prompt();
        // In the pawan repo, PAWAN.md exists, so it should be in the prompt
        if std::path::Path::new("PAWAN.md").exists() {
            assert!(prompt.contains("Project Context"), "Should inject project context when PAWAN.md exists");
            assert!(prompt.contains("from PAWAN.md"), "Should identify source as PAWAN.md");
        }
    }

    #[test]
    fn test_system_prompt_injection_format() {
        // Verify the injection format includes the source filename
        let config = PawanConfig {
            system_prompt: Some("Base prompt.".into()),
            ..Default::default()
        };
        let prompt = config.get_system_prompt();
        // If any context file is found, it should show "from <filename>"
        if prompt.contains("Project Context") {
            assert!(prompt.contains("from "), "Injection should include source filename");
        }
    }

    // --- resolve_skills_repo tests ---

    #[test]
    fn test_resolve_skills_repo_env_var_takes_priority() {
        // PAWAN_SKILLS_REPO pointing at a real tempdir must win over the
        // config.skills_repo field (priority 1 in the resolution chain).
        let env_dir = tempfile::TempDir::new().expect("tempdir");
        let cfg_dir = tempfile::TempDir::new().expect("tempdir");

        let config = PawanConfig {
            skills_repo: Some(cfg_dir.path().to_path_buf()),
            ..Default::default()
        };

        std::env::set_var("PAWAN_SKILLS_REPO", env_dir.path());
        let resolved = config.resolve_skills_repo();
        std::env::remove_var("PAWAN_SKILLS_REPO");

        let resolved = resolved.expect("env var path should resolve to Some");
        assert_eq!(
            resolved.canonicalize().unwrap(),
            env_dir.path().canonicalize().unwrap(),
            "env var should take priority over config.skills_repo"
        );
    }

    #[test]
    fn test_resolve_skills_repo_env_var_nonexistent_falls_through() {
        // PAWAN_SKILLS_REPO pointing at a nonexistent path must be ignored
        // (warning logged) and the function continues to the next priority.
        // Here config.skills_repo is also nonexistent, and we cannot control
        // ~/.config/pawan/skills from a test, so we only assert that the
        // function does NOT panic and returns either None or the default dir
        // — crucially it does NOT return the bogus env var path.
        let bogus = PathBuf::from("/tmp/pawan-nonexistent-skills-repo-for-test-xyz123");
        assert!(!bogus.exists(), "precondition: bogus path must not exist");

        let config = PawanConfig {
            skills_repo: Some(PathBuf::from("/tmp/pawan-also-nonexistent-abc789")),
            ..Default::default()
        };

        std::env::set_var("PAWAN_SKILLS_REPO", &bogus);
        let resolved = config.resolve_skills_repo();
        std::env::remove_var("PAWAN_SKILLS_REPO");

        // Must never return the bogus path
        if let Some(ref p) = resolved {
            assert_ne!(p, &bogus, "nonexistent env var path must not be returned");
            assert!(p.is_dir(), "any returned path must be an existing directory");
        }
    }

    // --- auto_discover_mcp_servers tests ---

    #[test]
    fn test_auto_discover_mcp_is_idempotent() {
        // Two consecutive calls: first may discover some servers, second must
        // return an empty Vec (because all are already registered). The mcp
        // hashmap length must be identical between the two calls.
        let mut config = PawanConfig::default();

        let first = config.auto_discover_mcp_servers();
        let len_after_first = config.mcp.len();

        let second = config.auto_discover_mcp_servers();
        let len_after_second = config.mcp.len();

        assert!(
            second.is_empty(),
            "second call must discover nothing (got {:?})",
            second
        );
        assert_eq!(
            len_after_first, len_after_second,
            "mcp map length must not change between calls (first discovered {:?})",
            first
        );
    }

    #[test]
    fn test_auto_discover_mcp_preserves_existing_entries() {
        // Pre-populate config.mcp with a custom "eruka" entry. Even if
        // which::which("eruka-mcp") would find a binary on the test machine,
        // the existing entry MUST NOT be overwritten.
        let mut config = PawanConfig::default();
        let custom = McpServerEntry {
            command: "custom-eruka".to_string(),
            args: vec!["--custom-flag".to_string()],
            env: HashMap::new(),
            enabled: true,
        };
        config.mcp.insert("eruka".to_string(), custom);

        let discovered = config.auto_discover_mcp_servers();

        // "eruka" must not appear in the discovered list
        assert!(
            !discovered.contains(&"eruka".to_string()),
            "pre-existing 'eruka' entry must not be rediscovered, got {:?}",
            discovered
        );

        // Custom entry must be intact
        let entry = config.mcp.get("eruka").expect("eruka entry must still exist");
        assert_eq!(entry.command, "custom-eruka", "custom command must be preserved");
        assert_eq!(entry.args, vec!["--custom-flag".to_string()]);
    }

    // --- discover_skills_from_repo tests ---

    #[test]
    fn test_discover_skills_from_repo_returns_parsed_skills() {
        // Build a skills repo with one valid SKILL.md and verify that
        // discover_skills_from_repo parses it via thulp_skill_files::SkillFile.
        let repo = tempfile::TempDir::new().expect("tempdir");

        // Each skill lives in its own subdirectory containing a SKILL.md
        let skill_dir = repo.path().join("example-skill");
        std::fs::create_dir(&skill_dir).expect("mkdir example-skill");
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: example-skill\ndescription: A test skill used in pawan unit tests\n---\n# Instructions\n\nDo the thing.\n",
        )
        .expect("write SKILL.md");

        // Also drop an empty subdirectory with no SKILL.md — should be skipped
        let empty_dir = repo.path().join("not-a-skill");
        std::fs::create_dir(&empty_dir).expect("mkdir not-a-skill");

        let config = PawanConfig {
            skills_repo: Some(repo.path().to_path_buf()),
            ..Default::default()
        };

        // Ensure env var does not interfere
        std::env::remove_var("PAWAN_SKILLS_REPO");

        let skills = config.discover_skills_from_repo();
        assert_eq!(skills.len(), 1, "expected exactly 1 skill, got {:?}", skills);

        let (name, desc, path) = &skills[0];
        assert_eq!(name, "example-skill");
        assert_eq!(desc, "A test skill used in pawan unit tests");
        assert_eq!(path, &skill_md);
    }

    // ─── PawanConfig::load() edge cases (task #24) ──────────────────────

    #[test]
    fn test_load_with_explicit_pawan_toml_path() {
        // Happy path: explicit path to a valid pawan.toml
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("pawan.toml");
        std::fs::write(
            &path,
            r#"
provider = "nvidia"
model = "meta/llama-3.1-405b-instruct"
"#,
        )
        .expect("write pawan.toml");

        let config = PawanConfig::load(Some(&path)).expect("load should succeed");
        assert_eq!(config.model, "meta/llama-3.1-405b-instruct");
    }

    #[test]
    fn test_load_with_invalid_toml_returns_error() {
        // Malformed TOML should return a Config error, not panic
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("pawan.toml");
        std::fs::write(&path, "this is not [[valid] toml @@").expect("write bad toml");

        let result = PawanConfig::load(Some(&path));
        assert!(result.is_err(), "malformed TOML must return Err");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.to_lowercase().contains("parse")
                || err_msg.to_lowercase().contains("failed"),
            "error should mention parse/failed, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_load_with_nonexistent_path_returns_error() {
        // An explicit path to a file that doesn't exist must return Err,
        // not silently fall through to defaults (defaults only apply when
        // path=None and no auto-discovered config exists).
        let bogus = PathBuf::from("/tmp/definitely-does-not-exist-abc123-xyz.toml");
        let result = PawanConfig::load(Some(&bogus));
        assert!(
            result.is_err(),
            "non-existent explicit path must return Err"
        );
    }

    #[test]
    fn test_load_ares_toml_with_pawan_section() {
        // ares.toml loading must extract the [pawan] section specifically
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("ares.toml");
        std::fs::write(
            &path,
            r#"
# ares config (unrelated to pawan)
[server]
port = 3000

[pawan]
provider = "ollama"
model = "qwen3-coder:30b"
"#,
        )
        .expect("write ares.toml");

        let config = PawanConfig::load(Some(&path)).expect("ares.toml load should succeed");
        assert_eq!(config.provider, LlmProvider::Ollama);
        assert_eq!(config.model, "qwen3-coder:30b");
    }

    #[test]
    fn test_load_ares_toml_without_pawan_section_returns_defaults() {
        // ares.toml with no [pawan] section must fall back to defaults,
        // not error out. This is the common case on VPS where ares runs
        // alongside pawan but pawan has its own config elsewhere.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("ares.toml");
        std::fs::write(
            &path,
            r#"
[server]
port = 3000
workers = 4
"#,
        )
        .expect("write ares.toml without pawan section");

        let config = PawanConfig::load(Some(&path)).expect("load should succeed");
        // Should match defaults
        let defaults = PawanConfig::default();
        assert_eq!(config.provider, defaults.provider);
        assert_eq!(config.model, defaults.model);
    }

    #[test]
    fn test_load_empty_toml_file_returns_defaults() {
        // A completely empty pawan.toml is valid TOML and must parse as
        // all-defaults via serde(default). This is a common first-run case.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("pawan.toml");
        std::fs::write(&path, "").expect("write empty toml");

        let config = PawanConfig::load(Some(&path)).expect("empty toml should load");
        let defaults = PawanConfig::default();
        assert_eq!(config.provider, defaults.provider);
    }
}
    #[test]
    fn test_default_config_version() {
        assert_eq!(default_config_version(), 1);
    }

    #[test]
    fn test_default_tool_idle_timeout() {
        assert_eq!(default_tool_idle_timeout(), 300);
    }

    #[test]
    fn test_config_version_field_exists() {
        let config = PawanConfig::default();
        assert_eq!(config.config_version, 1);
    }

    #[test]
    fn test_tool_idle_timeout_field_exists() {
        let config = PawanConfig::default();
        assert_eq!(config.tool_call_idle_timeout_secs, 300);
    }

    #[test]
    fn test_migration_result_fields() {
        let result = MigrationResult {
            migrated: true,
            from_version: 0,
            to_version: 1,
            backup_path: Some(std::path::PathBuf::from("/tmp/backup.toml")),
        };
        assert!(result.migrated);
        assert_eq!(result.from_version, 0);
        assert_eq!(result.to_version, 1);
        assert!(result.backup_path.is_some());
    }

    #[test]
    fn test_migrate_to_latest_no_migration_needed() {
        let mut config = PawanConfig::default();
        config.config_version = 1; // Already at latest version
        
        let result = migrate_to_latest(&mut config, None);
        
        assert!(!result.migrated, "Should not migrate if already at latest version");
        assert_eq!(result.from_version, 1);
        assert_eq!(result.to_version, 1);
    }

    #[test]
    fn test_migrate_to_latest_performs_migration() {
        let mut config = PawanConfig::default();
        config.config_version = 0; // Old version
        
        let result = migrate_to_latest(&mut config, None);
        
        assert!(result.migrated, "Should migrate from old version");
        assert_eq!(result.from_version, 0);
        assert_eq!(result.to_version, 1);
        assert_eq!(config.config_version, 1, "Config version should be updated");
    }

    #[test]
    fn test_migrate_to_v1_adds_default_fields() {
        let mut config = PawanConfig::default();
        config.config_version = 0;
        
        let result = migrate_to_v1(&mut config);
        
        assert!(result.is_ok(), "Migration should succeed");
        assert_eq!(result.unwrap(), 1, "Should return new version");
        assert_eq!(config.config_version, 1, "Config version should be updated");
    }

    #[test]
    fn test_migration_result_no_migration() {
        let result = MigrationResult::no_migration(1);
        
        assert!(!result.migrated, "Should indicate no migration");
        assert_eq!(result.from_version, 1);
        assert_eq!(result.to_version, 1);
        assert!(result.backup_path.is_none(), "Should not have backup path");
    }

    #[test]
    fn test_migration_result_with_backup() {
        let backup_path = std::path::PathBuf::from("/tmp/backup.toml");
        let result = MigrationResult::new(0, 1, Some(backup_path.clone()));
        
        assert!(result.migrated, "Should indicate migration occurred");
        assert_eq!(result.from_version, 0);
        assert_eq!(result.to_version, 1);
        assert_eq!(result.backup_path, Some(backup_path), "Should have backup path");
    }

