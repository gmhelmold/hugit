//! `GET /v1/repos/{repo}/compare/{base}/{head}` → [`CompareVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: branches /
//! generated_branches from `replay()` RefState (mirrors commits.rs and
//! branches.rs exactly) AND the `diff` numstat — a REAL `base`→`head` tree-diff
//! computed from the per-repo git source + the live refs ([`crate::handlers::diff`]),
//! wall-clock bounded by `hugit_proto::DIFF_BUDGET`. HONEST-STUB: commits
//! (Vec<CommitRowVm>), can_merge, stats_note, commits_note — no commit-graph-walk
//! / merge-ability seam at this altitude. The route arm passes raw path-segment
//! strings as `base`/`head`; callers that embed slashes must percent-encode them
//! as `%2F` (path segments are split on `/` before dispatch).

use std::sync::Arc;

use hugit_http_contracts::compare::CompareVm;
use hugit_refstore::{EventLog, replay};

use crate::fmt::scrub;
use crate::handlers::diff::{diff_compare, empty_diff};

/// Build the compare view-model from a verified event log.
///
/// `base` and `head` arrive as raw path-segment strings (already split from the
/// URL by the router); they are scrubbed at the read boundary since they are
/// free-text echoed back to the client.
///
/// `git_source` is the per-repo object source and `git_refs` the live ref
/// snapshot (ref-name → oid hex); together they resolve `base`/`head` to commits
/// and compute the REAL numstat. Both `None`/empty (no git seam) → an honest-empty
/// diff, never a fabrication.
pub fn build_compare(
    log: &EventLog,
    repo: &str,
    base: &str,
    head: &str,
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    git_refs: &std::collections::BTreeMap<String, String>,
) -> CompareVm {
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

    // ── REAL: the base→head numstat (per-repo git source + live refs) ────────
    // Resolves each ref-ish against `git_refs` (then a raw-oid fallback), walks
    // the two root trees, and projects the changed-file rows — wall-clock bounded
    // by `hugit_proto::DIFF_BUDGET`, paths scrubbed at the read boundary. No git
    // seam / an unresolvable side → the honest-empty diff (never a fabrication).
    let diff = match git_source {
        Some(_) => diff_compare(git_source, git_refs, base, head),
        None => empty_diff(),
    };

    // ── HONEST-STUB: commits, can_merge, notes ───────────────────────────────
    // No commit-graph-walk / merge-ability seam at this altitude. These fields are
    // honestly disclosed as stubs — never faked.
    CompareVm {
        repo: repo.to_string(),
        base: scrub(base), // free text echoed from the URL — scrubbed
        head: scrub(head), // free text echoed from the URL — scrubbed
        branches,
        generated_branches,
        can_merge: false,            // HONEST-STUB — no merge-ability seam
        stats_note: String::new(),   // HONEST-STUB — no commit-graph walk seam
        commits: vec![],             // HONEST-STUB — no commit-graph walk seam
        commits_note: String::new(), // HONEST-STUB — no commit-graph walk seam
        diff,                        // REAL — base→head numstat (honest-empty w/o git seam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix_hash::ObjectId;
    use hugit_http_contracts::compare::CompareVm;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind};
    use std::collections::BTreeMap;

    /// No git source (the common case in the older log-only tests).
    fn no_git() -> Option<&'static Arc<dyn hugit_proto::ObjectSource + Send + Sync>> {
        None
    }
    fn no_refs() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    // ── git-object fixture builders (mirror diff.rs / blob.rs test helpers) ──
    fn blob(src: &mut CasObjectSource, body: &str) -> ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(src: &mut CasObjectSource, mut entries: Vec<(&str, &str, ObjectId)>) -> ObjectId {
        entries.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &entries {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }
    /// A commit object naming `tree` and (optionally) one `parent`.
    fn commit(src: &mut CasObjectSource, tree: ObjectId, parent: Option<ObjectId>) -> ObjectId {
        let mut body = format!("tree {tree}\n");
        if let Some(p) = parent {
            body.push_str(&format!("parent {p}\n"));
        }
        body.push_str("author a <a@x> 0 +0000\ncommitter a <a@x> 0 +0000\n\nmsg\n");
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    #[test]
    fn empty_log_yields_empty_branches() {
        let log = EventLog::new();
        let vm = build_compare(&log, "hugit", "main", "feat/sessions", no_git(), &no_refs());
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

        let vm = build_compare(&log, "hugit", "main", "feat/sessions", no_git(), &no_refs());

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
        let vm = build_compare(&log, "hugit", "main", "feat/x", no_git(), &no_refs());
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
        let vm = build_compare(&log, "hugit", "main", "feat/x", no_git(), &no_refs());
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

    /// REAL numstat: a 2-commit fixture (base → head adds one line) resolved by
    /// SHORT branch name against the live refs map → a real changed-file row.
    #[test]
    fn compare_real_numstat_for_two_commit_fixture() {
        let mut s = CasObjectSource::new();
        // base: f.txt = "a\nb\n"; head: f.txt = "a\nb\nc\n" (one line added).
        let old_blob = blob(&mut s, "a\nb\n");
        let new_blob = blob(&mut s, "a\nb\nc\n");
        let base_tree = tree(&mut s, vec![("100644", "f.txt", old_blob)]);
        let head_tree = tree(&mut s, vec![("100644", "f.txt", new_blob)]);
        let base_commit = commit(&mut s, base_tree, None);
        let head_commit = commit(&mut s, head_tree, Some(base_commit));
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), base_commit.to_string());
        refs.insert("refs/heads/feat/x".to_string(), head_commit.to_string());

        let log = EventLog::new();
        let vm = build_compare(&log, "hugit", "main", "feat/x", Some(&src), &refs);
        assert_eq!(vm.diff.files.len(), 1, "one changed file");
        assert_eq!(vm.diff.files[0].path, "f.txt");
        assert_eq!(
            (vm.diff.files[0].added, vm.diff.files[0].removed),
            (1, 0),
            "one line added, none removed"
        );
    }

    /// Raw-oid fallback: a compare whose `base`/`head` are 40-hex commit oids (not
    /// ref names) still resolves to the real numstat.
    #[test]
    fn compare_resolves_raw_commit_oids() {
        let mut s = CasObjectSource::new();
        let b0 = blob(&mut s, "x\n");
        let b1 = blob(&mut s, "x\ny\n");
        let t0 = tree(&mut s, vec![("100644", "f", b0)]);
        let t1 = tree(&mut s, vec![("100644", "f", b1)]);
        let c0 = commit(&mut s, t0, None);
        let c1 = commit(&mut s, t1, Some(c0));
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let log = EventLog::new();
        // No refs map — the bare oid hex is the ref-ish.
        let vm = build_compare(
            &log,
            "hugit",
            &c0.to_string(),
            &c1.to_string(),
            Some(&src),
            &no_refs(),
        );
        assert_eq!(vm.diff.files.len(), 1);
        assert_eq!((vm.diff.files[0].added, vm.diff.files[0].removed), (1, 0));
    }

    /// HONEST-EMPTY: a git source is wired but the refs are unresolvable (an
    /// unknown branch / a non-oid) → an empty diff, NEVER an error or a fabrication.
    #[test]
    fn compare_unresolvable_refs_is_honest_empty() {
        let mut s = CasObjectSource::new();
        let _ = blob(&mut s, "x\n");
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let log = EventLog::new();
        let vm = build_compare(
            &log,
            "hugit",
            "no-such-branch",
            "also-missing",
            Some(&src),
            &no_refs(),
        );
        assert!(vm.diff.files.is_empty(), "unresolvable refs → empty diff");
        assert!(vm.diff.hunks.is_empty());
    }

    /// HONEST-EMPTY: no git source at all → an empty diff regardless of refs.
    #[test]
    fn compare_no_git_source_is_honest_empty() {
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), "a".repeat(40));
        let log = EventLog::new();
        let vm = build_compare(&log, "hugit", "main", "main", no_git(), &refs);
        assert!(vm.diff.files.is_empty());
        assert!(vm.diff.hunks.is_empty());
    }

    /// A secret-shaped FILE PATH in the diff must be scrubbed at the read boundary
    /// (the numstat is a new echo surface — same audit class as the branch names).
    #[test]
    fn compare_secret_shaped_diff_path_is_scrubbed() {
        let mut s = CasObjectSource::new();
        let b = blob(&mut s, "z\n");
        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let name = format!("{secret}.key");
        let base_tree = tree(&mut s, vec![]);
        let head_tree = tree(&mut s, vec![("100644", name.as_str(), b)]);
        let c0 = commit(&mut s, base_tree, None);
        let c1 = commit(&mut s, head_tree, Some(c0));
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), c0.to_string());
        refs.insert("refs/heads/feat/x".to_string(), c1.to_string());

        let log = EventLog::new();
        let vm = build_compare(&log, "hugit", "main", "feat/x", Some(&src), &refs);
        let j = serde_json::to_string(&vm.diff).unwrap();
        assert!(!j.contains("ghp_"), "secret-shaped path must not echo raw");
        assert!(j.contains("REDACTED"));
    }
}
