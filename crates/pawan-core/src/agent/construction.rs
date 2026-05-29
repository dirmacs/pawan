//! PawanAgent construction — factory, backend selection, and builder methods.

use super::PawanAgent;
use crate::config::{LlmProvider, PawanConfig};
use crate::credentials;
use crate::tools::ToolRegistry;
use crate::{PawanError, Result};
use super::backend::openai_compat::{OpenAiCompatBackend, OpenAiCompatConfig};
use super::backend::LlmBackend;
use std::path::PathBuf;

pub(crate) fn probe_local_endpoint(url: &str) -> bool {
    use std::net::TcpStream;
    use std::time::Duration;

    // Strip scheme and path — we only need host:port
    let hostport = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("");

    // Ensure port is present; default http → 80, https → 443
    let addr = if hostport.contains(':') {
        hostport.to_string()
    } else if url.starts_with("https://") {
        format!("{hostport}:443")
    } else {
        format!("{hostport}:80")
    };

    // Normalise "localhost" → "127.0.0.1" so we don't accidentally resolve
    // to ::1 (IPv6) when the listener is bound only to IPv4.
    let addr = addr.replace("localhost", "127.0.0.1");

    let socket_addr = match addr.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };

    TcpStream::connect_timeout(&socket_addr, Duration::from_millis(100)).is_ok()
}

/// Retrieve an API key with fallback chain:
/// 1. Environment variable
/// 2. Secure credential store
/// 3. Return None (caller should prompt user)
///
/// If the key is found in the secure store, it's also set as an env var
/// for subsequent calls.
pub(crate) fn get_api_key_with_secure_fallback(env_var: &str, key_name: &str) -> Option<String> {
    // First, check environment variable
    if let Ok(key) = std::env::var(env_var) {
        return Some(key);
    }

    // Second, try secure credential store
    match credentials::get_api_key(key_name) {
        Ok(Some(key)) => {
            // Cache in env var for subsequent calls
            std::env::set_var(env_var, &key);
            Some(key)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("Failed to retrieve {} from secure store: {}", key_name, e);
            None
        }
    }
}

/// Prompt user to enter an API key and store it securely.
///
/// This function:
/// 1. Prompts the user to enter the API key
/// 2. Stores it in the secure credential store
/// 3. Sets it as an environment variable for the current session
///
/// Returns the entered key on success, or None if the user cancels.
fn prompt_and_store_api_key(env_var: &str, key_name: &str, provider: &str) -> Option<String> {
    eprintln!("\n🔑 {} API key not found.", provider);
    eprintln!("You can set it via:");
    eprintln!("  - Environment variable: export {}=<your-key>", env_var);
    eprintln!("  - Interactive entry (recommended for security)");
    eprintln!("\nEnter your {} API key:", provider);
    eprintln!("  (Your key will be stored securely in the OS credential store)\n");

    // Read input securely (no echo)
    #[cfg(unix)]
    let key = {
        use std::io::{self, Write};

        // Use termios to disable echo on Unix
        let mut stdout = io::stdout();
        stdout.flush().ok();

        // Read password without echo
        rpassword::prompt_password("> ").ok()
    };

    #[cfg(windows)]
    let key = {
        use std::io::{self, Write};

        let mut stdout = io::stdout();
        stdout.flush().ok();

        // On Windows, use a simple prompt (rpassword handles this)
        rpassword::prompt_password("> ").ok()
    };

    #[cfg(not(any(unix, windows)))]
    let key = {
        use std::io::{self, BufRead, Write};

        let mut stdout = io::stdout();
        let mut stdin = io::stdin();
        stdout.flush().ok();
        print!("> ");
        stdout.flush().ok();

        let mut input = String::new();
        stdin.lock().read_line(&mut input).ok();
        Some(input.trim().to_string())
    };

    match key {
        Some(k) if !k.trim().is_empty() => {
            let key = k.trim().to_string();

            // Store in secure credential store
            match credentials::store_api_key(key_name, &key) {
                Ok(()) => {
                    tracing::info!("{} API key stored securely", provider);
                    std::env::set_var(env_var, &key);
                    Some(key)
                }
                Err(e) => {
                    tracing::warn!("Failed to store key securely: {}. Using session-only.", e);
                    std::env::set_var(env_var, &key);
                    Some(key)
                }
            }
        }
        _ => {
            eprintln!(
                "\n⚠️  No key entered. {} will not work until a key is set.",
                provider
            );
            None
        }
    }
}

pub(crate) fn scan_context_file(content: &str, source: &str) -> Result<String> {
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
    let allow = source.ends_with("AGENTS.md") || source.ends_with("CLAUDE.md");

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
            return Err(PawanError::Config(format!(
                "Suspicious content in {}: contains '{}'",
                source, pattern
            )));
        }
    }
    Ok(content.to_string())
}

/// Load per-turn architecture context from `<workspace_root>/.pawan/arch.md`.
///
/// Returns `None` if the file is absent or empty.
/// Caps content at 2 000 chars to avoid context bloat from large files;
/// an ellipsis marker is appended when truncation occurs.
pub(crate) fn load_arch_context(workspace_root: &std::path::Path) -> Result<Option<String>> {
    let path = workspace_root.join(".pawan").join("arch.md");
    if !path.exists() {
        return Ok(None);
    }

    let bytes = std::fs::read(&path).map_err(PawanError::Io)?;
    let content = String::from_utf8(bytes).map_err(|_| {
        PawanError::Config(
            "Suspicious content in .pawan/arch.md: file is not valid UTF-8 (binary?)".to_string(),
        )
    })?;

    if content.trim().is_empty() {
        return Ok(None);
    }

    let content = scan_context_file(&content, ".pawan/arch.md")?;

    const MAX_CHARS: usize = 2_000;
    if content.len() > MAX_CHARS {
        // Truncate on a char boundary
        let boundary = content
            .char_indices()
            .map(|(i, _)| i)
            .nth(MAX_CHARS)
            .unwrap_or(content.len());
        Ok(Some(format!("{}…(truncated)", &content[..boundary])))
    } else {
        Ok(Some(content))
    }
}

impl PawanAgent {
    /// Create a new PawanAgent with auto-selected backend
    pub fn new(config: PawanConfig, workspace_root: PathBuf) -> Self {
        let tools = ToolRegistry::with_defaults(workspace_root.clone());
        let system_prompt = config.get_system_prompt();
        let backend = Self::create_backend(&config, &system_prompt);
        let eruka = if config.eruka.enabled {
            Some(crate::eruka_bridge::ErukaClient::new(config.eruka.clone()))
        } else {
            None
        };
        let (arch_context, arch_context_error) = match load_arch_context(&workspace_root) {
            Ok(v) => (v, None),
            Err(e) => (None, Some(e.to_string())),
        };

        Self {
            config,
            tools,
            history: Vec::new(),
            workspace_root,
            backend,
            context_tokens_estimate: 0,
            eruka,
            session_id: uuid::Uuid::new_v4().to_string(),
            arch_context,
            arch_context_error,
            last_tool_call_time: None,
        }
    }

    /// Create the appropriate backend based on config.
    ///
    /// If `use_ares_backend` is true and the `ares` feature is compiled in,
    /// delegates to ares-server's LLMClient (unified provider abstraction with
    /// connection pooling). Otherwise uses pawan's built-in OpenAI-compatible
    /// backend (the original path).
    pub(crate) fn create_backend(config: &PawanConfig, system_prompt: &str) -> Box<dyn LlmBackend> {
        // Local-inference-first cost guard: if enabled and the local server
        // responds within 100 ms, route all traffic there instead of cloud.
        if config.local_first {
            let local_url = config
                .local_endpoint
                .clone()
                .unwrap_or_else(|| "http://localhost:11434/v1".to_string());
            if probe_local_endpoint(&local_url) {
                tracing::info!(
                    url = %local_url,
                    model = %config.model,
                    "local_first: local server reachable, using local inference"
                );
                return Box::new(OpenAiCompatBackend::new(
                    super::backend::openai_compat::OpenAiCompatConfig {
                        api_url: local_url,
                        api_key: None,
                        model: config.model.clone(),
                        temperature: config.temperature,
                        top_p: config.top_p,
                        max_tokens: config.max_tokens,
                        system_prompt: system_prompt.to_string(),
                        use_thinking: false,
                        max_retries: config.max_retries,
                        fallback_models: Vec::new(),
                        cloud: None,
                    },
                ));
            }
            tracing::info!(
                url = %local_url,
                "local_first: local server unreachable, falling back to cloud provider"
            );
        }

        // Try ares backend first if requested
        if config.use_ares_backend {
            if let Some(backend) = Self::try_create_ares_backend(config, system_prompt) {
                return backend;
            }
            tracing::warn!(
                "use_ares_backend=true but ares backend creation failed; \
                 falling back to pawan's native backend"
            );
        }

        match config.provider {
            LlmProvider::Nvidia | LlmProvider::OpenAI | LlmProvider::Mlx => {
                let (api_url, api_key) = match config.provider {
                    LlmProvider::Nvidia => {
                        let url = std::env::var("NVIDIA_API_URL")
                            .unwrap_or_else(|_| crate::DEFAULT_NVIDIA_API_URL.to_string());

                        // Try to get key from env or secure store
                        let key =
                            get_api_key_with_secure_fallback("NVIDIA_API_KEY", "nvidia_api_key");

                        // If no key found, prompt user (skip interactive prompts in unit tests)
                        let key = if key.is_some() {
                            key
                        } else if cfg!(test) {
                            Some("pawan-test-dummy-key".to_string())
                        } else {
                            prompt_and_store_api_key("NVIDIA_API_KEY", "nvidia_api_key", "NVIDIA")
                        };

                        if key.is_none() {
                            tracing::warn!("NVIDIA_API_KEY not set. Model calls will fail until a key is provided.");
                        }
                        (url, key)
                    }
                    LlmProvider::OpenAI => {
                        let url = config
                            .base_url
                            .clone()
                            .or_else(|| std::env::var("OPENAI_API_URL").ok())
                            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

                        let key =
                            get_api_key_with_secure_fallback("OPENAI_API_KEY", "openai_api_key");
                        let key = if key.is_some() {
                            key
                        } else if cfg!(test) {
                            Some("pawan-test-dummy-key".to_string())
                        } else {
                            prompt_and_store_api_key("OPENAI_API_KEY", "openai_api_key", "OpenAI")
                        };

                        (url, key)
                    }
                    LlmProvider::Mlx => {
                        // MLX LM server — Apple Silicon native, always local
                        let url = config
                            .base_url
                            .clone()
                            .unwrap_or_else(|| "http://localhost:8080/v1".to_string());
                        tracing::info!(url = %url, "Using MLX LM server (Apple Silicon native)");
                        (url, None) // mlx_lm.server requires no API key
                    }
                    _ => unreachable!(),
                };

                // Build cloud fallback if configured
                let cloud = config.cloud.as_ref().map(|c| {
                    let (cloud_url, cloud_key) = match c.provider {
                        LlmProvider::Nvidia => {
                            let url = std::env::var("NVIDIA_API_URL")
                                .unwrap_or_else(|_| crate::DEFAULT_NVIDIA_API_URL.to_string());
                            let key = get_api_key_with_secure_fallback(
                                "NVIDIA_API_KEY",
                                "nvidia_api_key",
                            );
                            (url, key)
                        }
                        LlmProvider::OpenAI => {
                            let url = std::env::var("OPENAI_API_URL")
                                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
                            let key = get_api_key_with_secure_fallback(
                                "OPENAI_API_KEY",
                                "openai_api_key",
                            );
                            (url, key)
                        }
                        LlmProvider::Mlx => ("http://localhost:8080/v1".to_string(), None),
                        _ => {
                            tracing::warn!(
                                "Cloud fallback only supports nvidia/openai/mlx providers"
                            );
                            ("https://integrate.api.nvidia.com/v1".to_string(), None)
                        }
                    };
                    super::backend::openai_compat::CloudFallback {
                        api_url: cloud_url,
                        api_key: cloud_key,
                        model: c.model.clone(),
                        fallback_models: c.fallback_models.clone(),
                    }
                });

                Box::new(OpenAiCompatBackend::new(OpenAiCompatConfig {
                    api_url,
                    api_key,
                    model: config.model.clone(),
                    temperature: config.temperature,
                    top_p: config.top_p,
                    max_tokens: config.max_tokens,
                    system_prompt: system_prompt.to_string(),
                    // Enforce thinking budget: if set, disable thinking entirely
                    // and give all tokens to action output
                    use_thinking: config.thinking_budget == 0 && config.use_thinking_mode(),
                    max_retries: config.max_retries,
                    fallback_models: config.fallback_models.clone(),
                    cloud,
                }))
            }
            LlmProvider::Ollama => {
                let url = std::env::var("OLLAMA_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());

                Box::new(super::backend::ollama::OllamaBackend::new(
                    url,
                    config.model.clone(),
                    config.temperature,
                    system_prompt.to_string(),
                ))
            }
        }
    }

    /// Try to construct an ares-backed LLM backend from pawan config.
    /// Returns `None` if the provider isn't supported by ares or required
    /// credentials are missing — the caller should fall back to pawan's
    /// native backend.
    fn try_create_ares_backend(
        config: &PawanConfig,
        system_prompt: &str,
    ) -> Option<Box<dyn LlmBackend>> {
        use ares::llm::client::{ModelParams, Provider};

        // Map pawan LlmProvider → ares Provider variants.
        // ares supports: OpenAI (with custom base_url), Ollama, LlamaCpp, Anthropic.
        // Pawan's Nvidia/OpenAI/Mlx all use OpenAI-compatible endpoints, so they
        // all map to ares Provider::OpenAI with different base URLs.
        let params = ModelParams {
            temperature: Some(config.temperature),
            max_tokens: Some(config.max_tokens as u32),
            top_p: Some(config.top_p),
            frequency_penalty: None,
            presence_penalty: None,
        };

        let provider = match config.provider {
            LlmProvider::Nvidia => {
                let api_base = std::env::var("NVIDIA_API_URL")
                    .unwrap_or_else(|_| crate::DEFAULT_NVIDIA_API_URL.to_string());
                let api_key = std::env::var("NVIDIA_API_KEY").ok()?;
                Provider::OpenAI {
                    api_key,
                    api_base,
                    model: config.model.clone(),
                    params,
                }
            }
            LlmProvider::OpenAI => {
                let api_base = config
                    .base_url
                    .clone()
                    .or_else(|| std::env::var("OPENAI_API_URL").ok())
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
                Provider::OpenAI {
                    api_key,
                    api_base,
                    model: config.model.clone(),
                    params,
                }
            }
            LlmProvider::Mlx => {
                // MLX LM server is OpenAI-compatible, no API key needed
                let api_base = config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8080/v1".to_string());
                Provider::OpenAI {
                    api_key: String::new(),
                    api_base,
                    model: config.model.clone(),
                    params,
                }
            }
            LlmProvider::Ollama => {
                // Ares Ollama client is async-constructed (async with_params),
                // which doesn't fit pawan's sync PawanAgent::new path.
                // Fall back to pawan's native OllamaBackend for now.
                return None;
            }
        };

        // OpenAI variants construct synchronously — we skip the async
        // Provider::create_client() entirely for sync construction.
        let client: Box<dyn ares::llm::LLMClient> = match provider {
            Provider::OpenAI {
                api_key,
                api_base,
                model,
                params,
            } => Box::new(ares::llm::openai::OpenAIClient::with_params(
                api_key, api_base, model, params,
            )),
            _ => return None,
        };

        tracing::info!(
            provider = ?config.provider,
            model = %config.model,
            "Using ares-backed LLM backend"
        );

        Some(Box::new(super::backend::ares_backend::AresBackend::new(
            client,
            system_prompt.to_string(),
        )))
    }

    /// Create with a specific tool registry
    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    /// Get mutable access to the tool registry (for registering MCP tools)
    pub fn tools_mut(&mut self) -> &mut ToolRegistry {
        &mut self.tools
    }

    /// Create with a custom backend
    pub fn with_backend(mut self, backend: Box<dyn LlmBackend>) -> Self {
        self.backend = backend;
        self
    }
}
