//! The pinned canonical `CheckDef` for hugit's OWN CI gate on the rota-A moat
//! check-host (#68).
//!
//! The check-host (B-path) hydrates a content-addressed toolchain
//! (`clw hydrate --manifest-digest <toolchain_ref>`) into `/toolchain` and runs
//! `CheckDef.command` in it. Per the ratified producer-side contract
//! (`docs/handoff/2026-06-28-…-toolchain-ref-is-the-snapshot-digest-ACCEPTED…`),
//! `CheckDef.toolchain_ref` is the clw snapshot **root digest** — no
//! human-readable alias — so the memo axis equals the hydrated content by
//! construction (no false cache hit across two toolchains under one label).
//!
//! This module freezes that pin now that the snapshot + hydrate round-trip are
//! done (#68): the gate toolchain (rust 1.96.0 + rustfmt/clippy + cargo-deny
//! 0.19.9 + cargo-audit 0.22.2, all at `/toolchain/bin`) was snapshotted to CAS
//! tenant `d863fafb` and the runners TL hydrate-verified the digest complete +
//! self-consistent (2026-07-07). So when the owner flips rota-A, a check acquire
//! carrying this `toolchain_digest` hydrates exactly this tree and runs exactly
//! this command — zero further hugit work at flip time. (Live memoized-check
//! dispatch through the moat is the P2 that waits on rota-A being on.)

use crate::client::memo_key::compute_def_digest;
use hugit_contracts::CheckDef;

/// The content-addressed toolchain digest (clw snapshot `root`) for hugit's CI
/// gate toolchain. This IS `CheckDef.toolchain_ref` — the memo axis AND what the
/// check-host hydrates. It was snapshotted to CAS tenant
/// `d863fafb-17c3-4ec3-92f6-b5a85c27d7bd` (hugit's live-evidenced acquire-tenant)
/// and hydrate-verified complete/self-consistent by the runners TL on 2026-07-07
/// (#68 closed). The tree holds rust `1.96.0` (`ac68faa20`) with `rustfmt` and
/// `clippy`, `cargo-deny 0.19.9`, and `cargo-audit 0.22.2` — all binaries at
/// `/toolchain/bin` (168 files, 668,404,519 bytes).
pub const HUGIT_CI_TOOLCHAIN_REF: &str =
    "4e3da22efe8c5ee8a6b57820ca93f4c772e972e9021db2dd05e73a5f611cc6c8";

/// The moat check-host runs `CheckDef.command` with `cwd = /toolchain` and PATH
/// NOT auto-set — so the command PREPENDS `/toolchain/bin` to resolve the whole
/// hydrated gate. Mirrors `.github/workflows/ci.yml` step-for-step (the SAME
/// gate `main` is held to): fmt → clippy `-D warnings` → test → cargo-deny →
/// cargo-audit. Kept byte-for-byte in sync with CI so a moat run and a CI run
/// are the identical gate.
pub const HUGIT_CI_GATE_COMMAND: &str = "PATH=/toolchain/bin:$PATH \
cargo fmt --all --check && \
cargo clippy --workspace --all-targets --locked -- -D warnings && \
cargo test --workspace --locked && \
cargo-deny check && \
cargo-audit audit --deny warnings";

/// The pinned canonical [`CheckDef`] for hugit's own gate on the moat
/// check-host: the exact gate command + the content-addressed `toolchain_ref`,
/// with the `def_digest` computed over the canonical body. The `glob_set` scopes
/// materialization to the workspace sources + the lockfile (the inputs that
/// change the gate result).
pub fn hugit_gate_check_def() -> CheckDef {
    let mut def = CheckDef {
        def_digest: String::new(),
        command: HUGIT_CI_GATE_COMMAND.to_string(),
        inputs: vec![],
        toolchain_ref: HUGIT_CI_TOOLCHAIN_REF.to_string(),
        env_manifest: String::new(),
        glob_set: vec![
            "crates/**".to_string(),
            "Cargo.toml".to_string(),
            "Cargo.lock".to_string(),
            "rust-toolchain.toml".to_string(),
        ],
    };
    def.def_digest = compute_def_digest(&def);
    def
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolchain_ref_is_the_verified_content_addressed_digest() {
        // A 64-hex blake3/sha root — never a human-readable alias (the ratified
        // producer-side contract: the memo axis equals the hydrated content).
        assert_eq!(HUGIT_CI_TOOLCHAIN_REF.len(), 64);
        assert!(
            HUGIT_CI_TOOLCHAIN_REF
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        );
        assert_eq!(hugit_gate_check_def().toolchain_ref, HUGIT_CI_TOOLCHAIN_REF);
    }

    #[test]
    fn command_prepends_the_toolchain_bin_path_and_runs_the_full_gate() {
        let cmd = &hugit_gate_check_def().command;
        // PATH prepend (the check-host does not auto-set it).
        assert!(cmd.starts_with("PATH=/toolchain/bin:$PATH "));
        // Every gate leg the CI workflow runs — a moat run is the same gate.
        for leg in [
            "cargo fmt --all --check",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
            "cargo test --workspace --locked",
            "cargo-deny check",
            "cargo-audit audit --deny warnings",
        ] {
            assert!(cmd.contains(leg), "gate command must run `{leg}`");
        }
    }

    #[test]
    fn def_digest_is_computed_stable_and_non_empty() {
        let a = hugit_gate_check_def();
        let b = hugit_gate_check_def();
        assert!(!a.def_digest.is_empty());
        assert_eq!(a.def_digest, b.def_digest, "the pin is deterministic");
        // The digest binds the command + toolchain_ref + glob_set (the memo body),
        // so changing any of them changes the memo key — the whole point.
        assert_eq!(a.def_digest.len(), 64);
    }
}
