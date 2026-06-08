//! Item ① — real 5-PR wave e2e.
//!
//! Drives a 5-PR agent-fleet wave through the REAL Phase-B engine:
//!   hugit-queue (batch → union-evaluate → ordered landing) +
//!   hugit-checks (memoized executor, InMemoryAc) +
//!   hugit-refstore (EventLog, append-only audit chain).
//!
//! No GitHub API, no network: the wave is driven in-process against the
//! exact same engine the production path uses.

use std::collections::HashMap;

use hugit_checks::client::{
    ac::InMemoryAc,
    executor::{CheckRunner, ExecError, run_memoized},
    memo_key::FileContent,
};
use hugit_contracts::{CheckDef, CheckResult, LandableEntry};
use hugit_queue::core::{
    AffectedSet, Batch, EntryState, MemoCheck, UnionVerdict, evaluate_union, land_in_order,
    landed_in_order,
};
use hugit_refstore::{EventLog, canonical_json};

/// Configuration for a 5-PR wave.
#[derive(Debug, Clone)]
pub struct WaveConfig {
    /// Ordered list of (pr_id, set-of-affected-keys).
    pub entries: Vec<(String, Vec<String>)>,
    /// Which pair of PR IDs should fail their union (empty = all green).
    pub failing_pair: Option<(String, String)>,
    /// The shared [`CheckDef`] every PR is checked against (same tree/def/toolchain
    /// → same memo key → AC hits on repeat).
    pub check_def: CheckDef,
    /// The toolchain digest (same for all PRs in the wave).
    pub toolchain_digest: String,
}

impl WaveConfig {
    /// 5 disjoint-green PRs: pr-0 … pr-4, each touching a distinct file.
    pub fn five_pr_disjoint_green() -> Self {
        WaveConfig {
            entries: (0..5)
                .map(|i| (format!("pr-{i}"), vec![format!("file-{i}")]))
                .collect(),
            failing_pair: None,
            check_def: make_check_def("cargo test --all", "rust-stable"),
            toolchain_digest: "toolchain-rust-stable-1.78".to_string(),
        }
    }

    /// 5 PRs where `bad_a` and `bad_b` form a failing pair (their union is red).
    pub fn five_pr_with_failing_pair(bad_a: &str, bad_b: &str) -> Self {
        WaveConfig {
            entries: (0..5)
                .map(|i| (format!("pr-{i}"), vec![format!("file-{i}")]))
                .collect(),
            failing_pair: Some((bad_a.to_string(), bad_b.to_string())),
            check_def: make_check_def("cargo test --all", "rust-stable"),
            toolchain_digest: "toolchain-rust-stable-1.78".to_string(),
        }
    }
}

/// Result of running a wave.
pub struct WaveReport {
    /// PR IDs that landed, in queue order.
    pub landed: Vec<String>,
    /// PR IDs excluded (failing-pair members or single-item failures).
    pub excluded: Vec<String>,
    /// Total number of local check executions (0 on a fully-warmed AC).
    pub local_executions: u32,
    /// Audit event log (one event per landed PR).
    pub event_log: EventLog,
}

/// Run a 5-PR wave through the real Phase-B engine.
///
/// The `InMemoryAc` is created fresh per call.  Calling `run_wave` twice with
/// the same `WaveConfig` but sharing the SAME `InMemoryAc` would give 0
/// local_executions on the second call — callers that need to test the
/// memoization wedge should share a pre-warmed AC (see
/// `item_1_repeat_wave_zero_local_executions_memoization_wedge`).
///
/// This function creates its own fresh AC.  The item_1 memoization test
/// calls `run_wave_with_ac` twice with a shared AC.
pub fn run_wave(cfg: &WaveConfig) -> WaveReport {
    let ac = InMemoryAc::new();
    run_wave_with_ac(cfg, &ac)
}

/// Run a 5-PR wave through the real Phase-B engine with a SHARED `InMemoryAc`.
///
/// On a cold AC the first wave executes all checks; on a warm AC the second
/// wave is all hits (0 `local_executions`).
pub fn run_wave_with_ac(cfg: &WaveConfig, ac: &InMemoryAc) -> WaveReport {
    // Build the landable entries and batch.
    let entries: Vec<(LandableEntry, AffectedSet)> = cfg
        .entries
        .iter()
        .enumerate()
        .map(|(i, (pr_id, affected))| {
            let landable = LandableEntry {
                item_id: pr_id.clone(),
                intent_id: format!("intent-{pr_id}"),
                tree_hash: format!("tree-{pr_id}"),
                order_index: i as u64,
            };
            (
                landable,
                AffectedSet::new(affected.iter().map(|s| s.as_str())),
            )
        })
        .collect();
    let mut batch = Batch::from_entries("wave-batch", entries.iter().cloned());

    // Build a per-PR file map (each PR touches its own distinct file).
    // The tree hash is derived from the actual file contents via the memo key.
    let file_maps: Vec<HashMap<String, FileContent>> = cfg
        .entries
        .iter()
        .map(|(pr_id, _)| {
            let mut m = HashMap::new();
            // Each PR has one file containing its id as content.
            m.insert(format!("src/{pr_id}.rs"), pr_id.as_bytes().to_vec());
            m
        })
        .collect();

    // Determine the failing pair for the oracle.
    let failing_pair = cfg.failing_pair.as_ref();

    // Build the MemoCheck oracle: evaluates the union of a set of PRs,
    // using the real InMemoryAc to serve cached results on repeat calls.
    // On a MISS the in-process runner executes the check deterministically.
    let mut oracle = WaveOracle {
        cfg,
        file_maps: &file_maps,
        ac,
        failing_pair,
        total_local_executions: 0,
    };

    // Evaluate the union and compute per-entry outcomes.
    let evaluation = evaluate_union(&batch, &mut oracle);
    let ids: Vec<&str> = cfg.entries.iter().map(|(id, _)| id.as_str()).collect();
    let outcomes = evaluation.outcomes_for_landing(&ids);

    // Land in queue order.
    land_in_order(&mut batch, &outcomes).expect("landing must not error on a well-formed batch");

    let landed = landed_in_order(&batch);
    let excluded: Vec<String> = batch
        .entries()
        .iter()
        .filter(|e| e.state == EntryState::UnionFail)
        .map(|e| e.item_id().to_string())
        .collect();

    // Append one audit event per landed PR to the event log.
    let mut event_log = EventLog::new();
    for pr_id in &landed {
        let payload_raw = format!(r#"{{"pr_id":"{pr_id}","union_verdict":"green"}}"#);
        let payload = canonical_json(&payload_raw).unwrap_or_else(|| payload_raw.clone());
        event_log.append(
            "pr.landed",
            vec!["dogfood-harness".to_string()],
            payload,
            0, // deterministic recorded_at for hermetic tests
        );
    }

    WaveReport {
        landed,
        excluded,
        local_executions: oracle.total_local_executions,
        event_log,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal oracle wiring
// ─────────────────────────────────────────────────────────────────────────────

struct WaveOracle<'a> {
    cfg: &'a WaveConfig,
    file_maps: &'a [HashMap<String, FileContent>],
    ac: &'a InMemoryAc,
    failing_pair: Option<&'a (String, String)>,
    total_local_executions: u32,
}

impl<'a> MemoCheck for WaveOracle<'a> {
    fn evaluate(
        &mut self,
        item_ids: &[&str],
    ) -> (UnionVerdict, Vec<hugit_queue::core::CheckSource>) {
        use hugit_queue::core::CheckSource;

        // Check the failing-pair condition first.
        if let Some((bad_a, bad_b)) = self.failing_pair {
            let has_a = item_ids.contains(&bad_a.as_str());
            let has_b = item_ids.contains(&bad_b.as_str());
            if has_a && has_b {
                // Both members present → red union, all hits.
                let sources = item_ids.iter().map(|_| CheckSource::Hit).collect();
                return (UnionVerdict::Red, sources);
            }
        }

        // Run the memoized check for each item in the union and accumulate.
        let mut any_red = false;
        let mut sources = Vec::new();

        for pr_id in item_ids {
            // Find the index of this PR to get its file map.
            let idx = self
                .cfg
                .entries
                .iter()
                .position(|(id, _)| id == *pr_id)
                .unwrap_or(0);
            let files = &self.file_maps[idx];

            let runner = DeterministicRunner {
                pr_id: pr_id.to_string(),
            };
            let file_iter: Vec<(&str, &FileContent)> =
                files.iter().map(|(k, v)| (k.as_str(), v)).collect();

            let outcome = run_memoized(
                self.ac,
                &runner,
                &self.cfg.check_def,
                file_iter,
                &self.cfg.toolchain_digest,
            )
            .expect("check execution must not fail in the dogfood harness");

            if outcome.result.exit != 0 {
                any_red = true;
            }
            self.total_local_executions += outcome.local_executions;
            sources.push(if outcome.from_cache {
                CheckSource::Hit
            } else {
                CheckSource::Executed
            });
        }

        let verdict = if any_red {
            UnionVerdict::Red
        } else {
            UnionVerdict::Green
        };
        (verdict, sources)
    }
}

/// A deterministic in-process check runner.  Always exits 0 (green).
/// The result is a pure function of the memo key + tree/def/toolchain axes
/// → byte-identical across all callers with the same inputs.
struct DeterministicRunner {
    pr_id: String,
}

impl CheckRunner for DeterministicRunner {
    fn run(
        &self,
        _def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, ExecError> {
        Ok(CheckResult {
            memo_key: memo_key.to_string(),
            tree_hash: tree_root.to_string(),
            def_digest: def_digest.to_string(),
            toolchain_digest: toolchain_digest.to_string(),
            exit: 0,
            artifacts: vec![],
            stdout_ref: format!("stdout-ref-{}", &self.pr_id),
            stderr_ref: format!("stderr-ref-{}", &self.pr_id),
            // Deterministic small duration for the in-process runner (ms).
            duration_ms: 10,
            runner_ref: "in-process-deterministic".to_string(),
            produced_at: 0,
        })
    }
}

/// Build a minimal [`CheckDef`] for dogfood tests.
fn make_check_def(command: &str, toolchain_ref: &str) -> CheckDef {
    use hugit_checks::client::memo_key::compute_def_digest;
    let mut def = CheckDef {
        def_digest: String::new(),
        command: command.to_string(),
        inputs: vec![],
        toolchain_ref: toolchain_ref.to_string(),
        env_manifest: String::new(),
        glob_set: vec!["src/**".to_string()],
    };
    def.def_digest = compute_def_digest(&def);
    def
}
