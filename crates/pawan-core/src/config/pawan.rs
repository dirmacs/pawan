use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing;

use super::defaults::{default_config_version, default_tool_idle_timeout};
use super::healing::HealingConfig;
use super::mcp::McpServerEntry;
use super::migration::{migrate_to_latest, save_config};
use super::permissions::ToolPermission;
use super::prompt::DEFAULT_SYSTEM_PROMPT;
use super::provider::LlmProvider;
use super::routing::{CloudConfig, ModelRouting};
use super::target::TargetConfig;
use super::tui::TuiConfig;

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
                _ => tracing::warn!(
                    provider = provider.as_str(),
                    "Unknown PAWAN_PROVIDER, ignoring"
                ),
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
            self.fallback_models = models
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
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
        match self.get_system_prompt_checked() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to load project context for system prompt: {}", e);
                self.system_prompt
                    .clone()
                    .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string())
            }
        }
    }

    /// Checked variant of `get_system_prompt` that rejects suspicious context
    /// files with a clear error.
    pub fn get_system_prompt_checked(&self) -> crate::Result<String> {
        let base = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        let mut prompt = base;

        if let Some((filename, ctx)) = Self::load_context_file()? {
            prompt = format!(
                "{}

## Project Context (from {})

{}",
                prompt, filename, ctx
            );
        }

        if let Some(skill_ctx) = Self::load_skill_context() {
            prompt = format!(
                "{}

## Active Skill (from SKILL.md)

{}",
                prompt, skill_ctx
            );
        }

        #[cfg(feature = "memory")]
        {
            if let Ok(store) = crate::memory::MemoryStore::new_default() {
                prompt = crate::memory::inject_memory_guidance_into_prompt(prompt, &store);
            }
        }

        Ok(prompt)
    }

    fn scan_context_file(content: &str, source: &str) -> crate::Result<String> {
        // Check for suspicious patterns
        let suspicious = [
            "IGNORE ALL PREVIOUS",
            "DISREGARD ALL",
            "OVERRIDE",
            "You are now",
            "Your new role",
            "IMPORTANT: do not",
            "<system-directive>",
            "<role>",
            "<contract>",
            // Invisible unicode
            "\u{200B}",
            "\u{200C}",
            "\u{200D}",
            "\u{FEFF}",
            "\u{202E}",
            "\u{2060}",
            "\u{2061}",
            "\u{2062}",
        ];

        let upper = content.to_uppercase();
        let allow = source == "AGENTS.md" || source == "CLAUDE.md";

        for pattern in &suspicious {
            let hit = if pattern.is_ascii() {
                upper.contains(&pattern.to_uppercase())
            } else {
                content.contains(pattern)
            };

            if hit {
                tracing::warn!(source = %source, pattern = %pattern, "prompt injection pattern detected");
                if allow {
                    continue;
                }
                return Err(crate::PawanError::Config(format!(
                    "Suspicious content in {}: contains '{}'",
                    source, pattern
                )));
            }
        }

        Ok(content.to_string())
    }

    /// Load project context file from current directory (if it exists).
    /// Checks PAWAN.md, AGENTS.md (cross-tool standard), CLAUDE.md, then .pawan/context.md.
    /// Returns (filename, content) of the first found file.
    fn load_context_file() -> crate::Result<Option<(String, String)>> {
        for path in &["PAWAN.md", "AGENTS.md", "CLAUDE.md", ".pawan/context.md"] {
            let p = PathBuf::from(path);
            if p.exists() {
                let bytes = std::fs::read(&p).map_err(crate::PawanError::Io)?;
                let content = String::from_utf8(bytes).map_err(|_| {
                    crate::PawanError::Config(format!(
                        "Suspicious content in {}: file is not valid UTF-8 (binary?)",
                        path
                    ))
                })?;

                let content = Self::scan_context_file(&content, path)?;
                if !content.trim().is_empty() {
                    return Ok(Some((path.to_string(), content)));
                }
            }
        }
        Ok(None)
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
                let desc = skill
                    .frontmatter
                    .description
                    .as_deref()
                    .unwrap_or("no description");
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
