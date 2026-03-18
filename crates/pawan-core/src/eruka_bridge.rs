//! Eruka bridge — connect Eruka's 3-tier memory to pawan's context window
//!
//! When enabled, pawan injects Core memory before each LLM call and
//! archives completed sessions to Eruka's Archival tier.
//!
//! This wires the DIRMACS context engine into the coding agent.

use crate::agent::{Message, Role};
use crate::agent::session::Session;
use crate::{PawanError, Result};
use serde::{Deserialize, Serialize};

/// Eruka client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErukaConfig {
    /// Whether Eruka integration is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Eruka API URL (default: http://localhost:8081)
    #[serde(default = "default_eruka_url")]
    pub url: String,
    /// API key for authentication (optional, depends on Eruka auth setup)
    #[serde(default)]
    pub api_key: Option<String>,
    /// Max tokens for core memory injection
    #[serde(default = "default_core_max_tokens")]
    pub core_max_tokens: usize,
}

fn default_eruka_url() -> String {
    "http://localhost:8081".into()
}

fn default_core_max_tokens() -> usize {
    500
}

impl Default for ErukaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: default_eruka_url(),
            api_key: None,
            core_max_tokens: default_core_max_tokens(),
        }
    }
}

/// Eruka HTTP client
pub struct ErukaClient {
    config: ErukaConfig,
    http: reqwest::Client,
}

/// Search result from Eruka
#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub content: Option<String>,
    pub field_name: Option<String>,
    pub score: Option<f64>,
}

/// Context response from Eruka
#[derive(Debug, Deserialize)]
pub struct ContextResponse {
    pub fields: Option<Vec<ContextField>>,
}

#[derive(Debug, Deserialize)]
pub struct ContextField {
    pub name: Option<String>,
    pub value: Option<String>,
    pub category: Option<String>,
}

impl ErukaClient {
    /// Create a new Eruka client
    pub fn new(config: ErukaConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Check if Eruka integration is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Fetch core memory from Eruka and build a system message
    pub async fn fetch_core_memory(&self) -> Result<Option<String>> {
        if !self.config.enabled {
            return Ok(None);
        }

        let url = format!("{}/api/v1/context", self.config.url);
        let mut req = self.http.get(&url);
        if let Some(key) = &self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req.send().await.map_err(|e| {
            tracing::warn!("Eruka context fetch failed: {}", e);
            PawanError::Agent(format!("Eruka: {}", e))
        })?;

        if !resp.status().is_success() {
            tracing::warn!("Eruka returned {}", resp.status());
            return Ok(None);
        }

        let body = resp.text().await.map_err(|e| {
            PawanError::Agent(format!("Eruka body: {}", e))
        })?;

        // Parse context fields and build memory string
        if let Ok(ctx) = serde_json::from_str::<ContextResponse>(&body) {
            if let Some(fields) = ctx.fields {
                let memory: Vec<String> = fields
                    .iter()
                    .filter_map(|f| {
                        let name = f.name.as_deref()?;
                        let value = f.value.as_deref()?;
                        Some(format!("{}: {}", name, value))
                    })
                    .collect();

                if memory.is_empty() {
                    return Ok(None);
                }

                // Truncate to core_max_tokens worth of chars (~4 chars/token)
                let max_chars = self.config.core_max_tokens * 4;
                let joined = memory.join("\n");
                let truncated: String = joined.chars().take(max_chars).collect();

                return Ok(Some(format!(
                    "[Eruka Core Memory]\n{}\n[End Core Memory]",
                    truncated
                )));
            }
        }

        // Fallback: try to use raw body if it's text
        if !body.is_empty() && body.len() < self.config.core_max_tokens * 4 {
            return Ok(Some(format!(
                "[Eruka Core Memory]\n{}\n[End Core Memory]",
                body
            )));
        }

        Ok(None)
    }

    /// Inject core memory into conversation history as a system message
    pub async fn inject_core_memory(&self, history: &mut Vec<Message>) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        if let Some(memory) = self.fetch_core_memory().await? {
            // Check if we already injected (avoid duplicates across iterations)
            let already_injected = history.iter().any(|m| {
                m.role == Role::System && m.content.contains("[Eruka Core Memory]")
            });

            if !already_injected {
                history.insert(
                    0,
                    Message {
                        role: Role::System,
                        content: memory,
                        tool_calls: vec![],
                        tool_result: None,
                    },
                );
                tracing::info!("Injected Eruka core memory into context");
            }
        }

        Ok(())
    }

    /// Search Eruka's archival memory for context relevant to a query
    pub async fn search_archival(&self, query: &str) -> Result<Vec<String>> {
        if !self.config.enabled {
            return Ok(vec![]);
        }

        let url = format!("{}/api/v1/context/search", self.config.url);
        let mut req = self.http
            .post(&url)
            .json(&serde_json::json!({"query": query, "limit": 5}));
        if let Some(key) = &self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req.send().await.map_err(|e| {
            tracing::warn!("Eruka search failed: {}", e);
            PawanError::Agent(format!("Eruka search: {}", e))
        })?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let body = resp.text().await.unwrap_or_default();
        if let Ok(results) = serde_json::from_str::<Vec<SearchResult>>(&body) {
            Ok(results
                .into_iter()
                .filter_map(|r| r.content)
                .collect())
        } else {
            Ok(vec![])
        }
    }

    /// Archive a completed session to Eruka's context store
    pub async fn archive_session(&self, session: &Session) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Build a summary of the session for archival
        let user_messages: Vec<&str> = session
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .collect();

        let assistant_messages: Vec<&str> = session
            .messages
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .map(|m| m.content.as_str())
            .collect();

        if user_messages.is_empty() {
            return Ok(());
        }

        let summary = format!(
            "Session {} (model: {}, {} messages)\nUser topics: {}\nAssistant summary: {}",
            session.id,
            session.model,
            session.messages.len(),
            user_messages.join(" | "),
            assistant_messages.last().map(|s| {
                let trunc: String = s.chars().take(500).collect();
                trunc
            }).unwrap_or_default(),
        );

        let url = format!("{}/api/v1/context", self.config.url);
        let mut req = self.http
            .post(&url)
            .json(&serde_json::json!({
                "field_name": format!("pawan_session_{}", session.id),
                "value": summary,
                "category": "operations",
                "source": "pawan",
            }));
        if let Some(key) = &self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        match req.send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    tracing::info!("Archived session {} to Eruka", session.id);
                } else {
                    tracing::warn!("Eruka archive returned {}", resp.status());
                }
            }
            Err(e) => {
                tracing::warn!("Eruka archive failed (non-fatal): {}", e);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_disabled() {
        let config = ErukaConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.url, "http://localhost:8081");
        assert_eq!(config.core_max_tokens, 500);
    }

    #[test]
    fn client_respects_enabled() {
        let config = ErukaConfig::default();
        let client = ErukaClient::new(config);
        assert!(!client.is_enabled());
    }

    #[tokio::test]
    async fn disabled_client_noops() {
        let client = ErukaClient::new(ErukaConfig::default());
        let mut history = vec![];
        client.inject_core_memory(&mut history).await.unwrap();
        assert!(history.is_empty());

        let results = client.search_archival("test").await.unwrap();
        assert!(results.is_empty());
    }
}
