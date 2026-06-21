//! WP-B2b acceptance oracle — runner-side execution + honesty.
//! Acceptance test for the runner-side execution contract (WP-B2b).
//!
//! Owned items (VERBATIM from decomposition v2.0 §2; B2b owns ③④⑤):
//!   ③ local≡runner BYTE-IDENTICAL (artifact/digest compare, NOT result-equal)
//!   ④ non-determinism flagged after 3 divergent runs, honest surface
//!   ⑤ npm fixture: partial hit-rate measured & displayed as-is, no full-memo
//!      claim
//!
//! HERMETIC by construction: every item is proven NOW against the real runner
//! surface — the `RunnerExecutor` trait (the runner analogue of B2a's
//! `CheckRunner`), an in-process reference executor that produces real artifacts
//! with canonical content digests, the byte-identity comparator, the
//! non-determinism tracker, and the hit-rate meter. The three memo axes + key
//! are derived through B2a's FROZEN memo-key surface (consumed, never re-done).
//!
//! The ONLY deferred piece is the LIVE runner BOX (`LiveBoxRunnerExecutor` over
//! `HUGIT_RUNNER_HOST`), a documented P2 seam. Its absence is asserted here so
//! the deferral cannot silently rot; when the env var IS present the live lane
//! runs (fail-not-skip) and the byte-identity comparison spans local↔real-box.

use hugit_checks::client::executor::{CheckRunner, ExecError, run_memoized};
use hugit_checks::client::memo_key::{self, FileContent, derive_memo_key};
use hugit_checks::runner::lease_exec::EntropySource;
use hugit_checks::runner::{
    ArtifactDiff, DeterminismState, HitRateMeter, InProcessRunnerExecutor, LiveBoxRunnerExecutor,
    NonDeterminismTracker, RunObservation, RunnerExecError, RunnerExecutor, compare_byte_identity,
    execute_on_lease,
};
use hugit_contracts::{CheckDef, CheckResult, RunnerLease, RunnerState};

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

const TOOLCHAIN: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// A representative deterministic check def.
fn def_deterministic() -> CheckDef {
    memo_key::with_canonical_def_digest(CheckDef {
        def_digest: String::new(),
        command: "cargo build".to_string(),
        inputs: vec!["src/**/*.rs".to_string()],
        toolchain_ref: "rust-1.86".to_string(),
        env_manifest: "blob:env".to_string(),
        glob_set: vec!["src/**/*.rs".to_string()],
    })
}

fn tree() -> Vec<(String, FileContent)> {
    vec![
        ("src/lib.rs".to_string(), b"fn a() {}".to_vec()),
        ("src/main.rs".to_string(), b"fn main() {}".to_vec()),
    ]
}

fn as_refs(t: &[(String, FileContent)]) -> Vec<(&str, &FileContent)> {
    t.iter().map(|(p, c)| (p.as_str(), c)).collect()
}

/// A held lease that permits execution.
fn held_lease() -> RunnerLease {
    RunnerLease {
        lease_id: "b2b-lease".to_string(),
        principal_chain: vec!["agent:acceptance".to_string()],
        path_set: vec!["src/".to_string()],
        expiry: u64::MAX,
        net_policy: "none".to_string(),
        tmp_root: "/hugit/tmp".to_string(),
        state: RunnerState::Held,
    }
}

/// Derive the three axes + key for a def/tree/toolchain via B2a's frozen surface.
fn axes(def: &CheckDef, t: &[(String, FileContent)]) -> (String, String, String) {
    let key = derive_memo_key(def, as_refs(t), TOOLCHAIN);
    let tree_root = memo_key::scoped_tree_root(&def.glob_set, as_refs(t));
    let def_digest = memo_key::compute_def_digest(def);
    (key, tree_root, def_digest)
}

/// An entropy source that yields a fresh, distinct value on every sample — models
/// a non-hermetic check (e.g. one stamping a timestamp into its output).
struct DivergentEntropy {
    counter: std::cell::Cell<u64>,
}
impl DivergentEntropy {
    fn new() -> Self {
        Self {
            counter: std::cell::Cell::new(0),
        }
    }
}
impl EntropySource for DivergentEntropy {
    fn sample(&self) -> Vec<u8> {
        let n = self.counter.get();
        self.counter.set(n + 1);
        n.to_be_bytes().to_vec()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ③ local≡runner BYTE-IDENTICAL (artifact/digest compare, not result-equal)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn item_3_local_runner_byte_identical_artifact_digest_compare() {
    let def = def_deterministic();
    let t = tree();
    let (key, tree_root, def_digest) = axes(&def, &t);
    let lease = held_lease();

    // The SAME pure check function, executed on two independent runner surfaces
    // (modelling "local" and "forge runner"). Both are deterministic.
    let local_exec = InProcessRunnerExecutor::new("runner:local");
    let runner_exec = InProcessRunnerExecutor::new("runner:forge");

    let local = execute_on_lease(
        &local_exec,
        &lease,
        &def,
        &key,
        &tree_root,
        &def_digest,
        TOOLCHAIN,
    )
    .expect("local execution");
    let forge = execute_on_lease(
        &runner_exec,
        &lease,
        &def,
        &key,
        &tree_root,
        &def_digest,
        TOOLCHAIN,
    )
    .expect("runner execution");

    // The comparison is ARTIFACT/DIGEST, not result-equal: at least one artifact
    // is produced and compared on both sides.
    let report = compare_byte_identity(&local, &forge);
    assert!(
        report.artifacts_compared >= 1,
        "byte-identity must compare ≥1 artifact (was {}), not just exit codes",
        report.artifacts_compared
    );
    assert!(
        report.is_identical(),
        "deterministic check must be byte-identical local↔runner; diffs: {:?}",
        report.diffs
    );

    // Cross-check: byte-identity is a STRICTER property than result-equality.
    // Tamper ONE artifact's content digest on the runner side — exits still
    // match (result-equal would PASS) but byte-identity MUST catch it.
    let mut tampered = forge.clone();
    tampered.artifacts[0].digest = "deadbeef".repeat(8);
    assert_eq!(
        local.exit, tampered.exit,
        "tampered case is result-EQUAL (same exit) — proving result-equal is too weak"
    );
    let tampered_report = compare_byte_identity(&local, &tampered);
    assert!(
        !tampered_report.is_identical(),
        "a differing artifact digest MUST be flagged (byte-identity, not result-equal)"
    );
    assert!(
        tampered_report
            .diffs
            .iter()
            .any(|d| matches!(d, ArtifactDiff::DigestMismatch { .. })),
        "the divergence must be a DigestMismatch; got {:?}",
        tampered_report.diffs
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ④ non-determinism flagged after 3 divergent runs, honest surface
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn item_4_non_determinism_flagged_after_3_divergent_runs() {
    let def = def_deterministic();
    let t = tree();
    let (key, tree_root, def_digest) = axes(&def, &t);
    let lease = held_lease();

    // A non-hermetic check: same key, but each run produces DIFFERENT artifacts.
    let executor = InProcessRunnerExecutor::with_entropy("runner:flaky", DivergentEntropy::new());
    let mut tracker = NonDeterminismTracker::new();

    // Run 1: one fingerprint observed → deterministic so far (no divergence yet).
    let r1 = execute_on_lease(
        &executor,
        &lease,
        &def,
        &key,
        &tree_root,
        &def_digest,
        TOOLCHAIN,
    )
    .unwrap();
    let s1 = tracker.record(RunObservation {
        memo_key: &key,
        result: &r1,
    });
    assert!(
        matches!(s1, DeterminismState::Deterministic { .. }),
        "after 1 run there is no divergence yet; got {s1:?}"
    );

    // Run 2: a second distinct fingerprint → diverging, but NOT yet flagged
    // (honest interim state — never prematurely flagged, never a clean hit).
    let r2 = execute_on_lease(
        &executor,
        &lease,
        &def,
        &key,
        &tree_root,
        &def_digest,
        TOOLCHAIN,
    )
    .unwrap();
    let s2 = tracker.record(RunObservation {
        memo_key: &key,
        result: &r2,
    });
    assert!(
        matches!(s2, DeterminismState::Diverging { .. }),
        "after 2 divergent runs: diverging, not yet flagged; got {s2:?}"
    );
    assert!(
        !s2.is_non_deterministic(),
        "MUST NOT flag before the 3-run threshold"
    );

    // Run 3: a third distinct fingerprint → FLAGGED non-deterministic.
    let r3 = execute_on_lease(
        &executor,
        &lease,
        &def,
        &key,
        &tree_root,
        &def_digest,
        TOOLCHAIN,
    )
    .unwrap();
    let s3 = tracker.record(RunObservation {
        memo_key: &key,
        result: &r3,
    });
    assert!(
        s3.is_non_deterministic(),
        "after 3 divergent runs the check MUST be flagged non-deterministic; got {s3:?}"
    );
    if let DeterminismState::NonDeterministic {
        runs,
        distinct_fingerprints,
    } = s3
    {
        assert_eq!(runs, 3, "exactly 3 runs observed");
        assert_eq!(distinct_fingerprints, 3, "3 distinct artifact fingerprints");
    }

    // Honest surface: a flagged key is NEVER a clean memoized hit.
    assert!(
        tracker.state(&key).unwrap().is_non_deterministic(),
        "the flag persists on the honest surface"
    );

    // Negative control: a DETERMINISTIC check run 3× stays clean — the flag is
    // load-bearing, not a counter that trips on any 3 runs.
    let det_exec = InProcessRunnerExecutor::new("runner:hermetic");
    let mut det_tracker = NonDeterminismTracker::new();
    let mut last = None;
    for _ in 0..3 {
        let r = execute_on_lease(
            &det_exec,
            &lease,
            &def,
            &key,
            &tree_root,
            &def_digest,
            TOOLCHAIN,
        )
        .unwrap();
        last = Some(det_tracker.record(RunObservation {
            memo_key: &key,
            result: &r,
        }));
    }
    assert!(
        !last.unwrap().is_non_deterministic(),
        "a deterministic check run 3× must NOT be flagged (no false non-determinism)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑤ npm fixture: partial hit-rate measured & displayed as-is, no full-memo claim
// ─────────────────────────────────────────────────────────────────────────────

/// A small npm-shaped fixture: a sequence of checks, some content-stable across
/// runs (memoizable → AC HIT on repeat) and some non-hermetic (a fresh tree each
/// run → always a MISS). This is the representative PARTIAL case: an `npm`
/// workload cannot be fully memoized.
fn npm_checks() -> Vec<(CheckDef, Vec<(String, FileContent)>)> {
    let stable = |name: &str, content: &[u8]| -> (CheckDef, Vec<(String, FileContent)>) {
        let def = memo_key::with_canonical_def_digest(CheckDef {
            def_digest: String::new(),
            command: format!("npm run {name}"),
            inputs: vec!["**/*.js".to_string()],
            toolchain_ref: "node-20".to_string(),
            env_manifest: "blob:npm-env".to_string(),
            glob_set: vec!["**/*.js".to_string()],
        });
        (def, vec![(format!("{name}.js"), content.to_vec())])
    };
    vec![
        stable("lint", b"// lint config v1"),
        stable("typecheck", b"// tsconfig v1"),
        stable("test", b"// test suite v1"),
    ]
}

#[test]
fn item_5_npm_fixture_partial_hit_rate_measured_as_is_no_full_memo_claim() {
    use hugit_checks::client::ac::{ActionCache, InMemoryAc};

    let ac = InMemoryAc::new();
    let runner = NpmCountingRunner::default();
    let checks = npm_checks();
    let mut meter = HitRateMeter::new();

    // Pass 1 (cold): every check is a MISS — nothing memoized yet.
    for (def, t) in &checks {
        let key = derive_memo_key(def, as_refs(t), TOOLCHAIN);
        let pre = ac.lookup(&key).unwrap();
        meter.observe_lookup(&pre);
        run_memoized(&ac, &runner, def, as_refs(t), TOOLCHAIN).unwrap();
    }

    // Pass 2 (warm): the content-STABLE checks now HIT; a NON-HERMETIC check (a
    // build whose input tree changed run-to-run — the npm reality) still MISSes.
    // Re-run the 3 stable checks (HIT) + introduce 2 non-hermetic checks (MISS).
    for (def, t) in &checks {
        let key = derive_memo_key(def, as_refs(t), TOOLCHAIN);
        let pre = ac.lookup(&key).unwrap();
        meter.observe_lookup(&pre);
        run_memoized(&ac, &runner, def, as_refs(t), TOOLCHAIN).unwrap();
    }
    for i in 0..2u8 {
        // A non-hermetic check: its input tree differs every invocation (e.g. an
        // npm install reaching the network), so it can never be a cached hit.
        let def = memo_key::with_canonical_def_digest(CheckDef {
            def_digest: String::new(),
            command: "npm install".to_string(),
            inputs: vec!["package.json".to_string()],
            toolchain_ref: "node-20".to_string(),
            env_manifest: "blob:npm-env".to_string(),
            glob_set: vec!["package.json".to_string()],
        });
        let content = format!("{{\"fetched\": {i}}}");
        let t = vec![("package.json".to_string(), content.into_bytes())];
        let key = derive_memo_key(&def, as_refs(&t), TOOLCHAIN);
        let pre = ac.lookup(&key).unwrap();
        meter.observe_lookup(&pre);
        run_memoized(&ac, &runner, &def, as_refs(&t), TOOLCHAIN).unwrap();
    }

    let report = meter.report();

    // The rate is MEASURED, not asserted to a target. We measured: 3 cold misses
    // + (3 warm hits + 2 non-hermetic misses) = 8 lookups, 3 hits.
    assert_eq!(report.total, 8, "8 lookups measured");
    assert_eq!(report.hits, 3, "exactly the 3 content-stable checks HIT");
    assert_eq!(report.misses, 5, "cold + non-hermetic checks MISS");

    // Honest shape: PARTIAL — some work memoized, some not. NEVER a full-memo
    // claim for the npm ecosystem.
    assert!(
        report.is_partial(),
        "npm hit-rate must be PARTIAL (0<rate<1); report: {}",
        report.display_line()
    );
    assert!(
        !report.is_full_memo(),
        "MUST NOT claim full memoization for npm; report: {}",
        report.display_line()
    );
    assert!(
        report.rate() > 0.0 && report.rate() < 1.0,
        "measured rate must be strictly partial, was {}",
        report.rate()
    );

    // Displayed AS-IS: the line carries the measured figure and the PARTIAL label.
    let line = report.display_line();
    assert!(
        line.contains("PARTIAL") && line.contains("37.5%"),
        "display must show the measured partial rate as-is; got {line:?}"
    );

    // The meter NEVER fabricates a hit: an empty meter reports 0%, no full claim.
    let empty = HitRateMeter::new().report();
    assert_eq!(empty.rate(), 0.0);
    assert!(!empty.is_full_memo());
}

/// A runner for the npm fixture that produces a content-defined artifact (so the
/// memoized record is stable) and counts executions for evidence.
#[derive(Default)]
struct NpmCountingRunner {
    runs: std::sync::atomic::AtomicU32,
}

impl CheckRunner for NpmCountingRunner {
    fn run(
        &self,
        _def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, ExecError> {
        self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(CheckResult {
            memo_key: memo_key.to_string(),
            tree_hash: tree_root.to_string(),
            def_digest: def_digest.to_string(),
            toolchain_digest: toolchain_digest.to_string(),
            exit: 0,
            artifacts: vec![],
            stdout_ref: "blob:stdout".to_string(),
            stderr_ref: "blob:stderr".to_string(),
            duration_ms: 0,
            runner_ref: "runner:npm".to_string(),
            produced_at: 0,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fail-closed: execution requires a HELD lease
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn execution_requires_a_held_lease_fail_closed() {
    let def = def_deterministic();
    let t = tree();
    let (key, tree_root, def_digest) = axes(&def, &t);
    let executor = InProcessRunnerExecutor::new("runner:x");

    for bad in [
        RunnerState::Expired,
        RunnerState::Released,
        RunnerState::Crashed,
    ] {
        let mut lease = held_lease();
        lease.state = bad.clone();
        let err = execute_on_lease(
            &executor,
            &lease,
            &def,
            &key,
            &tree_root,
            &def_digest,
            TOOLCHAIN,
        )
        .expect_err("a non-held lease must refuse execution");
        assert_eq!(err, RunnerExecError::LeaseNotHeld(bad));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// P2 SEAM: the live runner box. Asserted absent in the bare gate (cannot rot);
// run-not-skip when HUGIT_RUNNER_HOST is present.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn live_runner_box_seam_documented_and_gated() {
    let def = def_deterministic();
    let t = tree();
    let (key, tree_root, def_digest) = axes(&def, &t);
    let lease = held_lease();

    match std::env::var("HUGIT_RUNNER_HOST") {
        Err(_) => {
            // No box: the live seam MUST be explicitly NotWired (the deferral is
            // self-documenting and cannot silently rot to green).
            let live = LiveBoxRunnerExecutor::new("runner.example");
            let err = live
                .execute(&lease, &def, &key, &tree_root, &def_digest, TOOLCHAIN)
                .expect_err("the P2 box seam must be NotWired without a box");
            assert!(
                matches!(err, RunnerExecError::BoxNotWired(_)),
                "deferral must surface as BoxNotWired; got {err:?}"
            );
        }
        Ok(host) => {
            // Box present: run the live lane (fail-not-skip). The live executor
            // is the documented P2 seam; until its transport is wired this FAILS
            // loudly rather than passing silently — exactly the contract for
            // box-dependent assertions.
            let live = LiveBoxRunnerExecutor::new(&host);
            let real = execute_on_lease(
                &live,
                &lease,
                &def,
                &key,
                &tree_root,
                &def_digest,
                TOOLCHAIN,
            )
            .expect("live runner box execution must succeed when HUGIT_RUNNER_HOST is set");

            // local↔real-box byte-identity (③ spanning the live seam).
            let local_exec = InProcessRunnerExecutor::new("runner:local");
            let local = execute_on_lease(
                &local_exec,
                &lease,
                &def,
                &key,
                &tree_root,
                &def_digest,
                TOOLCHAIN,
            )
            .expect("local execution");
            let report = compare_byte_identity(&local, &real);
            assert!(
                report.is_identical(),
                "local≡real-box must be byte-identical; diffs: {:?}",
                report.diffs
            );
        }
    }
}
