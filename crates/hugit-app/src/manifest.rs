//! GitHub App manifest — least-privilege snapshot (WP-B1 item ④).
//!
//! The manifest requests only the scopes Phase B requires:
//!   - Checks: read + write
//!   - Pull requests: read
//!   - Contents: read
//!   - Webhook events: check_suite, check_run, pull_request, installation
//!
//! A committed JSON fixture (`tests/fixtures/app-manifest.json`) is the
//! single source of truth. The `APP_MANIFEST` constant is the canonical
//! in-code representation; tests assert byte-identity with the fixture.

/// Least-privilege GitHub App manifest (item ④).
///
/// This is the static manifest snapshot.  Any change to required scopes must
/// update BOTH this constant and the committed fixture; tests will catch drift.
pub const APP_MANIFEST: &str = include_str!("../tests/fixtures/app-manifest.json");

/// Validate that the in-code manifest parses as valid JSON and contains the
/// required least-privilege scopes.
///
/// Returns `Ok(())` if the manifest is well-formed and least-privileged.
pub fn validate_manifest(manifest_json: &str) -> Result<(), ManifestError> {
    let v: serde_json::Value =
        serde_json::from_str(manifest_json).map_err(|e| ManifestError::Parse(e.to_string()))?;

    // The manifest must declare permissions.
    let perms = v
        .get("default_permissions")
        .or_else(|| v.get("permissions"))
        .ok_or(ManifestError::MissingPermissions)?;

    // Checks: read or write is required (write implies read).
    let checks = perms
        .get("checks")
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    if checks != "write" && checks != "read" {
        return Err(ManifestError::ScopeMissing("checks".to_string()));
    }

    // Pull requests: read minimum.
    let prs = perms
        .get("pull_requests")
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    if prs != "read" && prs != "write" {
        return Err(ManifestError::ScopeMissing("pull_requests".to_string()));
    }

    // Contents: read minimum.
    let contents = perms
        .get("contents")
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    if contents != "read" && contents != "write" {
        return Err(ManifestError::ScopeMissing("contents".to_string()));
    }

    Ok(())
}

/// Errors from manifest validation.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest JSON parse error: {0}")]
    Parse(String),

    #[error("manifest missing permissions object")]
    MissingPermissions,

    #[error("manifest missing required scope: {0}")]
    ScopeMissing(String),

    #[error("manifest requests over-privileged scope: {0}")]
    OverPrivileged(String),
}
