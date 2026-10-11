//! The three-invariant funnel tests for the embedded OpenAPPA guard, plus
//! the measured permission-defect regressions.
//!
//! Every decision shape asserted here was first MEASURED against the real
//! engine at the pinned rev (1c52575) with a path-dep probe harness, and
//! the measured shape is named in each test so the funnel mapping stays
//! auditable:
//!
//! | measured engine shape | test |
//! |---|---|
//! | `requires={attention=["blocked"]}` → `DenyCall(offers=0)` | invariant 1 |
//! | battery on a closed port → `Refuse(annotator=… error=transport)` | invariant 3 (outage arm) |
//! | same battery answering → `AllowCall` / result `Ack` | invariant 3 (healthy control) |
//! | confined result, sanitizer down → `ReplaceOutput("[appa] no registered sanitizer answered…")` | invariant 2 (outage arm) |
//! | confined result, sanitizer live → `DeliverValue("scrubbed")` | invariant 2 (healthy control) |
//! | remedy flow: `DenyCall(offers=2)` → control call `PassControl` → `execute_embedded_remedy Authorized` → recall `AllowCall` | invariant 2 driver |
//!
//! Attribution discipline: the canary tools report `mutating() == false` and
//! are absent from the default-Prompt permission list, so the permission
//! gates cannot deny them — only the guardrail can. A denial in these
//! tests is the guardrail's or the test is lying.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::appa::{AppaGuard, Battery, GuardState};
use super::backend::LlmBackend;
use super::types::{LLMResponse, ToolCallRequest};
use super::{Message, PawanAgent, Role};
use crate::config::{PawanConfig, ToolPermission};
use crate::tools::{Tool, ToolRegistry};
use crate::{PawanError, Result};

/// The root policy under test, spelled with pawan's native tool names (the
/// adapter identifies them as `host/pawan/<name>`; the engine matches the
/// native spelling — measured).
const ROOT_POLICY: &str = r#"
[policy]
version = 2
trust_chain = ["suspicious", "trusted"]

[[policy.tool]]
name = "read_status"
delta = {}

# The deterministic hard deny: `blocked` is the reserved attention mark no
# authority can permit, so this contract has no remedy — measured
# DenyCall(offers=0).
[[policy.tool]]
name = "canary_write"
delta = {}
requires = { attention = ["blocked"] }
"#;

/// The hosted battery: a classifier annotator on `classified_read` (invariant
/// 3's battery) and a confined result on `leaky_read` with an output
/// sanitizer (invariant 2). The two URLs are templated so each arm points
/// them at a closed port or a live stub.
const BATTERY_POLICY: &str = r#"
[policy]
version = 2

[[policy.annotator]]
name = "classify"
ranks = ["suspicious", "trusted"]
audiences = ["internal"]
marks = []
effects = []

[[policy.tool]]
name = "host/pawan/classified_read"
annotator = "classify"

[externals.annotators.classify]
url = "__CLASSIFIER_URL__"

# A confined result: the delta restricts the readers, so the raw body can
# never be delivered as produced — the sanitizer's derivation crosses, or
# nothing does.
[[policy.tool]]
name = "host/pawan/leaky_read"
delta = { audience = ["insider"] }

[[policy.sanitizer]]
name = "scrub"
on = ["tool_output"]
[policy.sanitizer.permits]
audience = { from = ["insider"], to = ["public"] }

[externals.sanitizers.scrub]
url = "__SANITIZER_URL__"

[policy.deployment]
confined_results = ["host/pawan/leaky_read"]
"#;

/// A port nothing listens on: every consult against it fails fast. The
/// outage arm points its battery here.
const CLOSED_PORT: &str = "http://127.0.0.1:1/";

// ---------------------------------------------------------------------------
// Canary tools — observable side effects for the "never executed" proofs
// ---------------------------------------------------------------------------

/// A tool the guardrail must deny: records an invocation count and writes a
/// canary file. If the funnel ever lets a denied call through, both the
/// counter and the file prove it.
struct CanaryWriteTool {
    invocations: Arc<AtomicUsize>,
    canary_path: std::path::PathBuf,
}

#[async_trait::async_trait]
impl Tool for CanaryWriteTool {
    fn name(&self) -> &str {
        "canary_write"
    }
    fn description(&self) -> &str {
        "Writes a canary file (test instrumentation)"
    }
    // Deliberately NOT mutating and NOT in the default-Prompt list: only the
    // guardrail can deny this tool.
    fn mutating(&self) -> bool {
        false
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn execute(&self, _args: Value) -> Result<Value> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        std::fs::write(&self.canary_path, b"canary-side-effect").map_err(PawanError::Io)?;
        Ok(json!({"written": true}))
    }
}

/// A read tool whose result is the canary payload the model must never see
/// when the result check withholds it.
struct LeakyReadTool {
    invocations: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for LeakyReadTool {
    fn name(&self) -> &str {
        "leaky_read"
    }
    fn description(&self) -> &str {
        "Returns the canary payload (test instrumentation)"
    }
    fn mutating(&self) -> bool {
        false
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"q": {"type": "string"}}})
    }
    async fn execute(&self, _args: Value) -> Result<Value> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(json!("TOPSECRET-CANARY"))
    }
}

/// The tool invariant 3's battery classifies.
struct ClassifiedReadTool {
    invocations: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for ClassifiedReadTool {
    fn name(&self) -> &str {
        "classified_read"
    }
    fn description(&self) -> &str {
        "Returns a body the classifier annotates (test instrumentation)"
    }
    fn mutating(&self) -> bool {
        false
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn execute(&self, _args: Value) -> Result<Value> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(json!("classified-body"))
    }
}

// ---------------------------------------------------------------------------
// Deterministic backends — the model's side of the loop, scripted
// ---------------------------------------------------------------------------

/// Turn 1: one tool call. Turn 2+: a final text answer.
struct OneToolBackend {
    name: String,
    args: Value,
    turn: AtomicUsize,
}

impl OneToolBackend {
    fn new(name: &str, args: Value) -> Self {
        Self {
            name: name.to_string(),
            args,
            turn: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl LlmBackend for OneToolBackend {
    async fn generate(
        &self,
        _messages: &[Message],
        _tools: &[crate::tools::ToolDefinition],
        _on_token: Option<&super::TokenCallback>,
    ) -> Result<LLMResponse> {
        let turn = self.turn.fetch_add(1, Ordering::SeqCst);
        if turn == 0 {
            return Ok(LLMResponse {
                content: String::new(),
                reasoning: None,
                tool_calls: vec![ToolCallRequest {
                    id: "one-1".to_string(),
                    name: self.name.clone(),
                    arguments: self.args.clone(),
                }],
                finish_reason: "tool_calls".to_string(),
                usage: None,
            });
        }
        Ok(LLMResponse {
            content: "done".to_string(),
            reasoning: None,
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
            usage: None,
        })
    }
}

/// The measured remedy flow, played as a model plays it:
/// 1. propose `leaky_read`;
/// 2. read the `offer_id` the denial feedback names and quote it on the
///    control tool (the engine's `PassControl` → `execute_embedded_remedy`);
/// 3. re-propose `leaky_read` (now `AllowCall`);
/// 4. answer.
struct RemedyFlowBackend {
    name: String,
    args: Value,
    turn: AtomicUsize,
}

#[async_trait::async_trait]
impl LlmBackend for RemedyFlowBackend {
    async fn generate(
        &self,
        messages: &[Message],
        _tools: &[crate::tools::ToolDefinition],
        _on_token: Option<&super::TokenCallback>,
    ) -> Result<LLMResponse> {
        let turn = self.turn.fetch_add(1, Ordering::SeqCst);
        let call = |name: String, arguments: Value| ToolCallRequest {
            id: format!("flow-{turn}"),
            name,
            arguments,
        };
        match turn {
            0 => Ok(LLMResponse {
                content: String::new(),
                reasoning: None,
                tool_calls: vec![call(self.name.clone(), self.args.clone())],
                finish_reason: "tool_calls".to_string(),
                usage: None,
            }),
            1 => {
                let offer = latest_offer_id(messages);
                Ok(LLMResponse {
                    content: String::new(),
                    reasoning: None,
                    tool_calls: vec![call(
                        super::appa::CONTROL_SPELLING.to_string(),
                        json!({"offer_id": offer.unwrap_or_default()}),
                    )],
                    finish_reason: "tool_calls".to_string(),
                    usage: None,
                })
            }
            2 => Ok(LLMResponse {
                content: String::new(),
                reasoning: None,
                tool_calls: vec![call(self.name.clone(), self.args.clone())],
                finish_reason: "tool_calls".to_string(),
                usage: None,
            }),
            _ => Ok(LLMResponse {
                content: "done".to_string(),
                reasoning: None,
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
                usage: None,
            }),
        }
    }
}

/// The offer id the runtime's feedback spells for the model:
/// `execute_remedy_plan(offer_id: "…")`, taken from the newest tool message.
///
/// The sanitizer offer is preferred when the feedback names one: the
/// measured confined-result flow takes `Use sanitizer …'s result` (the
/// narrowing-acceptance offer alone admits the RAW result — measured).
fn latest_offer_id(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find_map(|m| pick_offer(&message_text(m)))
}

/// The text a tool message carries to the model: the denial's raw feedback
/// string when the record is a denial (NOT its JSON serialization — the
/// quotes in `offer_id: "…"` are escaped there and no marker matches).
fn message_text(m: &Message) -> String {
    if let Some(t) = &m.tool_result {
        if let Some(error) = t.content.get("error").and_then(|v| v.as_str()) {
            return error.to_string();
        }
        return t.content.to_string();
    }
    m.content.clone()
}

/// The offer id quoted after `marker`, or None when the marker is absent.
fn offer_after(text: &str, marker: &str) -> Option<String> {
    let at = text.find(marker)? + marker.len();
    let rest = &text[at..];
    let quoted = rest.find("offer_id: \"")? + "offer_id: \"".len();
    let rest = &rest[quoted..];
    Some(rest[..rest.find('"')?].to_string())
}

/// Prefer the sanitizer offer; fall back to the first quoted offer.
fn pick_offer(text: &str) -> Option<String> {
    offer_after(text, "Use sanitizer").or_else(|| {
        let marker = "offer_id: \"";
        let at = text.find(marker)? + marker.len();
        let rest = &text[at..];
        Some(rest[..rest.find('"')?].to_string())
    })
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn battery_with(classifier_url: &str, sanitizer_url: &str) -> Battery {
    Battery {
        name: "pawan".to_string(),
        policy: BATTERY_POLICY
            .replace("__CLASSIFIER_URL__", classifier_url)
            .replace("__SANITIZER_URL__", sanitizer_url),
        token_env: vec![],
    }
}

fn agent_with_guard(
    guard: Arc<AppaGuard>,
    backend: Box<dyn LlmBackend>,
    tools: ToolRegistry,
    workspace: &std::path::Path,
) -> PawanAgent {
    let mut config = PawanConfig::default();
    config.appa.enabled = true;
    let mut agent = PawanAgent::new(config, workspace.to_path_buf())
        .with_tools(tools)
        .with_backend(backend);
    // Install the guard built from documents (the config-file path is
    // exercised by the open_guard unit test).
    agent.appa = GuardState::Active(guard);
    agent
}

fn history_contains(agent: &PawanAgent, needle: &str) -> bool {
    agent.history.iter().any(|m| {
        m.content.contains(needle)
            || m.tool_result
                .as_ref()
                .map(|t| t.content.to_string().contains(needle))
                .unwrap_or(false)
    })
}

fn tool_results(agent: &PawanAgent) -> Vec<String> {
    agent
        .history
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| {
            m.tool_result
                .as_ref()
                .map(|t| t.content.to_string())
                .unwrap_or_default()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Invariant 1 — a denied call never executes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invariant_1_a_denied_call_never_executes() {
    // Measured: requires={attention=["blocked"]} → DenyCall(offers=0), the
    // deterministic hard deny.
    let dir = tempfile::tempdir().expect("temp dir");
    let canary_path = dir.path().join("canary.txt");
    let invocations = Arc::new(AtomicUsize::new(0));

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(CanaryWriteTool {
        invocations: invocations.clone(),
        canary_path: canary_path.clone(),
    }));

    let guard = AppaGuard::from_documents("invariant-1", ROOT_POLICY, vec![], 3_000, 65_536)
        .expect("the guard opens");
    let mut agent = agent_with_guard(
        guard,
        Box::new(OneToolBackend::new("canary_write", json!({}))),
        registry,
        dir.path(),
    );

    let response = agent
        .execute_with_all_callbacks("write the canary", None, None, None, None)
        .await
        .expect("the turn completes with the denial recorded");

    // (a) the tool's real execute path recorded ZERO invocations
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "a denied call must never reach the tool's execute path"
    );
    // (b) the canary side effect is absent
    assert!(
        !canary_path.exists(),
        "the canary side effect must be absent"
    );
    // (c) the model read the engine's words, not a success
    let results = tool_results(&agent);
    assert!(
        results.iter().any(|r| r.contains("cannot run yet")),
        "the model must read the denial feedback; got {results:?}"
    );
    assert_eq!(response.tool_calls.len(), 1);
    assert!(!response.tool_calls[0].success);
}

// ---------------------------------------------------------------------------
// Invariant 2 — a blocked result never reaches the model
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invariant_2_a_withheld_result_never_reaches_the_model() {
    // Measured: confined result with the sanitizer DOWN →
    // ReplaceOutput("[appa] no registered sanitizer answered; the result is
    // withheld…"): the raw body never crosses.
    let dir = tempfile::tempdir().expect("temp dir");
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(LeakyReadTool {
        invocations: invocations.clone(),
    }));

    let guard = AppaGuard::from_documents(
        "invariant-2-outage",
        ROOT_POLICY,
        vec![battery_with(CLOSED_PORT, CLOSED_PORT)],
        3_000,
        65_536,
    )
    .expect("the guard opens");
    let mut agent = agent_with_guard(
        guard,
        Box::new(RemedyFlowBackend {
            name: "leaky_read".to_string(),
            args: json!({"q": "x"}),
            turn: AtomicUsize::new(0),
        }),
        registry,
        dir.path(),
    );

    agent
        .execute_with_all_callbacks("read the secret", None, None, None, None)
        .await
        .expect("the turn completes with the result withheld");

    // the call itself ran (the call side was allowed after the remedy) —
    // this is a RESULT-side proof, not a call-side one
    assert_eq!(invocations.load(Ordering::SeqCst), 1);

    let joined = tool_results(&agent).join("\n");
    assert!(
        !joined.contains("TOPSECRET-CANARY"),
        "the raw body must never reach the model; got {joined}"
    );
    assert!(
        joined.contains("no registered sanitizer answered"),
        "the model must read the runtime's withholding words; got {joined}"
    );
}

#[tokio::test]
async fn invariant_2_control_a_healthy_battery_delivers_the_sanitized_derivation() {
    // Measured: the same call with the sanitizer LIVE →
    // DeliverValue("scrubbed"): the derivation crosses, the raw never does.
    // This is the control that proves invariant 2's withholding is
    // conditional, not a hard block of everything.
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex(r#""kind":"sanitizer""#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"version":1,"answer":{"body":"scrubbed"}}"#)
        .create_async()
        .await;

    let dir = tempfile::tempdir().expect("temp dir");
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(LeakyReadTool {
        invocations: invocations.clone(),
    }));

    let guard = AppaGuard::from_documents(
        "invariant-2-healthy",
        ROOT_POLICY,
        vec![battery_with(CLOSED_PORT, &server.url())],
        3_000,
        65_536,
    )
    .expect("the guard opens");
    let mut agent = agent_with_guard(
        guard,
        Box::new(RemedyFlowBackend {
            name: "leaky_read".to_string(),
            args: json!({"q": "x"}),
            turn: AtomicUsize::new(0),
        }),
        registry,
        dir.path(),
    );

    agent
        .execute_with_all_callbacks("read the secret", None, None, None, None)
        .await
        .expect("the turn completes with the derivation delivered");

    let joined = tool_results(&agent).join("\n");
    assert!(
        !joined.contains("TOPSECRET-CANARY"),
        "the raw body must never reach the model even on the healthy arm; got {joined}"
    );
    assert!(
        joined.contains("scrubbed"),
        "the model must read the sanitizer's derivation; got {joined}"
    );
}

// ---------------------------------------------------------------------------
// Invariant 3 — an outage refuses, and the healthy control proceeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invariant_3_a_battery_outage_refuses_the_call() {
    // Measured: a battery on a closed port → Refuse(annotator=classify
    // error=transport): the call does not run, never a default allow.
    let dir = tempfile::tempdir().expect("temp dir");
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ClassifiedReadTool {
        invocations: invocations.clone(),
    }));

    let guard = AppaGuard::from_documents(
        "invariant-3-outage",
        ROOT_POLICY,
        vec![battery_with(CLOSED_PORT, CLOSED_PORT)],
        3_000,
        65_536,
    )
    .expect("the guard opens");
    let mut agent = agent_with_guard(
        guard,
        Box::new(OneToolBackend::new("classified_read", json!({}))),
        registry,
        dir.path(),
    );

    agent
        .execute_with_all_callbacks("classify this", None, None, None, None)
        .await
        .expect("the turn completes with the refusal recorded");

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "an unanswered battery is an operational refuse: the call must not run"
    );
    let joined = tool_results(&agent).join("\n");
    assert!(
        joined.contains("annotator") || joined.contains("transport") || joined.contains("refus"),
        "the model must read the runtime's refusal; got {joined}"
    );
}

#[tokio::test]
async fn invariant_3_control_a_healthy_battery_proceeds() {
    // Measured: the same call with the battery answering → AllowCall, the
    // tool executes, the result is admitted (Ack). Proves the refusal is
    // conditional — a gate that blocks everything is not a gate.
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex(r#""kind":"annotation""#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"version":1,"answer":{"delta":{},"requires":{"history":[],"attention":[]},"emits":[]}}"#,
        )
        .create_async()
        .await;

    let dir = tempfile::tempdir().expect("temp dir");
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ClassifiedReadTool {
        invocations: invocations.clone(),
    }));

    let guard = AppaGuard::from_documents(
        "invariant-3-healthy",
        ROOT_POLICY,
        vec![battery_with(&server.url(), CLOSED_PORT)],
        3_000,
        65_536,
    )
    .expect("the guard opens");
    let mut agent = agent_with_guard(
        guard,
        Box::new(OneToolBackend::new("classified_read", json!({}))),
        registry,
        dir.path(),
    );

    agent
        .execute_with_all_callbacks("classify this", None, None, None, None)
        .await
        .expect("the healthy turn completes");

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "the same call must proceed when the battery answers"
    );
    assert!(
        history_contains(&agent, "classified-body"),
        "the admitted result must reach the model"
    );
}

// ---------------------------------------------------------------------------
// The measured permission defects, locked as regressions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_missing_callback_is_a_hard_deny_for_a_mutating_tool() {
    // The measured defect (execute.rs:842-847 old): "No permission callback,
    // auto-approving mutating tool". Now: hard deny, zero executions.
    struct MutatingCanary {
        invocations: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl Tool for MutatingCanary {
        fn name(&self) -> &str {
            "canary_write"
        }
        fn description(&self) -> &str {
            "mutating canary (test instrumentation)"
        }
        fn mutating(&self) -> bool {
            true
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _args: Value) -> Result<Value> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"written": true}))
        }
    }

    let dir = tempfile::tempdir().expect("temp dir");
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MutatingCanary {
        invocations: invocations.clone(),
    }));

    // NO guard this time: this is the permission-gate defect on its own.
    let mut config = PawanConfig::default();
    config.appa.enabled = false;
    let mut agent = PawanAgent::new(config, dir.path().to_path_buf())
        .with_tools(registry)
        .with_backend(Box::new(OneToolBackend::new("canary_write", json!({}))));

    agent
        .execute_with_all_callbacks("write the canary", None, None, None, None)
        .await
        .expect("the turn completes with the denial recorded");

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "a mutating tool with no approval channel must be denied, never auto-approved"
    );
    assert!(
        history_contains(&agent, "no approval channel"),
        "the denial reason must reach the model"
    );
}

#[test]
fn mutating_tools_default_to_prompt() {
    let empty = std::collections::HashMap::new();
    for tool in [
        "bash",
        "write_file",
        "edit_file",
        "edit_file_lines",
        "insert_after",
        "append_file",
        "git_add",
        "git_commit",
        "git_checkout",
        "git_stash",
        "git_branch",
        "ast_grep",
        "sd",
        "deagle_map",
        "mise",
    ] {
        assert_eq!(
            ToolPermission::resolve(tool, &empty),
            ToolPermission::Prompt,
            "{tool} must default to Prompt"
        );
    }
    // Read-only tools stay Allow.
    for tool in [
        "read_file",
        "rg",
        "fd",
        "grep_search",
        "git_status",
        "git_log",
    ] {
        assert_eq!(
            ToolPermission::resolve(tool, &empty),
            ToolPermission::Allow,
            "{tool} must default to Allow"
        );
    }
    // Explicit config still wins over the default.
    let mut configured = std::collections::HashMap::new();
    configured.insert("write_file".to_string(), ToolPermission::Deny);
    assert_eq!(
        ToolPermission::resolve("write_file", &configured),
        ToolPermission::Deny
    );
}

#[test]
fn the_mutating_overrides_are_real_on_the_default_registry() {
    let registry = ToolRegistry::with_defaults(std::path::PathBuf::from("."));
    for tool in [
        "write_file",
        "edit_file_lines",
        "insert_after",
        "append_file",
        "git_add",
        "git_commit",
        "git_checkout",
        "git_stash",
        "git_branch",
        "ast_grep",
        "sd",
        "deagle_map",
        "mise",
        "bash",
    ] {
        let entry = registry
            .get(tool)
            .unwrap_or_else(|| panic!("{tool} is registered by default"));
        assert!(entry.mutating(), "{tool} must report mutating() == true");
    }
    for tool in [
        "read_file",
        "list_directory",
        "rg",
        "fd",
        "git_status",
        "git_log",
        "git_diff",
    ] {
        let entry = registry
            .get(tool)
            .unwrap_or_else(|| panic!("{tool} is registered by default"));
        assert!(
            !entry.mutating(),
            "{tool} is read-only and must report mutating() == false"
        );
    }
}

#[test]
fn a_guard_that_fails_to_open_denies_every_call() {
    // Fail closed at open: a policy path that does not exist is GuardState::
    // Failed, and the funnel denies rather than running unguarded.
    let state = super::appa::open_guard(
        &crate::config::AppaConfig {
            enabled: true,
            policy_path: Some(std::path::PathBuf::from("/nonexistent/appa.toml")),
            batteries: vec![],
            timeout_ms: 3_000,
            max_body_bytes: 65_536,
        },
        "open-failure",
    );
    assert!(
        matches!(state, GuardState::Failed { .. }),
        "an unopenable policy must fail the guard closed"
    );
    assert!(state.guard().is_none());
}

#[tokio::test]
async fn the_remedy_flow_authorizes_then_allows_the_recall() {
    // The full measured remedy loop against the guard directly, without the
    // funnel: DenyCall(offers) → PassControl → Authorized → AllowCall.
    let dir = tempfile::tempdir().expect("temp dir");
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex(r#""kind":"sanitizer""#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"version":1,"answer":{"body":"scrubbed"}}"#)
        .create_async()
        .await;

    let guard = AppaGuard::from_documents(
        "remedy-flow",
        ROOT_POLICY,
        vec![battery_with(CLOSED_PORT, &server.url())],
        3_000,
        65_536,
    )
    .expect("the guard opens");
    guard.ensure_started().await;

    let args = json!({"q": "x"});
    let first = guard.check_call("leaky_read", &args, "rf-1", None).await;
    let super::appa::CallDecision::Deny { feedback } = first else {
        panic!("the confined read must block first; got {first:?}");
    };

    // The engine's offers are quoted by id; take the sanitizer offer minted
    // for THIS denial (measured: taken first, in a fresh trajectory).
    let offer = pick_offer(&feedback).expect("the denial names its offers");
    let outcome = guard
        .take_remedy(&json!({"offer_id": offer}), "rf-vouch")
        .await
        .expect("the sanitizer offer authorizes");
    assert!(
        matches!(outcome, appa_runtime::api::RemedyOutcome::Authorized { .. }),
        "the sanitizer offer must authorize; got {outcome:?}"
    );

    let second = guard.check_call("leaky_read", &args, "rf-2", None).await;
    assert_eq!(
        second,
        super::appa::CallDecision::Allow,
        "after the remedy the confined read runs"
    );

    let result = guard
        .check_result(
            "leaky_read",
            &args,
            "rf-2",
            None,
            true,
            "\"TOPSECRET-CANARY\"",
        )
        .await;
    assert_eq!(
        result,
        super::appa::ResultDecision::Deliver {
            value: "scrubbed".to_string()
        },
        "the derivation crosses, the raw is withheld"
    );
    drop(dir);
}

#[tokio::test]
async fn an_undeclared_tool_is_refused_before_it_runs() {
    // Measured: a tool no contract names, with no wildcard covering it, is
    // Refuse — refused before it runs. (`appa replay` reports this shape as
    // "cannot run", so the proof lives here, not in a replay trace.)
    let guard = AppaGuard::from_documents("undeclared", ROOT_POLICY, vec![], 3_000, 65_536)
        .expect("the guard opens");
    guard.ensure_started().await;
    let decision = guard
        .check_call("never_declared", &json!({}), "u-1", None)
        .await;
    assert!(
        matches!(decision, super::appa::CallDecision::Refuse { .. }),
        "an undeclared tool must be refused before it runs; got {decision:?}"
    );
}
