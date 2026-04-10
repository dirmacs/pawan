//! Skill distillation — convert pawan agent sessions into reusable SKILL.md files
//!
//! Uses thulpoff's GenerationEngine to extract patterns from completed agent sessions
//! and produce skill files that can be loaded back via thulp-skill-files.
//!
//! Flow: PawanSession → TeacherSession → GenerationEngine → GeneratedSkill → SKILL.md

use crate::agent::session::Session;
use crate::agent::{Message, Role, TokenUsage as PawanUsage};
use crate::config::{LlmProvider, PawanConfig};
use crate::{PawanError, Result};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thulpoff_core::{
    CompletionRequest, CompletionResponse, EvaluationResult, GeneratedSkill,
    LlmProvider as ThulpoffProvider, Message as ToffMessage, MessageRole, TeacherSession,
    TokenUsage as ToffUsage, ToolCall as ToffToolCall,
};
use thulpoff_engine::{EvaluationEngine, GenerationEngine, RefinementEngine};

// ---------------------------------------------------------------------------
// Type conversions: pawan → thulpoff
// ---------------------------------------------------------------------------

fn convert_role(role: &Role) -> MessageRole {
    match role {
        Role::System => MessageRole::System,
        Role::User => MessageRole::User,
        Role::Assistant => MessageRole::Assistant,
        Role::Tool => MessageRole::Tool,
    }
}

fn convert_message(msg: &Message) -> ToffMessage {
    let tool_calls = if msg.tool_calls.is_empty() {
        None
    } else {
        Some(
            msg.tool_calls
                .iter()
                .map(|tc| ToffToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .collect(),
        )
    };

    let tool_call_id = msg.tool_result.as_ref().map(|tr| tr.tool_call_id.clone());

    ToffMessage {
        role: convert_role(&msg.role),
        content: msg.content.clone(),
        tool_calls,
        tool_call_id,
    }
}

fn convert_usage(usage: &PawanUsage) -> ToffUsage {
    ToffUsage {
        input_tokens: usage.prompt_tokens as u32,
        output_tokens: usage.completion_tokens as u32,
    }
}

/// Convert a pawan Session into a thulpoff TeacherSession.
///
/// Extracts the first user message as the task description, collects all messages
/// and tool calls, and maps token usage.
pub fn session_to_teacher(session: &Session, usage: &PawanUsage) -> TeacherSession {
    // Find the first user message as task description
    let task_description = session
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.clone())
        .unwrap_or_else(|| "Unknown task".to_string());

    // Collect all tool calls from assistant messages
    let tool_calls: Vec<ToffToolCall> = session
        .messages
        .iter()
        .flat_map(|m| {
            m.tool_calls.iter().map(|tc| ToffToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            })
        })
        .collect();

    TeacherSession {
        task_description,
        messages: session.messages.iter().map(convert_message).collect(),
        tool_calls,
        model: session.model.clone(),
        usage: convert_usage(usage),
    }
}

// ---------------------------------------------------------------------------
// Provider adapter: reuse pawan's NVIDIA/OpenAI config for thulpoff
// ---------------------------------------------------------------------------

/// Adapter that wraps pawan's HTTP config to implement thulpoff's LlmProvider trait.
struct PawanProviderAdapter {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl PawanProviderAdapter {
    fn from_config(config: &PawanConfig) -> Result<Self> {
        let (base_url, api_key) = match config.provider {
            LlmProvider::Nvidia => {
                let url = config
                    .base_url
                    .clone()
                    .or_else(|| std::env::var("NVIDIA_API_URL").ok())
                    .unwrap_or_else(|| crate::DEFAULT_NVIDIA_API_URL.to_string());
                let key = std::env::var("NVIDIA_API_KEY").ok();
                (url, key)
            }
            LlmProvider::Ollama => {
                let url = config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434/v1".to_string());
                (url, None)
            }
            LlmProvider::OpenAI => {
                let url = config
                    .base_url
                    .clone()
                    .or_else(|| std::env::var("OPENAI_API_URL").ok())
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                let key = std::env::var("OPENAI_API_KEY").ok();
                (url, key)
            }
            LlmProvider::Mlx => {
                let url = config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8080/v1".to_string());
                (url, None)
            }
        };

        let api_key = api_key.unwrap_or_default();
        if api_key.is_empty() && config.provider == LlmProvider::Nvidia {
            return Err(PawanError::Config(
                "NVIDIA_API_KEY not set — needed for skill distillation".into(),
            ));
        }

        Ok(Self {
            client: reqwest::Client::new(),
            api_key,
            base_url,
        })
    }
}

#[async_trait::async_trait]
impl ThulpoffProvider for PawanProviderAdapter {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> thulpoff_core::Result<CompletionResponse> {
        let url = format!("{}/chat/completions", self.base_url);

        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages,
        });

        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(max_tokens);
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = serde_json::json!(temperature);
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| thulpoff_core::ThulpoffError::Provider(e.to_string()))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| thulpoff_core::ThulpoffError::Provider(e.to_string()))?;

        if !status.is_success() {
            return Err(thulpoff_core::ThulpoffError::Provider(format!(
                "API error {}: {}",
                status, text
            )));
        }

        let json: serde_json::Value = serde_json::from_str(&text)?;

        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = ToffUsage {
            input_tokens: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: json["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
        };

        let finish_reason = json["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("stop");
        let fr = match finish_reason {
            "tool_calls" => thulpoff_core::FinishReason::ToolUse,
            "length" => thulpoff_core::FinishReason::MaxTokens,
            _ => thulpoff_core::FinishReason::Stop,
        };

        Ok(CompletionResponse {
            content,
            tool_calls: vec![],
            usage,
            finish_reason: fr,
        })
    }

    fn name(&self) -> &str {
        "pawan-adapter"
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Distill a pawan session into a GeneratedSkill using the configured LLM.
///
/// This calls thulpoff's GenerationEngine to analyze the session trace and
/// extract a reusable skill definition.
pub async fn distill_session(
    session: &Session,
    usage: &PawanUsage,
    config: &PawanConfig,
) -> Result<GeneratedSkill> {
    let teacher = session_to_teacher(session, usage);

    let adapter = PawanProviderAdapter::from_config(config)?;
    let engine = GenerationEngine::new(Arc::new(adapter));

    engine
        .generate(&teacher)
        .await
        .map_err(|e| PawanError::Agent(format!("Skill distillation failed: {}", e)))
}

/// Distill and save a session as a SKILL.md file.
///
/// Returns the path where the skill was written.
pub async fn distill_and_save(
    session: &Session,
    usage: &PawanUsage,
    config: &PawanConfig,
    output_dir: &Path,
) -> Result<PathBuf> {
    let skill = distill_session(session, usage, config).await?;
    save_skill(&skill, output_dir)
}

/// Evaluate a distilled skill against a student model using thulpoff's
/// EvaluationEngine. Runs each test case through the student model with
/// the skill in context and scores how well the skill's pass_criteria
/// are met.
///
/// Returns an `EvaluationResult` with per-test scores and an `overall_score`
/// between 0.0 and 1.0. A perfect score means every pass_criterion was met
/// in every test case.
pub async fn evaluate_skill(
    skill: &GeneratedSkill,
    student_model: &str,
    config: &PawanConfig,
) -> Result<EvaluationResult> {
    let adapter = PawanProviderAdapter::from_config(config)?;
    let engine = EvaluationEngine::new(Arc::new(adapter));

    engine
        .evaluate(skill, student_model)
        .await
        .map_err(|e| PawanError::Agent(format!("Skill evaluation failed: {}", e)))
}

/// Refine a skill based on its evaluation results using thulpoff's
/// RefinementEngine. If the skill scored less than 1.0, the refinement
/// engine calls the teacher model with the skill + failing test details
/// and generates an improved version of the skill content.
///
/// The refined skill keeps the original name, frontmatter, and test_cases
/// — only description and content are updated based on failure analysis.
pub async fn refine_skill(
    skill: &GeneratedSkill,
    eval_result: &EvaluationResult,
    config: &PawanConfig,
) -> Result<GeneratedSkill> {
    let adapter = PawanProviderAdapter::from_config(config)?;
    let engine = RefinementEngine::new(Arc::new(adapter));

    engine
        .refine(skill, eval_result, &config.model)
        .await
        .map_err(|e| PawanError::Agent(format!("Skill refinement failed: {}", e)))
}

/// Full distill → evaluate → refine → save loop.
///
/// 1. Distills the session into a skill
/// 2. Evaluates against the student model (or primary model if None)
/// 3. If score < 1.0, refines once using the teacher model
/// 4. Saves the final skill to disk
///
/// Returns a tuple of `(saved_path, initial_score, final_score)`.
pub async fn distill_eval_refine_save(
    session: &Session,
    usage: &PawanUsage,
    config: &PawanConfig,
    output_dir: &Path,
    student_model: Option<&str>,
) -> Result<(PathBuf, f64, f64)> {
    let skill = distill_session(session, usage, config).await?;
    let student = student_model.unwrap_or(&config.model);

    let eval = evaluate_skill(&skill, student, config).await?;
    let initial_score = eval.overall_score;

    let final_skill = if initial_score < 1.0 {
        refine_skill(&skill, &eval, config).await?
    } else {
        skill
    };

    // Re-evaluate the refined skill to get the final score
    let final_score = if initial_score < 1.0 {
        let eval2 = evaluate_skill(&final_skill, student, config).await?;
        eval2.overall_score
    } else {
        initial_score
    };

    let path = save_skill(&final_skill, output_dir)?;
    Ok((path, initial_score, final_score))
}

/// Save a GeneratedSkill as a SKILL.md file in a named subdirectory.
pub fn save_skill(skill: &GeneratedSkill, output_dir: &Path) -> Result<PathBuf> {
    let skill_dir = output_dir.join(&skill.name);
    std::fs::create_dir_all(&skill_dir)
        .map_err(PawanError::Io)?;

    let content = format_skill_md(skill);
    let path = skill_dir.join("SKILL.md");
    std::fs::write(&path, content)?;

    Ok(path)
}

/// Format a GeneratedSkill into the SKILL.md format that thulp-skill-files can parse.
fn format_skill_md(skill: &GeneratedSkill) -> String {
    let frontmatter = if skill.frontmatter.is_null() {
        serde_json::json!({
            "name": skill.name,
            "description": skill.description,
        })
    } else {
        skill.frontmatter.clone()
    };

    let frontmatter_yaml = format!(
        "name: {}\ndescription: {}",
        skill.name, skill.description
    );

    let mut md = String::new();
    md.push_str("---\n");
    md.push_str(&frontmatter_yaml);

    // Add any extra frontmatter fields from the generation
    if let Some(obj) = frontmatter.as_object() {
        for (key, val) in obj {
            if key != "name" && key != "description" {
                md.push_str(&format!("\n{}: {}", key, val));
            }
        }
    }

    md.push_str("\n---\n\n");
    md.push_str(&skill.content);

    if !skill.test_cases.is_empty() {
        md.push_str("\n\n## Test Cases\n\n```json\n");
        if let Ok(json) = serde_json::to_string_pretty(&skill.test_cases) {
            md.push_str(&json);
        }
        md.push_str("\n```\n");
    }

    md
}

/// Get the default skills output directory (~/.pawan/skills/).
pub fn skills_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let dir = PathBuf::from(home).join(".pawan").join("skills");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

/// Check if a session is worth distilling (has enough substance).
///
/// A session needs at least 1 user message and 1 tool call to be useful.
pub fn is_distillable(session: &Session) -> bool {
    let has_user_msg = session.messages.iter().any(|m| m.role == Role::User);
    let has_tool_calls = session
        .messages
        .iter()
        .any(|m| !m.tool_calls.is_empty());
    let min_messages = session.messages.len() >= 4; // system + user + assistant + tool result

    has_user_msg && has_tool_calls && min_messages
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{ToolCallRequest, ToolResultMessage};

    fn make_test_session() -> Session {
        Session {
            id: "test-123".to_string(),
            model: "test-model".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:01:00Z".to_string(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are a coding agent.".to_string(),
                    tool_calls: vec![],
                    tool_result: None,
                },
                Message {
                    role: Role::User,
                    content: "Fix the bug in main.rs".to_string(),
                    tool_calls: vec![],
                    tool_result: None,
                },
                Message {
                    role: Role::Assistant,
                    content: "I'll read the file first.".to_string(),
                    tool_calls: vec![ToolCallRequest {
                        id: "tc-1".to_string(),
                        name: "read_file".to_string(),
                        arguments: serde_json::json!({"path": "main.rs"}),
                    }],
                    tool_result: None,
                },
                Message {
                    role: Role::Tool,
                    content: "fn main() { panic!() }".to_string(),
                    tool_calls: vec![],
                    tool_result: Some(ToolResultMessage {
                        tool_call_id: "tc-1".to_string(),
                        content: serde_json::json!("fn main() { panic!() }"),
                        success: true,
                    }),
                },
                Message {
                    role: Role::Assistant,
                    content: "Found the issue. Fixing...".to_string(),
                    tool_calls: vec![ToolCallRequest {
                        id: "tc-2".to_string(),
                        name: "write_file".to_string(),
                        arguments: serde_json::json!({"path": "main.rs", "content": "fn main() { println!(\"hello\"); }"}),
                    }],
                    tool_result: None,
                },
                Message {
                    role: Role::Tool,
                    content: "File written.".to_string(),
                    tool_calls: vec![],
                    tool_result: Some(ToolResultMessage {
                        tool_call_id: "tc-2".to_string(),
                        content: serde_json::json!("File written successfully"),
                        success: true,
                    }),
                },
            ],
            total_tokens: 1500,
            iteration_count: 2,
        }
    }

    fn make_usage() -> PawanUsage {
        PawanUsage {
            prompt_tokens: 1000,
            completion_tokens: 500,
            total_tokens: 1500,
            reasoning_tokens: 100,
            action_tokens: 400,
        }
    }

    #[test]
    fn test_convert_role() {
        assert_eq!(convert_role(&Role::System), MessageRole::System);
        assert_eq!(convert_role(&Role::User), MessageRole::User);
        assert_eq!(convert_role(&Role::Assistant), MessageRole::Assistant);
        assert_eq!(convert_role(&Role::Tool), MessageRole::Tool);
    }

    #[test]
    fn test_convert_message_simple() {
        let msg = Message {
            role: Role::User,
            content: "hello".to_string(),
            tool_calls: vec![],
            tool_result: None,
        };
        let converted = convert_message(&msg);
        assert_eq!(converted.role, MessageRole::User);
        assert_eq!(converted.content, "hello");
        assert!(converted.tool_calls.is_none());
        assert!(converted.tool_call_id.is_none());
    }

    #[test]
    fn test_convert_message_with_tool_calls() {
        let msg = Message {
            role: Role::Assistant,
            content: "Reading file".to_string(),
            tool_calls: vec![ToolCallRequest {
                id: "tc-1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "foo.rs"}),
            }],
            tool_result: None,
        };
        let converted = convert_message(&msg);
        let calls = converted.tool_calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].id, "tc-1");
    }

    #[test]
    fn test_convert_message_tool_result() {
        let msg = Message {
            role: Role::Tool,
            content: "result".to_string(),
            tool_calls: vec![],
            tool_result: Some(ToolResultMessage {
                tool_call_id: "tc-1".to_string(),
                content: serde_json::json!("ok"),
                success: true,
            }),
        };
        let converted = convert_message(&msg);
        assert_eq!(converted.tool_call_id, Some("tc-1".to_string()));
    }

    #[test]
    fn test_convert_usage() {
        let usage = make_usage();
        let converted = convert_usage(&usage);
        assert_eq!(converted.input_tokens, 1000);
        assert_eq!(converted.output_tokens, 500);
    }

    #[test]
    fn test_session_to_teacher() {
        let session = make_test_session();
        let usage = make_usage();
        let teacher = session_to_teacher(&session, &usage);

        assert_eq!(teacher.task_description, "Fix the bug in main.rs");
        assert_eq!(teacher.model, "test-model");
        assert_eq!(teacher.messages.len(), 6);
        assert_eq!(teacher.tool_calls.len(), 2);
        assert_eq!(teacher.tool_calls[0].name, "read_file");
        assert_eq!(teacher.tool_calls[1].name, "write_file");
        assert_eq!(teacher.usage.input_tokens, 1000);
    }

    #[test]
    fn test_is_distillable() {
        let session = make_test_session();
        assert!(is_distillable(&session));
    }

    #[test]
    fn test_not_distillable_no_tools() {
        let session = Session {
            id: "empty".to_string(),
            model: "m".to_string(),
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: "hi".to_string(),
                    tool_calls: vec![],
                    tool_result: None,
                },
                Message {
                    role: Role::Assistant,
                    content: "hello".to_string(),
                    tool_calls: vec![],
                    tool_result: None,
                },
            ],
            total_tokens: 100,
            iteration_count: 1,
        };
        assert!(!is_distillable(&session));
    }

    #[test]
    fn test_format_skill_md() {
        let skill = GeneratedSkill {
            name: "fix-bug".to_string(),
            description: "Fix common bugs in Rust code".to_string(),
            frontmatter: serde_json::json!({"name": "fix-bug", "description": "Fix common bugs"}),
            content: "## Steps\n\n1. Read the file\n2. Identify the bug\n3. Fix it".to_string(),
            test_cases: vec![thulpoff_core::TestCase {
                name: "basic-fix".to_string(),
                input: serde_json::json!({"file": "main.rs"}),
                expected_behavior: "Bug is fixed".to_string(),
                pass_criteria: vec!["compiles".to_string(), "no panic".to_string()],
            }],
            source_session: Some("test-123".to_string()),
        };

        let md = format_skill_md(&skill);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("name: fix-bug"));
        assert!(md.contains("## Steps"));
        assert!(md.contains("## Test Cases"));
        assert!(md.contains("basic-fix"));
    }

    #[test]
    fn test_save_skill_creates_dir() {
        let skill = GeneratedSkill {
            name: "test-skill".to_string(),
            description: "A test".to_string(),
            frontmatter: serde_json::json!({}),
            content: "Do the thing.".to_string(),
            test_cases: vec![],
            source_session: None,
        };

        let dir = tempfile::tempdir().unwrap();
        let path = save_skill(&skill, dir.path()).unwrap();

        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "SKILL.md");
        assert!(path.parent().unwrap().ends_with("test-skill"));

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("name: test-skill"));
        assert!(content.contains("Do the thing."));
    }

    #[test]
    fn test_skills_dir_creates_directory() {
        let dir = skills_dir().unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with("skills"));
    }

    #[test]
    fn test_is_distillable_too_few_messages() {
        // Even with a tool call, a session with <4 messages is not worth
        // distilling — you can't learn a pattern from one round trip.
        let session = Session {
            id: "short".into(),
            model: "m".into(),
            created_at: "now".into(),
            updated_at: "now".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: "do stuff".into(),
                    tool_calls: vec![],
                    tool_result: None,
                },
                Message {
                    role: Role::Assistant,
                    content: "running tool".into(),
                    tool_calls: vec![ToolCallRequest {
                        id: "tc-1".into(),
                        name: "ls".into(),
                        arguments: serde_json::json!({}),
                    }],
                    tool_result: None,
                },
                Message {
                    role: Role::Tool,
                    content: "output".into(),
                    tool_calls: vec![],
                    tool_result: Some(ToolResultMessage {
                        tool_call_id: "tc-1".into(),
                        content: serde_json::json!("ok"),
                        success: true,
                    }),
                },
            ],
            total_tokens: 100,
            iteration_count: 1,
        };
        // Has user + tools but only 3 messages < min_messages (4)
        assert!(!is_distillable(&session), "sessions with <4 messages must not be distillable");
    }

    #[test]
    fn test_session_to_teacher_without_user_falls_back_to_unknown() {
        // A session that somehow has no user message (system-only prompt)
        // must still produce a TeacherSession — just with a placeholder task.
        let session = Session {
            id: "no-user".into(),
            model: "m".into(),
            created_at: "now".into(),
            updated_at: "now".into(),
            messages: vec![Message {
                role: Role::System,
                content: "you are a bot".into(),
                tool_calls: vec![],
                tool_result: None,
            }],
            total_tokens: 0,
            iteration_count: 0,
        };
        let teacher = session_to_teacher(&session, &make_usage());
        assert_eq!(teacher.task_description, "Unknown task");
        assert_eq!(teacher.messages.len(), 1);
        assert_eq!(teacher.tool_calls.len(), 0);
    }

    #[test]
    fn test_format_skill_md_without_test_cases_omits_test_section() {
        // When test_cases is empty, the "## Test Cases" section must be
        // omitted entirely — not left dangling with empty code fences.
        let skill = GeneratedSkill {
            name: "just-steps".into(),
            description: "No tests attached".into(),
            frontmatter: serde_json::json!({}),
            content: "## Do this\nThen do that".into(),
            test_cases: vec![],
            source_session: None,
        };
        let md = format_skill_md(&skill);
        assert!(md.contains("## Do this"), "content must be present");
        assert!(!md.contains("## Test Cases"), "no test cases ⇒ no test section");
        assert!(!md.contains("```json"), "no test cases ⇒ no json fence");
    }

    #[test]
    fn test_format_skill_md_preserves_extra_frontmatter_fields() {
        // Frontmatter fields beyond name/description must be emitted after
        // the standard two — regression test for the object iteration
        // branch in format_skill_md.
        let skill = GeneratedSkill {
            name: "skill-with-meta".into(),
            description: "Has extra metadata".into(),
            frontmatter: serde_json::json!({
                "name": "skill-with-meta",
                "description": "Has extra metadata",
                "version": "1.2.3",
                "tags": ["rust", "test"]
            }),
            content: "body".into(),
            test_cases: vec![],
            source_session: None,
        };
        let md = format_skill_md(&skill);
        assert!(md.contains("name: skill-with-meta"));
        assert!(md.contains("description: Has extra metadata"));
        assert!(md.contains("version:"), "extra 'version' field must be emitted");
        assert!(md.contains("tags:"), "extra 'tags' field must be emitted");
    }

    #[test]
    fn test_save_skill_on_existing_dir_overwrites() {
        // Calling save_skill twice with the same name must succeed —
        // create_dir_all + write overwrites, and we don't want the second
        // call to fail because the first already ran.
        let skill_v1 = GeneratedSkill {
            name: "iter-skill".into(),
            description: "v1 description".into(),
            frontmatter: serde_json::json!({}),
            content: "v1 body".into(),
            test_cases: vec![],
            source_session: None,
        };
        let skill_v2 = GeneratedSkill {
            name: "iter-skill".into(),
            description: "v2 description".into(),
            frontmatter: serde_json::json!({}),
            content: "v2 body".into(),
            test_cases: vec![],
            source_session: None,
        };

        let dir = tempfile::tempdir().unwrap();
        let p1 = save_skill(&skill_v1, dir.path()).unwrap();
        let p2 = save_skill(&skill_v2, dir.path()).unwrap();
        assert_eq!(p1, p2, "second save should write to the same path");

        let content = std::fs::read_to_string(&p2).unwrap();
        assert!(content.contains("v2 body"), "file should contain v2 content");
        assert!(!content.contains("v1 body"), "v1 content should be overwritten");
    }
}
