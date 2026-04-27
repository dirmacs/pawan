//! Pawan-Aegis Integration
//!
//! Reads `[pawan]` sections from aegis manifests and generates `pawan.toml`.
//! Follows the same pattern as `aegis-opencode`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Error type for pawan-aegis operations
#[derive(Debug, thiserror::Error)]
pub enum AegisError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Config error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, AegisError>;

/// Wrapper for deserializing `[pawan]` section from aegis manifest
#[derive(Debug, Deserialize)]
struct Wrapper {
    pawan: Option<PawanInput>,
}

/// Input from aegis manifest `[pawan]` section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PawanInput {
    /// Provider (nvidia, ollama, openai)
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Default model key or full model ID
    #[serde(default)]
    pub model: Option<String>,

    /// Temperature
    pub temperature: Option<f32>,
    /// Top-p
    pub top_p: Option<f32>,
    /// Max tokens
    pub max_tokens: Option<usize>,

    /// MCP servers
    #[serde(default)]
    pub mcp: HashMap<String, McpInput>,

    /// Healing config
    pub healing: Option<HealingInput>,
}

fn default_provider() -> String {
    "nvidia".to_string()
}

/// MCP server input from aegis manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpInput {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Healing input from aegis manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealingInput {
    pub fix_errors: Option<bool>,
    pub fix_warnings: Option<bool>,
    pub fix_tests: Option<bool>,
    pub auto_commit: Option<bool>,
}

/// Output pawan.toml structure
#[derive(Debug, Serialize)]
pub struct PawanToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub mcp: HashMap<String, McpInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub healing: Option<HealingInput>,
}

impl PawanInput {
    /// Load `[pawan]` section from an aegis manifest file
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let content = std::fs::read_to_string(path)?;
        let wrapper: Wrapper = toml::from_str(&content)?;
        Ok(wrapper.pawan)
    }

    /// Generate pawan.toml content from input
    pub fn generate(&self) -> Result<String> {
        let model = self
            .model
            .clone()
            .unwrap_or_else(|| "mistralai/devstral-2-123b-instruct-2512".to_string());

        let output = PawanToml {
            provider: if self.provider != "nvidia" {
                Some(self.provider.clone())
            } else {
                None
            },
            model,
            temperature: self.temperature,
            top_p: self.top_p,
            max_tokens: self.max_tokens,
            mcp: self.mcp.clone(),
            healing: self.healing.clone(),
        };

        toml::to_string_pretty(&output)
            .map_err(|e| AegisError::Config(format!("Failed to serialize pawan.toml: {}", e)))
    }

    /// Generate and write pawan.toml to a path
    pub fn write_to(&self, path: &Path) -> Result<()> {
        let content = self.generate()?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pawan_section() {
        let toml_str = r#"
[pawan]
provider = "nvidia"
model = "mistralai/devstral-2-123b-instruct-2512"
temperature = 0.6

[pawan.mcp.daedra]
command = "daedra"
args = ["serve", "--transport", "stdio", "--quiet"]
"#;

        let wrapper: Wrapper = toml::from_str(toml_str).unwrap();
        let input = wrapper.pawan.unwrap();

        assert_eq!(input.provider, "nvidia");
        assert_eq!(
            input.model.unwrap(),
            "mistralai/devstral-2-123b-instruct-2512"
        );
        assert_eq!(input.temperature.unwrap(), 0.6);
        assert!(input.mcp.contains_key("daedra"));
    }

    #[test]
    fn test_generate_pawan_toml() {
        let input = PawanInput {
            provider: "nvidia".to_string(),
            model: Some("test/model".to_string()),
            temperature: Some(0.8),
            top_p: None,
            max_tokens: Some(8192),
            mcp: HashMap::new(),
            healing: None,
        };

        let output = input.generate().unwrap();
        assert!(output.contains("test/model"));
        assert!(output.contains("0.8"));
        assert!(output.contains("8192"));
        // nvidia is default, so provider should be omitted
        assert!(!output.contains("provider"));
    }

    #[test]
    fn test_missing_pawan_section() {
        let toml_str = r#"
[some_other_section]
key = "value"
"#;

        let wrapper: Wrapper = toml::from_str(toml_str).unwrap();
        assert!(wrapper.pawan.is_none());
    }

    #[test]
    fn test_generate_with_non_default_provider_is_emitted() {
        // When provider != "nvidia", it MUST appear in the generated TOML so
        // pawan-cli picks the right backend.
        let input = PawanInput {
            provider: "ollama".into(),
            model: Some("llama3:8b".into()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            mcp: HashMap::new(),
            healing: None,
        };
        let out = input.generate().unwrap();
        assert!(
            out.contains("provider"),
            "non-nvidia provider must be emitted, got:\n{}",
            out
        );
        assert!(out.contains("ollama"));
        assert!(out.contains("llama3:8b"));
    }

    #[test]
    fn test_generate_with_no_model_falls_back_to_default() {
        // When model = None, generate() must fall back to the hardcoded
        // devstral default. Regression guard on the fallback path.
        let input = PawanInput {
            provider: "nvidia".into(),
            model: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            mcp: HashMap::new(),
            healing: None,
        };
        let out = input.generate().unwrap();
        assert!(
            out.contains("mistralai/devstral-2-123b-instruct-2512"),
            "default model should be devstral fallback, got:\n{}",
            out
        );
    }

    #[test]
    fn test_generate_with_healing_section_emits_fields() {
        let input = PawanInput {
            provider: "nvidia".into(),
            model: Some("m".into()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            mcp: HashMap::new(),
            healing: Some(HealingInput {
                fix_errors: Some(true),
                fix_warnings: Some(false),
                fix_tests: Some(true),
                auto_commit: Some(false),
            }),
        };
        let out = input.generate().unwrap();
        assert!(out.contains("[healing]") || out.contains("healing"));
        assert!(out.contains("fix_errors"));
        assert!(out.contains("fix_tests"));
    }

    #[test]
    fn test_load_then_generate_roundtrip() {
        // Write a TOML manifest with a [pawan] section to a temp file,
        // load it via PawanInput::load, then generate and verify the
        // loaded model/provider/mcp all survive. Uses std::env::temp_dir
        // + unique filename to avoid a dev-dep on tempfile.
        let mut path = std::env::temp_dir();
        path.push(format!("pawan-aegis-roundtrip-{}.toml", std::process::id()));
        let manifest = r#"
[pawan]
provider = "nvidia"
model = "roundtrip/model"
temperature = 0.42

[pawan.mcp.daedra]
command = "daedra"
args = ["serve"]
enabled = true
"#;
        std::fs::write(&path, manifest).unwrap();

        let loaded = PawanInput::load(&path)
            .unwrap()
            .expect("pawan section present");
        assert_eq!(loaded.provider, "nvidia");
        assert_eq!(loaded.model.as_deref(), Some("roundtrip/model"));
        assert_eq!(loaded.temperature, Some(0.42));
        assert!(loaded.mcp.contains_key("daedra"));
        assert!(loaded.mcp["daedra"].enabled);

        // Regenerating should include the model, though nvidia-default
        // provider is omitted.
        let generated = loaded.generate().unwrap();
        assert!(generated.contains("roundtrip/model"));
        assert!(generated.contains("daedra"));

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_mcp_input_enabled_defaults_to_true_when_omitted() {
        // An MCP entry without explicit `enabled` must default to true
        // (via default_true helper). Regression guard.
        let toml_str = r#"
[pawan]
provider = "nvidia"

[pawan.mcp.svc]
command = "svc"
"#;
        let wrapper: Wrapper = toml::from_str(toml_str).unwrap();
        let input = wrapper.pawan.unwrap();
        assert!(
            input.mcp["svc"].enabled,
            "mcp.svc.enabled should default to true"
        );
        assert!(input.mcp["svc"].args.is_empty(), "args default is empty");
        assert!(input.mcp["svc"].env.is_empty(), "env default is empty");
    }
}
