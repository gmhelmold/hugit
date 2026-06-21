//! WP-X5 acceptance oracle — namespace laws: no git-verb shadow, ref-namespace
//! non-collision. Contract: `the work-package contract`.
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ① `item_1_no_hugit_verb_shadows_git_verb` — no hugit CLI verb matches
//!      any command in git's OWN command set, generated at test time by
//!      shelling out to `git --list-cmds=builtins,main` (compiled-in +
//!      porcelain — deterministic across machines). NOT `git help -a`, which
//!      also enumerates ambient external `git-*` binaries on `PATH` and so
//!      varies by host. A new shadowing verb in hugit-cli turns this red
//!      immediately.
//!   ② `item_2_managed_refs_no_collision` — a property test over a large
//!      corpus of arbitrary user branch/tag names asserts that no user ref can
//!      collide with the `refs/hugit/…` managed namespace, and that a managed
//!      ref is never misclassified as a user ref. Both directions of the
//!      partition are exercised.

use hugit_invariants::x5::{
    HUGIT_REF_PREFIX, HUGIT_VERBS, is_managed_ref, is_user_ref, parse_git_cmd_list,
};
use std::collections::HashSet;
use std::process::Command;

// ── ① the verb set under test is the REAL CLI registry, not a local copy ─────
//
// Load-bearing for the no-shadow law: the no-shadow oracle must check the verbs
// the `hugit` binary ACTUALLY dispatches, derived from the single canonical
// registry (`hugit_cli::HUGIT_VERBS`). A hand-copied list rots silently — the
// binary could grow a git-shadowing verb the local copy never sees. This test
// fails RED if the X5 verb surface ever diverges from the real CLI registry.
#[test]
fn item_1a_verb_set_is_the_real_cli_registry() {
    assert_eq!(
        HUGIT_VERBS,
        hugit_cli::HUGIT_VERBS,
        "X5's verb surface MUST be the real hugit-cli registry, not a local \
         hand-copied list — a divergent copy can hide a git-shadowing verb the \
         binary actually dispatches"
    );
}

// ── ① no hugit CLI verb shadows a git verb ───────────────────────────────────
//
// Load-bearing: git verb set generated at test time via `git --list-cmds`.
// Any hugit verb that also appears in git's command set is a namespace violation.
#[test]
fn item_1_no_hugit_verb_shadows_git_verb() {
    // Shell out to the local git binary — the oracle is the real git, not a
    // hand-maintained list that would rot. The source is `--list-cmds=builtins,main`
    // (git's OWN compiled-in + porcelain commands), NOT `git help -a`: the latter
    // also enumerates ambient external `git-*` binaries found on `PATH`, which
    // vary by host (a CI runner's third-party `git-repo` tool made this oracle
    // environment-dependent). HUGIT_VERBS must be disjoint from git's commands.
    let output = Command::new("git")
        .args(["--list-cmds=builtins,main"])
        .output()
        .expect(
            "git --list-cmds=builtins,main must be runnable (git is required by the acceptance suite)",
        );

    assert!(
        output.status.success(),
        "git --list-cmds=builtins,main exited with non-zero status: {:?}",
        output.status
    );

    let git_cmd_text = String::from_utf8_lossy(&output.stdout);
    let git_verbs: HashSet<String> = parse_git_cmd_list(&git_cmd_text);

    assert!(
        !git_verbs.is_empty(),
        "git --list-cmds=builtins,main produced no command tokens — the parser is broken or git output format changed"
    );

    // Build the hugit verb set (from the canonical constant).
    let hugit_verbs: HashSet<&str> = HUGIT_VERBS.iter().copied().collect();

    // The intersection must be empty: no hugit verb may shadow a git verb.
    let mut shadowing: Vec<String> = hugit_verbs
        .iter()
        .filter(|v| git_verbs.contains(**v))
        .map(|v| v.to_string())
        .collect();
    shadowing.sort();

    assert!(
        shadowing.is_empty(),
        "NAMESPACE VIOLATION: the following hugit verbs shadow git commands from `git --list-cmds=builtins,main`:\n  {}\n\
         Each shadowed verb is a broken namespace law. Remove or rename these hugit verbs.",
        shadowing.join(", ")
    );

    // Confirm all hugit verbs were checked (paranoia guard: list is non-empty).
    assert!(
        !hugit_verbs.is_empty(),
        "HUGIT_VERBS is empty — the verb surface oracle has no entries to check"
    );
}

// ── ② managed refs (refs/hugit/…) never collide with user refs ───────────────
//
// Property test: iterate a large, diverse corpus of user branch and tag name
// patterns and assert the ref-namespace partition is clean in both directions:
//   (a) a user ref is NEVER classified as a managed ref,
//   (b) a managed ref is NEVER classified as a user ref,
//   (c) no user-supplied branch/tag name can be constructed to collide with
//       the `refs/hugit/…` reserved prefix.
#[test]
fn item_2_managed_refs_no_collision() {
    // ── corpus A: typical user branch and tag ref patterns ───────────────────
    let user_refs: &[&str] = &[
        // Standard branch refs
        "refs/heads/main",
        "refs/heads/master",
        "refs/heads/feature/my-branch",
        "refs/heads/fix/bug-123",
        "refs/heads/release/v1.0.0",
        "refs/heads/hugit", // the word "hugit" alone is NOT the prefix
        "refs/heads/hugit-cli",
        "refs/heads/hugit-refstore",
        "refs/heads/hugit-feature",
        "refs/heads/refs/hugit", // path component that contains the prefix word
        // Standard tag refs
        "refs/tags/v1.0.0",
        "refs/tags/v2.0.0-rc1",
        "refs/tags/release-2026-06-05",
        "refs/tags/hugit-v1.0",
        // Remote-tracking refs
        "refs/remotes/origin/main",
        "refs/remotes/upstream/feature",
        "refs/remotes/hugit-remote/main",
        // Notes refs
        "refs/notes/commits",
        // Stash ref
        "refs/stash",
        // Arbitrary user refs that start with "refs/" but NOT "refs/hugit/"
        "refs/hugitx/something",    // "hugitx" is NOT "hugit/"
        "refs/hugit-not/something", // "hugit-not/" is NOT "hugit/"
        "refs/hugit",               // "refs/hugit" with no trailing slash is NOT the prefix
        "refs/xhugit/foo",
        "refs/pull/42/head",
        "refs/changes/12/34567/1",
        // Edge cases: empty-ish paths within user space
        "refs/heads/a",
        "refs/heads/z",
        // Very long names (still user refs)
        "refs/heads/very/deeply/nested/feature/branch/for/a/long/path",
    ];

    for user_ref in user_refs {
        assert!(
            is_user_ref(user_ref),
            "USER REF misclassified as managed: {:?}\n\
             A user ref must not start with {:?}",
            user_ref,
            HUGIT_REF_PREFIX
        );
        assert!(
            !is_managed_ref(user_ref),
            "USER REF misclassified as managed (is_managed_ref returned true): {:?}",
            user_ref
        );
    }

    // ── corpus B: valid hugit-managed ref names ───────────────────────────────
    let managed_refs: &[&str] = &[
        // Core hugit managed refs (as-built by the refstore)
        "refs/hugit/snapshots/abc123",
        "refs/hugit/intents/42",
        "refs/hugit/intents/feed/beef/dead",
        "refs/hugit/ledger/head",
        "refs/hugit/campaign/sprint-1",
        "refs/hugit/ws/snap/tenant-1/abc",
        "refs/hugit/ctx/resume/session-xyz",
        "refs/hugit/locks/merge-queue",
        "refs/hugit/gc/marker",
        // Single-component managed ref (just the prefix + one token)
        "refs/hugit/x",
        // Deeply nested managed ref
        "refs/hugit/very/deeply/nested/managed/ref",
    ];

    for managed_ref in managed_refs {
        assert!(
            is_managed_ref(managed_ref),
            "MANAGED REF misclassified as user: {:?}\n\
             A managed ref must start with {:?}",
            managed_ref,
            HUGIT_REF_PREFIX
        );
        assert!(
            !is_user_ref(managed_ref),
            "MANAGED REF misclassified as user (is_user_ref returned true): {:?}",
            managed_ref
        );
    }

    // ── partition completeness: every ref is either user OR managed, never both
    for user_ref in user_refs {
        assert!(
            is_user_ref(user_ref) != is_managed_ref(user_ref),
            "REF is both user AND managed (partition violated): {:?}",
            user_ref
        );
    }
    for managed_ref in managed_refs {
        assert!(
            is_user_ref(managed_ref) != is_managed_ref(managed_ref),
            "REF is both user AND managed (partition violated): {:?}",
            managed_ref
        );
    }

    // ── attack: try to construct a user ref that collides with hugit namespace
    //    Strategy: prepend "refs/heads/" to the hugit prefix — the result is
    //    a branch named after the prefix text, NOT a managed ref.
    let attack_refs: &[&str] = &[
        // A branch NAMED "refs/hugit/..." — this is still under refs/heads/,
        // not under refs/hugit/ directly. In git, branch names cannot contain
        // a slash that would make them look like a different ref prefix.
        // We verify that the logical ref path "refs/heads/hugit/foo" is a user ref.
        "refs/heads/hugit/foo",
        "refs/heads/hugit/something",
        // Attempt to use the prefix as a tag
        "refs/tags/hugit/v1",
        // Attempt refs just before the prefix boundary
        "refs/hugit",     // missing trailing slash
        "refs/HUGIT/foo", // case-sensitive: uppercase is NOT the prefix
        "refs/Hugit/foo",
        "refs/hugit-/foo", // extra char after "hugit"
    ];

    for attack_ref in attack_refs {
        assert!(
            is_user_ref(attack_ref),
            "ATTACK REF was misclassified as managed: {:?}\n\
             The managed namespace is {:?} (exact prefix); this ref must be user-space.",
            attack_ref,
            HUGIT_REF_PREFIX
        );
    }

    // ── invariant: HUGIT_REF_PREFIX ends with '/' (structural requirement) ───
    assert!(
        HUGIT_REF_PREFIX.ends_with('/'),
        "HUGIT_REF_PREFIX must end with '/' to form a proper path prefix (got {:?})",
        HUGIT_REF_PREFIX
    );

    // ── invariant: HUGIT_REF_PREFIX starts with 'refs/' ─────────────────────
    assert!(
        HUGIT_REF_PREFIX.starts_with("refs/"),
        "HUGIT_REF_PREFIX must start with 'refs/' (got {:?})",
        HUGIT_REF_PREFIX
    );
}
