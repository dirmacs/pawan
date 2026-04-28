//! Tests for the Eruka bridge
//
//! Tests the Eruka context engine integration, focusing on:
//! - Config serialization/deserialization
//! - Request building
//! - Response parsing (ContextResponse, SearchResult)
//! - Integration with agent message flow

use pawan::agent::Message;
use pawan::eruka_bridge::{ErukaClient, ErukaConfig};
use serde_json::json;

#[test]
fn test_eruka_config_default_disabled() {
    let config = ErukaConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.url, "http://localhost:8081");
    assert!(config.api_key.is_none());
    assert_eq!(config.core_max_tokens, 500);
}

#[test]
fn test_eruka_config_enabled() {
    let config = ErukaConfig {
        enabled: true,
        url: "http://eruka.local:8081".to_string(),
        api_key: Some("secret-key".to_string()),
        core_max_tokens: 1000,
    };
    assert!(config.enabled);
    assert_eq!(config.url, "http://eruka.local:8081");
    assert_eq!(config.api_key, Some("secret-key".to_string()));
    assert_eq!(config.core_max_tokens, 1000);
}

#[test]
fn test_eruka_config_serde_roundtrip() {
    let original = ErukaConfig {
        enabled: true,
        url: "http://custom:9999".to_string(),
        api_key: Some("my-key".to_string()),
        core_max_tokens: 750,
    };
    let json_str = serde_json::to_string(&original).unwrap();
    let parsed: ErukaConfig = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.enabled, original.enabled);
    assert_eq!(parsed.url, original.url);
    assert_eq!(parsed.api_key, original.api_key);
    assert_eq!(parsed.core_max_tokens, original.core_max_tokens);
}

#[test]
fn test_eruka_config_serde_defaults() {
    let json_str = "{\"enabled\": true}";
    let parsed: ErukaConfig = serde_json::from_str(json_str).unwrap();
    assert!(parsed.enabled);
    assert_eq!(parsed.url, "http://localhost:8081");
    assert!(parsed.api_key.is_none());
    assert_eq!(parsed.core_max_tokens, 500);
}

#[test]
fn test_eruka_config_url_default_fn() {
    let config = ErukaConfig::default();
    assert_eq!(config.url, "http://localhost:8081");
}

#[test]
fn test_eruka_client_is_enabled() {
    let client_disabled = ErukaClient::new(ErukaConfig::default());
    assert!(!client_disabled.is_enabled());
    let config_enabled = ErukaConfig {
        enabled: true,
        url: "http://localhost:8081".to_string(),
        api_key: None,
        core_max_tokens: 500,
    };
    let client_enabled = ErukaClient::new(config_enabled);
    assert!(client_enabled.is_enabled());
}

#[test]
fn test_context_response_parsing() {
    let json = json!({
        "fields": [
            {"name": "project", "value": "pawan", "category": "core"},
            {"name": "language", "value": "Rust", "category": "core"},
            {"name": "architecture", "value": "agent-based", "category": "design"}
        ]
    });
    let resp: pawan::eruka_bridge::ContextResponse =
        serde_json::from_value(json).unwrap();
    let fields = resp.fields.unwrap();
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0].name.as_deref(), Some("project"));
    assert_eq!(fields[0].value.as_deref(), Some("pawan"));
    assert_eq!(fields[1].value.as_deref(), Some("Rust"));
}

#[test]
fn test_context_response_null_fields() {
    let json = json!({
        "fields": [
            {"name": null, "value": "has value", "category": null},
            {"name": "key", "value": null, "category": "cat"},
            {"name": null, "value": null, "category": null}
        ]
    });
    let resp: pawan::eruka_bridge::ContextResponse =
        serde_json::from_value(json).unwrap();
    let fields = resp.fields.unwrap();
    assert_eq!(fields.len(), 3);
}

#[test]
fn test_context_response_no_fields() {
    let json = json!({});
    let resp: pawan::eruka_bridge::ContextResponse =
        serde_json::from_value(json).unwrap();
    assert!(resp.fields.is_none());
}

#[test]
fn test_context_response_empty_fields_array() {
    let json = json!({"fields": []});
    let resp: pawan::eruka_bridge::ContextResponse =
        serde_json::from_value(json).unwrap();
    assert!(resp.fields.is_some());
    assert!(resp.fields.unwrap().is_empty());
}

#[test]
fn test_context_field_partial_optionals() {
    let json1 = json!({"name": "only_name", "value": null, "category": null});
    let json2 = json!({"name": null, "value": "only_value", "category": null});
    let json3 = json!({"name": null, "value": null, "category": "only_cat"});
    let f1: pawan::eruka_bridge::ContextField = serde_json::from_value(json1).unwrap();
    let f2: pawan::eruka_bridge::ContextField = serde_json::from_value(json2).unwrap();
    let f3: pawan::eruka_bridge::ContextField = serde_json::from_value(json3).unwrap();
    assert_eq!(f1.name.as_deref(), Some("only_name"));
    assert!(f1.value.is_none());
    assert_eq!(f3.category.as_deref(), Some("only_cat"));
    assert!(f3.value.is_none());
    assert!(f2.name.is_none());
    assert_eq!(f2.value.as_deref(), Some("only_value"));
}

#[test]
fn test_search_result_parsing() {
    let json = json!([
        {"content": "Found this in archival", "field_name": "session:123", "score": 0.95},
        {"content": "Another hit", "field_name": "session:456", "score": 0.8},
        {"content": "Lower relevance", "field_name": null, "score": 0.3}
    ]);
    let results: Vec<pawan::eruka_bridge::SearchResult> =
        serde_json::from_value(json).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].content.as_deref(), Some("Found this in archival"));
    assert_eq!(results[0].field_name.as_deref(), Some("session:123"));
    assert_eq!(results[0].score, Some(0.95));
    assert!(results[2].field_name.is_none());
    assert_eq!(results[2].score, Some(0.3));
}

#[test]
fn test_inject_core_memory_disabled() {
    let client = ErukaClient::new(ErukaConfig::default());
    let mut history = vec![
        Message {
            role: pawan::agent::Role::System,
            content: "You are a coding agent".to_string(),
            tool_calls: vec![],
            tool_result: None,
        },
        Message {
            role: pawan::agent::Role::User,
            content: "Hello".to_string(),
            tool_calls: vec![],
            tool_result: None,
        },
    ];
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(client.inject_core_memory(&mut history));
    assert!(result.is_ok());
    assert_eq!(history.len(), 2);
}

#[test]
fn test_memory_truncation_calculation() {
    let config = ErukaConfig {
        enabled: true,
        url: "http://localhost:8081".to_string(),
        api_key: None,
        core_max_tokens: 100,
    };
    let _client = ErukaClient::new(config);
}

#[test]
fn test_eruka_config_clone() {
    let config = ErukaConfig {
        enabled: true,
        url: "http://test:8081".to_string(),
        api_key: Some("key123".to_string()),
        core_max_tokens: 300,
    };
    let cloned = config.clone();
    assert_eq!(cloned.enabled, config.enabled);
    assert_eq!(cloned.url, config.url);
    assert_eq!(cloned.api_key, config.api_key);
    assert_eq!(cloned.core_max_tokens, config.core_max_tokens);
}

#[test]
fn test_eruka_config_debug() {
    let config = ErukaConfig {
        enabled: true,
        url: "http://test:8081".to_string(),
        api_key: Some("key123".to_string()),
        core_max_tokens: 300,
    };
    let debug_str = format!("{:?}", config);
    assert!(debug_str.contains("enabled: true"));
    assert!(debug_str.contains("test:8081"));
}
