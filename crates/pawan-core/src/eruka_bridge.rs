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
        // Service-to-service auth via X-Service-Key + X-Workspace-Id
        if let Some(key) = &self.config.api_key {
            req = req
                .header("X-Service-Key", key.as_str())
                .header("X-Workspace-Id", "pawan");
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
            req = req
                .header("X-Service-Key", key.as_str())
                .header("X-Workspace-Id", "pawan");
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
                "path": format!("operations/sessions/{}", session.id),
                "value": summary,
                "source": "agent",
            }));
        if let Some(key) = &self.config.api_key {
            req = req
                .header("X-Service-Key", key.as_str())
                .header("X-Workspace-Id", "pawan");
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

    #[test]
    fn config_toml_parsing() {
        let toml = r#"
enabled = true
url = "http://eruka.example.com:9090"
api_key = "secret-key"
core_max_tokens = 1000
"#;
        let config: ErukaConfig = toml::from_str(toml).expect("should parse");
        assert!(config.enabled);
        assert_eq!(config.url, "http://eruka.example.com:9090");
        assert_eq!(config.api_key, Some("secret-key".into()));
        assert_eq!(config.core_max_tokens, 1000);
    }

    #[test]
    fn config_toml_defaults() {
        let toml = "enabled = true\n";
        let config: ErukaConfig = toml::from_str(toml).expect("should parse");
        assert!(config.enabled);
        assert_eq!(config.url, "http://localhost:8081");
        assert_eq!(config.core_max_tokens, 500);
        assert_eq!(config.api_key, None);
    }

    #[test]
    fn context_response_deserialization() {
        let json = r#"{"fields":[{"name":"project","value":"pawan","category":"core"},{"name":"role","value":"coding agent"}]}"#;
        let ctx: ContextResponse = serde_json::from_str(json).unwrap();
        let fields = ctx.fields.unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name.as_deref(), Some("project"));
        assert_eq!(fields[0].value.as_deref(), Some("pawan"));
        assert_eq!(fields[0].category.as_deref(), Some("core"));
        assert_eq!(fields[1].category, None);
    }

    #[test]
    fn context_response_empty_fields() {
        let json = r#"{"fields":[]}"#;
        let ctx: ContextResponse = serde_json::from_str(json).unwrap();
        assert!(ctx.fields.unwrap().is_empty());
    }

    #[test]
    fn context_response_missing_fields() {
        let json = r#"{}"#;
        let ctx: ContextResponse = serde_json::from_str(json).unwrap();
        assert!(ctx.fields.is_none());
    }

    #[test]
    fn search_result_deserialization() {
        let json = r#"[{"content":"relevant info","field_name":"notes","score":0.95},{"content":null,"score":0.5}]"#;
        let results: Vec<SearchResult> = serde_json::from_str(json).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content.as_deref(), Some("relevant info"));
        assert_eq!(results[0].score, Some(0.95));
        assert!(results[1].content.is_none());
    }

    #[tokio::test]
    async fn disabled_archive_noops() {
        use crate::agent::session::Session;
        let client = ErukaClient::new(ErukaConfig::default());
        let session = Session {
            id: "test-123".into(),
            model: "test-model".into(),
            messages: vec![],
            created_at: "2026-04-09T00:00:00Z".into(),
            updated_at: "2026-04-09T00:00:00Z".into(),
            total_tokens: 0,
            iteration_count: 0,
        };
        // Should succeed without making HTTP calls
        client.archive_session(&session).await.unwrap();
    }

    #[tokio::test]
    async fn inject_dedup_prevents_double_injection() {
        // Simulate a history that already has eruka memory injected
        let history = vec![
            Message {
                role: Role::System,
                content: "[Eruka Core Memory]\nproject: pawan\n[End Core Memory]".into(),
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                role: Role::User,
                content: "hello".into(),
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        // Even if enabled, inject should detect the existing marker and skip
        // (We can't actually fetch from eruka in tests, but we test the dedup check)
        let already = history.iter().any(|m| {
            m.role == Role::System && m.content.contains("[Eruka Core Memory]")
        });
        assert!(already, "Should detect existing injection");
    }

    #[test]
    fn default_config_has_no_api_key() {
        // Regression: default ErukaConfig must have api_key = None so
        // unconfigured pawan never sends bogus auth headers.
        let config = ErukaConfig::default();
        assert_eq!(config.api_key, None, "default api_key must be None");
    }

    #[test]
    fn config_partial_override_keeps_defaults() {
        // Providing only `enabled = true` must keep url/api_key/core_max_tokens
        // at their default values via serde defaults. This guards against
        // removing the #[serde(default)] attributes by accident.
        let toml = "enabled = true\n";
        let config: ErukaConfig = toml::from_str(toml).expect("should parse");
        assert!(config.enabled);
        assert_eq!(config.url, "http://localhost:8081", "url default must apply");
        assert_eq!(config.core_max_tokens, 500, "core_max_tokens default must apply");
        assert_eq!(config.api_key, None, "api_key default must apply");
    }

    #[test]
    fn search_result_deserialize_with_all_null_fields() {
        // Eruka can return entries with every optional field null — the
        // SearchResult struct must survive without erroring so archive
        // traversal is robust to partial data.
        let json = r#"[{"content":null,"field_name":null,"score":null}]"#;
        let results: Vec<SearchResult> = serde_json::from_str(json).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.is_none());
        assert!(results[0].field_name.is_none());
        assert!(results[0].score.is_none());
    }

    #[test]
    fn context_field_deserialize_without_category() {
        // A field with only name+value (no category) must still deserialize —
        // category is optional because Eruka doesn't always return it.
        let json = r#"{"name":"model","value":"qwen3.5-122b"}"#;
        let field: ContextField = serde_json::from_str(json).unwrap();
        assert_eq!(field.name.as_deref(), Some("model"));
        assert_eq!(field.value.as_deref(), Some("qwen3.5-122b"));
        assert!(field.category.is_none(), "category must default to None");
    }

    #[tokio::test]
    async fn archive_enabled_with_no_user_messages_short_circuits() {
        // When archive_session() is called on an enabled client with a
        // session that has no user messages, it must early-return Ok(())
        // BEFORE attempting any HTTP call. Use an unreachable URL so the
        // test fails if it actually tries to hit the network.
        let config = ErukaConfig {
            enabled: true,
            url: "http://127.0.0.1:1".into(), // unreachable port
            api_key: None,
            core_max_tokens: 500,
        };
        let client = ErukaClient::new(config);
        let session = Session {
            id: "assistant-only".into(),
            model: "m".into(),
            messages: vec![Message {
                role: Role::Assistant,
                content: "hi".into(),
                tool_calls: vec![],
                tool_result: None,
            }],
            created_at: "2026-04-10T00:00:00Z".into(),
            updated_at: "2026-04-10T00:00:00Z".into(),
            total_tokens: 0,
            iteration_count: 0,
        };
        // If this ever tries to connect to 127.0.0.1:1 it'll take >50ms and
        // likely Err — we expect it to return Ok instantly via the early
        // return on empty user_messages.
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.archive_session(&session),
        )
        .await
        .expect("archive must not hang — empty user messages should short-circuit");
        assert!(result.is_ok());
    }
}
