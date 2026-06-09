//! Local↔runner BYTE-IDENTITY comparison (item ③).
//!
//! Whitepaper §6.2: "local `hugit check` and forge execution are the same
//! function — byte-identical by construction." This module is the proof
//! surface: given the [`CheckResult`] from a LOCAL execution and the
//! [`CheckResult`] from a RUNNER execution of the SAME deterministic check, it
//! verifies they are byte-identical at the ARTIFACT/DIGEST level.
//!
//! The comparison is **artifact/digest**, NOT result-equal. Two checks could
//! both exit 0 (a "result-equal" pass) while producing different output bytes —
//! that is a divergence the wedge must catch. So byte-identity is asserted by
//! comparing, per output path, the artifact CONTENT DIGEST on both sides, plus
//! the three memo axes (a deterministic check over the same axes must produce
//! the same outputs). Fields that legitimately differ between two execution
//! environments (wall-clock `duration_ms`, `produced_at`, the executing
//! `runner_ref`, and the captured stdout/stderr blob refs) are deliberately
//! EXCLUDED — they are not part of the check's content identity.

use std::collections::{BTreeMap, BTreeSet};

use hugit_contracts::CheckResult;

/// A per-artifact divergence between the local and runner executions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactDiff {
    /// An output path produced by exactly one side.
    PresenceMismatch {
        /// The output path in question.
        path: String,
        /// True iff the LOCAL execution produced it.
        in_local: bool,
        /// True iff the RUNNER execution produced it.
        in_runner: bool,
    },
    /// An output path produced by both sides but with different content digests
    /// — the core byte-identity violation.
    DigestMismatch {
        /// The output path whose content diverged.
        path: String,
        /// Content digest from the local execution.
        local_digest: String,
        /// Content digest from the runner execution.
        runner_digest: String,
    },
    /// A memo axis differed between the two results (the two sides did not even
    /// execute the same pure function — a precondition violation that would
    /// otherwise mask an artifact divergence).
    AxisMismatch {
        /// Which axis differed (`tree_hash` / `def_digest` / `toolchain_digest`
        /// / `memo_key`).
        axis: &'static str,
        /// Value on the local side.
        local: String,
        /// Value on the runner side.
        runner: String,
    },
    /// The two sides reported different process exit codes for the same inputs.
    ExitMismatch {
        /// Local exit code.
        local: i32,
        /// Runner exit code.
        runner: i32,
    },
}

/// The result of a local↔runner byte-identity comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteIdentityReport {
    /// Every detected divergence. Empty ⇒ byte-identical.
    pub diffs: Vec<ArtifactDiff>,
    /// Number of artifact paths compared on both sides (evidence figure).
    pub artifacts_compared: usize,
}

impl ByteIdentityReport {
    /// True iff the local and runner executions are byte-identical (no diffs).
    pub fn is_identical(&self) -> bool {
        self.diffs.is_empty()
    }
}

/// Compare a LOCAL and a RUNNER [`CheckResult`] for byte-identity.
///
/// Returns a [`ByteIdentityReport`]: empty `diffs` ⇒ byte-identical. The
/// comparison is order-independent (artifacts are matched by path) and covers
/// presence, per-path content digest, the three memo axes + memo key, and exit
/// code — never the environment-specific timing/runner/blob-ref fields.
pub fn compare_byte_identity(local: &CheckResult, runner: &CheckResult) -> ByteIdentityReport {
    let mut diffs = Vec::new();

    // The two sides must have executed the SAME pure function: equal axes.
    for (axis, l, r) in [
        ("memo_key", &local.memo_key, &runner.memo_key),
        ("tree_hash", &local.tree_hash, &runner.tree_hash),
        ("def_digest", &local.def_digest, &runner.def_digest),
        (
            "toolchain_digest",
            &local.toolchain_digest,
            &runner.toolchain_digest,
        ),
    ] {
        if l != r {
            diffs.push(ArtifactDiff::AxisMismatch {
                axis,
                local: l.clone(),
                runner: r.clone(),
            });
        }
    }

    if local.exit != runner.exit {
        diffs.push(ArtifactDiff::ExitMismatch {
            local: local.exit,
            runner: runner.exit,
        });
    }

    // Index artifacts by path on both sides (path is the artifact identity).
    let local_map: BTreeMap<&str, &str> = local
        .artifacts
        .iter()
        .map(|a| (a.path.as_str(), a.digest.as_str()))
        .collect();
    let runner_map: BTreeMap<&str, &str> = runner
        .artifacts
        .iter()
        .map(|a| (a.path.as_str(), a.digest.as_str()))
        .collect();

    let mut compared = 0usize;
    let all_paths: BTreeSet<&str> = local_map.keys().chain(runner_map.keys()).copied().collect();

    for path in &all_paths {
        match (local_map.get(path), runner_map.get(path)) {
            (Some(l), Some(r)) => {
                compared += 1;
                if l != r {
                    diffs.push(ArtifactDiff::DigestMismatch {
                        path: path.to_string(),
                        local_digest: (*l).to_string(),
                        runner_digest: (*r).to_string(),
                    });
                }
            }
            (l, r) => {
                diffs.push(ArtifactDiff::PresenceMismatch {
                    path: path.to_string(),
                    in_local: l.is_some(),
                    in_runner: r.is_some(),
                });
            }
        }
    }

    ByteIdentityReport {
        diffs,
        artifacts_compared: compared,
    }
}
