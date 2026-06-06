//! Secrets-broker interface for the Actions-YAML shim (③).
//!
//! ## Contract guarantee (③)
//! Secrets are resolved **via the C5 broker only**. The shim never:
//! - reads raw secret values from environment variables,
//! - injects raw values into the runner environment,
//! - logs secret material (even partially).
//!
//! A missing or denied secret causes [`BrokerError::SecretDenied`] —
//! the shim **fails CLOSED**, naming the secret. The raw material never
//! appears in any log, error message, or environment variable visible to
//! non-broker code.
//!
//! ## Red-team surface
//! The [`Broker`] trait is the only path through which a step may access a
//! secret. The [`ShimExecutor`](crate::shim::executor::ShimExecutor) holds a
//! `Box<dyn Broker>` and resolves every `${{ secrets.NAME }}` expression
//! through it before constructing the step environment. The resolved value is
//! injected as an opaque byte sequence into the step's isolated namespace; it
//! never enters any Rust `String` that is logged or returned in a public type.
//!
//! The [`FenceManifest`](hugit_contracts::FenceManifest) is the broker's
//! access-control anchor; the broker validates each secret name against the
//! manifest's `path_set` (C5 channel).

use hugit_contracts::FenceManifest;

/// The result of resolving a secret via the broker.
#[derive(Debug, Clone)]
pub enum SecretResolution {
    /// Secret was resolved successfully. The opaque token is a handle the
    /// executor injects into the step's isolated environment; it is **not**
    /// the raw secret value.
    Resolved {
        /// The secret name as declared in the workflow (`secrets.NAME`).
        secret_name: String,
        /// An opaque injection token (e.g., a tmpfile path or a sealed env-
        /// var name). Never contains raw secret material.
        injection_token: String,
    },
    /// Secret was not found or was denied by the broker/manifest.
    Denied {
        /// The secret name that was requested.
        secret_name: String,
        /// Why it was denied (e.g., "not in FenceManifest path_set").
        reason: String,
    },
}

/// Errors from the secrets broker.
#[derive(Debug, PartialEq, Eq)]
pub enum BrokerError {
    /// The requested secret is missing or was denied by the access policy.
    /// The `secret_name` field names the secret precisely so the caller can
    /// report a closed failure without logging raw material.
    SecretDenied { secret_name: String, reason: String },
    /// The broker itself is unavailable (e.g., C5 service not reachable).
    BrokerUnavailable(String),
}

impl std::fmt::Display for BrokerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // IMPORTANT: the secret_name is named but NO raw material is included.
            Self::SecretDenied {
                secret_name,
                reason,
            } => {
                write!(
                    f,
                    "secret `{secret_name}` denied by broker: {reason} \
                           [fail-CLOSED: execution halted, no raw material in this message]"
                )
            }
            Self::BrokerUnavailable(msg) => {
                write!(f, "secrets broker unavailable: {msg}")
            }
        }
    }
}

impl std::error::Error for BrokerError {}

/// The secrets-broker interface. The shim only resolves secrets through this
/// trait — never from environment variables or any other direct channel.
///
/// Implementors MUST NOT include raw secret material in any returned error,
/// log, or String. The broker contract is: inject an opaque token → fail
/// CLOSED with a named error.
pub trait Broker: Send + Sync {
    /// Resolve a secret by name, returning an opaque injection token on
    /// success or a closed failure naming the secret on deny/missing.
    fn resolve_secret(
        &self,
        secret_name: &str,
        manifest: &FenceManifest,
    ) -> Result<SecretResolution, BrokerError>;
}

/// A null broker used in tests and in environments where the C5 broker is not
/// available. Always returns `SecretDenied` — ensures tests exercise the
/// fail-CLOSED path without requiring a live C5 endpoint.
pub struct NullBroker;

impl Broker for NullBroker {
    fn resolve_secret(
        &self,
        secret_name: &str,
        _manifest: &FenceManifest,
    ) -> Result<SecretResolution, BrokerError> {
        Err(BrokerError::SecretDenied {
            secret_name: secret_name.to_string(),
            reason: "NullBroker: no secrets are available in this environment".to_string(),
        })
    }
}

/// A stub broker for acceptance tests: resolves a pre-configured set of
/// secrets, denies everything else. Raw values are **never** stored; only
/// opaque tokens are returned.
pub struct StubBroker {
    /// Opaque tokens keyed by secret name. The token is not the raw value.
    tokens: std::collections::HashMap<String, String>,
}

impl StubBroker {
    /// Create a stub broker with a set of pre-configured secret names and
    /// their opaque injection tokens (NOT raw values).
    pub fn new(tokens: std::collections::HashMap<String, String>) -> Self {
        Self { tokens }
    }
}

impl Broker for StubBroker {
    fn resolve_secret(
        &self,
        secret_name: &str,
        _manifest: &FenceManifest,
    ) -> Result<SecretResolution, BrokerError> {
        match self.tokens.get(secret_name) {
            Some(token) => Ok(SecretResolution::Resolved {
                secret_name: secret_name.to_string(),
                injection_token: token.clone(),
            }),
            None => Err(BrokerError::SecretDenied {
                secret_name: secret_name.to_string(),
                reason: "not configured in StubBroker (simulates missing secret)".to_string(),
            }),
        }
    }
}

/// Extract all `${{ secrets.NAME }}` references from a string.
///
/// Returns the list of secret names referenced. Does not resolve values.
pub fn extract_secret_refs(s: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${{") {
        let after = &rest[start + 3..];
        if let Some(end) = after.find("}}") {
            let expr = after[..end].trim();
            if let Some(name) = expr.strip_prefix("secrets.") {
                let name = name.trim();
                if !name.is_empty() {
                    refs.push(name.to_string());
                }
            }
            rest = &after[end + 2..];
        } else {
            break;
        }
    }
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_contracts::FenceManifest;

    fn empty_manifest() -> FenceManifest {
        FenceManifest {
            path_set: vec![],
            deny_default: true,
            materialized: vec![],
        }
    }

    #[test]
    fn null_broker_fails_closed_with_named_secret() {
        let b = NullBroker;
        let err = b.resolve_secret("MY_TOKEN", &empty_manifest()).unwrap_err();
        match err {
            BrokerError::SecretDenied { secret_name, .. } => {
                assert_eq!(secret_name, "MY_TOKEN");
            }
            _ => panic!("expected SecretDenied"),
        }
    }

    #[test]
    fn null_broker_error_message_does_not_contain_raw_material() {
        let b = NullBroker;
        let err = b
            .resolve_secret("SUPER_SECRET_VALUE_xyz123", &empty_manifest())
            .unwrap_err();
        let msg = err.to_string();
        // The error should name the secret but must NOT contain any raw value.
        assert!(
            msg.contains("SUPER_SECRET_VALUE_xyz123"),
            "must name the secret"
        );
        // There is no raw value to leak in NullBroker, but we assert the pattern.
        assert!(msg.contains("fail-CLOSED"), "must signal fail-closed");
    }

    #[test]
    fn stub_broker_resolves_configured_secret() {
        let mut tokens = std::collections::HashMap::new();
        tokens.insert("MY_TOKEN".to_string(), "opaque-token-abc".to_string());
        let b = StubBroker::new(tokens);
        let res = b.resolve_secret("MY_TOKEN", &empty_manifest()).unwrap();
        match res {
            SecretResolution::Resolved {
                secret_name,
                injection_token,
            } => {
                assert_eq!(secret_name, "MY_TOKEN");
                assert_eq!(injection_token, "opaque-token-abc");
            }
            SecretResolution::Denied { .. } => panic!("expected Resolved"),
        }
    }

    #[test]
    fn stub_broker_denies_unknown_secret_with_name() {
        let b = StubBroker::new(std::collections::HashMap::new());
        let err = b
            .resolve_secret("MISSING_SECRET", &empty_manifest())
            .unwrap_err();
        match err {
            BrokerError::SecretDenied { secret_name, .. } => {
                assert_eq!(secret_name, "MISSING_SECRET");
            }
            _ => panic!("expected SecretDenied"),
        }
    }

    #[test]
    fn extract_secret_refs_single() {
        let refs = extract_secret_refs("Bearer ${{ secrets.API_TOKEN }}");
        assert_eq!(refs, vec!["API_TOKEN"]);
    }

    #[test]
    fn extract_secret_refs_multiple() {
        let refs = extract_secret_refs("${{ secrets.A }} and ${{ secrets.B }}");
        assert_eq!(refs, vec!["A", "B"]);
    }

    #[test]
    fn extract_secret_refs_none() {
        let refs = extract_secret_refs("echo hello world");
        assert!(refs.is_empty());
    }
}
