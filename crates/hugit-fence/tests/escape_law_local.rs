//! WP-WA5 — local escape-law regression for `hugit-fence`.
//!
//! Item ⑤ of the C5b acceptance suite transferred (WP-R4, runner-transfer
//! campaign 2026-06-10) the active `RedTeamHarness` — which drove six escape
//! vectors against a live box — to `corelink-runners` with the enforcement
//! half (`materialize`/`enforce`) it exercises. The classification and
//! traversal pieces that **remain here** (the fence broker, [`seam`] types,
//! and `util::normalize_path`) must carry a local, hermetic regression that:
//!
//! 1. Covers every escape-vector *class* the departed harness exercised,
//!    translated to pure `normalize_path` assertions (no box, no network).
//! 2. Contains a cross-repo sentinel that fails if the pointer in
//!    `acceptance_c5b.rs` to the full harness's new home is ever silently
//!    deleted — the gap stays disclosed, never silent.
//!
//! **Vector classes covered** (7):
//!
//! | # | Vector class | Attack |
//! |---|--------------|--------|
//! | 1 | `DotDotTraversal` | plain `../`, nested `../../`, mixed `./..` forms |
//! | 2 | `AbsolutePathInjection` | leading `/`, absolute-looking `result_rel` |
//! | 3 | `SymlinkShapedString` | path strings that mirror symlink targets |
//! | 4 | `PrefixCollision` | `/fence` prefix shared with `/fence-evil` |
//! | 5 | `UnicodeEncodedDots` | non-ASCII codepoints or percent-encoded dots |
//! | 6 | `EmptyAndRootEdge` | `""`, `"."`, `"./"`, root collapse |
//! | 7 | `CrossRepoSentinel` | item ⑤ pointer in acceptance_c5b.rs is intact |
//!
//! All tests are hermetic (pure string operations on `normalize_path`), so
//! they run in the bare `cargo test` gate with no external dependencies.

// `normalize_path` is `pub(crate)` — use the integration-test-visible public
// surface of the `broker` module, which re-exposes the traversal rule via the
// `ResultPathEscapes` guard on `execute_into_container`. For the lower-level
// assertions we access `normalize_path` through the broker's
// `result_path_escapes` behaviour, keeping the tests black-box against the
// public API where possible. Where the rule itself is the subject (prefix
// collision, unicode, edge cases) we drive the broker's `execute_into_container`
// with a minimal `SpyBox` so the guard is the only thing that can reject.
//
// The seam types (`BoxExec`, `CmdOutput`, `RunningContainer`) are public and
// used directly.

use std::cell::Cell;
use std::collections::BTreeMap;

use hugit_contracts::{RunnerLease, RunnerState};
use hugit_fence::broker::{Broker, BrokerError, BrokerOp, BrokerRequest, InMemoryStore, SecretRef};
use hugit_fence::seam::{BoxExec, CmdOutput, RunningContainer};

// ── helpers ──────────────────────────────────────────────────────────────────

/// A hermetic [`BoxExec`] that accepts every command (returns exit 0, empty
/// output) and records whether `run` was ever called. Used to prove the
/// traversal guard fires **before** any remote write.
struct SpyBox {
    ran: Cell<bool>,
}

impl SpyBox {
    fn new() -> Self {
        Self {
            ran: Cell::new(false),
        }
    }
}

impl BoxExec for SpyBox {
    fn run(&self, _argv: &[&str]) -> anyhow::Result<CmdOutput> {
        self.ran.set(true);
        Ok(CmdOutput {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

fn held_lease() -> RunnerLease {
    RunnerLease {
        lease_id: "lease/escape-law-local".to_string(),
        principal_chain: vec!["user:test-oracle".to_string()],
        path_set: vec!["src/".to_string()],
        expiry: 0,
        net_policy: "none".to_string(),
        tmp_root: "/work/tmp".to_string(),
        state: RunnerState::Held,
    }
}

fn store() -> InMemoryStore {
    let mut m = BTreeMap::new();
    m.insert("k".to_string(), b"local-oracle-key".to_vec());
    InMemoryStore::new(m)
}

/// Assert that `result_rel` is rejected by `execute_into_container` with
/// `ResultPathEscapes` and that the `SpyBox` was **never** called (the guard
/// fires before any remote write — fail-closed). Returns the spy so the caller
/// can assert `!spy.ran.get()` for extra clarity if desired.
fn assert_escape_rejected(result_rel: &str) -> SpyBox {
    let broker = Broker::new(store());
    let l = held_lease();
    let container = RunningContainer {
        name: "hugit-escape-law-local".to_string(),
    };
    let req = BrokerRequest {
        lease: &l,
        op: BrokerOp::Sign {
            secret: SecretRef::new("k"),
            message: b"test-message".to_vec(),
        },
    };
    let spy = SpyBox::new();
    let err = broker
        .execute_into_container(&spy, &container, "/job-ws", result_rel, &req)
        .expect_err(&format!(
            "escape vector {result_rel:?} must be rejected fail-closed"
        ));
    assert!(
        matches!(err, BrokerError::ResultPathEscapes { .. }),
        "vector {result_rel:?}: expected ResultPathEscapes, got {err:?}"
    );
    assert!(
        !spy.ran.get(),
        "vector {result_rel:?}: guard must fire BEFORE any box command"
    );
    spy
}

/// Assert that `result_rel` is accepted (not an escape) and a box write occurs.
fn assert_accepted(result_rel: &str) {
    let broker = Broker::new(store());
    let l = held_lease();
    let container = RunningContainer {
        name: "hugit-escape-law-local".to_string(),
    };
    let req = BrokerRequest {
        lease: &l,
        op: BrokerOp::Sign {
            secret: SecretRef::new("k"),
            message: b"test-message".to_vec(),
        },
    };
    let spy = SpyBox::new();
    let res = broker.execute_into_container(&spy, &container, "/job-ws", result_rel, &req);
    assert!(
        res.is_ok(),
        "fence-relative path {result_rel:?} must be accepted; got {res:?}"
    );
    assert!(
        spy.ran.get(),
        "accepted path {result_rel:?} must trigger a box write"
    );
}

// ── Vector class 1: DotDotTraversal ──────────────────────────────────────────

/// **DotDotTraversal** — plain `../` at the root of the path.
///
/// Corresponds to the `Traversal` vector in the departed `RedTeamHarness`:
/// a `..` component at any position resolves above the workspace root and must
/// be rejected immediately (fail-closed) before any remote write.
#[test]
fn dotdot_plain_at_root() {
    assert_escape_rejected("../escape.hex");
}

/// **DotDotTraversal** — single `..` with a trailing segment.
///
/// `../sibling/file` — a `..` followed by a seemingly innocuous path: still
/// escapes on the first segment.
#[test]
fn dotdot_with_trailing_segment() {
    assert_escape_rejected("../sibling/file.txt");
}

/// **DotDotTraversal** — nested `../../`.
///
/// Two levels of climb. The rule applies uniformly regardless of depth: any
/// `..` component returns `None`.
#[test]
fn dotdot_nested_two_levels() {
    assert_escape_rejected("../../etc/passwd");
}

/// **DotDotTraversal** — deeply nested `..` inside a legitimate-looking prefix.
///
/// `ws/../../etc/cron.d/evil` — the leading `ws/` looks benign; the `../..`
/// following it climbs back out. Multi-`..` climb must be rejected.
#[test]
fn dotdot_buried_in_deep_path() {
    assert_escape_rejected("ws/../../etc/cron.d/evil");
}

/// **DotDotTraversal** — `..` mixed with `.` (current-dir segments).
///
/// `./src/./../../../outside` — `.` segments are stripped before the `..`
/// is evaluated. The combined path still escapes.
#[test]
fn dotdot_mixed_with_dot_segments() {
    assert_escape_rejected("./src/./../../../outside");
}

/// **DotDotTraversal** — `..` at a non-root position after valid segments.
///
/// `a/b/../../../outside` — the initial `a/b/` descent does not provide
/// enough depth to absorb the three climbs; the path escapes.
#[test]
fn dotdot_multi_climb_after_segments() {
    assert_escape_rejected("a/b/../../../outside");
}

/// **DotDotTraversal** — `src/../src/main.rs` (even depth-neutral `..` is
/// rejected).
///
/// The rule is that ANY `..` component returns `None` regardless of whether
/// the net result would stay inside the root. This mirrors the `normalize_path`
/// design: the fence does not attempt to track depth; it refuses on first `..`.
#[test]
fn dotdot_depth_neutral_still_rejected() {
    assert_escape_rejected("src/../src/main.rs");
}

// ── Vector class 2: AbsolutePathInjection ────────────────────────────────────

/// **AbsolutePathInjection** — leading `/`.
///
/// An absolute path bypasses the workspace-root join entirely: `result_rel`
/// is supposed to be a fence-relative segment, but a leading `/` makes it
/// resolve anywhere on the host filesystem. Must be rejected at the first
/// byte.
#[test]
fn absolute_leading_slash() {
    assert_escape_rejected("/etc/passwd");
}

/// **AbsolutePathInjection** — plausible-looking absolute path.
///
/// `/abs/result.hex` looks like a reasonable file name but is absolute.
#[test]
fn absolute_plausible_looking() {
    assert_escape_rejected("/abs/result.hex");
}

/// **AbsolutePathInjection** — `/tmp`-targeting escape.
///
/// A classic target for out-of-fence delivery; must be caught by the leading
/// `/` check.
#[test]
fn absolute_tmp_target() {
    assert_escape_rejected("/tmp/escaped.hex");
}

/// **AbsolutePathInjection** — root path `/`.
///
/// The root path itself is absolute; must be rejected.
#[test]
fn absolute_root_only() {
    assert_escape_rejected("/");
}

/// **AbsolutePathInjection** — absolute path combined with `..`.
///
/// `/workspace/../etc/shadow` — the absolute prefix is caught before `..` is
/// even evaluated (the absolute check is the first guard).
#[test]
fn absolute_combined_with_dotdot() {
    assert_escape_rejected("/workspace/../etc/shadow");
}

// ── Vector class 3: SymlinkShapedString ──────────────────────────────────────

/// **SymlinkShapedString** — path string that mirrors a symlink target.
///
/// The `SymlinkEscape` vector in the departed harness created a symlink to
/// an absolute host path inside the container and read through it. Locally,
/// the equivalent is a result_rel whose resolved destination would be
/// absolute — e.g. a path that starts with `/etc/shadow` (an absolute target
/// a symlink might point at). The absolute-path guard catches this class.
#[test]
fn symlink_shaped_absolute_target() {
    assert_escape_rejected("/etc/shadow");
}

/// **SymlinkShapedString** — symlink-like path with `..` traversal inside.
///
/// A common symlink chain: link points to `../../host-secret`. The `..`
/// guard catches this before any write.
#[test]
fn symlink_shaped_dotdot_target() {
    assert_escape_rejected("../../host-secret");
}

/// **SymlinkShapedString** — a `/proc/self/fd`-style path (proc-escape).
///
/// A crafted path that tries to reach `/proc` via an absolute string; caught
/// by the absolute-path guard. Mirrors the class of attacks that use proc
/// filesystem paths to escape container namespaces.
#[test]
fn symlink_shaped_proc_path() {
    assert_escape_rejected("/proc/self/environ");
}

// ── Vector class 4: PrefixCollision ──────────────────────────────────────────

/// **PrefixCollision** — `fence` prefix shared with `fence-evil`.
///
/// A workspace root of `/fence` must not be reachable from a result_rel
/// pointing at `/fence-evil/...`. The absolute guard catches the leading `/`,
/// but this test also validates that a bare `fence-evil` (no slash) is
/// accepted as a legitimate fence-relative name — the two paths are
/// DISTINCT, not confused.
///
/// The key law: the fence boundary is a **prefix of the workspace root**,
/// not a string-prefix match on path segments. `/fence` and `/fence-evil`
/// are different roots; a result delivered into `/fence` must not land in
/// `/fence-evil` by prefix conflation.
#[test]
fn prefix_collision_absolute_sibling_is_rejected() {
    // `/fence-evil/...` is absolute → rejected. The fence root is `/fence`.
    assert_escape_rejected("/fence-evil/secret.txt");
}

/// **PrefixCollision** — a relative path that LOOKS like a sibling root.
///
/// `fence-evil/secret.txt` is NOT absolute and contains no `..`, so it is
/// accepted as a legitimate fence-relative path (it would be delivered to
/// `<workspace_root>/fence-evil/secret.txt`, inside the fence). This is the
/// POSITIVE counterpart: the fence does not confuse string-prefix similarity
/// with namespace escape.
#[test]
fn prefix_collision_relative_sibling_is_accepted() {
    // `fence-evil/secret.txt` relative to `/fence` → `/fence/fence-evil/secret.txt`.
    // No traversal, no absolute prefix — legitimately inside the fence.
    assert_accepted("fence-evil/secret.txt");
}

/// **PrefixCollision** — a result_rel with a `..` that tries to jump to the
/// sibling root.
///
/// `../fence-evil/secret.txt` — starts inside, then climbs out to the
/// sibling root. The `..` guard rejects this.
#[test]
fn prefix_collision_dotdot_to_sibling() {
    assert_escape_rejected("../fence-evil/secret.txt");
}

// ── Vector class 5: UnicodeEncodedDots ───────────────────────────────────────

/// **UnicodeEncodedDots** — full-width dot (U+FF0E, `．`).
///
/// A naive string-comparison guard might not recognise a Unicode full-width
/// period as a dot component. `normalize_path` splits on ASCII `/` only, so
/// `"．．/escape"` is a single non-`..` segment — it normalizes to
/// `Some(["．．", "escape"])` (inside the fence). The law is: the guard must
/// not be bypassed; full-width dots are treated as ordinary characters, not
/// as `..` equivalents. This test pins that the rule does NOT conflate
/// Unicode dots with ASCII `..`.
///
/// The POSITIVE variant: full-width dots are inside the fence (not an escape).
#[test]
fn unicode_fullwidth_dots_are_not_dotdot() {
    // U+FF0E FULLWIDTH FULL STOP — two of them look like `..` visually but
    // are a single distinct character. normalize_path must NOT treat this as
    // an escape. We drive this via accept (positive path).
    assert_accepted("\u{FF0E}\u{FF0E}/escape-attempt");
}

/// **UnicodeEncodedDots** — percent-encoded `%2e%2e` (URL encoding of `..`).
///
/// A path arriving via an HTTP layer might carry `%2e%2e` instead of `..`.
/// `normalize_path` splits on `/` and compares segments byte-for-byte to
/// `".."`. `%2e%2e` is NOT byte-equal to `..`, so it normalizes to
/// `Some(["%2e%2e", "escape"])` — inside the fence. This test pins that the
/// rule does not URL-decode inputs: the caller is responsible for
/// normalization before invoking the guard.
///
/// The POSITIVE variant: percent-encoded dots pass through as ordinary chars.
#[test]
fn percent_encoded_dotdot_is_not_dotdot() {
    // `%2e%2e/escape` — byte-level, NOT `..`. Inside the fence.
    assert_accepted("%2e%2e/escape");
}

/// **UnicodeEncodedDots** — null byte in the path.
///
/// A path with a null byte (`\0`) embedded is a single non-`..` non-`/`
/// segment. The guard treats it as a normal character. This pins that the
/// rule does not silently truncate at null (which could create a false
/// positive or false negative depending on downstream handling).
#[test]
fn null_byte_in_path_is_inside_fence() {
    // A null-byte segment: not `..`, not `/` — inside the fence.
    assert_accepted("path\x00with-null");
}

/// **UnicodeEncodedDots** — an actual `..` literal in a UTF-8 string.
///
/// A plain ASCII `..` — the real escape — must be rejected even in a Unicode-
/// aware context. Regression: confirm the ASCII guard still fires when the
/// surrounding string is otherwise UTF-8 valid.
#[test]
fn ascii_dotdot_in_unicode_context_is_rejected() {
    // Surrounding Unicode characters do not mask the ASCII `..` segment.
    assert_escape_rejected("café/../../../etc/passwd");
}

// ── Vector class 6: EmptyAndRootEdge ─────────────────────────────────────────

/// **EmptyAndRootEdge** — empty result_rel.
///
/// An empty `result_rel` collapses to the workspace root itself; delivering a
/// result to `<workspace_root>/` with no file name is meaningless, and the
/// broker rejects it (the `rel.is_empty()` check after `normalize_path`
/// succeeds with `Some([])`).
#[test]
fn empty_result_rel_is_rejected() {
    assert_escape_rejected("");
}

/// **EmptyAndRootEdge** — `"."` (current directory) collapses to root and is
/// ACCEPTED.
///
/// `.` normalizes to `Some([])` via `normalize_path` (no `..`, not absolute).
/// The broker's `trim_start_matches("./")` does NOT strip the bare `"."` (it
/// requires the trailing `/`), so `rel` remains `"."` (non-empty) and the
/// `rel.is_empty()` guard does not fire. The delivery proceeds to
/// `<workspace_root>/.`, which is inside the fence. This pins the exact
/// edge-case law: a bare `"."` is not an escape — it is an inside-fence
/// root reference (distinct from `""` and `"./"`, which ARE rejected).
#[test]
fn dot_only_result_rel_is_accepted() {
    // `"."` is normalized inside the fence: accepted, not rejected.
    assert_accepted(".");
}

/// **EmptyAndRootEdge** — `"./"` (current directory with trailing slash).
///
/// Same as `"."`: normalizes to `Some([])`. Rejected.
#[test]
fn dot_slash_result_rel_is_rejected() {
    assert_escape_rejected("./");
}

/// **EmptyAndRootEdge** — legitimate fence-relative path is accepted.
///
/// Positive counterpart: a non-empty, traversal-free, relative path is
/// inside the fence and must be accepted. Pins that the edge-case rejections
/// above do not over-fire and break valid paths.
#[test]
fn fence_relative_plain_path_is_accepted() {
    assert_accepted("out/signature.hex");
}

/// **EmptyAndRootEdge** — `.` and empty segments embedded in a valid path.
///
/// `./src/./main.rs` — the `.` and empty segments are stripped; the result
/// is `["src", "main.rs"]`, a valid fence-relative path. Accepted.
#[test]
fn dot_and_empty_segments_stripped_and_accepted() {
    assert_accepted("./src/./main.rs");
}

/// **EmptyAndRootEdge** — deeply nested valid path.
///
/// `a/b/c/d/e/f.txt` — no traversal, no absolute prefix. Accepted.
#[test]
fn deeply_nested_valid_path_is_accepted() {
    assert_accepted("a/b/c/d/e/f.txt");
}

// ── Vector class 7: CrossRepoSentinel ────────────────────────────────────────

/// **CrossRepoSentinel** — item ⑤ in `acceptance_c5b.rs` still names the
/// full harness's new home (corelink-runners).
///
/// When item ⑤ transferred to `corelink-runners` (WP-R4, 2026-06-10), a
/// comment block was left in `acceptance_c5b.rs` that explicitly names where
/// the active `RedTeamHarness` (all six escape vectors, including the
/// load-bearing `FenceMaterializedEscape`) now lives. This sentinel reads
/// that file and asserts the pointer is still present. If someone deletes
/// the comment without updating this test, the test fails — the transfer gap
/// stays disclosed, never silent.
///
/// What it pins: the string `"corelink-runners"` must appear in the item ⑤
/// comment block of `acceptance_c5b.rs`, confirming the pointer to the
/// harness's new home has not been silently removed.
#[test]
fn sentinel_item_5_comment_names_corelink_runners() {
    // Read acceptance_c5b.rs from the source tree. The path is relative to
    // the crate root (tests/ is a sibling of src/); we use CARGO_MANIFEST_DIR
    // to anchor the read correctly across worktree and bare-repo invocations.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let acceptance_path = std::path::Path::new(manifest_dir)
        .join("tests")
        .join("acceptance_c5b.rs");

    let source = std::fs::read_to_string(&acceptance_path).unwrap_or_else(|e| {
        panic!(
            "sentinel: could not read {}: {e}\n\
             This file must exist — it contains the item ⑤ pointer to the \
             full harness's new home (corelink-runners).",
            acceptance_path.display()
        )
    });

    // The item ⑤ comment block must reference corelink-runners: that is the
    // new home of the RedTeamHarness and all six escape vectors.
    assert!(
        source.contains("corelink-runners"),
        "sentinel FAILED: acceptance_c5b.rs no longer names 'corelink-runners' \
         in the item ⑤ comment block.\n\
         The pointer to the full escape red-team harness (WP-R4 transfer, all \
         six vectors incl. FenceMaterializedEscape) has been removed. \
         Either restore the pointer or update this sentinel to reflect the \
         new location of the harness. The gap must stay DISCLOSED, never silent."
    );

    // Additionally confirm the item ⑤ marker itself is still present (the
    // comment or line referencing the transferred item must not have been
    // stripped entirely).
    assert!(
        source.contains("TRANSFERRED") || source.contains("⑤"),
        "sentinel FAILED: acceptance_c5b.rs no longer contains the item ⑤ \
         transfer marker.\n\
         The comment that documents item ⑤ as TRANSFERRED to corelink-runners \
         has been removed. Restore it or update this sentinel."
    );
}
