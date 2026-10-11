//! OpenAPPA embedded guardrail — the pre-call and post-call checks that sit at
//! pawan's tool-execution seam.
//!
//! The engine answers one question before every tool call — *is this data
//! allowed to go to this destination?* — and one after — *may this result
//! re-enter the conversation?* Both hooks are wired in
//! [`crate::agent::execute`]: [`AppaGuard::check_call`] runs inside
//! `check_tool_permissions` (before the tool's execute path is ever entered)
//! and [`AppaGuard::check_result`] runs after `tools.execute` returns, before
//! the result is built into the `ToolCallRecord` that reaches the model.
//!
//! Fail-closed contract (measured against the engine, rev 1c52575): an
//! unanswered battery is an operational [`CallDecision::Refuse`] — the call
//! does not run — never an allow. A guard that fails to open denies every
//! call; see [`AppaGuard::failed`].

use std::path::Path;
use std::sync::Arc;

use appa_eventlog::{Backend, LogStore};
use appa_runtime::api::{ExecuteRemedyPlanArgs, OpenError, RemedyOutcome, Runtime};
use appa_runtime::config::{Config, HostDefaults, HostedBattery};
use appa_runtime::hooks;
use appa_runtime_api::{
    Actor, Adapter, AdapterName, CanonicalTool, HookDecision, HookEvent, IdentifiedTool,
    OutcomeBody, ParseRefusal, ProposedCall, ToolOutcome, TrajectoryId, CONTROL_TOOL,
};

/// The advertised spelling of the runtime's control tool. The model quotes an
/// `offer_id` on this name to take a remedy; pawan routes it to the runtime
/// instead of its own registry. `appa/execute_remedy_plan` (the canonical id)
/// is accepted as an alias. Both are reserved: a pawan tool may never register
/// under either name.
pub const CONTROL_SPELLING: &str = "execute_remedy_plan";

/// The canonical namespace every pawan tool is identified under.
const PAWAN_NAMESPACE: &str = "pawan";

/// Default consult timeout for hosted batteries (milliseconds).
pub const DEFAULT_APPA_TIMEOUT_MS: u64 = 3_000;

/// Default response body cap for hosted batteries (bytes).
pub const DEFAULT_APPA_MAX_BODY_BYTES: usize = 65_536;

/// Whether a raw tool name the model dispatched belongs to the runtime's own
/// control tool, under either the advertised spelling or the canonical id.
pub fn is_control_tool_name(name: &str) -> bool {
    name == CONTROL_SPELLING || name == CONTROL_TOOL
}

fn identify_tool(raw: &str) -> Result<IdentifiedTool, ParseRefusal> {
    let canonical = if is_control_tool_name(raw) {
        CanonicalTool::control()
    } else {
        CanonicalTool::of("host", PAWAN_NAMESPACE, raw).map_err(|error| {
            ParseRefusal::Malformed {
                detail: error.to_string(),
            }
        })?
    };
    Ok(IdentifiedTool {
        canonical,
        spawn: None,
    })
}

fn spell(canonical: &CanonicalTool) -> Option<String> {
    if canonical.is_control() {
        return Some(CONTROL_SPELLING.to_string());
    }
    let mut segments = canonical.as_str().splitn(3, '/');
    match (segments.next(), segments.next(), segments.next()) {
        (Some("host"), Some(PAWAN_NAMESPACE), Some(name)) if name != CONTROL_SPELLING => {
            Some(name.to_string())
        }
        _ => None,
    }
}

fn names_children(_: &Actor, _: &ProposedCall) -> Vec<TrajectoryId> {
    // pawan's `spawn_agent` starts another *process* with its own loop; the
    // engine's children are in-process trajectories. Nothing in pawan's
    // arguments names a child trajectory, so no call does.
    Vec::new()
}

/// The host-side adapter: pawan spells each tool by its bare registered name
/// and identifies it as `host/pawan/<name>`; MCP bridges keep their
/// `mcp_<server>_<tool>` names (host tools from pawan's perspective — the
///
/// policy names them by that spelling or canonically).
fn adapter() -> Adapter {
    Adapter {
        name: AdapterName::Embedded,
        identify_tool,
        names_children,
        spell,
        // A pawan spawn is another agent called as a tool: only a contract
        // written for it may release one, so the wildcard does not cover it.
        wildcard_covers_spawn: false,
        // No pawan tool name spells a server in its own grammar (MCP servers
        // are namespaced with `mcp_`, not dotted).
        spells_server: |_| false,
    }
}

/// Open the guard a configured agent runs under.
///
/// `Disabled` when the config leaves the guard off or names no policy.
/// `Failed` when it was configured but the policy could not be read or
/// composed, or the runtime refused the open — a failed guard denies every
/// tool call, so a typo in a policy path can never silently unguard a loop.
pub fn open_guard(config: &crate::config::AppaConfig, session_id: &str) -> GuardState {
    if !config.is_openable() {
        return GuardState::Disabled;
    }
    let policy_path = config.policy_path.clone().expect("is_openable checked");
    let batteries = config
        .batteries
        .iter()
        .map(|b| (b.name.clone(), b.policy_path.clone(), b.token_env.clone()))
        .collect::<Vec<_>>();
    match AppaGuard::open(
        session_id,
        &policy_path,
        &batteries,
        config.timeout_ms,
        config.max_body_bytes,
    ) {
        Ok(guard) => GuardState::Active(guard),
        Err(error) => {
            tracing::error!(
                error = %error,
                "appa guard failed to open; every tool call will stop (fail closed)"
            );
            GuardState::Failed {
                reason: error.to_string(),
            }
        }
    }
}

/// The runtime's answer to a proposed tool call, as pawan acts on it.
#[derive(Debug, Clone, PartialEq)]
pub enum CallDecision {
    /// The call may execute.
    Allow,
    /// The call is denied by policy: the tool's execute path is never
    /// entered. `feedback` is the engine's words for the model.
    Deny { feedback: String },
    /// The call is blocked outright: the tool's execute path is never
    /// entered. `reason` is the engine's words for the model.
    Stop { reason: String },
    /// An operational fault (an annotator, sanitizer, audience source or the
    /// engine itself did not answer). The call does not run — never an allow.
    Refuse { detail: String },
}

/// The runtime's answer to a completed tool call's result, as pawan acts on it.
#[derive(Debug, Clone, PartialEq)]
pub enum ResultDecision {
    /// The result crosses to the model as the tool produced it.
    Admit,
    /// The engine admitted a derivation in place of the raw body: the model
    /// receives `value`, never the raw bytes.
    Deliver { value: String },
    /// The raw body is withheld; the model receives the runtime's own
    /// `output` (the block reason or the narrowing notice) instead.
    Replace { output: String },
    /// Nothing crosses: an operational fault or an outright block. The
    /// result never reaches the model.
    Refuse { detail: String },
}

/// A hosted battery: a policy document that composes under the root, served
/// by an annotator/sanitizer/directory the host aliases.
#[derive(Debug, Clone)]
pub struct Battery {
    pub name: String,
    pub policy: String,
    pub token_env: Vec<String>,
}

/// Why the guard could not open. Opening fails closed: an agent whose guard
/// fails to open denies every call.
#[derive(Debug, thiserror::Error)]
pub enum GuardOpenError {
    #[error("appa policy read failed: {0}")]
    PolicyRead(String),
    #[error("appa policy composition failed: {0}")]
    Compose(String),
    #[error("appa runtime open failed: {0}")]
    Open(String),
}

/// Whether the guard runs for this agent, and why not when it does not.
///
/// `Failed` is the fail-closed state: the guard was configured but did not
/// open, so every tool call stops. `Disabled` is the guardrail absent — the
/// permission gates alone stand.
#[derive(Debug, Clone)]
pub enum GuardState {
    Disabled,
    Active(Arc<AppaGuard>),
    Failed { reason: String },
}

impl GuardState {
    /// The guard, when one is active.
    pub fn guard(&self) -> Option<&AppaGuard> {
        match self {
            GuardState::Active(guard) => Some(guard),
            _ => None,
        }
    }
}

/// The embedded OpenAPPA guard for one pawan agent session.
pub struct AppaGuard {
    runtime: Runtime,
    actor: Actor,
    /// Set once the session-start event has been dispatched. A session start
    /// that does not answer `Ack`/`Context` fails the whole guard closed.
    started: std::sync::atomic::AtomicBool,
    /// Set when the session-start event was refused: every later call stops.
    start_failed: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for AppaGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppaGuard")
            .field("actor", &self.actor)
            .finish()
    }
}

impl AppaGuard {
    /// Open a guard from policy documents already in memory (tests, embedded
    /// hosts). `root` is the root policy TOML; each [`Battery`] composes
    /// under it under its own timeout and body cap.
    pub fn from_documents(
        session_id: &str,
        root: &str,
        batteries: Vec<Battery>,
        timeout_ms: u64,
        max_body_bytes: usize,
    ) -> Result<Arc<Self>, GuardOpenError> {
        let token_envs = batteries
            .iter()
            .map(|battery| {
                battery
                    .token_env
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let hosted = batteries
            .iter()
            .zip(token_envs.iter())
            .map(|(battery, token_env)| HostedBattery {
                name: &battery.name,
                policy: &battery.policy,
                token_env,
            })
            .collect::<Vec<_>>();
        let config = Config::hosted_composed(
            root,
            &hosted,
            HostDefaults::new(std::time::Duration::from_millis(timeout_ms), max_body_bytes),
            |_| None,
        )
        .map_err(|e| GuardOpenError::Compose(e.to_string()))?;
        Self::open_with(session_id, config)
    }

    /// Open a guard from files: the root policy at `policy_path` and one file
    /// per battery. A missing or unreadable file fails closed.
    pub fn open(
        session_id: &str,
        policy_path: &Path,
        batteries: &[(String, std::path::PathBuf, Vec<String>)],
        timeout_ms: u64,
        max_body_bytes: usize,
    ) -> Result<Arc<Self>, GuardOpenError> {
        let root = std::fs::read_to_string(policy_path)
            .map_err(|e| GuardOpenError::PolicyRead(format!("{}: {e}", policy_path.display())))?;
        let mut composed = Vec::with_capacity(batteries.len());
        for (name, path, token_env) in batteries {
            let policy = std::fs::read_to_string(path)
                .map_err(|e| GuardOpenError::PolicyRead(format!("{}: {e}", path.display())))?;
            composed.push(Battery {
                name: name.clone(),
                policy,
                token_env: token_env.clone(),
            });
        }
        Self::from_documents(session_id, &root, composed, timeout_ms, max_body_bytes)
    }

    fn open_with(session_id: &str, config: Config) -> Result<Arc<Self>, GuardOpenError> {
        let store = Arc::new(
            LogStore::open(Backend::Memory).map_err(|e| GuardOpenError::Open(e.to_string()))?,
        );
        // `open_with_store_as` refuses an adapter that spells no control tool:
        // the open itself fails closed (OpenError::UnspelledControlTool).
        let runtime = Runtime::open_with_store_as(config, store, None, adapter())
            .map_err(|e: OpenError| GuardOpenError::Open(e.to_string()))?;
        Ok(Arc::new(Self {
            runtime,
            actor: Actor {
                root: TrajectoryId(format!("pawan:{session_id}")),
                child: None,
            },
            started: std::sync::atomic::AtomicBool::new(false),
            start_failed: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// Dispatch the session-start event once. The decision is not policy
    /// feedback: a `Refuse` here marks the guard failed and every later call
    /// stops. `Context` (peer messages pawan has no channel for) is accepted
    /// and its text dropped.
    pub async fn ensure_started(&self) {
        if self.started.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let decision = hooks::handle(
            &self.runtime,
            HookEvent::SessionStart {
                root: self.actor.root.clone(),
                principal: None,
                address: None,
                title: None,
                launch: None,
                start: None,
            },
        )
        .await;
        if !matches!(decision, HookDecision::Ack | HookDecision::Context { .. }) {
            tracing::error!(
                decision = ?decision,
                "appa session start refused; the guard fails closed for this session"
            );
            self.start_failed
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// The pre-call check. `arguments` is the value pawan is about to hand to
    /// the tool; it crosses as its serialized bytes so the engine — not a
    /// parser — decides what the call is (duplicate keys and other refusable
    /// forms would be collapsed by a parse).
    pub async fn check_call(
        &self,
        tool: &str,
        arguments: &serde_json::Value,
        call_id: &str,
        cwd: Option<&str>,
    ) -> CallDecision {
        if self.start_failed.load(std::sync::atomic::Ordering::SeqCst) {
            return CallDecision::Refuse {
                detail: "appa session start was refused; the guard fails closed".into(),
            };
        }
        let call = match identified_call(tool, arguments, cwd) {
            Ok(call) => call,
            Err(refusal) => {
                return CallDecision::Refuse {
                    detail: format!("appa refused the tool spelling: {refusal:?}"),
                }
            }
        };
        let decision = hooks::handle(
            &self.runtime,
            HookEvent::ToolCall {
                actor: self.actor.clone(),
                call,
                call_id: Some(call_id.to_string()),
                spawn: None,
                prompt: None,
                ruling: None,
            },
        )
        .await;
        match decision {
            HookDecision::AllowCall { .. } => CallDecision::Allow,
            HookDecision::DenyCall { feedback, .. } => CallDecision::Deny { feedback },
            HookDecision::Block { reason } => CallDecision::Stop { reason },
            HookDecision::Refuse { detail } => CallDecision::Refuse { detail },
            // A ToolCall never answers Ack/PassControl/ReplaceOutput/
            // DeliverValue/ChildReturn: anything unexpected fails closed.
            other => CallDecision::Refuse {
                detail: format!("appa returned an impossible decision for a tool call: {other:?}"),
            },
        }
    }

    /// The post-call check, before the result is built into the record the
    /// model sees. `arguments` must be the same value the call carried — the
    /// engine matches the outcome to the open dispatch by (tool, arguments).
    /// `cwd` is pawan's workspace root, where the harness ran the call.
    pub async fn check_result(
        &self,
        tool: &str,
        arguments: &serde_json::Value,
        call_id: &str,
        cwd: Option<&str>,
        succeeded: bool,
        body: &str,
    ) -> ResultDecision {
        if self.start_failed.load(std::sync::atomic::Ordering::SeqCst) {
            return ResultDecision::Refuse {
                detail: "appa session start was refused; the guard fails closed".into(),
            };
        }
        let outcome = if succeeded {
            ToolOutcome::Success {
                body: OutcomeBody::Available(body.to_string()),
            }
        } else {
            ToolOutcome::Failure {
                message: body.to_string(),
            }
        };
        let call = match identified_call(tool, arguments, cwd) {
            Ok(call) => call,
            Err(refusal) => {
                return ResultDecision::Refuse {
                    detail: format!("appa refused the tool spelling: {refusal:?}"),
                }
            }
        };
        let decision = hooks::handle(
            &self.runtime,
            HookEvent::ToolResult {
                actor: self.actor.clone(),
                call,
                call_id: Some(call_id.to_string()),
                outcome,
            },
        )
        .await;
        match decision {
            // Keep: the result crosses exactly as the tool produced it.
            HookDecision::Ack => ResultDecision::Admit,
            HookDecision::DeliverValue { value } => ResultDecision::Deliver { value },
            HookDecision::ReplaceOutput { output } => ResultDecision::Replace { output },
            HookDecision::Block { reason } => ResultDecision::Refuse { detail: reason },
            HookDecision::Refuse { detail } => ResultDecision::Refuse { detail },
            other => ResultDecision::Refuse {
                detail: format!(
                    "appa returned an impossible decision for a tool result: {other:?}"
                ),
            },
        }
    }

    /// Take a remedy the model quoted on the control tool: the runtime
    /// vouches the quoted offer first (the control call answers
    /// `PassControl`), and only then does the plan execute — inside its
    /// mandate envelope, where `rulings_cover` refuses a plan claiming a
    /// gap outside its authority. Executing without the vouch reads as
    /// `Refused { Unvouched }` (measured).
    pub async fn take_remedy(
        &self,
        plan: &serde_json::Value,
        vouch_call_id: &str,
    ) -> Result<RemedyOutcome, String> {
        let args: ExecuteRemedyPlanArgs = serde_json::from_value(plan.clone())
            .map_err(|e| format!("the quoted plan is malformed: {e}"))?;
        // The vouch carries the bytes the model quoted, unrewritten: the
        // engine validates the plan, not this host's interpretation of it.
        let bytes = serde_json::to_string(plan).unwrap_or_else(|_| "{}".to_string());
        let decision = hooks::handle(
            &self.runtime,
            HookEvent::ToolCall {
                actor: self.actor.clone(),
                call: ProposedCall {
                    tool: CONTROL_TOOL.to_string(),
                    arguments: serde_json::value::RawValue::from_string(bytes)
                        .map_err(|e| format!("the quoted plan is not one JSON value: {e}"))?,
                    cwd: None,
                },
                call_id: Some(vouch_call_id.to_string()),
                spawn: None,
                prompt: None,
                ruling: None,
            },
        )
        .await;
        if !matches!(decision, HookDecision::PassControl) {
            return Err(format!(
                "the runtime refused the control call: {decision:?}"
            ));
        }
        Ok(self
            .runtime
            .execute_embedded_remedy(&self.actor, args)
            .await)
    }
}

/// The call as the engine reads it: identified through the adapter first,
/// the way the wire identifies a served host's calls. A spelling outside
/// the adapter's domain is a parse refusal — the call fails closed.
fn identified_call(
    tool: &str,
    arguments: &serde_json::Value,
    cwd: Option<&str>,
) -> Result<ProposedCall, ParseRefusal> {
    let identified = identify_tool(tool)?;
    let canonical = identified.canonical.into_string();
    let mut call = proposed(&canonical, arguments, cwd);
    call.tool = canonical;
    Ok(call)
}

fn proposed(tool: &str, arguments: &serde_json::Value, cwd: Option<&str>) -> ProposedCall {
    // The engine canonicalizes; the runtime and adapter do not parse. pawan's
    // arguments arrive already parsed at the LLM client, so the bytes pawan
    // would execute are the serialization of that Value — the engine sees
    // exactly what the tool would receive.
    let bytes = serde_json::to_string(arguments).unwrap_or_else(|_| "null".to_string());
    ProposedCall {
        tool: tool.to_string(),
        arguments: serde_json::value::RawValue::from_string(bytes).unwrap_or_else(|_| {
            serde_json::value::RawValue::from_string("null".to_string()).expect("null parses")
        }),
        cwd: cwd.map(|c| c.to_string()),
    }
}
