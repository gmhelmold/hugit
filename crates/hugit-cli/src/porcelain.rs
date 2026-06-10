//! Porcelain shared helpers (WP-PC0 scaffold).
//!
//! Single-sources the JSON-on-stdout conventions the flow porcelain
//! (`campaign` / `intent` / `pr`) shares: stable JSON always (a `--human`
//! pretty mode lands later), and a structured error envelope carrying an honest
//! `kind` + the owning WP so callers (LLMs) get a machine-parseable signal
//! rather than a fake success.
//!
//! The scaffold's stubs route through [`not_implemented`] so PC1/PC2/PC3 inherit
//! one error shape and replace behavior, not wiring.

use std::process::ExitCode;

/// The process exit code for a structured (non-crash) command error.
///
/// `0` = success, `1` is reserved for the binary's generic library-error path
/// (`main.rs`), `2` = a structured porcelain error emitted as JSON on stdout.
pub const PORCELAIN_ERROR_EXIT: u8 = 2;

/// Emit the canonical NOT-IMPLEMENTED error as JSON on **stdout** and return the
/// structured-error exit code.
///
/// Shape (stable contract for callers): `{"error":{"kind":"not_implemented",
/// "wp":"PC1"}}`. Honest stub — never a fake success. `wp` names the work
/// package that will replace the stub with the real projection.
pub fn not_implemented(wp: &str) -> ExitCode {
    println!("{}", not_implemented_json(wp));
    ExitCode::from(PORCELAIN_ERROR_EXIT)
}

/// The canonical NOT-IMPLEMENTED JSON line for `wp` (the stable wire shape).
///
/// Split out from [`not_implemented`] so the exact contract can be asserted in
/// tests without capturing stdout. Hand-built — the two-field object IS the
/// contract; `wp` is a fixed ASCII WP token, never untrusted input.
pub fn not_implemented_json(wp: &str) -> String {
    format!(r#"{{"error":{{"kind":"not_implemented","wp":"{wp}"}}}}"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_implemented_json_is_the_stable_shape() {
        assert_eq!(
            not_implemented_json("PC1"),
            r#"{"error":{"kind":"not_implemented","wp":"PC1"}}"#
        );
    }
}
