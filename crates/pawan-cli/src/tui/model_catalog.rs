//! Dynamic NVIDIA model catalog — live fetch with hardcoded fallback.
use super::types::ModelInfo;
use std::time::Duration;
use serde::Deserialize;

/// Fetch live model list from a custom endpoint URL. Returns sorted vec on success.
/// 5-second timeout; on any failure returns None (caller uses fallback).
pub async fn fetch_live_models_from(endpoint_url: &str) -> Option<Vec<ModelInfo>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    // Try to read NVIDIA_API_KEY from env (optional — API works without it for public models)
    let _api_key = std::env::var("NVIDIA_API_KEY").ok();
    let resp = client
        .get(endpoint_url)
        .send()
        .await
        .ok()?;
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
            let provider = m.owned_by.unwrap_or_else(|| {
                id.split('/').next().unwrap_or("unknown").to_string()
            });
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
        models.sort_by(|a, b| b.quality_score.cmp(&a.quality_score));
        Some(models)
    }
}

/// Fetch live model list from NVIDIA API. Returns sorted vec on success.
/// 5-second timeout; on any failure returns None (caller uses fallback).
pub async fn fetch_live_models() -> Option<Vec<ModelInfo>> {
    fetch_live_models_from("https://integrate.api.nvidia.com/v1/models").await
}


/// Compute a heuristic quality score (0-100) for a model based on its ID.
pub fn quality_score_for(model_id: &str) -> u8 {
    // Known high-quality families get top scores
    let id_lower = model_id.to_lowercase();
    if id_lower.contains("deepseek-r1") || id_lower.contains("deepseek-v3") {
        return 95;
    }
    if id_lower.contains("qwen3") || id_lower.contains("qwen-3") || id_lower.contains("qwen2.5") {
        return 95;
    }
    if id_lower.contains("llama-3.3") || id_lower.contains("llama-3.1-405") {
        return 93;
    }
    if id_lower.contains("llama-3.1") || id_lower.contains("llama3.1") {
        return 90;
    }
    if id_lower.contains("llama-3") {
        return 88;
    }
    if id_lower.contains("mistral-large") || id_lower.contains("mistral-small") {
        return 90;
    }
    if id_lower.contains("codestral") {
        return 92;
    }
    if id_lower.contains("gemma-3") || id_lower.contains("gemma3") {
        return 85;
    }
    if id_lower.contains("step-3") || id_lower.contains("step3") {
        return 88;
    }
    if id_lower.contains("glm-5") || id_lower.contains("glm5") {
        return 90;
    }
    if id_lower.contains("phi-4") || id_lower.contains("phi4") {
        return 85;
    }
    if id_lower.contains("star") {
        return 80;
    }
    if id_lower.contains("70b") || id_lower.contains("405b") {
        return 85;
    }
    if id_lower.contains("34b") || id_lower.contains("32b") {
        return 82;
    }
    if id_lower.contains("13b") || id_lower.contains("14b") || id_lower.contains("15b") {
        return 79;
    }
    if id_lower.contains("7b") || id_lower.contains("8b") || id_lower.contains("9b") {
        return 77;
    }
    if id_lower.contains("3b") || id_lower.contains("4b") {
        return 75;
    }
    if id_lower.contains("1b") || id_lower.contains("2b") {
        return 73;
    }
    75 // default
}

/// Hardcoded fallback model list (used when live fetch fails).
/// Keep this as a curated subset — the live API is the primary source.
pub fn default_models() -> Vec<ModelInfo> {
    vec![
            ModelInfo {
                id: "01-ai/yi-large".to_string(),
                provider: "01-ai".to_string(),
                quality_score: 75,
            },
            // Abacusai models (1)
            ModelInfo {
                id: "abacusai/dracarys-llama-3.1-70b-instruct".to_string(),
                provider: "Abacusai".to_string(),
                quality_score: 93,
            },
            // Ai21labs models (1)
            ModelInfo {
                id: "ai21labs/jamba-1.5-large-instruct".to_string(),
                provider: "Ai21labs".to_string(),
                quality_score: 75,
            },
            // Aisingapore models (1)
            ModelInfo {
                id: "aisingapore/sea-lion-7b-instruct".to_string(),
                provider: "Aisingapore".to_string(),
                quality_score: 79,
            },
            // Bigcode models (1)
            ModelInfo {
                id: "bigcode/starcoder2-15b".to_string(),
                provider: "Bigcode".to_string(),
                quality_score: 75,
            },
            // Bytedance models (1)
            ModelInfo {
                id: "bytedance/seed-oss-36b-instruct".to_string(),
                provider: "Bytedance".to_string(),
                quality_score: 75,
            },
            // Databricks models (1)
            ModelInfo {
                id: "databricks/dbrx-instruct".to_string(),
                provider: "Databricks".to_string(),
                quality_score: 75,
            },
            // DeepSeek models (3)
            ModelInfo {
                id: "deepseek-ai/deepseek-coder-6.7b-instruct".to_string(),
                provider: "Deepseek-ai".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "deepseek-ai/deepseek-v3.1-terminus".to_string(),
                provider: "Deepseek-ai".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "deepseek-ai/deepseek-v3.2".to_string(),
                provider: "Deepseek-ai".to_string(),
                quality_score: 93,
            },
            // Google models (10)
            ModelInfo {
                id: "google/codegemma-1.1-7b".to_string(),
                provider: "Google".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "google/codegemma-7b".to_string(),
                provider: "Google".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "google/gemma-2-2b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "google/gemma-2b".to_string(),
                provider: "Google".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "google/gemma-3-12b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "google/gemma-3-27b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "google/gemma-3-4b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "google/gemma-3n-e2b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "google/gemma-3n-e4b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "google/gemma-4-31b-it".to_string(),
                provider: "Google".to_string(),
                quality_score: 87,
            },
            // IBM models (4)
            ModelInfo {
                id: "ibm/granite-3.0-3b-a800m-instruct".to_string(),
                provider: "Ibm".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "ibm/granite-3.0-8b-instruct".to_string(),
                provider: "Ibm".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "ibm/granite-34b-code-instruct".to_string(),
                provider: "Ibm".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "ibm/granite-8b-code-instruct".to_string(),
                provider: "Ibm".to_string(),
                quality_score: 81,
            },
            // Meta models (8)
            ModelInfo {
                id: "meta/llama-3.1-405b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "meta/llama-3.1-70b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 93,
            },
            ModelInfo {
                id: "meta/llama-3.1-8b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "meta/llama-3.2-11b-vision-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "meta/llama-3.2-1b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "meta/llama-3.2-3b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "meta/llama-3.3-70b-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "meta/llama-4-maverick-17b-128e-instruct".to_string(),
                provider: "Meta".to_string(),
                quality_score: 95,
            },
            // Microsoft models (4)
            ModelInfo {
                id: "microsoft/phi-3-vision-128k-instruct".to_string(),
                provider: "Microsoft".to_string(),
                quality_score: 83,
            },
            ModelInfo {
                id: "microsoft/phi-3.5-moe-instruct".to_string(),
                provider: "Microsoft".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "microsoft/phi-4-mini-instruct".to_string(),
                provider: "Microsoft".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "microsoft/phi-4-multimodal-instruct".to_string(),
                provider: "Microsoft".to_string(),
                quality_score: 91,
            },
            // MiniMax models (2)
            ModelInfo {
                id: "minimaxai/minimax-m2.5".to_string(),
                provider: "Minimaxai".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "minimaxai/minimax-m2.7".to_string(),
                provider: "Minimaxai".to_string(),
                quality_score: 89,
            },
            // Mistral models (14)
            ModelInfo {
                id: "mistralai/codestral-22b-instruct-v0.1".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "mistralai/devstral-2-123b-instruct-2512".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "mistralai/magistral-small-2506".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 75,
            },
            ModelInfo {
                id: "mistralai/ministral-14b-instruct-2512".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "mistralai/mistral-7b-instruct-v0.3".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 79,
            },
            ModelInfo {
                id: "mistralai/mistral-large".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "mistralai/mistral-large-2-instruct".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 93,
            },
            ModelInfo {
                id: "mistralai/mistral-large-3-675b-instruct-2512".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "mistralai/mistral-medium-3-instruct".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "mistralai/mistral-nemotron".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "mistralai/mistral-small-4-119b-2603".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "mistralai/mixtral-8x22b-instruct-v0.1".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "mistralai/mixtral-8x22b-v0.1".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "mistralai/mixtral-8x7b-instruct-v0.1".to_string(),
                provider: "Mistralai".to_string(),
                quality_score: 83,
            },
            // Moonshot models (4)
            ModelInfo {
                id: "moonshotai/kimi-k2-instruct".to_string(),
                provider: "Moonshotai".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "moonshotai/kimi-k2-instruct-0905".to_string(),
                provider: "Moonshotai".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "moonshotai/kimi-k2-thinking".to_string(),
                provider: "Moonshotai".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "moonshotai/kimi-k2.5".to_string(),
                provider: "Moonshotai".to_string(),
                quality_score: 93,
            },
            // NV-Mistral models (1)
            ModelInfo {
                id: "nv-mistralai/mistral-nemo-12b-instruct".to_string(),
                provider: "Nv-mistralai".to_string(),
                quality_score: 81,
            },
            // NVIDIA models (15)
            ModelInfo {
                id: "nvidia/ising-calibration-1-35b-a3b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-51b-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-70b-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 93,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-nano-8b-v1".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-nano-vl-8b-v1".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "nvidia/llama-3.1-nemotron-ultra-253b-v1".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 93,
            },
            ModelInfo {
                id: "nvidia/llama-3.3-nemotron-super-49b-v1".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 75,
            },
            ModelInfo {
                id: "nvidia/llama-3.3-nemotron-super-49b-v1.5".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 75,
            },
            ModelInfo {
                id: "nvidia/llama3-chatqa-1.5-70b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "nvidia/mistral-nemo-minitron-8b-8k-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "nvidia/nemotron-3-nano-30b-a3b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "nvidia/nemotron-3-super-120b-a12b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "nvidia/nemotron-4-340b-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "nvidia/nemotron-4-340b-reward".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "nvidia/nemotron-mini-4b-instruct".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "nvidia/nemotron-nano-12b-v2-vl".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 81,
            },
            ModelInfo {
                id: "nvidia/nemotron-nano-3-30b-a3b".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "nvidia/nvidia-nemotron-nano-9b-v2".to_string(),
                provider: "NVIDIA".to_string(),
                quality_score: 75,
            },
            // OpenAI models (4)
            ModelInfo {
                id: "openai/gpt-oss-120b".to_string(),
                provider: "OpenAI".to_string(),
                quality_score: 75,
            },
            ModelInfo {
                id: "openai/gpt-oss-20b".to_string(),
                provider: "OpenAI".to_string(),
                quality_score: 75,
            },
            // Qwen models (6)
            ModelInfo {
                id: "qwen/qwen2.5-coder-32b-instruct".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 89,
            },
            ModelInfo {
                id: "qwen/qwen3-coder-480b-a35b-instruct".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "qwen/qwen3-next-80b-a3b-instruct".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "qwen/qwen3-next-80b-a3b-thinking".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 91,
            },
            ModelInfo {
                id: "qwen/qwen3.5-122b-a10b".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 85,
            },
            ModelInfo {
                id: "qwen/qwen3.5-397b-a17b".to_string(),
                provider: "Qwen".to_string(),
                quality_score: 95,
            },
            // Sarvamai models (1)
            ModelInfo {
                id: "sarvamai/sarvam-m".to_string(),
                provider: "Sarvamai".to_string(),
                quality_score: 75,
            },
            // StepFun models (1)
            ModelInfo {
                id: "stepfun-ai/step-3.5-flash".to_string(),
                provider: "Stepfun-ai".to_string(),
                quality_score: 85,
            },
            // Stockmark models (1)
            ModelInfo {
                id: "stockmark/stockmark-2-100b-instruct".to_string(),
                provider: "Stockmark".to_string(),
                quality_score: 75,
            },
            // Upstage models (1)
            ModelInfo {
                id: "upstage/solar-10.7b-instruct".to_string(),
                provider: "Upstage".to_string(),
                quality_score: 79,
            },
            // Writer models (4)
            ModelInfo {
                id: "writer/palmyra-creative-122b".to_string(),
                provider: "Writer".to_string(),
                quality_score: 77,
            },
            ModelInfo {
                id: "writer/palmyra-fin-70b-32k".to_string(),
                provider: "Writer".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "writer/palmyra-med-70b".to_string(),
                provider: "Writer".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "writer/palmyra-med-70b-32k".to_string(),
                provider: "Writer".to_string(),
                quality_score: 87,
            },
            // Z-AI models (3)
            ModelInfo {
                id: "z-ai/glm-5.1".to_string(),
                provider: "Z-ai".to_string(),
                quality_score: 95,
            },
            ModelInfo {
                id: "z-ai/glm4.7".to_string(),
                provider: "Z-ai".to_string(),
                quality_score: 87,
            },
            ModelInfo {
                id: "z-ai/glm5".to_string(),
                provider: "Z-ai".to_string(),
                quality_score: 95,
            },
            // Zyphra models (1)
            ModelInfo {
                id: "zyphra/zamba2-7b-instruct".to_string(),
                provider: "Zyphra".to_string(),
                quality_score: 79,
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
        assert!(models.len() >= 50, "Should have at least 50 fallback models");
    }

    #[test]
    fn test_default_models_have_valid_ids() {
        for m in default_models() {
            assert!(!m.id.is_empty());
            assert!(m.id.contains('/'), "Model ID should be provider/name: {}", m.id);
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
    use serde_json;
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
        let models = fetch_live_models_from(&url).await.expect("mock should return models");
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

