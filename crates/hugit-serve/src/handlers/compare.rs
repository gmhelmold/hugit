//! `GET /v1/repos/{repo}/compare/{base}/{head}` → [`CompareVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: branches /
//! generated_branches from `replay()` RefState (mirrors commits.rs and
//! branches.rs exactly). HONEST-STUB: diff (DiffVm), commits (Vec<CommitRowVm>),
//! can_merge, stats_note, commits_note — no diffstat seam exists in this codebase
//! (every DiffVm in hugit-serve is an empty stub; confirmed by grep of pr_detail,
//! review, intent_detail). The route arm passes raw path-segment strings as
//! `base`/`head`; callers that embed slashes must percent-encode them as `%2F`
//! (path segments are split on `/` before dispatch).

use hugit_http_contracts::common::DiffVm;
use hugit_http_contracts::compare::CompareVm;
use hugit_refstore::{EventLog, replay};

use crate::fmt::scrub;

/// Build the compare view-model from a verified event log.
///
/// `base` and `head` arrive as raw path-segment strings (already split from the
/// URL by the router); they are scrubbed at the read boundary since they are
/// free-text echoed back to the client.
pub fn build_compare(log: &EventLog, repo: &str, base: &str, head: &str) -> CompareVm {
    // ── REAL: branches and generated_branches via replay() RefState ──────────
    // Mirrors commits.rs and branches.rs exactly: replay → strip refs/heads/ →
    // partition on "intent/" prefix.
    let ref_state = replay(log).unwrap_or_default();

    let all_head_refs: Vec<String> = ref_state
        .iter()
        .filter(|(name, _)| name.starts_with("refs/heads/"))
        .map(|(name, _)| name.strip_prefix("refs/heads/").unwrap_or(name).to_string())
        .collect();

    // Ref short-names are ATTACKER-CONTROLLABLE now that `git push` is live (a pushed
    // branch name is free text), so they are scrubbed at the read boundary exactly like
    // `base`/`head` below — a secret-shaped branch name must never echo to a viewer.
    let generated_branches: Vec<String> = all_head_refs
        .iter()
        .filter(|b| b.starts_with("intent/"))
        .map(|b| scrub(b))
        .collect();

    let branches: Vec<String> = all_head_refs
        .iter()
        .filter(|b| !b.starts_with("intent/"))
        .map(|b| scrub(b))
        .collect();

    // ── HONEST-STUB: diff, commits, can_merge, notes ─────────────────────────
    // No diffstat seam exists in this codebase (every DiffVm in hugit-serve is
    // an empty struct-literal stub; confirmed by inspecting pr_detail, review,
    // and intent_detail). These fields are honestly disclosed as stubs —
    // never faked.
    CompareVm {
        repo: repo.to_string(),
        base: scrub(base), // free text echoed from the URL — scrubbed
        head: scrub(head), // free text echoed from the URL — scrubbed
        branches,
        generated_branches,
        can_merge: false,            // HONEST-STUB — no merge-ability seam
        stats_note: String::new(),   // HONEST-STUB — no diffstat seam
        commits: vec![],             // HONEST-STUB — no commit-graph walk seam
        commits_note: String::new(), // HONEST-STUB — no commit-graph walk seam
        diff: DiffVm {
            files: vec![],
            hunks: vec![],
        }, // HONEST-STUB — no diffstat seam
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_http_contracts::compare::CompareVm;

    #[test]
    fn empty_log_yields_empty_branches() {
        let log = EventLog::new();
        let vm = build_compare(&log, "hugit", "main", "feat/sessions");
        assert_eq!(vm.repo, "hugit");
        assert_eq!(vm.base, "main");
        assert_eq!(vm.head, "feat/sessions");
        assert!(vm.branches.is_empty(), "empty log → no plain branches");
        assert!(
            vm.generated_branches.is_empty(),
            "empty log → no generated branches"
        );
        // Honest-stubs must be empty/false, never fabricated.
        assert!(!vm.can_merge);
        assert!(vm.stats_note.is_empty());
        assert!(vm.commits.is_empty());
        assert!(vm.commits_note.is_empty());
        assert!(vm.diff.files.is_empty());
        assert!(vm.diff.hunks.is_empty());
    }

    #[test]
    fn populated_log_surfaces_real_branches() {
        let mut log = EventLog::new();
        // Use append_for_test (feature = "test-support", already enabled in
        // hugit-serve dev-dependencies) — the ONLY idiomatic cross-crate test
        // append. It computes the correct prev_hash/this_hash chain automatically,
        // so replay() → verify_chain() passes and the ref fold succeeds.
        // Kind must be "ref.update" (no trailing 'd') — the only kind the
        // replay fold recognises for a ref mutation (see hugit-refstore/src/replay/mod.rs).
        log.append_for_test(
            "ref.update",
            vec!["test".to_string()],
            r#"{"ref":"refs/heads/main","target":"aabbcc112233"}"#.to_string(),
            0,
        );
        log.append_for_test(
            "ref.update",
            vec!["test".to_string()],
            r#"{"ref":"refs/heads/feat/sessions","target":"ddeeff445566"}"#.to_string(),
            1,
        );
        log.append_for_test(
            "ref.update",
            vec!["test".to_string()],
            r#"{"ref":"refs/heads/intent/a31","target":"112233aabbcc"}"#.to_string(),
            2,
        );

        let vm = build_compare(&log, "hugit", "main", "feat/sessions");

        assert!(
            vm.branches.contains(&"main".to_string()),
            "plain branch 'main' must appear"
        );
        assert!(
            vm.branches.contains(&"feat/sessions".to_string()),
            "plain branch 'feat/sessions' must appear"
        );
        assert!(
            !vm.branches.contains(&"intent/a31".to_string()),
            "intent branch must NOT be in plain branches"
        );
        assert!(
            vm.generated_branches.contains(&"intent/a31".to_string()),
            "intent branch must appear in generated_branches"
        );
        assert_eq!(vm.repo, "hugit");
        assert_eq!(vm.base, "main");
        assert_eq!(vm.head, "feat/sessions");

        // Honest-stubs must remain empty/false regardless of log content.
        assert!(!vm.can_merge);
        assert!(vm.commits.is_empty());
        assert!(vm.diff.files.is_empty());
        assert!(vm.diff.hunks.is_empty());
    }

    #[test]
    fn vm_round_trips_json() {
        let log = EventLog::new();
        let vm = build_compare(&log, "hugit", "main", "feat/x");
        let json = serde_json::to_string(&vm).expect("CompareVm serializes");
        let back: CompareVm = serde_json::from_str(&json).expect("CompareVm round-trips");
        assert_eq!(vm, back, "JSON round-trip must be lossless");
    }

    /// SECRET-MATRIX (read-path audit 2026-06-30): a branch name is ATTACKER-CONTROLLABLE
    /// now that `git push` is live, so a secret-shaped branch name MUST be `[REDACTED]` in
    /// the compare VM's `branches`/`generated_branches` — never echoed raw to a viewer.
    #[test]
    fn secret_shaped_branch_name_is_scrubbed() {
        use hugit_ledger::redact::REDACTED;
        // The canonical classic-PAT specimen the detector redacts.
        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let mut log = EventLog::new();
        log.append_for_test(
            "ref.update",
            vec!["test".to_string()],
            format!(r#"{{"ref":"refs/heads/{secret}","target":"aabbcc112233"}}"#),
            0,
        );
        log.append_for_test(
            "ref.update",
            vec!["test".to_string()],
            format!(r#"{{"ref":"refs/heads/intent/{secret}","target":"ddeeff445566"}}"#),
            1,
        );
        let vm = build_compare(&log, "hugit", "main", "feat/x");
        for b in vm.branches.iter().chain(vm.generated_branches.iter()) {
            assert!(
                !b.contains("ghp_"),
                "a secret-shaped branch name must never echo raw: {b}"
            );
            assert!(
                b.contains(REDACTED),
                "the secret-shaped branch name must be redacted: {b}"
            );
        }
        assert!(
            !vm.branches.is_empty() || !vm.generated_branches.is_empty(),
            "the seeded branches must surface (scrubbed)"
        );
    }
}
