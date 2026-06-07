//! WP-B2a acceptance oracle — checks-as-code client + local executor +
//! three-axis memo key. Contract: `docs/plan/wp-contracts/WP-B2a.md`.
//!
//! Owned items (VERBATIM from decomposition v2.0 §2; B2a owns ①②⑥⑦):
//!   ① repeat tree+def→AC hit, 0 exec, <500ms
//!   ② glob sensitivity (in→rerun, out→hit)
//!   ⑥ (R2) toolchain sensitivity: different toolchain → MISS, never false hit
//!   ⑦ (R3) def sensitivity: changed check definition, same tree+toolchain →
//!      MISS + re-execute (3rd key axis)
//!
//! HERMETIC by construction: every item is proven against an in-process
//! [`InMemoryAc`] implementing the SAME [`ActionCache`] trait the live HTTP
//! client uses, plus a counting [`CountingRunner`] that proves the
//! zero-local-execution-on-hit wedge. The ONLY deferred piece is the live HTTP
//! transport (`HttpAcClient`), a documented P2 seam — its absence is asserted
//! here too so the deferral cannot silently rot.
//!
//! Memo-key sensitivity is proven as a SECURITY property: changing ANY axis
//! changes the key; a false hit (different inputs, same key) is impossible. The
//! key is single-sourced via `hugit_refstore::compute_memo_key` and pinned
//! cross-crate by `hugit-refstore/tests/canonical_format_pin.rs`, so neither
//! suite can launder a formula change past the other.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use hugit_checks::client::ac::{AcError, ActionCache, HttpAcClient, InMemoryAc};
use hugit_checks::client::executor::{CheckRunner, ExecError, run_memoized};
use hugit_checks::client::memo_key::{
    self, FileContent, compute_def_digest, derive_memo_key, scoped_tree_root,
};
use hugit_checks::client::parser::{ParseError, parse_check_def};
use hugit_contracts::{CheckDef, CheckResult};

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

const TOOLCHAIN_A: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TOOLCHAIN_B: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// A representative check def: lint the Rust sources under `crates/`.
fn def_a() -> CheckDef {
    memo_key::with_canonical_def_digest(CheckDef {
        def_digest: String::new(),
        command: "cargo clippy -- -D warnings".to_string(),
        inputs: vec!["crates/**/*.rs".to_string()],
        toolchain_ref: "rust-1.86".to_string(),
        env_manifest: "blob:env-a".to_string(),
        glob_set: vec!["crates/**/*.rs".to_string()],
    })
}

/// A workspace tree: paths → content. Two source files inside the glob and one
/// doc file OUTSIDE it.
fn tree_base() -> Vec<(String, FileContent)> {
    vec![
        ("crates/a/src/lib.rs".to_string(), b"fn a() {}".to_vec()),
        ("crates/b/src/lib.rs".to_string(), b"fn b() {}".to_vec()),
        ("docs/readme.md".to_string(), b"# hugit".to_vec()),
    ]
}

fn as_refs(tree: &[(String, FileContent)]) -> Vec<(&str, &FileContent)> {
    tree.iter().map(|(p, c)| (p.as_str(), c)).collect()
}

/// A counting runner: records how many times a check was ACTUALLY executed, so
/// the zero-execution-on-hit wedge is observable, not asserted on faith.
struct CountingRunner {
    runs: AtomicU32,
    /// Exit code the synthesized result reports.
    exit: i32,
}

impl CountingRunner {
    fn new() -> Self {
        Self {
            runs: AtomicU32::new(0),
            exit: 0,
        }
    }
    fn run_count(&self) -> u32 {
        self.runs.load(Ordering::SeqCst)
    }
}

impl CheckRunner for CountingRunner {
    fn run(
        &self,
        _def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, ExecError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(CheckResult {
            memo_key: memo_key.to_string(),
            tree_hash: tree_root.to_string(),
            def_digest: def_digest.to_string(),
            toolchain_digest: toolchain_digest.to_string(),
            exit: self.exit,
            artifacts: vec![],
            stdout_ref: "blob:stdout".to_string(),
            stderr_ref: "blob:stderr".to_string(),
            duration_ms: 1234,
            runner_ref: "runner:local".to_string(),
            produced_at: 1_717_000_000_000,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ① repeat tree+def→AC hit, 0 exec, <500ms
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn item_1_repeat_tree_def_ac_hit_zero_exec_under_500ms() {
    let ac = InMemoryAc::new();
    let runner = CountingRunner::new();
    let def = def_a();
    let tree = tree_base();

    // First run: MISS → executes exactly once → stores.
    let first = run_memoized(&ac, &runner, &def, as_refs(&tree), TOOLCHAIN_A).expect("first run");
    assert!(!first.from_cache, "first run must be a MISS");
    assert_eq!(first.local_executions, 1, "miss executes exactly once");
    assert_eq!(runner.run_count(), 1, "runner invoked exactly once on miss");
    assert_eq!(ac.len(), 1, "miss stores the result");

    // Repeat with the SAME tree + def + toolchain: HIT, ZERO local executions.
    let start = Instant::now();
    let second = run_memoized(&ac, &runner, &def, as_refs(&tree), TOOLCHAIN_A).expect("second run");
    let elapsed = start.elapsed();

    assert!(second.from_cache, "repeat must be an AC HIT");
    assert_eq!(
        second.local_executions, 0,
        "AC hit must perform ZERO local executions (the wedge)"
    );
    assert_eq!(
        runner.run_count(),
        1,
        "runner must NOT be invoked again on a hit (still 1, structural proof)"
    );
    assert!(
        elapsed.as_millis() < 500,
        "AC hit must return in <500ms (took {}ms)",
        elapsed.as_millis()
    );

    // The memoized result is byte-identical to what was stored.
    assert_eq!(first.result, second.result, "hit returns the stored record");
    assert_eq!(second.result.memo_key, first.result.memo_key);
}

// ─────────────────────────────────────────────────────────────────────────────
// ② glob sensitivity (in→rerun, out→hit)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn item_2_glob_sensitivity_in_reruns_out_hits() {
    let ac = InMemoryAc::new();
    let runner = CountingRunner::new();
    let def = def_a(); // glob_set = ["crates/**/*.rs"]
    let tree = tree_base();

    // Prime the cache.
    let primed = run_memoized(&ac, &runner, &def, as_refs(&tree), TOOLCHAIN_A).unwrap();
    assert!(!primed.from_cache);
    assert_eq!(runner.run_count(), 1);

    // Edit OUTSIDE the glob (the doc file) → key unchanged → HIT, 0 exec.
    let mut tree_out = tree.clone();
    tree_out[2].1 = b"# hugit (edited docs)".to_vec();
    let out = run_memoized(&ac, &runner, &def, as_refs(&tree_out), TOOLCHAIN_A).unwrap();
    assert!(out.from_cache, "edit OUTSIDE glob must HIT");
    assert_eq!(out.local_executions, 0, "out-of-glob edit: 0 executions");
    assert_eq!(
        runner.run_count(),
        1,
        "out-of-glob edit must not re-execute"
    );

    // Edit INSIDE the glob (a source file) → key changes → MISS → rerun.
    let mut tree_in = tree.clone();
    tree_in[0].1 = b"fn a() { /* changed */ }".to_vec();
    let inside = run_memoized(&ac, &runner, &def, as_refs(&tree_in), TOOLCHAIN_A).unwrap();
    assert!(!inside.from_cache, "edit INSIDE glob must MISS");
    assert_eq!(inside.local_executions, 1, "in-glob edit re-executes");
    assert_eq!(runner.run_count(), 2, "in-glob edit must re-execute");

    // Direct key-level proof (no executor): the tree_root differs only for the
    // in-glob edit, never the out-of-glob edit.
    let root_base = scoped_tree_root(&def.glob_set, as_refs(&tree));
    let root_out = scoped_tree_root(&def.glob_set, as_refs(&tree_out));
    let root_in = scoped_tree_root(&def.glob_set, as_refs(&tree_in));
    assert_eq!(
        root_base, root_out,
        "out-of-glob edit must not change tree_root"
    );
    assert_ne!(root_base, root_in, "in-glob edit MUST change tree_root");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑥ (R2) toolchain sensitivity: different toolchain → MISS, never false hit
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn item_6_toolchain_sensitivity_different_toolchain_misses_never_false_hit() {
    let ac = InMemoryAc::new();
    let runner = CountingRunner::new();
    let def = def_a();
    let tree = tree_base();

    // Prime under toolchain A.
    let primed = run_memoized(&ac, &runner, &def, as_refs(&tree), TOOLCHAIN_A).unwrap();
    assert!(!primed.from_cache);
    assert_eq!(runner.run_count(), 1);

    // SAME tree + def, DIFFERENT toolchain → MUST miss + re-execute.
    let other = run_memoized(&ac, &runner, &def, as_refs(&tree), TOOLCHAIN_B).unwrap();
    assert!(
        !other.from_cache,
        "a different toolchain must MISS — never a false hit (correctness fault)"
    );
    assert_eq!(other.local_executions, 1, "different toolchain re-executes");
    assert_eq!(runner.run_count(), 2);

    // Key-level proof: only the toolchain axis changed, key MUST differ.
    let key_a = derive_memo_key(&def, as_refs(&tree), TOOLCHAIN_A);
    let key_b = derive_memo_key(&def, as_refs(&tree), TOOLCHAIN_B);
    assert_ne!(
        key_a, key_b,
        "different toolchain_digest MUST yield a different memo key"
    );

    // Back under toolchain A: the original entry still hits (no clobber).
    let back = run_memoized(&ac, &runner, &def, as_refs(&tree), TOOLCHAIN_A).unwrap();
    assert!(back.from_cache, "toolchain A entry must still hit");
    assert_eq!(back.local_executions, 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑦ (R3) def sensitivity: changed definition, same tree+toolchain → MISS + rerun
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn item_7_def_sensitivity_changed_def_same_tree_toolchain_misses_and_reexecutes() {
    let ac = InMemoryAc::new();
    let runner = CountingRunner::new();
    let def = def_a();
    let tree = tree_base();

    // Prime with def A.
    let primed = run_memoized(&ac, &runner, &def, as_refs(&tree), TOOLCHAIN_A).unwrap();
    assert!(!primed.from_cache);
    assert_eq!(runner.run_count(), 1);

    // Change ONLY the definition (the command), keep tree + toolchain identical.
    let def2 = memo_key::with_canonical_def_digest(CheckDef {
        command: "cargo clippy -- -D warnings -W clippy::pedantic".to_string(),
        ..def_a()
    });
    assert_ne!(
        def.def_digest, def2.def_digest,
        "a changed definition body MUST yield a different def_digest"
    );

    let changed = run_memoized(&ac, &runner, &def2, as_refs(&tree), TOOLCHAIN_A).unwrap();
    assert!(
        !changed.from_cache,
        "changed definition (same tree+toolchain) MUST MISS (3rd key axis)"
    );
    assert_eq!(changed.local_executions, 1, "changed def re-executes");
    assert_eq!(runner.run_count(), 2);

    // Key-level proof: only the def axis changed, key MUST differ.
    let key1 = derive_memo_key(&def, as_refs(&tree), TOOLCHAIN_A);
    let key2 = derive_memo_key(&def2, as_refs(&tree), TOOLCHAIN_A);
    assert_ne!(
        key1, key2,
        "changed def_digest MUST yield a different memo key"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Cross-axis NO-FALSE-HIT property: each axis is necessary AND sufficient.
// A single function changing each axis independently proves all keys distinct —
// the security invariant that underpins ②⑥⑦ and the safety of ①'s hit path.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn three_axes_each_necessary_no_false_hit_across_any_axis() {
    let def = def_a();
    let tree = tree_base();
    let base = derive_memo_key(&def, as_refs(&tree), TOOLCHAIN_A);

    // Axis 1 (tree): change an in-glob file.
    let mut tree2 = tree.clone();
    tree2[0].1 = b"fn a() { changed }".to_vec();
    let k_tree = derive_memo_key(&def, as_refs(&tree2), TOOLCHAIN_A);

    // Axis 2 (def): change the command.
    let def2 = memo_key::with_canonical_def_digest(CheckDef {
        command: "different command".to_string(),
        ..def_a()
    });
    let k_def = derive_memo_key(&def2, as_refs(&tree), TOOLCHAIN_A);

    // Axis 3 (toolchain): change the toolchain digest.
    let k_tc = derive_memo_key(&def, as_refs(&tree), TOOLCHAIN_B);

    let keys = [&base, &k_tree, &k_def, &k_tc];
    for (i, a) in keys.iter().enumerate() {
        for (j, b) in keys.iter().enumerate() {
            if i != j {
                assert_ne!(
                    a, b,
                    "axis collision: keys {i} and {j} matched (false-hit vector)"
                );
            }
        }
    }

    // Identical inputs MUST yield the identical key (determinism — the hit side).
    let again = derive_memo_key(&def, as_refs(&tree), TOOLCHAIN_A);
    assert_eq!(
        base, again,
        "identical inputs must yield identical key (hit path)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// def_digest covers every load-bearing definition field (no silent collisions).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn def_digest_covers_all_load_bearing_fields() {
    let base = def_a();
    let baseline = compute_def_digest(&base);

    let variants: Vec<(&str, CheckDef)> = vec![
        (
            "command",
            CheckDef {
                command: "x".into(),
                ..def_a()
            },
        ),
        (
            "inputs",
            CheckDef {
                inputs: vec!["other/**".into()],
                ..def_a()
            },
        ),
        (
            "toolchain_ref",
            CheckDef {
                toolchain_ref: "rust-9.99".into(),
                ..def_a()
            },
        ),
        (
            "env_manifest",
            CheckDef {
                env_manifest: "blob:env-z".into(),
                ..def_a()
            },
        ),
        (
            "glob_set",
            CheckDef {
                glob_set: vec!["src/**".into()],
                ..def_a()
            },
        ),
    ];
    for (field, v) in variants {
        assert_ne!(
            baseline,
            compute_def_digest(&v),
            "changing `{field}` MUST change def_digest"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parser: fail-closed validation — empty fields and forged digests are rejected.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn parser_normalizes_and_rejects_forged_digest() {
    // A well-formed authored def (def_digest empty → parser fills it canonically).
    let json = serde_json::to_string(&CheckDef {
        def_digest: String::new(),
        command: "cargo test".into(),
        inputs: vec!["crates/**/*.rs".into()],
        toolchain_ref: "rust-1.86".into(),
        env_manifest: "blob:env".into(),
        glob_set: vec!["crates/**/*.rs".into()],
    })
    .unwrap();
    let parsed = parse_check_def(&json).expect("valid def parses");
    assert_eq!(parsed.def_digest, compute_def_digest(&parsed));

    // A FORGED self-reported digest (≠ body) must be rejected — never silently
    // corrected — so it cannot smuggle a different body past the memo key.
    let forged = serde_json::to_string(&CheckDef {
        def_digest: "deadbeef".repeat(8),
        command: "cargo test".into(),
        inputs: vec!["crates/**/*.rs".into()],
        toolchain_ref: "rust-1.86".into(),
        env_manifest: "blob:env".into(),
        glob_set: vec!["crates/**/*.rs".into()],
    })
    .unwrap();
    assert!(matches!(
        parse_check_def(&forged),
        Err(ParseError::DigestMismatch { .. })
    ));

    // Empty command is rejected (a check that cannot execute cannot be memoized).
    let empty_cmd = serde_json::to_string(&CheckDef {
        def_digest: String::new(),
        command: "  ".into(),
        inputs: vec![],
        toolchain_ref: "rust-1.86".into(),
        env_manifest: "blob:env".into(),
        glob_set: vec![],
    })
    .unwrap();
    assert!(matches!(
        parse_check_def(&empty_cmd),
        Err(ParseError::EmptyField("command"))
    ));

    // Unknown fields are rejected (deny_unknown_fields on the frozen type).
    assert!(matches!(
        parse_check_def(r#"{"command":"x","extra":1}"#),
        Err(ParseError::Json(_))
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// The in-memory AC and the live HTTP client share the SAME trait — so the
// client logic is genuinely exercised, and the live transport is a documented
// P2 seam whose absence is asserted (the deferral cannot silently rot to green).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ac_trait_store_then_hit_roundtrip_in_memory() {
    let ac = InMemoryAc::new();
    assert!(ac.is_empty());

    let runner = CountingRunner::new();
    let key = derive_memo_key(&def_a(), as_refs(&tree_base()), TOOLCHAIN_A);
    let result = runner
        .run(&def_a(), &key, "tree", "def", TOOLCHAIN_A)
        .unwrap();

    assert_eq!(ac.lookup(&key).unwrap(), None, "cold lookup is a MISS");
    ac.store(&result).unwrap();
    assert_eq!(
        ac.lookup(&result.memo_key).unwrap().as_ref(),
        Some(&result),
        "store-then-lookup is a HIT returning the stored record"
    );
    assert_eq!(ac.len(), 1);
}

#[test]
fn http_ac_client_seam_is_explicitly_deferred_not_silently_passing() {
    // The live HTTP client implements the SAME trait but its transport is the
    // P2 seam: both verbs return NotWired carrying the endpoint they WILL call.
    // This proves the deferral is honest — a constant-true fake would fail here.
    let client = HttpAcClient::new("https://ac.corelink.dev/");
    let key = "a".repeat(64);

    match client.lookup(&key) {
        Err(AcError::NotWired(ep)) => assert!(ep.contains("/v1/ac/") && ep.starts_with("GET ")),
        other => panic!("HTTP lookup must be a documented P2 seam, got {other:?}"),
    }

    let runner = CountingRunner::new();
    let result = runner.run(&def_a(), &key, "t", "d", TOOLCHAIN_A).unwrap();
    match client.store(&result) {
        Err(AcError::NotWired(ep)) => assert!(ep.contains("/v1/ac/") && ep.starts_with("PUT ")),
        other => panic!("HTTP store must be a documented P2 seam, got {other:?}"),
    }
}
