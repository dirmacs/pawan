//! Dynamic NVIDIA model catalog — live fetch with hardcoded fallback.
use super::types::ModelInfo;
use serde::Deserialize;
use std::time::Duration;

/// Fetch live model list from a custom endpoint URL. Returns sorted vec on success.
/// 5-second timeout; on any failure returns None (caller uses fallback).
pub async fn fetch_live_models_from(endpoint_url: &str) -> Option<Vec<ModelInfo>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let mut request = client.get(endpoint_url);
    if let Ok(key) = std::env::var("NVIDIA_API_KEY") {
        request = request.header("Authorization", format!("Bearer {}", key));
    }
    let resp = request.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    #[derive(Deserialize)]
    struct ModelEntry {
        id: String,
        owned_by: Option<String>,
    }
    #[derive(Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }
    let body: ModelsResponse = resp.json().await.ok()?;
    let mut models: Vec<ModelInfo> = body
        .data
        .into_iter()
        .map(|m| {
            let id = m.id;
            let provider = m
                .owned_by
                .unwrap_or_else(|| id.split('/').next().unwrap_or("unknown").to_string());
            ModelInfo {
                quality_score: quality_score_for(&id),
                id,
                provider,
            }
        })
        .collect();
    if models.is_empty() {
        None
    } else {
        models.sort_by_key(|b| std::cmp::Reverse(b.quality_score));
        Some(models)
    }
}

/// Fetch live model list from NVIDIA API. Returns sorted vec on success.
/// 5-second timeout; on any failure returns None (caller uses fallback).
pub async fn fetch_live_models() -> Option<Vec<ModelInfo>> {
    fetch_live_models_from("https://integrate.api.nvidia.com/v1/models").await
}

/// Score a model based on known vendor/model-family prefixes.
fn score_by_vendor(model_id: &str) -> Option<u8> {
    let id = model_id.to_lowercase();
    if id.contains("deepseek-r1") || id.contains("deepseek-v3") {
        return Some(95);
    }
    if id.contains("qwen3") || id.contains("qwen-3") || id.contains("qwen2.5") {
        return Some(95);
    }
    if id.contains("llama-3.3") || id.contains("llama-3.1-405") {
        return Some(93);
    }
    if id.contains("llama-3.1") || id.contains("llama3.1") {
        return Some(90);
    }
    if id.contains("llama-3") {
        return Some(88);
    }
    if id.contains("mistral-large") || id.contains("mistral-small") {
        return Some(90);
    }
    if id.contains("codestral") {
        return Some(92);
    }
    if id.contains("gemma-3") || id.contains("gemma3") {
        return Some(85);
    }
    if id.contains("step-3") || id.contains("step3") {
        return Some(88);
    }
    if id.contains("glm-5") || id.contains("glm5") {
        return Some(90);
    }
    if id.contains("phi-4") || id.contains("phi4") {
        return Some(85);
    }
    if id.contains("star") {
        return Some(80);
    }
    None
}

/// Score a model based on capability keywords (instruct, chat, etc.).
///
/// Placeholder for future capability-based scoring.
fn score_by_capability(_model_id: &str) -> Option<u8> {
    None
}

/// Score a model based on parameter-size markers in its ID (e.g. `7b`, `405b`).
fn score_by_size(model_id: &str) -> Option<u8> {
    let id = model_id.to_lowercase();
    if id.contains("70b") || id.contains("405b") {
        return Some(85);
    }
    if id.contains("34b") || id.contains("32b") {
        return Some(82);
    }
    if id.contains("13b") || id.contains("14b") || id.contains("15b") {
        return Some(79);
    }
    if id.contains("7b") || id.contains("8b") || id.contains("9b") {
        return Some(77);
    }
    if id.contains("3b") || id.contains("4b") {
        return Some(75);
    }
    if id.contains("1b") || id.contains("2b") {
        return Some(73);
    }
    None
}

/// Compute a heuristic quality score (0-100) for a model based on its ID.
///
/// Combines vendor, capability, and size scores, returning the highest.
/// Defaults to 75 when no heuristic matches.
pub fn quality_score_for(model_id: &str) -> u8 {
    let vendor = score_by_vendor(model_id);
    let capability = score_by_capability(model_id);
    let size = score_by_size(model_id);
    vendor
        .into_iter()
        .chain(capability)
        .chain(size)
        .max()
        .unwrap_or(75)
}

/// Hardcoded fallback model list (used when live fetch fails).
/// Keep this as a curated subset — the live API is the primary source.
pub fn default_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "meta/llama-3.1-70b-instruct".into(),
            provider: "Meta".into(),
            quality_score: quality_score_for("meta/llama-3.1-70b-instruct"),
        },
        ModelInfo {
            id: "meta/llama-3.3-70b-instruct".into(),
            provider: "Meta".into(),
            quality_score: quality_score_for("meta/llama-3.3-70b-instruct"),
        },
        ModelInfo {
            id: "meta/llama-4-maverick-17b-128e-instruct".into(),
            provider: "Meta".into(),
            quality_score: quality_score_for("meta/llama-4-maverick-17b-128e-instruct"),
        },
        ModelInfo {
            id: "deepseek-ai/deepseek-v3.2".into(),
            provider: "Deepseek".into(),
            quality_score: quality_score_for("deepseek-ai/deepseek-v3.2"),
        },
        ModelInfo {
            id: "deepseek-ai/deepseek-v4-flash".into(),
            provider: "Deepseek".into(),
            quality_score: quality_score_for("deepseek-ai/deepseek-v4-flash"),
        },
        ModelInfo {
            id: "google/gemma-3-12b-it".into(),
            provider: "Google".into(),
            quality_score: quality_score_for("google/gemma-3-12b-it"),
        },
        ModelInfo {
            id: "mistralai/mistral-large-3-675b-instruct-2512".into(),
            provider: "Mistral".into(),
            quality_score: quality_score_for("mistralai/mistral-large-3-675b-instruct-2512"),
        },
        ModelInfo {
            id: "qwen/qwen3.5-397b-a17b".into(),
            provider: "Qwen".into(),
            quality_score: quality_score_for("qwen/qwen3.5-397b-a17b"),
        },
        ModelInfo {
            id: "nvidia/llama-3.1-nemotron-ultra-253b-v1".into(),
            provider: "Nvidia".into(),
            quality_score: quality_score_for("nvidia/llama-3.1-nemotron-ultra-253b-v1"),
        },
        ModelInfo {
            id: "stepfun-ai/step-3.7-flash".into(),
            provider: "Stepfun".into(),
            quality_score: quality_score_for("stepfun-ai/step-3.7-flash"),
        },
        ModelInfo {
            id: "stepfun-ai/step-3.5-flash".into(),
            provider: "Stepfun".into(),
            quality_score: quality_score_for("stepfun-ai/step-3.5-flash"),
        },
        ModelInfo {
            id: "z-ai/glm-5.1".into(),
            provider: "Z-Ai".into(),
            quality_score: quality_score_for("z-ai/glm-5.1"),
        },
        ModelInfo {
            id: "openai/gpt-oss-120b".into(),
            provider: "OpenAI".into(),
            quality_score: quality_score_for("openai/gpt-oss-120b"),
        },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quality_score_for_known_models() {
        assert!(quality_score_for("deepseek-ai/deepseek-r1") >= 90);
        assert!(quality_score_for("meta/llama-3.1-405b-instruct") >= 90);
        assert!(quality_score_for("unknown/7b-model") >= 75);
    }

    #[test]
    fn test_quality_score_for_small_models() {
        assert!(quality_score_for("some/1b-tiny") < 80);
        assert!(quality_score_for("some/2b-small") < 80);
    }

    #[test]
    fn test_default_models_not_empty() {
        let models = default_models();
        assert!(!models.is_empty());
        assert!(
            models.len() >= 10,
            "Should have at least 10 fallback models"
        );
    }

    #[test]
    fn test_default_models_have_valid_ids() {
        for m in default_models() {
            assert!(!m.id.is_empty());
            assert!(
                m.id.contains('/'),
                "Model ID should be provider/name: {}",
                m.id
            );
            assert!(m.quality_score > 0);
        }
    }

    #[test]
    fn test_fetch_live_models_live() {
        if std::env::var("E2E").unwrap_or_default() != "1" {
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let models = rt.block_on(fetch_live_models());
        assert!(models.is_some(), "Live NVIDIA API should return models");
        let models = models.unwrap();
        assert!(!models.is_empty());
        assert!(
            models.len() > 50,
            "Should have at least 50 models from live API"
        );
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        let has_deepseek = ids.iter().any(|id| id.contains("deepseek"));
        let has_llama = ids.iter().any(|id| id.contains("llama"));
        assert!(
            has_deepseek || has_llama,
            "Live API should have at least deepseek or llama models"
        );
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_fetch_live_models_from_http_500_returns_none() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let url = format!("{}/v1/models", mock_server.uri());
        assert!(fetch_live_models_from(&url).await.is_none());
    }

    #[tokio::test]
    async fn test_fetch_live_models_from_empty_data_returns_none() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": []
            })))
            .mount(&mock_server)
            .await;

        let url = format!("{}/v1/models", mock_server.uri());
        assert!(fetch_live_models_from(&url).await.is_none());
    }

    #[tokio::test]
    async fn test_fetch_live_models_from_sorts_by_quality_score_descending() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "aaa/vendor/small-1b", "owned_by": "VendorA"},
                    {"id": "zzz/vendor/large-70b", "owned_by": "VendorZ"},
                ]
            })))
            .mount(&mock_server)
            .await;

        let url = format!("{}/v1/models", mock_server.uri());
        let models = fetch_live_models_from(&url)
            .await
            .expect("mock should return models");
        assert_eq!(models.len(), 2);
        assert!(
            models[0].quality_score >= models[1].quality_score,
            "expected descending quality_score order"
        );
        assert_eq!(models[0].id, "zzz/vendor/large-70b");
        assert_eq!(models[1].id, "aaa/vendor/small-1b");
        assert!(models[0].quality_score > models[1].quality_score);
    }

    #[test]
    fn test_quality_score_for_empty_string() {
        assert_eq!(quality_score_for(""), 75);
    }

    #[test]
    fn test_quality_score_for_unknown_vendor() {
        assert_eq!(quality_score_for("totally-unknown-corp/obscure-widget"), 75);
    }

    #[tokio::test]
    async fn test_fetch_live_models_from_mock_server() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "test-org/test-model-7b", "owned_by": "TestOrg"},
                    {"id": "test-org/test-model-70b", "owned_by": "TestOrg"},
                ]
            })))
            .mount(&mock_server)
            .await;

        let url = format!("{}/v1/models", mock_server.uri());
        let models = fetch_live_models_from(&url).await;
        assert!(models.is_some(), "Mock server should return models");
        let models = models.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "test-org/test-model-70b");
        assert_eq!(models[1].id, "test-org/test-model-7b");
        for m in &models {
            assert_eq!(m.provider, "TestOrg");
            assert_eq!(m.quality_score, quality_score_for(&m.id));
        }
    }
}
