//! Focus-gate enforcement (X10②).
//!
//! corelink-server MUST NEVER be enrolled as a dogfood target. This is
//! enforced at compile-time (a const assertion) AND at runtime via
//! `assert_excluded`. The two layers together make it structurally impossible
//! to enroll the forbidden target by accident or malice.

/// The set of repositories that MAY be used as dogfood targets.
/// `corelink-server` is structurally absent.
pub const DOGFOOD_TARGET_ALLOWLIST: &[&str] = &[
    "hugit",
    "corelink-workspaces",
    "synthetic-fleet-alpha",
    "synthetic-fleet-beta",
];

/// Compile-time proof: corelink-server is not in the allowlist.
///
/// This `const` is evaluated at compile time; if anyone adds "corelink-server"
/// to `DOGFOOD_TARGET_ALLOWLIST` this assertion fires before link.
pub const CORELINK_SERVER_EXCLUDED: bool = {
    let forbidden = "corelink-server";
    let mut i = 0;
    while i < DOGFOOD_TARGET_ALLOWLIST.len() {
        let t = DOGFOOD_TARGET_ALLOWLIST[i].as_bytes();
        let f = forbidden.as_bytes();
        if t.len() == f.len() {
            let mut j = 0;
            let mut eq = true;
            while j < t.len() {
                if t[j] != f[j] {
                    eq = false;
                    break;
                }
                j += 1;
            }
            if eq {
                // The forbidden target is present — this const becomes `false`
                // and the `const _: () = assert!(…)` below fires.
                // We cannot `panic!` in a const context on stable; returning
                // `false` is the signal.
                break;
            }
        }
        i += 1;
    }
    i == DOGFOOD_TARGET_ALLOWLIST.len() // true iff never found
};

// Compile-time hard stop: if corelink-server ever slips into the allowlist
// this assertion fires before the crate links.
const _: () = assert!(
    CORELINK_SERVER_EXCLUDED,
    "corelink-server MUST NOT appear in DOGFOOD_TARGET_ALLOWLIST (X10②)"
);

/// Error from the runtime focus gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusGateError(pub String);

impl std::fmt::Display for FocusGateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FocusGateError {}

/// Assert that `target` is allowed by the focus gate.
///
/// - If `target` is `"corelink-server"` (or contains it as a substring in
///   any canonical form) this returns an error: enrollment is forbidden.
/// - If `target` is in [`DOGFOOD_TARGET_ALLOWLIST`] this returns `Ok(())`.
/// - Any other target is also rejected (fail-closed: unknown targets are not
///   implicitly permitted).
pub fn assert_excluded(target: &str) -> Result<(), FocusGateError> {
    // Hard exclusion: corelink-server must never be enrolled (X10②).
    if target.contains("corelink-server") {
        return Err(FocusGateError(format!(
            "corelink-server is excluded from dogfood targets (X10②): \
             refusing enrollment of '{target}'"
        )));
    }
    // Allowlist check: only explicitly-listed targets are permitted (fail-closed).
    if DOGFOOD_TARGET_ALLOWLIST.contains(&target) {
        return Ok(());
    }
    Err(FocusGateError(format!(
        "target '{target}' is not in DOGFOOD_TARGET_ALLOWLIST; \
         dogfood targets are: {DOGFOOD_TARGET_ALLOWLIST:?}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corelink_server_excluded_const_is_true() {
        const { assert!(CORELINK_SERVER_EXCLUDED) }
    }

    #[test]
    fn corelink_server_enrollment_is_rejected() {
        assert!(assert_excluded("corelink-server").is_err());
    }

    #[test]
    fn allowed_targets_are_accepted() {
        for t in DOGFOOD_TARGET_ALLOWLIST {
            assert!(assert_excluded(t).is_ok(), "expected {t} to be allowed");
        }
    }

    #[test]
    fn unknown_target_is_rejected() {
        assert!(assert_excluded("unknown-repo").is_err());
    }
}
