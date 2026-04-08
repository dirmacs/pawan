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
}
