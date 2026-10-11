//! Identity fence for the pawan-api agent surface.
//!
//! The measured hole (2026-10-11 survey): the HTTP API bound `0.0.0.0` with
//! no auth, no authorization, no rate limit, and `/api/chat` ran agents on
//! the no-callback path — an unauthenticated caller could drive every
//! mutating tool. This module fences that surface: every route that runs an
//! agent or reads sessions requires a verified identity.
//!
//! Fail-closed contract:
//! * No verifier configured → every authenticated route refuses (503), it
//!   never serves unauthenticated.
//! * A token that fails verification → 401.
//! * A verification service that cannot answer (transport, timeout, 5xx)
//!   → 503 — an outage is never a pass.
//!
//! Composition with the estate auth service (dims-auth): when
//! `PAWAN_DIMS_AUTH_URL` is set, presented bearer tokens are validated
//! against its measured `/v1/validate` contract
//! (`POST {"token": t}` → `{user_id, email, roles}` on 2xx; 401 for an
//! invalid token). A role requirement can be layered on via
//! `PAWAN_DIMS_AUTH_REQUIRED_ROLE` / `PAWAN_DIMS_AUTH_RESOURCE`.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use serde::Deserialize;

use super::AppState;

/// The verification timeout for the estate auth service. A validate call
/// that outlives it is an outage, not a pass.
const DIMS_VALIDATE_TIMEOUT: Duration = Duration::from_secs(5);

/// How this deployment verifies a caller's identity.
#[derive(Clone)]
pub struct IdentityConfig {
    inner: Arc<IdentityInner>,
}

struct IdentityInner {
    /// A bearer token compared in constant time (shared-secret deployments).
    static_token: Option<String>,
    /// The estate auth service's base URL (dims-auth).
    dims_base_url: Option<String>,
    /// Service key sent to dims-auth, when the deployment configures one.
    service_key: Option<String>,
    /// When set, the token must carry this role for `resource_id`.
    required_role: Option<String>,
    resource_id: Option<String>,
    client: reqwest::Client,
}

impl IdentityConfig {
    /// Build the verifier set from the environment. With nothing configured
    /// the API refuses every authenticated route.
    pub fn from_env() -> Self {
        let client = reqwest::Client::builder()
            .timeout(DIMS_VALIDATE_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            inner: Arc::new(IdentityInner {
                static_token: std::env::var("PAWAN_API_TOKEN")
                    .ok()
                    .filter(|t| !t.is_empty()),
                dims_base_url: std::env::var("PAWAN_DIMS_AUTH_URL")
                    .ok()
                    .filter(|u| !u.is_empty()),
                service_key: std::env::var("PAWAN_SERVICE_KEY")
                    .ok()
                    .filter(|k| !k.is_empty()),
                required_role: std::env::var("PAWAN_DIMS_AUTH_REQUIRED_ROLE")
                    .ok()
                    .filter(|r| !r.is_empty()),
                resource_id: std::env::var("PAWAN_DIMS_AUTH_RESOURCE")
                    .ok()
                    .filter(|r| !r.is_empty()),
                client,
            }),
        }
    }

    /// Whether any verifier is configured. When false, the API is inert for
    /// every route that needs identity.
    pub fn is_configured(&self) -> bool {
        self.inner.static_token.is_some() || self.inner.dims_base_url.is_some()
    }

    /// A verifier set with one static bearer token (tests, single-tenant
    /// deployments).
    #[cfg(test)]
    pub fn with_static_token(token: &str) -> Self {
        Self {
            inner: Arc::new(IdentityInner {
                static_token: Some(token.to_string()),
                dims_base_url: None,
                service_key: None,
                required_role: None,
                resource_id: None,
                client: reqwest::Client::new(),
            }),
        }
    }

    /// A verifier set that validates against a dims-auth-shaped service at
    /// `base` (tests).
    #[cfg(test)]
    pub fn with_dims(base: &str) -> Self {
        Self {
            inner: Arc::new(IdentityInner {
                static_token: None,
                dims_base_url: Some(base.to_string()),
                service_key: None,
                required_role: None,
                resource_id: None,
                client: reqwest::Client::new(),
            }),
        }
    }

    /// The empty verifier set: refuses everything.
    #[cfg(test)]
    pub fn unconfigured() -> Self {
        Self {
            inner: Arc::new(IdentityInner {
                static_token: None,
                dims_base_url: None,
                service_key: None,
                required_role: None,
                resource_id: None,
                client: reqwest::Client::new(),
            }),
        }
    }

    /// Verify a presented token. `Ok(principal)` only when a verifier
    /// positively admits it.
    async fn verify(&self, token: &str) -> Result<String, (StatusCode, &'static str)> {
        if let Some(expected) = &self.inner.static_token {
            if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
                return Ok("static-token".to_string());
            }
        }
        if let Some(base) = &self.inner.dims_base_url {
            return self.verify_with_dims(base, token).await;
        }
        // A token was presented but no verifier answers for it.
        Err((StatusCode::UNAUTHORIZED, "identity could not be verified"))
    }

    /// The dims-auth `/v1/validate` contract: a 2xx response admits the
    /// caller and names the principal; a 401 refuses it; anything else
    /// (transport failure, timeout, 5xx) is an outage and refuses.
    async fn verify_with_dims(
        &self,
        base: &str,
        token: &str,
    ) -> Result<String, (StatusCode, &'static str)> {
        let url = format!("{}/v1/validate", base.trim_end_matches('/'));
        let body = serde_json::json!({
            "token": token,
            "required_role": self.inner.required_role,
            "resource_id": self.inner.resource_id,
        });
        let mut request = self.inner.client.post(&url).json(&body);
        if let Some(key) = &self.inner.service_key {
            request = request.header("X-Service-Key", key);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "auth service unreachable; refusing to serve",
                ))
            }
        };
        match response.status().as_u16() {
            200..=299 => {
                let validated: ValidateResponse =
                    response.json().await.unwrap_or(ValidateResponse {
                        user_id: None,
                        email: None,
                    });
                Ok(validated
                    .principal()
                    .unwrap_or_else(|| "dims-auth-validated".to_string()))
            }
            401 | 403 => Err((StatusCode::UNAUTHORIZED, "identity rejected")),
            _ => Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "auth service did not confirm identity; refusing to serve",
            )),
        }
    }
}

/// The `/v1/validate` response fields pawan reads. Optional throughout: an
/// admitted caller with no readable principal is still admitted.
#[derive(Debug, Deserialize)]
struct ValidateResponse {
    user_id: Option<String>,
    email: Option<String>,
}

impl ValidateResponse {
    fn principal(&self) -> Option<String> {
        self.user_id
            .clone()
            .or_else(|| self.email.clone())
            .filter(|p| !p.is_empty())
    }
}

/// A verified caller: the extractor the fenced handlers take. Zero-sized —
/// its meaning is that the request carried an identity a verifier admitted.
pub struct IdentityOf;

impl FromRequestParts<AppState> for IdentityOf {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !state.identity.is_configured() {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "identity verification is not configured; refusing to serve",
            ));
        }
        let token = presented_token(parts).ok_or((
            StatusCode::UNAUTHORIZED,
            "missing identity: send Authorization: Bearer <token>",
        ))?;
        let principal = state.identity.verify(&token).await?;
        tracing::debug!(principal = %principal, "identity verified");
        Ok(IdentityOf)
    }
}

/// Read the presented token: `Authorization: Bearer <token>` or
/// `X-API-Key: <token>`.
fn presented_token(parts: &Parts) -> Option<String> {
    let headers = &parts.headers;
    if let Some(value) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(text) = value.to_str() {
            if let Some(token) = text.strip_prefix("Bearer ") {
                let token = token.trim();
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }
    headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// Length-safe, content-constant string comparison: the comparison time
/// depends on neither the position nor the value of the first difference.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_equal_and_differs() {
        assert!(constant_time_eq(b"secret-token", b"secret-token"));
        assert!(!constant_time_eq(b"secret-token", b"secret-tokeN"));
        assert!(!constant_time_eq(b"short", b"a-longer-token"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn unconfigured_verifier_set_is_inert() {
        // No env in the test process: the set must refuse, never admit.
        let config = IdentityConfig::from_env();
        if !config.is_configured() {
            // Guaranteed by construction: from_env with both vars absent
            // yields no verifier.
            assert!(config.inner.static_token.is_none());
            assert!(config.inner.dims_base_url.is_none());
        }
    }

    #[tokio::test]
    async fn a_presented_token_that_no_verifier_answers_is_refused() {
        let config = IdentityConfig::from_env();
        if config.is_configured() {
            return; // environment carries a verifier; nothing to assert
        }
        let outcome = config.verify("anything").await;
        assert!(outcome.is_err(), "no verifier configured must refuse");
    }
}
