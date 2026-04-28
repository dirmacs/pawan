//! Unit tests for TUI type utilities
//!
//! Tests pure parsing, formatting, and text manipulation functions
//! from src/tui/types.rs that don’t require a terminal.

use pawan_cli_lib::tui::types::{
    format_tool_result, one_line_preview, strip_reasoning_tags, summarize_args,
    ContentBlock, ExportFormat, KeybindContext, Panel, SessionSortMode, ToolBlockState,
};

use pawan::agent::{ToolCallRecord, ToolCallRequest};

// ============================================================================
// ExportFormat Tests
// ============================================================================

#[test]
fn test_export_format_from_str_lowercase() {
    assert_eq!(ExportFormat::from_str("html"), ExportFormat::Html);
    assert_eq!(ExportFormat::from_str("json"), ExportFormat::Json);
    assert_eq!(ExportFormat::from_str("txt"), ExportFormat::Txt);
    assert_eq!(ExportFormat::from_str("md"), ExportFormat::Markdown);
}

#[test]
fn test_export_format_from_str_uppercase() {
    assert_eq!(ExportFormat::from_str("HTML"), ExportFormat::Html);
    assert_eq!(ExportFormat::from_str("JSON"), ExportFormat::Json);
    assert_eq!(ExportFormat::from_str("TXT"), ExportFormat::Txt);
    assert_eq!(ExportFormat::from_str("MD"), ExportFormat::Markdown);
}

#[test]
fn test_export_format_from_str_mixed_case() {
    assert_eq!(ExportFormat::from_str("Html"), ExportFormat::Html);
    assert_eq!(ExportFormat::from_str("Json"), ExportFormat::Json);
}

#[test]
fn test_export_format_from_str_full_words() {
    assert_eq!(ExportFormat::from_str("markdown"), ExportFormat::Markdown);
    assert_eq!(ExportFormat::from_str("MARKDOWN"), ExportFormat::Markdown);
    assert_eq!(ExportFormat::from_str("text"), ExportFormat::Txt);
    assert_eq!(ExportFormat::from_str("TEXT"), ExportFormat::Txt);
}

#[test]
fn test_export_format_from_str_unknown_defaults_to_markdown() {
    assert_eq!(ExportFormat::from_str("pdf"), ExportFormat::Markdown);
    assert_eq!(ExportFormat::from_str(""), ExportFormat::Markdown);
    assert_eq!(ExportFormat::from_str("xyz"), ExportFormat::Markdown);
}

#[test]
fn test_export_format_extension() {
    assert_eq!(ExportFormat::Markdown.extension(), ".md");
    assert_eq!(ExportFormat::Html.extension(), ".html");
    assert_eq!(ExportFormat::Json.extension(), ".json");
    assert_eq!(ExportFormat::Txt.extension(), ".txt");
}

    // ============================================================================
// summarize_args Tests
    // ============================================================================

#[test]
fn test_summarize_args_empty_object() {
    let empty = serde_json::Value::Object(serde_json::Map::new());
    let result = summarize_args(&empty);
    assert_eq!(result, "");
}

#[test]
fn test_summarize_args_single_string() {
    let args = serde_json::json!({"path": "/tmp/test.txt"});
    let result = summarize_args(&args);
    assert!(result.contains("path"));
    assert!(result.contains("/tmp/test.txt"));
}

#[test]
fn test_summarize_args_multiple_keys() {
    let args = serde_json::json!({"file": "main.rs", "line": 42});
    let result = summarize_args(&args);
    assert!(result.contains("file"));
    assert!(result.contains("main.rs"));
}

#[test]
fn test_summarize_args_long_string_truncated() {
    let long_path = "x".repeat(100);
    let args = serde_json::json!({"path": long_path});
    let result = summarize_args(&args);
    assert!(result.len() <= 100);
}

#[test]
fn test_summarize_args_non_object() {
    let arr = serde_json::json!([1, 2, 3]);
    let result = summarize_args(&arr);
    assert_eq!(result, "");
}

#[test]
fn test_summarize_args_string_value() {
    let s = serde_json::Value::String("just a string".to_string());
    let result = summarize_args(&s);
    assert_eq!(result, "");
}

    // ============================================================================
// one_line_preview Tests
    // ============================================================================

#[test]
fn test_one_line_preview_string_short() {
    let val = serde_json::Value::String("hello world".to_string());
    let preview = one_line_preview(&val, 30);
    assert_eq!(preview, "hello world");
}

#[test]
fn test_one_line_preview_string_truncated() {
    let val = serde_json::Value::String("x".repeat(50));
    let preview = one_line_preview(&val, 20);
    assert!(preview.len() <= 23);
}

#[test]
fn test_one_line_preview_map_with_content() {
    let val = serde_json::json!({"content": "important text"});
    let preview = one_line_preview(&val, 50);
    assert!(preview.contains("important text"));
}

#[test]
fn test_one_line_preview_map_no_content() {
    let val = serde_json::json!({"error": "something failed"});
    let preview = one_line_preview(&val, 50);
    assert!(!preview.is_empty());
}

#[test]
fn test_one_line_preview_null() {
    let val = serde_json::Value::Null;
    let preview = one_line_preview(&val, 20);
    assert_eq!(preview, "null");
}

#[test]
fn test_one_line_preview_number() {
    let val = serde_json::Value::Number(42.into());
    let preview = one_line_preview(&val, 20);
    assert!(preview.contains("42"));
}

#[test]
fn test_one_line_preview_multiline_truncates_first_line() {
    let val = serde_json::Value::String("first line\nsecond line\nthird line".to_string());
    let preview = one_line_preview(&val, 50);
    assert!(preview.contains("first line"));
    assert!(!preview.contains("second line"));
}

    // ============================================================================
// format_tool_result Tests
    // ============================================================================

#[test]
fn test_format_tool_result_string() {
    let val = serde_json::Value::String("success output".to_string());
    let formatted = format_tool_result(&val);
    assert_eq!(formatted, "success output");
}

#[test]
fn test_format_tool_result_null() {
    let val = serde_json::Value::Null;
    let formatted = format_tool_result(&val);
    assert_eq!(formatted, "null");
}

#[test]
fn test_format_tool_result_number() {
    let val = serde_json::Value::Number(99.into());
    let formatted = format_tool_result(&val);
    assert!(formatted.contains("99"));
}

#[test]
fn test_format_tool_result_bool() {
    let val = serde_json::Value::Bool(true);
    let formatted = format_tool_result(&val);
    assert!(formatted.contains("true"));
}

#[test]
fn test_format_tool_result_object_pretty() {
    let val = serde_json::json!({"key": "value", "num": 1});
    let formatted = format_tool_result(&val);
    assert!(formatted.contains("key"));
    assert!(formatted.contains("value"));
}

#[test]
fn test_format_tool_result_empty_string() {
    let val = serde_json::Value::String("".to_string());
    let formatted = format_tool_result(&val);
    assert_eq!(formatted, "");
}

    // ============================================================================
// strip_reasoning_tags Tests
    // ============================================================================

#[test]
fn test_strip_reasoning_tags_with_think_tags() {
    let input = "Hello <think>inner content</think> world";
    let stripped = strip_reasoning_tags(input);
    // Debug: check stripped is shorter
    assert!(stripped.len() < input.len(), "expected stripping to reduce length");
    assert!(stripped.starts_with("Hello "));
    assert!(stripped.ends_with(" world"));
    assert!(!stripped.contains("inner content"));
}

#[test]
fn test_strip_reasoning_tags_no_tags() {
    let input = "Plain text without any tags";
    let stripped = strip_reasoning_tags(input);
    assert_eq!(stripped, "Plain text without any tags");
}

#[test]
fn test_strip_reasoning_tags_multiple_tags() {
    let input = "[^]first[/^] middle [^]second[/^] end";
    let stripped = strip_reasoning_tags(input);
    assert!(!stripped.contains("reasoning"));
    assert!(stripped.contains("middle"));
    assert!(stripped.contains("end"));
    assert!(stripped.contains("first"));
}

#[test]
fn test_strip_reasoning_tags_square_bracket_think() {
    let input = "Before [/think] after reasoning";
    let stripped = strip_reasoning_tags(input);
    assert!(!stripped.contains("[/think]"));
    assert!(stripped.contains("Before"));
    assert!(stripped.contains("after reasoning"));
}

#[test]
fn test_strip_reasoning_tags_empty_string() {
    let stripped = strip_reasoning_tags("");
    assert_eq!(stripped, "");
}

#[test]
fn test_strip_reasoning_tags_multiline() {
    let input = "Start\n[^]\nmulti\nline\ncontent\n[/^]\nEnd";
    let stripped = strip_reasoning_tags(input);
    assert!(!stripped.contains("reasoning"));
    assert!(stripped.contains("Start"));
    assert!(stripped.contains("End"));
}

    // ============================================================================
// ContentBlock Tests
    // ============================================================================

#[test]
fn test_content_block_text() {
    let block = ContentBlock::Text { content: "hello".to_string(), streaming: false };
    match block {
        ContentBlock::Text { content, streaming } => {
            assert_eq!(content, "hello");
            assert!(!streaming);
        }
        _ => panic!("expected Text variant"),
    }
}

#[test]
fn test_content_block_text_streaming() {
    let block = ContentBlock::Text { content: "streaming".to_string(), streaming: true };
    match block {
        ContentBlock::Text { content, streaming } => {
            assert!(streaming);
        }
        _ => panic!("expected Text variant"),
    }
}

#[test]
fn test_content_block_tool_call() {
    let state = ToolBlockState::Running;
    let block = ContentBlock::ToolCall {
        name: "bash".to_string(),
        args_summary: "cmd=ls".to_string(),
        state: Box::new(state),
    };
    match block {
        ContentBlock::ToolCall { name, args_summary, state } => {
            assert_eq!(name, "bash");
            assert!(matches!(*state, ToolBlockState::Running));
        }
        _ => panic!("expected ToolCall variant"),
    }
}

    // ============================================================================
// ToolBlockState Tests
    // ============================================================================

#[test]
fn test_tool_block_state_running() {
    let state = ToolBlockState::Running;
    match state {
        ToolBlockState::Running => { }
        ToolBlockState::Done { .. } => panic!("expected Running"),
    }
}

#[test]
fn test_tool_block_state_done() {
    let request = ToolCallRequest {
        id: "call_1".to_string(),
        name: "bash".to_string(),
        arguments: serde_json::json!({"cmd": "echo hi"}),
    };
    let record = ToolCallRecord {
        id: request.id.clone(),
        name: request.name.clone(),
        arguments: request.arguments.clone(),
        result: serde_json::json!("output"),
        success: true,
        duration_ms: 100,
    };
    let state = ToolBlockState::Done { record, expanded: false };
    match state {
        ToolBlockState::Done { expanded, .. } => assert!(!expanded),
        ToolBlockState::Running => panic!("expected Done"),
    }
}

#[test]
fn test_tool_block_state_done_expanded_on_error() {
    let request = ToolCallRequest {
        id: "call_2".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({"path": "/nonexistent"}),
    };
    let record = ToolCallRecord {
        id: request.id.clone(),
        name: request.name.clone(),
        arguments: request.arguments.clone(),
        result: serde_json::json!({"error": "not found"}),
        success: false,
        duration_ms: 50,
    };
    let state = ToolBlockState::Done { record, expanded: true };
    match state {
        ToolBlockState::Done { expanded, .. } => assert!(expanded),
        ToolBlockState::Running => panic!("expected Done"),
    }
}

    // ============================================================================
// Panel Tests
    // ============================================================================

#[test]
fn test_panel_input() {
    let panel = Panel::Input;
    match panel {
        Panel::Input => { }
        Panel::Messages => panic!("expected Input"),
    }
}

#[test]
fn test_panel_messages() {
    let panel = Panel::Messages;
    match panel {
        Panel::Messages => { }
        Panel::Input => panic!("expected Messages"),
    }
}

    // ============================================================================
// KeybindContext Tests
    // ============================================================================

#[test]
fn test_keybind_context_input() {
    let ctx = KeybindContext::Input;
    match ctx {
        KeybindContext::Input => { }
        _ => panic!("expected Input"),
    }
}

#[test]
fn test_keybind_context_normal() {
    let ctx = KeybindContext::Normal;
    match ctx {
        KeybindContext::Normal => { }
        _ => panic!("expected Normal"),
    }
}

#[test]
fn test_keybind_context_command() {
    let ctx = KeybindContext::Command;
    match ctx {
        KeybindContext::Command => { }
        _ => panic!("expected Command"),
    }
}

#[test]
fn test_keybind_context_help() {
    let ctx = KeybindContext::Help;
    match ctx {
        KeybindContext::Help => { }
        _ => panic!("expected Help"),
    }
}

#[test]
fn test_keybind_context_model_picker() {
    let ctx = KeybindContext::ModelPicker;
    match ctx {
        KeybindContext::ModelPicker => { }
        _ => panic!("expected ModelPicker"),
    }
}

    // ============================================================================
// SessionSortMode Tests
    // ============================================================================

#[test]
fn test_session_sort_mode_newest_first() {
    let mode = SessionSortMode::NewestFirst;
    match mode {
        SessionSortMode::NewestFirst => { }
        _ => panic!("expected NewestFirst"),
    }
}

#[test]
fn test_session_sort_mode_alphabetical() {
    let mode = SessionSortMode::Alphabetical;
    match mode {
        SessionSortMode::Alphabetical => { }
        _ => panic!("expected Alphabetical"),
    }
}

#[test]
fn test_session_sort_mode_most_used() {
    let mode = SessionSortMode::MostUsed;
    match mode {
        SessionSortMode::MostUsed => { }
        _ => panic!("expected MostUsed"),
    }
}

    // ============================================================================
// ToolCallRequest Tests
    // ============================================================================

#[test]
fn test_tool_call_request_fields() {
    let request = ToolCallRequest {
        id: "call_3".to_string(),
        name: "edit".to_string(),
        arguments: serde_json::json!({
            "path": "src/main.rs",
            "old_text": "fn main()",
            "new_text": "fn run()",
        }),
    };
    assert_eq!(request.id, "call_3");
    assert_eq!(request.name, "edit");
    assert_eq!(request.arguments["path"], "src/main.rs");
}

#[test]
fn test_tool_call_record_successful() {
    let request = ToolCallRequest {
        id: "call_1".to_string(),
        name: "bash".to_string(),
        arguments: serde_json::json!({"cmd": "ls -la"}),
    };
    let record = ToolCallRecord {
        id: request.id.clone(),
        name: request.name.clone(),
        arguments: request.arguments.clone(),
        result: serde_json::json!({"stdout": "files"}),
        success: true,
        duration_ms: 150,
    };
    assert!(record.success);
    assert_eq!(record.duration_ms, 150);
}

#[test]
fn test_tool_call_record_failed() {
    let request = ToolCallRequest {
        id: "call_2".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({"path": "/nonexistent"}),
    };
    let record = ToolCallRecord {
        id: request.id.clone(),
        name: request.name.clone(),
        arguments: request.arguments.clone(),
        result: serde_json::json!({"error": "not found"}),
        success: false,
        duration_ms: 50,
    };
    assert!(!record.success);
}

    // ============================================================================
// Edge Cases
    // ============================================================================

#[test]
fn test_export_format_case_insensitive_all() {
    assert_eq!(ExportFormat::from_str("hTmL"), ExportFormat::Html);
    assert_eq!(ExportFormat::from_str("JsOn"), ExportFormat::Json);
    assert_eq!(ExportFormat::from_str("TxT"), ExportFormat::Txt);
    assert_eq!(ExportFormat::from_str("MaRkDoWn"), ExportFormat::Markdown);
}

#[test]
fn test_strip_reasoning_tags_preserves_remaining() {
    let input = "prefix OKEN secret CLOSE suffix";
    let stripped = strip_reasoning_tags(input);
    assert!(stripped.contains("prefix"));
    assert!(stripped.contains("suffix"));
    assert!(!stripped.contains("junk"));
}

#[test]
fn test_summarize_args_deeply_nested() {
    let args = serde_json::json!({"nested": {"deep": "value"}});
    let result = summarize_args(&args);
    assert!(result.contains("nested"));
}
