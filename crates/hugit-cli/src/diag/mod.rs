//! `hugit diag --log <path> --def-digest <hex> [--toolchain <hex>]` — structured
//! failure diagnosis (bisect) over the recorded check history.
//!
//! The `hugit-diag` bisect engine ([`hugit_diag::bisect`]) is auto-trigger-only:
//! `on_red_signal` consumes a [`RedSignal`] built from a resolved [`History`]
//! (ordered tree hashes + the def/toolchain digests) and probes a
//! [`CheckOracle`]. There is no live ActionCache on the CLI, so this verb drives
//! the SAME engine over a **log-backed oracle**: it projects a `History` from the
//! `check.recorded` events already on the canonical `--log` (grouped by the
//! `(def_digest, toolchain_digest)` axes, ordered by chain seq) and answers each
//! probe from those recorded results — no re-execution, no network.
//!
//! Read-only: `diag` emits the `DiagnosisObject` (culprit ref, diff-vs-green,
//! suspect targets, bisect path) as stable JSON on stdout. It appends NO event
//! (there is no `diag.recorded` kind; this mirrors `checks show` / `queue show`).
//! A green/empty tip is an honest `{"diagnosis": null, …}` (exit 0), not an error.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::checks::load_event_log;
use crate::porcelain::PorcelainError;

use hugit_diag::bisect::{
    BisectError, CheckOracle, History, ProbeOutcome, ProbeVerdict, RedSignal, on_red_signal,
};

/// The event kind the wedge EXECUTE path (`hugit check --store`) appends.
const CHECK_RECORDED_KIND: &str = "check.recorded";

/// Arguments for `hugit diag`.
#[derive(clap::Args, Debug)]
pub struct DiagArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`). Read + chain-
    /// verified; the `check.recorded` events on it are the diagnosis input.
    #[arg(long)]
    pub log: PathBuf,

    /// The check definition digest (axis 2 of the memo key) to diagnose. Get it
    /// from `hugit check` output or `hugit checks show`.
    #[arg(long = "def-digest")]
    pub def_digest: String,

    /// The toolchain digest (axis 3). Optional: if the log carries exactly one
    /// toolchain for this def, it is inferred; if several, you must disambiguate.
    #[arg(long)]
    pub toolchain: Option<String>,
}

/// A [`CheckOracle`] backed by `check.recorded` events on the canonical log.
///
/// Each probe recomputes the memo key from the history's `(tree, def, toolchain)`
/// axes (`hugit_refstore::compute_memo_key` — the exact formula the engine's
/// `History::memo_key_at` uses) and answers Green/Red from the recorded `exit`
/// code. A key with no recorded result is a MISS → Red, `was_real_execution`
/// (fail-closed, identical to `MemoizedCheckOracle`'s miss semantics) — but the
/// projection only ever builds histories from recorded trees, so misses are not
/// expected on the happy path.
struct LogOracle {
    /// memo_key → recorded exit code (0 = green).
    memo: HashMap<String, i32>,
    probes: Cell<u64>,
    real_execs: Cell<u64>,
}

impl CheckOracle for LogOracle {
    fn probe(&self, history: &History, index: usize) -> ProbeOutcome {
        self.probes.set(self.probes.get() + 1);
        let key = hugit_refstore::compute_memo_key(
            &history.trees()[index],
            history.def_digest(),
            history.toolchain_digest(),
        );
        match self.memo.get(&key) {
            Some(&exit) => ProbeOutcome {
                verdict: if exit == 0 {
                    ProbeVerdict::Green
                } else {
                    ProbeVerdict::Red
                },
                was_real_execution: false,
            },
            None => {
                // Not recorded — fail-closed Red, counted as a real execution
                // (matches MemoizedCheckOracle's miss). Not expected: we only
                // build histories from recorded trees.
                self.real_execs.set(self.real_execs.get() + 1);
                ProbeOutcome {
                    verdict: ProbeVerdict::Red,
                    was_real_execution: true,
                }
            }
        }
    }

    fn probe_count(&self) -> u64 {
        self.probes.get()
    }

    fn real_execution_count(&self) -> u64 {
        self.real_execs.get()
    }
}

/// One projected `check.recorded` row.
struct CheckRow {
    tree_hash: String,
    toolchain_digest: String,
    memo_key: String,
    exit: i32,
}

/// Run `hugit diag` — emit the diagnosis JSON (exit 0) or the canonical error
/// envelope (exit 2) on a structured fault.
pub fn run(args: DiagArgs) -> ExitCode {
    match do_run(args) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", err.to_json());
            err.exit_code()
        }
    }
}

fn do_run(args: DiagArgs) -> Result<String, PorcelainError> {
    // Load + chain-verify the log (log_not_found / parse_log / chain_broken).
    let log = load_event_log(&args.log)?;

    // ── Project the check.recorded rows for this def_digest ───────────────────
    // Ordered by chain seq (records() is in seq order); filter by the def axis
    // and (when given) the toolchain axis.
    let rows: Vec<CheckRow> = log
        .records()
        .iter()
        .filter(|r| r.kind == CHECK_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<serde_json::Value>(&r.payload).ok())
        .filter_map(|v| {
            let def = v.get("def_digest")?.as_str()?.to_string();
            if def != args.def_digest {
                return None;
            }
            Some(CheckRow {
                tree_hash: v.get("tree_hash")?.as_str()?.to_string(),
                toolchain_digest: v.get("toolchain_digest")?.as_str()?.to_string(),
                memo_key: v.get("memo_key")?.as_str()?.to_string(),
                exit: v.get("exit")?.as_i64()? as i32,
            })
        })
        .filter(|row| {
            args.toolchain
                .as_deref()
                .is_none_or(|t| row.toolchain_digest == t)
        })
        .collect();

    if rows.is_empty() {
        // The def did not resolve — echo it scrubbed (a digest is not a secret,
        // but route through the engine as defence-in-depth) and fail exit-2.
        let safe_def = crate::redaction::scrub(&args.def_digest);
        return Err(PorcelainError::new(
            "no_history",
            format!(
                "no check.recorded events for def-digest '{safe_def}'{} on the log",
                args.toolchain
                    .as_deref()
                    .map(|t| format!(" + toolchain '{}'", crate::redaction::scrub(t)))
                    .unwrap_or_default()
            ),
            "record check runs first (`hugit check --def … --store`) or pass a \
             --def-digest that exists on the log (see `hugit checks show`)",
        ));
    }

    // ── Resolve the toolchain axis (History needs a single one) ───────────────
    let toolchain = match &args.toolchain {
        Some(t) => t.clone(),
        None => {
            let mut seen: Vec<&str> = rows.iter().map(|r| r.toolchain_digest.as_str()).collect();
            seen.sort_unstable();
            seen.dedup();
            match seen.as_slice() {
                [one] => (*one).to_string(),
                _ => {
                    return Err(PorcelainError::new(
                        "ambiguous_toolchain",
                        format!(
                            "def-digest resolves {} distinct toolchains on the log; \
                             bisect needs one",
                            seen.len()
                        ),
                        "pass --toolchain <digest> to pick the toolchain axis to diagnose",
                    )
                    .with_context("toolchains", json!(seen)));
                }
            }
        }
    };

    // ── Build the History + the log-backed oracle ─────────────────────────────
    // Trees in chain-seq order (oldest → tip); the tip is the latest recorded
    // state for this (def, toolchain). The memo map answers each probe.
    let trees: Vec<String> = rows
        .iter()
        .filter(|r| r.toolchain_digest == toolchain)
        .map(|r| r.tree_hash.clone())
        .collect();
    let memo: HashMap<String, i32> = rows
        .iter()
        .filter(|r| r.toolchain_digest == toolchain)
        .map(|r| (r.memo_key.clone(), r.exit))
        .collect();

    let history = History::new(trees, args.def_digest.clone(), toolchain);
    let oracle = LogOracle {
        memo,
        probes: Cell::new(0),
        real_execs: Cell::new(0),
    };

    // ── Drive the REAL bisect engine ──────────────────────────────────────────
    let signal = RedSignal::from_red_tip(history);
    match on_red_signal(&oracle, &signal) {
        Ok(Some(diag)) => Ok(json!({
            "diagnosis": diag,
            "probes": oracle.probe_count(),
            "real_executions": oracle.real_execution_count(),
        })
        .to_string()),
        // Green/empty tip → nothing to diagnose (honest, exit 0 — not a fault).
        Ok(None) => Ok(json!({
            "diagnosis": serde_json::Value::Null,
            "reason": "the recorded tip for this (def, toolchain) is green or the \
                       history is empty — there is no red failure to bisect",
        })
        .to_string()),
        Err(BisectError::DiagnosisTooLarge) => Err(PorcelainError::new(
            "diagnosis_too_large",
            "the bisect diagnosis exceeds the size bound",
            "the failure spans too many suspect targets to summarise; narrow the \
             history or inspect the culprit range manually",
        )),
        Err(BisectError::NoRedTip) => Ok(json!({
            "diagnosis": serde_json::Value::Null,
            "reason": "no red tip in the recorded history — nothing to bisect",
        })
        .to_string()),
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{EventLog, compute_memo_key};

    /// Append a `check.recorded` event with real memo-key axes via the test shim.
    fn record_check(log: &mut EventLog, tree: &str, def: &str, toolchain: &str, exit: i32) {
        let memo_key = compute_memo_key(tree, def, toolchain);
        let payload = json!({
            "name": "test",
            "memo_key": memo_key,
            "tree_hash": tree,
            "def_digest": def,
            "toolchain_digest": toolchain,
            "exit": exit,
            "cache_hit": true,
            "duration_ms": 0,
        })
        .to_string();
        log.append_for_test(
            CHECK_RECORDED_KIND,
            vec!["orchestrator:test".to_string()],
            payload,
            0,
        );
    }

    fn scratch_log(tag: &str, records: impl FnOnce(&mut EventLog)) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-diag-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.json");
        let mut log = EventLog::new();
        log.append_for_test("repo.init", vec!["orchestrator:test".to_string()], "{}", 0);
        records(&mut log);
        std::fs::write(&path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
        path
    }

    const DEF: &str = "def00000000000000000000000000000000000000000000000000000000000000";
    const TC: &str = "tc000000000000000000000000000000000000000000000000000000000000000";

    /// A history that goes green→green→red bisects to the first red tree.
    #[test]
    fn diagnoses_first_red_tree() {
        let path = scratch_log("firstred", |log| {
            record_check(log, &"a".repeat(64), DEF, TC, 0); // green
            record_check(log, &"b".repeat(64), DEF, TC, 0); // green
            record_check(log, &"c".repeat(64), DEF, TC, 1); // RED tip
        });

        let result = do_run(DiagArgs {
            log: path,
            def_digest: DEF.to_string(),
            toolchain: Some(TC.to_string()),
        })
        .expect("diag ok");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            !v["diagnosis"].is_null(),
            "a red tip must produce a diagnosis: {v}"
        );
        // The culprit is the first red tree (the 'c' tree).
        assert!(
            v["diagnosis"]["culprit_ref"]
                .as_str()
                .unwrap()
                .contains(&"c".repeat(64)),
            "culprit must be the first red tree: {v}"
        );
    }

    /// A green tip → no diagnosis (honest null, exit 0 — not an error).
    #[test]
    fn green_tip_is_null_diagnosis_not_error() {
        let path = scratch_log("greentip", |log| {
            record_check(log, &"a".repeat(64), DEF, TC, 0);
            record_check(log, &"b".repeat(64), DEF, TC, 0); // green tip
        });
        let result = do_run(DiagArgs {
            log: path,
            def_digest: DEF.to_string(),
            toolchain: Some(TC.to_string()),
        })
        .expect("green tip is exit-0");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(v["diagnosis"].is_null());
    }

    /// An unknown def-digest → `no_history`/exit-2.
    #[test]
    fn unknown_def_is_no_history() {
        let path = scratch_log("unknowndef", |log| {
            record_check(log, &"a".repeat(64), DEF, TC, 1);
        });
        let err = do_run(DiagArgs {
            log: path,
            def_digest: "deadbeef".to_string(),
            toolchain: None,
        })
        .expect_err("unknown def must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "no_history");
    }

    /// Two toolchains for one def + no `--toolchain` → `ambiguous_toolchain`/exit-2.
    #[test]
    fn ambiguous_toolchain_is_rejected() {
        let tc2 = "tc222222222222222222222222222222222222222222222222222222222222222";
        let path = scratch_log("ambig", |log| {
            record_check(log, &"a".repeat(64), DEF, TC, 1);
            record_check(log, &"a".repeat(64), DEF, tc2, 1);
        });
        let err = do_run(DiagArgs {
            log: path,
            def_digest: DEF.to_string(),
            toolchain: None,
        })
        .expect_err("ambiguous toolchain must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "ambiguous_toolchain");
    }

    /// A missing `--log` is `log_not_found`/exit-2.
    #[test]
    fn missing_log_is_log_not_found() {
        let dir = std::env::temp_dir().join(format!("hugit-diag-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = do_run(DiagArgs {
            log: dir.join("no-such.json"),
            def_digest: DEF.to_string(),
            toolchain: None,
        })
        .expect_err("missing log must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "log_not_found");
    }
}
