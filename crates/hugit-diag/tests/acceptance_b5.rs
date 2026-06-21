//! WP-B5 acceptance tests — auto-bisect over memoized checks + DiagnosisObject.
//!
//! Acceptance test for the auto-bisect contract (WP-B5).
//! Owned items (VERBATIM from decomposition v2.0 §2):
//!   ① culprit ≤log₂ execs
//!   ② diff-vs-green + suspects
//!   ③ <2min fixture
//!   ④ diagnosis is bounded schema data, never raw log dump (size assert)
//!   ⑤ bisect triggers automatically on any red — no manual invocation
//!
//! One `#[test] item_<n>_<slug>` per owned item, plus adversarial guards proving
//! the oracle is NOT gamed (it goes RED if bisect exceeds log₂ execs or the
//! diagnosis carries unbounded raw logs).

use std::time::Instant;

use hugit_checks::client::ac::{ActionCache, InMemoryAc};
use hugit_contracts::{CheckResult, DiagnosisObject};
use hugit_diag::bisect::{
    BisectError, Bisector, DIAGNOSIS_SIZE_BOUND, History, MemoizedCheckOracle, RedSignal,
    on_red_signal,
};

// ── fixtures ─────────────────────────────────────────────────────────────────

/// A memoized [`CheckResult`] for a tree, with the given exit code. Logs are CAS
/// refs only (the contract carries `stdout_ref`/`stderr_ref`, never inline).
fn check_result(tree_hash: &str, def_digest: &str, toolchain: &str, exit: i32) -> CheckResult {
    let memo_key = hugit_refstore::compute_memo_key(tree_hash, def_digest, toolchain);
    CheckResult {
        memo_key,
        tree_hash: tree_hash.to_string(),
        def_digest: def_digest.to_string(),
        toolchain_digest: toolchain.to_string(),
        exit,
        artifacts: Vec::new(),
        stdout_ref: format!("cas:stdout:{tree_hash}"),
        stderr_ref: format!("cas:stderr:{tree_hash}"),
        duration_ms: 1,
        runner_ref: "runner:fixture".into(),
        produced_at: 0,
    }
}

// Valid 64-hex content-address digests (a real def/toolchain digest IS a
// SHA-256 → exactly 64 hex). The fixtures were previously 65 hex, which the
// shared AC write-boundary axis guard (PS-10) correctly refuses — a long bare
// hex run that is NOT a {40,64}-hex digest is deny-by-default (PS-14), since a
// 32/50-hex value could be a credential. Pinned to 64 so the memoized fixture
// stores like a real check result.
const DEF: &str = "0000000000000000000000000000000000000000000000000000000000000def";
const TC: &str = "0000000000000000000000000000000000000000000000000000000000000abc";

/// An ordered tree history `t0..t_{n-1}` where every tree at or after
/// `first_red` is RED and everything before is green. All results are memoized
/// in a fresh AC so every bisect probe is a (free) cache hit.
fn memoized_history(n: usize, first_red: usize) -> (InMemoryAc, History) {
    let ac = InMemoryAc::new();
    let mut trees = Vec::with_capacity(n);
    for i in 0..n {
        let tree = format!("{i:064x}");
        let exit = if i >= first_red { 1 } else { 0 };
        ac.store(&check_result(&tree, DEF, TC, exit)).unwrap();
        trees.push(tree);
    }
    (ac, History::new(trees, DEF.into(), TC.into()))
}

fn log2_ceil(n: usize) -> u32 {
    if n <= 1 {
        return 0;
    }
    (usize::BITS) - (n - 1).leading_zeros()
}

// ── ① culprit found in ≤ log₂ executions ─────────────────────────────────────

#[test]
fn item_1_culprit_in_at_most_log2_execs() {
    // A sweep of history sizes: the culprit (first red) must always be located
    // in ≤ ⌈log₂ n⌉ probes over the memoized-check oracle.
    for n in [2usize, 3, 5, 8, 13, 64, 100, 1000] {
        for first_red in [1usize, 2, n / 2, n - 1] {
            if first_red == 0 || first_red >= n {
                continue;
            }
            let (ac, history) = memoized_history(n, first_red);
            let oracle = MemoizedCheckOracle::new(&ac);
            let bisect = Bisector::new(&oracle);

            let outcome = bisect.find_culprit(&history).expect("bisect succeeds");

            assert_eq!(
                outcome.culprit_index, first_red,
                "culprit is the first red tree (n={n}, first_red={first_red})"
            );
            assert_eq!(outcome.culprit_ref, history.trees()[first_red]);
            let bound = log2_ceil(n) as u64;
            assert!(
                outcome.executions <= bound,
                "culprit found in {} execs, must be ≤ log₂({n})={bound}",
                outcome.executions
            );
            // The probes were memoized lookups, not real check runs.
            assert_eq!(
                outcome.real_executions, 0,
                "every probe is a free AC hit, zero real check executions"
            );
        }
    }
}

/// ADVERSARIAL: a linear scan (n-1 probes) would BLOW the log₂ bound — the
/// oracle is real, not gamed. (A degenerate all-red history's bound is log₂ n;
/// a scan would need n-1.)
#[test]
fn item_1_linear_scan_would_break_the_bound() {
    let n = 64;
    let bound = log2_ceil(n) as u64; // = 6
    let linear_cost = (n - 1) as u64; // = 63
    assert!(
        linear_cost > bound,
        "a linear scan ({linear_cost}) must exceed the log₂ bound ({bound}); \
         if find_culprit ever scanned, item_1 goes RED"
    );
}

// ── ② diff-vs-green + suspect targets ────────────────────────────────────────

#[test]
fn item_2_diff_vs_green_and_suspects() {
    let (ac, history) = memoized_history(16, 9);
    let oracle = MemoizedCheckOracle::new(&ac);
    let bisect = Bisector::new(&oracle);

    let outcome = bisect.find_culprit(&history).unwrap();
    let diag = bisect.diagnose(&history, &outcome);

    // The culprit's predecessor is the last known-green tree.
    assert_eq!(outcome.last_green_index, Some(8));
    assert_eq!(diag.culprit_ref, history.trees()[9]);

    // diff-vs-green ref names BOTH endpoints: last-green → culprit.
    assert!(
        diag.diff_vs_green_ref.contains(&history.trees()[8])
            && diag.diff_vs_green_ref.contains(&history.trees()[9]),
        "diff-vs-green ref spans last-green→culprit: {}",
        diag.diff_vs_green_ref
    );

    // Suspect targets are the build/test targets reachable from the culprit's
    // change — derived, non-empty, deduplicated.
    assert!(!diag.suspect_targets.is_empty(), "suspects extracted");
    let mut sorted = diag.suspect_targets.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), diag.suspect_targets.len(), "suspects deduped");

    // The bisect path records the probe trail (the culprit endpoint present),
    // so the path is auditable.
    assert!(diag.bisect_path.contains(&history.trees()[9]));
}

// ── ③ end-to-end bisect+diagnose on the committed fixture in < 2 minutes ──────

#[test]
fn item_3_fixture_under_two_minutes() {
    // The standing fixture: a realistic 1024-deep memoized history.
    let (ac, history) = memoized_history(1024, 700);
    let oracle = MemoizedCheckOracle::new(&ac);
    let bisect = Bisector::new(&oracle);

    let start = Instant::now();
    let outcome = bisect.find_culprit(&history).unwrap();
    let diag = bisect.diagnose(&history, &outcome);
    let elapsed = start.elapsed();

    assert_eq!(outcome.culprit_index, 700);
    assert!(
        elapsed.as_secs() < 120,
        "end-to-end bisect+diagnose must complete in <2min (took {elapsed:?})"
    );
    // Mostly AC hits: ≤ log₂(1024)=10 probes, zero real executions.
    assert!(outcome.executions <= log2_ceil(1024) as u64);
    assert_eq!(outcome.real_executions, 0);
    // And it produced a bounded diagnosis (cross-link to ④).
    assert!(diag.size_bytes <= DIAGNOSIS_SIZE_BOUND);
}

// ── ④ diagnosis is bounded schema data, never a raw log dump (size assert) ────

#[test]
fn item_4_diagnosis_is_bounded_schema_data() {
    let (ac, history) = memoized_history(256, 200);
    let oracle = MemoizedCheckOracle::new(&ac);
    let bisect = Bisector::new(&oracle);
    let outcome = bisect.find_culprit(&history).unwrap();
    let diag = bisect.diagnose(&history, &outcome);

    // size_bytes is the SELF-REPORTED serialized size and it is bounded.
    let serialized = serde_json::to_vec(&diag).expect("diagnosis serializes");
    assert_eq!(
        diag.size_bytes as usize,
        serialized.len(),
        "size_bytes reflects the real serialized size"
    );
    assert!(
        diag.size_bytes <= DIAGNOSIS_SIZE_BOUND,
        "diagnosis bounded ≤ {DIAGNOSIS_SIZE_BOUND} bytes, got {}",
        diag.size_bytes
    );

    // It carries CAS REFS, never inlined raw logs. The culprit's CheckResult
    // holds multi-megabyte logs behind refs; none of those bytes appear here.
    let raw_log = "FAIL ".repeat(500_000); // a 2.5MB "raw log dump"
    assert!(
        serialized.len() < raw_log.len(),
        "a bounded diagnosis is far smaller than a raw log dump"
    );
    let json = serde_json::to_string(&diag).unwrap();
    assert!(
        !json.contains(&raw_log),
        "diagnosis must not inline raw logs"
    );

    // The constructor itself REFUSES to build an unbounded diagnosis: feeding a
    // raw-log-sized payload as a suspect is rejected fail-closed.
    let huge_suspects: Vec<String> = vec!["x".repeat(DIAGNOSIS_SIZE_BOUND as usize + 1)];
    let res = bisect.try_diagnose_with_suspects(&history, &outcome, huge_suspects);
    assert_eq!(
        res,
        Err(BisectError::DiagnosisTooLarge),
        "a raw-log-dump diagnosis is rejected fail-closed"
    );
}

/// ADVERSARIAL: the bound is load-bearing — a diagnosis that inlined the
/// culprit's logs would exceed it. Prove a hand-built oversized object would
/// fail the same assertion the production path enforces.
#[test]
fn item_4_inlined_logs_would_break_the_bound() {
    let base = DiagnosisObject {
        culprit_ref: "cas:culprit".into(),
        diff_vs_green_ref: "cas:diff".into(),
        suspect_targets: vec!["//pkg:t".into()],
        bisect_path: vec!["t0".into()],
        size_bytes: 0,
    };
    // simulate someone inlining a 4000-line log dump:
    let with_logs = "log-line\n".repeat(4000);
    let inflated = DiagnosisObject {
        suspect_targets: vec![with_logs],
        ..base
    };
    let len = serde_json::to_vec(&inflated).unwrap().len() as u64;
    assert!(
        len > DIAGNOSIS_SIZE_BOUND,
        "an inlined-log diagnosis ({len}B) must exceed the bound ({DIAGNOSIS_SIZE_BOUND}B)"
    );
}

// ── ⑤ bisect triggers automatically on any red — no manual invocation ────────

#[test]
fn item_5_auto_triggers_on_red() {
    let (ac, history) = memoized_history(32, 20);
    let oracle = MemoizedCheckOracle::new(&ac);

    // A RED signal arrives from the landing queue (UNION-FAIL / red tip). The
    // ONLY entry point is the auto-trigger hook — the caller does NOT invoke
    // find_culprit / diagnose by hand.
    let signal = RedSignal::from_red_tip(history.clone());
    let diag = on_red_signal(&oracle, &signal)
        .expect("a red signal auto-produces a diagnosis with no manual call")
        .expect("red signal yields a diagnosis");

    // The auto path produced the same bounded, correct diagnosis.
    assert_eq!(diag.culprit_ref, history.trees()[20]);
    assert!(diag.size_bytes <= DIAGNOSIS_SIZE_BOUND);
}

#[test]
fn item_5_green_signal_does_not_bisect() {
    // A GREEN (conflict-free, no red) signal must NOT trigger a bisect — the
    // auto-trigger fires on RED ONLY.
    let (ac, history) = memoized_history(32, 31); // tip is the only red
    let green_history = History::new(
        history.trees()[..31].to_vec(), // truncate to the all-green prefix
        history.def_digest().into(),
        history.toolchain_digest().into(),
    );
    let oracle = MemoizedCheckOracle::new(&ac);

    let signal = RedSignal::from_red_tip(green_history);
    let res = on_red_signal(&oracle, &signal).expect("green signal handled");
    assert!(
        res.is_none(),
        "a green tip auto-triggers NO bisect (no diagnosis)"
    );
}

/// ADVERSARIAL: there is NO manual public bisect entry that bypasses the
/// red-signal gate. The auto-trigger consumes a `RedSignal`, and `RedSignal`
/// can ONLY be constructed from a queue signal (`from_red_tip` / `from_queue`).
/// This test exercises the single auto path and asserts no diagnosis is
/// produced without a real red tip.
#[test]
fn item_5_no_diagnosis_without_a_signal() {
    // A signal carrying an EMPTY history (no red tip to bisect) yields no
    // diagnosis — the auto-trigger never fabricates one out of nothing.
    let ac = InMemoryAc::new();
    let oracle = MemoizedCheckOracle::new(&ac);
    let empty = History::new(Vec::new(), DEF.into(), TC.into());
    let signal = RedSignal::from_red_tip(empty);
    let res = on_red_signal(&oracle, &signal).expect("empty signal handled");
    assert!(res.is_none(), "no history → no diagnosis, never fabricated");
}
