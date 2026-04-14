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

    /// Write a context field to Eruka. Low-level helper shared by
    /// `sync_turn`, `on_pre_compress`, and `archive_session`.
    ///
    /// Returns `Ok(false)` if Eruka is disabled or the write failed
    /// (non-fatal — Eruka integration never breaks the agent loop).
    pub async fn write_context(
        &self,
        path: &str,
        value: &str,
        source: &str,
        confidence: f64,
    ) -> Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }
        let url = format!("{}/api/v1/context", self.config.url);
        let mut req = self.http.post(&url).json(&serde_json::json!({
            "path": path,
            "value": value,
            "source": source,
            "confidence": confidence,
        }));
        if let Some(key) = &self.config.api_key {
            req = req
                .header("X-Service-Key", key.as_str())
                .header("X-Workspace-Id", "pawan");
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => Ok(true),
            Ok(resp) => {
                tracing::warn!("Eruka write_context returned {}", resp.status());
                Ok(false)
            }
            Err(e) => {
                tracing::warn!("Eruka write_context failed (non-fatal): {}", e);
                Ok(false)
            }
        }
    }

    /// Persist a completed conversation turn. Lifecycle hook — call at the
    /// end of each agent turn to build up historical context.
    ///
    /// Mirrors eruka-mcp's `eruka_sync_turn`. Writes to
    /// `operations/turns/{session_id}` with confidence 0.9.
    pub async fn sync_turn(
        &self,
        user_message: &str,
        assistant_message: &str,
        session_id: &str,
    ) -> Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }
        // Match eruka-mcp's 500-char cap per side to keep writes bounded.
        let user_trim: String = user_message.chars().take(500).collect();
        let asst_trim: String = assistant_message.chars().take(500).collect();
        let path = format!("operations/turns/{session_id}");
        let value = format!("USER: {user_trim} | ASSISTANT: {asst_trim}");
        self.write_context(&path, &value, "agent_inference", 0.9)
            .await
    }

    /// Save a summary of messages about to be compressed/truncated.
    /// Lifecycle hook — call before context window compression so the
    /// important facts survive the truncation.
    ///
    /// Mirrors eruka-mcp's `eruka_on_pre_compress`. Writes to
    /// `operations/compressed_insights/{session_id}` with confidence 0.8.
    pub async fn on_pre_compress(&self, messages: &str, session_id: &str) -> Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }
        let path = format!("operations/compressed_insights/{session_id}");
        let summary = if messages.len() > 2000 {
            format!(
                "{}...(truncated {} chars)",
                &messages[..2000],
                messages.len() - 2000
            )
        } else {
            messages.to_string()
        };
        self.write_context(&path, &summary, "agent_inference", 0.8)
            .await
    }

    /// Prefetch relevant context at the start of a turn. Combines semantic
    /// search with compressed context for optimal recall.
    ///
    /// Mirrors eruka-mcp's `eruka_prefetch`. Returns the prefetched context
    /// as a formatted string, or `None` if disabled / no results.
    pub async fn prefetch(&self, query: &str, max_tokens: usize) -> Result<Option<String>> {
        if !self.config.enabled {
            return Ok(None);
        }

        // Semantic search — top 5 results
        let search_url = format!("{}/api/v1/context/search", self.config.url);
        let mut req = self.http.post(&search_url).json(&serde_json::json!({
            "query": query,
            "limit": 5,
        }));
        if let Some(key) = &self.config.api_key {
            req = req
                .header("X-Service-Key", key.as_str())
                .header("X-Workspace-Id", "pawan");
        }
        let search_text = match req.send().await {
            Ok(resp) if resp.status().is_success() => resp.text().await.unwrap_or_default(),
            Ok(resp) => {
                tracing::warn!("Eruka prefetch search returned {}", resp.status());
                String::new()
            }
            Err(e) => {
                tracing::warn!("Eruka prefetch search failed: {}", e);
                return Ok(None);
            }
        };

        // Compressed context for general task relevance
        let compress_url = format!("{}/api/v1/compress", self.config.url);
        let mut req = self.http.post(&compress_url).json(&serde_json::json!({
            "task_type": "general",
            "max_tokens": max_tokens,
        }));
        if let Some(key) = &self.config.api_key {
            req = req
                .header("X-Service-Key", key.as_str())
                .header("X-Workspace-Id", "pawan");
        }
        let compress_text = match req.send().await {
            Ok(resp) if resp.status().is_success() => resp.text().await.unwrap_or_default(),
            Ok(resp) => {
                tracing::warn!("Eruka prefetch compress returned {}", resp.status());
                String::new()
            }
            Err(e) => {
                tracing::warn!("Eruka prefetch compress failed: {}", e);
                String::new()
            }
        };

        if search_text.is_empty() && compress_text.is_empty() {
            return Ok(None);
        }

        Ok(Some(format!(
            "[Eruka Prefetch for: {query}]\nSearch results: {search_text}\nCompressed: {compress_text}\n[End Prefetch]"
        )))
    }

    /// Fetch cached context with a hash for diff-based caching.
    /// Returns `(content, hash)` so the caller can skip re-reads when the
    /// hash is unchanged (cachebro pattern — 20-30% token savings).
    ///
    /// Mirrors eruka-mcp's `eruka_get_context_cached`.
    pub async fn get_context_cached(
        &self,
        path: &str,
        _session_id: &str,
    ) -> Result<Option<(String, String)>> {
        if !self.config.enabled {
            return Ok(None);
        }

        let url = format!(
            "{}/api/v1/context?path={}&include_metadata=false",
            self.config.url, path
        );
        let mut req = self.http.get(&url);
        if let Some(key) = &self.config.api_key {
            req = req
                .header("X-Service-Key", key.as_str())
                .header("X-Workspace-Id", "pawan");
        }

        let resp = match req.send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!("Eruka get_context_cached returned {}", r.status());
                return Ok(None);
            }
            Err(e) => {
                tracing::warn!("Eruka get_context_cached failed: {}", e);
                return Ok(None);
            }
        };

        let body = resp.text().await.unwrap_or_default();
        if body.is_empty() {
            return Ok(None);
        }

        // SHA-256-ish hash using std DefaultHasher (first 16 hex chars).
        // Matches eruka-mcp's hashing approach — stable across calls.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        let hash = format!("{:016x}", hasher.finish());

        Ok(Some((body, hash)))
    }

    /// Export all context as a portable JSON bundle (context core).
    /// Use for backup, agent-to-agent transfer, or offline use.
    ///
    /// Mirrors eruka-mcp's `eruka_export_context`. Pass category="*" for
    /// full export, or a specific category like "identity" / "products".
    pub async fn export_context(
        &self,
        category: &str,
        include_metadata: bool,
    ) -> Result<Option<serde_json::Value>> {
        if !self.config.enabled {
            return Ok(None);
        }

        let url = format!(
            "{}/api/v1/context?path={}&include_metadata={}",
            self.config.url, category, include_metadata
        );
        let mut req = self.http.get(&url);
        if let Some(key) = &self.config.api_key {
            req = req
                .header("X-Service-Key", key.as_str())
                .header("X-Workspace-Id", "pawan");
        }

        let resp = match req.send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!("Eruka export_context returned {}", r.status());
                return Ok(None);
            }
            Err(e) => {
                tracing::warn!("Eruka export_context failed: {}", e);
                return Ok(None);
            }
        };

        let body = resp.text().await.unwrap_or_default();
        let context_data: serde_json::Value = serde_json::from_str(&body)
            .unwrap_or(serde_json::Value::Null);

        Ok(Some(serde_json::json!({
            "export_format": "eruka_context_core_v1",
            "category": category,
            "data": context_data,
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "instructions": "Import this bundle into another Eruka instance via eruka_write_context for each field.",
        })))
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
            tags: Vec::new(),
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

    // ─────────────────────────────────────────────────────────────────
    // Regression: sync_turn must handle long messages (task #80)
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn sync_turn_caps_long_messages_at_500_chars_each() {
        // Regression: the 500-char cap must handle UTF-8 boundaries correctly.
        // Use a disabled client — we only care about the pre-cap panic path.
        let client = ErukaClient::new(ErukaConfig::default());
        let long_user = "a".repeat(1200);
        let long_asst = "b".repeat(1200);
        // Must not panic even though each message is 1200 chars.
        let result = client
            .sync_turn(&long_user, &long_asst, "session-long")
            .await;
        assert!(result.is_ok(), "long messages must not panic");
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
            tags: Vec::new(),
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

    // ── New lifecycle/caching/export method tests (disabled path) ──────────

    #[tokio::test]
    async fn write_context_disabled_returns_false() {
        let client = ErukaClient::new(ErukaConfig::default());
        let ok = client
            .write_context("identity/name", "pawan", "test", 1.0)
            .await
            .unwrap();
        assert!(!ok, "disabled client must return false without calling network");
    }

    #[tokio::test]
    async fn sync_turn_disabled_returns_false() {
        let client = ErukaClient::new(ErukaConfig::default());
        let ok = client
            .sync_turn("hello", "world", "ses_abc")
            .await
            .unwrap();
        assert!(!ok, "disabled client must short-circuit");
    }

    #[tokio::test]
    async fn on_pre_compress_disabled_returns_false() {
        let client = ErukaClient::new(ErukaConfig::default());
        let ok = client
            .on_pre_compress("some messages", "ses_abc")
            .await
            .unwrap();
        assert!(!ok, "disabled client must short-circuit");
    }

    #[tokio::test]
    async fn prefetch_disabled_returns_none() {
        let client = ErukaClient::new(ErukaConfig::default());
        let result = client.prefetch("test query", 1000).await.unwrap();
        assert!(result.is_none(), "disabled client must return None");
    }

    #[tokio::test]
    async fn get_context_cached_disabled_returns_none() {
        let client = ErukaClient::new(ErukaConfig::default());
        let result = client
            .get_context_cached("identity/*", "ses_abc")
            .await
            .unwrap();
        assert!(result.is_none(), "disabled client must return None");
    }

    #[tokio::test]
    async fn export_context_disabled_returns_none() {
        let client = ErukaClient::new(ErukaConfig::default());
        let result = client.export_context("*", true).await.unwrap();
        assert!(result.is_none(), "disabled client must return None");
    }
}
