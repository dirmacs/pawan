//! Live API tests for Ollama backend against NVIDIA NIM endpoint.
//! These tests require NVIDIA_API_KEY environment variable and make real API calls.
//! Run with: cargo test --test live_ollama -- --ignored

use pawan::agent::backend::ollama::OllamaBackend;
use pawan::agent::backend::LlmBackend;
use pawan::agent::{Message, Role};
use thulp_core::{Parameter, ToolDefinition};

fn get_base_url() -> String {
    std::env::var("OLLAMA_BASE_URL")
        .unwrap_or_else(|_| "https://integrate.api.nvidia.com".to_string())
}

fn get_api_key() -> String {
    std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY must be set for live tests")
}

#[tokio::test]
#[ignore] // Requires NVIDIA_API_KEY, run with --ignored flag
async fn live_test_tool_generation_with_real_api() {
    let _ = get_api_key();
    let base_url = get_base_url();

    let backend = OllamaBackend::new(
        base_url,
        "meta/llama-3.1-8b-instruct".into(),
        0.7,
        "You are a helpful assistant.".into(),
    );

    let messages = vec![Message {
        role: Role::User,
        content: "What is 2+2?".into(),
        tool_calls: vec![],
        tool_result: None,
    }];

    // Test with a simple tool
    let mut expr_param = Parameter::required_string("expression");
    expr_param.description = "Math expression to evaluate".into();
    let tools = vec![ToolDefinition::builder("calculator")
        .description("Perform basic math calculations")
        .parameter(expr_param)
        .build()];

    let result = backend.generate(&messages, &tools, None).await;

    // Should succeed without "function calls mismatch" error
    assert!(
        result.is_ok(),
        "Live API call failed: {:?}. This may indicate tool schema mismatch.",
        result.err().unwrap()
    );
}

#[tokio::test]
#[ignore]
async fn live_test_empty_tool_params() {
    let _ = get_api_key();
    let base_url = get_base_url();

    let backend = OllamaBackend::new(
        base_url,
        "meta/llama-3.1-8b-instruct".into(),
        0.7,
        "You are helpful.".into(),
    );

    let messages = vec![Message {
        role: Role::User,
        content: "test".into(),
        tool_calls: vec![],
        tool_result: None,
    }];

    // Tool with no parameters
    let tools = vec![ToolDefinition::builder("simple_tool")
        .description("A tool with no parameters")
        .build()];

    let result = backend.generate(&messages, &tools, None).await;

    assert!(
        result.is_ok(),
        "Live API call with empty tool params failed: {:?}",
        result.err().unwrap()
    );
}
