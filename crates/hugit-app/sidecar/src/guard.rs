//! Non-authoritative guard (B6④).
//!
//! The sidecar NEVER gates or blocks landing. This module provides a
//! compile-time + runtime assertion that the `authoritative` field is
//! hard-`false` and that no code path can set it to true.
//!
//! ④(R2) negative: sidecar is non-authoritative — never gates or blocks
//! landing.

use hugit_contracts::IntentSidecar;
use thiserror::Error;

/// Error returned when a sidecar is found to be (incorrectly) authoritative.
#[derive(Debug, Error)]
#[error(
    "sidecar guard violation: sidecar claims authoritative=true for intent_id={intent_id}; \
         sidecars are non-authoritative by design and must never gate landing"
)]
pub struct AuthoritativeGuardError {
    /// The intent_id of the offending sidecar.
    pub intent_id: String,
}

/// Assert that a sidecar is non-authoritative.
///
/// This is the ④(R2) runtime guard: if any code path produces a sidecar
/// with `authoritative = true`, this function returns an error. The caller
/// MUST NOT use the sidecar to gate landing when this guard returns an error.
///
/// Note: the `parse_sidecar` function already rejects sidecars with
/// `authoritative = true` at parse time; this guard is a belt-and-suspenders
/// assertion for the landing path (B4) to call before consuming any sidecar.
pub fn assert_non_authoritative(sidecar: &IntentSidecar) -> Result<(), AuthoritativeGuardError> {
    if sidecar.authoritative {
        return Err(AuthoritativeGuardError {
            intent_id: sidecar.intent_id.clone(),
        });
    }
    Ok(())
}

/// Returns true if the sidecar is safe to use as a non-authoritative
/// provenance record (i.e., `authoritative` is false).
///
/// The landing path (B4) reads no authoritative signal from the sidecar.
/// The corpus is provenance, not a gate.
pub fn is_non_authoritative(sidecar: &IntentSidecar) -> bool {
    !sidecar.authoritative
}
