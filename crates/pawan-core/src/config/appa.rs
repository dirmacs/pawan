//! OpenAPPA guardrail configuration — the `[appa]` section of pawan.toml.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// OpenAPPA guardrail configuration — the `[appa]` section of pawan.toml.
///
/// The guard is OFF by default: with `enabled = false` (or the section
/// absent) pawan runs its permission gates alone. When enabled, every tool
/// call crosses the OpenAPPA pre-call check and every result the post-call
/// check; the policy document is the authority for what crosses. A guard
/// that cannot open fails closed — every tool call stops.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppaConfig {
    /// Embed the guardrail engine. Default: false.
    pub enabled: bool,
    /// Path to the root policy document (TOML). Required when enabled.
    pub policy_path: Option<PathBuf>,
    /// Hosted batteries composing under the root policy: each names its
    /// annotator/sanitizer/directory service. A battery that does not answer
    /// is an operational refuse — the flow stops, never an allow.
    pub batteries: Vec<AppaBattery>,
    /// Consult timeout applied to every hosted battery (milliseconds).
    pub timeout_ms: u64,
    /// Response body cap applied to every hosted battery (bytes).
    pub max_body_bytes: usize,
}

/// One hosted battery: a policy document and the env vars its credentials
/// are read from (the runtime never receives the values).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppaBattery {
    /// The battery's name, as its policy document declares it.
    pub name: String,
    /// Path to the battery's policy document (TOML).
    pub policy_path: PathBuf,
    /// Environment variables the runtime reads this battery's credentials
    /// from at consult time.
    #[serde(default)]
    pub token_env: Vec<String>,
}

impl Default for AppaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            policy_path: None,
            batteries: Vec::new(),
            timeout_ms: crate::agent::appa::DEFAULT_APPA_TIMEOUT_MS,
            max_body_bytes: crate::agent::appa::DEFAULT_APPA_MAX_BODY_BYTES,
        }
    }
}

impl AppaConfig {
    /// The guard can only run with a policy document.
    pub fn is_openable(&self) -> bool {
        self.enabled && self.policy_path.is_some()
    }
}
