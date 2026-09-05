//! dock::reconcile — the A4 attribution core (WP-DOCK-3).
//!
//! Reconciliation is the audit that makes per-branch cost honest: every
//! `cost.sample` and every captured branch commit is attributed to EXACTLY ONE
//! bucket (R1), the bucket describing the truth of that lane:
//!
//! - **matched** — dock cost that the branch's commits explain (the normal
//!   lane: work landed AND cost measured under the same dock).
//! - **investigated** — cost on a dock whose branch never landed a commit
//!   (A4: spent-but-nothing-committed — never hidden, R3).
//! - **unlabeled** — cost with NO dock binding (empty `dock_id`), or committed
//!   work under a dock with zero measured cost (never silently zero, R3).
//! - **reconciled** — a cost sample whose `dock_id` no known dock record can
//!   place (the A2 cost-before-dock micro-window, or an env/cwd residue) —
//!   carried visibly off any dock, never fabricated onto one.
//!
//! Reads ONLY the canonical log projection (WP impl note 4 — no parallel
//! store): `dock.record` (WP-DOCK-1/2) + `cost.sample` (WP-DOCK-4) + the
//! commit-kind `ref.update` records (capture).
//!
//! Honesty invariants (the design §7 "never bends"):
//! - cost is counted exactly once — a sample is read from the log and the
//!   branch from the dock record; multi-head worktrees on one branch AGGREGATE
//!   (R2), never duplicate.
//! - unbound intents (`ref.update` commits on a branch with no per-branch
//!   dock) LINK to the auto-coined repo-scope dock (M5); only when NO
//!   repo-scope dock exists do they surface as unlabeled.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::checks::load_event_log;

use super::DOCK_RECORD_KIND;
use super::attest::COST_SAMPLE_KIND;
use super::close::DOCK_CLOSE_KIND;

/// The frozen attribution buckets (A4) — every lane lands in exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// Cost measured AND the branch landed — the normal lane.
    Matched,
    /// Cost measured but the branch never landed — investigated, never hidden.
    Investigated,
    /// Cost with no dock binding, or work with zero measured cost.
    Unlabeled,
    /// Cost whose dock id no known dock record can place — carried visibly.
    Reconciled,
}

impl Bucket {
    /// The stable on-wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Matched => "matched",
            Bucket::Investigated => "investigated",
            Bucket::Unlabeled => "unlabeled",
            Bucket::Reconciled => "reconciled",
        }
    }
}

/// A parsed `dock.record` view we attribute against (WP-DOCK-1/2).
#[derive(Debug, Clone)]
struct DockView {
    /// The branch the dock was coined on.
    branch: String,
    /// `worktree` | `repo` | `clone`.
    origin: String,
    /// The absolute gitdir (absent ⇒ the dock is a ghost).
    gitdir: String,
    /// Whether a `dock.close` record already finalized this dock.
    closed: bool,
}

/// Per-dock attribution at the branch/projection level (F5).
#[derive(Debug, Clone)]
pub struct DockAttribution {
    pub dock_id: String,
    pub branch: String,
    /// `open` | `ghost` | `closed`.
    pub state: String,
    pub origin: String,
    /// Σ cost.sample under this dock — counted EXACTLY once (R1).
    pub cost_usd_micros: u64,
    /// Captured commits on the dock's branch.
    pub commit_count: u64,
    /// The single bucket this dock landed in (R1).
    pub bucket: Bucket,
}

/// The repo-wide attribution summary (A4 buckets + M5 links).
#[derive(Debug, Clone)]
pub struct AttributionSummary {
    /// Per-dock attributes, one per known dock (each in exactly one bucket).
    pub docks: Vec<DockAttribution>,
    /// Σ cost of samples with a dock id no known record can place (A2).
    pub reconciled_cost_usd_micros: u64,
    /// Σ cost of samples with an empty dock id (never dropped).
    pub unlabeled_cost_usd_micros: u64,
    /// Captured commits on branches with no dock cost and no repo-scope dock.
    pub unlabeled_commit_count: u64,
    /// Branches whose unbound intents linked to the repo-scope dock (M5).
    pub repo_scope_linked_branches: Vec<String>,
}

fn load_records(log: &Path) -> Result<Vec<(String, Value)>, String> {
    let log = load_event_log(log).map_err(|e| format!("load log: {}", e.to_json()))?;
    Ok(log
        .records()
        .iter()
        .map(|r| {
            (
                r.kind.clone(),
                serde_json::from_str(&r.payload).unwrap_or(Value::Null),
            )
        })
        .collect())
}

fn as_str(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn is_branch_commit(p: &Value) -> bool {
    (p.get("checkout").is_none() && p.get("attempt").is_none() && p.get("merged_from").is_none())
        && p.get("target").is_some()
        && !as_str(&p["branch"]).is_empty()
}

/// The dock's current display state (R4): a closed dock whose gitdir vanished
/// keeps showing as `ghost`; a finalized dock is `closed`.
fn state_of(gitdir: &str, closed: bool) -> &'static str {
    let exists = !gitdir.is_empty() && std::path::Path::new(gitdir).exists();
    if !exists {
        "ghost"
    } else if closed {
        "closed"
    } else {
        "open"
    }
}

/// Compute the repo-wide attribution — the projection that both `close` (A4)
/// and `insight` (F5) read. Every sample and every commit ends up in exactly
/// one lane/bucket (R1), never duplicated (R2).
#[allow(clippy::too_many_lines)]
pub fn attribute(log: &Path) -> Result<AttributionSummary, String> {
    let records = load_records(log)?;

    // Parse the inputs (WP-DOCK-1/2 records, WP-DOCK-4 samples, commits).
    let mut views: BTreeMap<String, DockView> = BTreeMap::new();
    let mut closed_ids: Vec<String> = Vec::new();
    let mut branch_commits: BTreeMap<String, u64> = BTreeMap::new();
    let mut samples: Vec<Value> = Vec::new();
    for (kind, p) in records {
        match kind.as_str() {
            DOCK_RECORD_KIND => {
                let id = as_str(&p["dock_id"]);
                if !id.is_empty() {
                    views.insert(
                        id.to_string(),
                        DockView {
                            branch: as_str(&p["branch"]).to_string(),
                            origin: as_str(&p["origin"]).to_string(),
                            gitdir: as_str(&p["gitdir"]).to_string(),
                            closed: false,
                        },
                    );
                }
            }
            DOCK_CLOSE_KIND => {
                let id = as_str(&p["dock_id"]);
                if !id.is_empty() {
                    closed_ids.push(id.to_string());
                }
            }
            COST_SAMPLE_KIND => samples.push(p),
            "ref.update" if is_branch_commit(&p) => {
                let b = as_str(&p["branch"]).to_string();
                *branch_commits.entry(b).or_insert(0) += 1;
            }
            _ => {}
        }
    }
    for id in &closed_ids {
        if let Some(v) = views.get_mut(id) {
            v.closed = true;
        }
    }

    // Build the dock lanes (all `open`/`ghost`/`closed` records, whatever
    // their landing/cost). Each lands in exactly one bucket.
    let mut docks: Vec<DockAttribution> = Vec::new();
    for (id, view) in &views {
        docks.push(DockAttribution {
            dock_id: id.clone(),
            branch: view.branch.clone(),
            state: state_of(&view.gitdir, view.closed).to_string(),
            origin: view.origin.clone(),
            cost_usd_micros: 0,
            commit_count: 0,
            bucket: Bucket::Unlabeled,
        });
    }

    // 1) Samples: empty dock → unlabeled; unknown dock id → reconciled; known
    //    → the dock's cost (landing decides the bucket after the commit pass).
    let mut reconciled = 0u64;
    let mut unlabeled_cost = 0u64;
    for s in &samples {
        let cost = s
            .get("cost_usd_micros")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let dock_id = as_str(&s["dock_id"]);
        if dock_id.is_empty() {
            unlabeled_cost += cost;
        } else if views.contains_key(dock_id) {
            if let Some(d) = docks.iter_mut().find(|d| d.dock_id == dock_id) {
                d.cost_usd_micros += cost;
            }
        } else {
            reconciled += cost;
        }
    }

    // 2) Commits: a branch's dock cost explains its commits (matched); a
    //    branch with docks but zero cost keeps the work on the dock as
    //    UNLABELED (R3 — committed work with no measured cost, never a silent
    //    zero); a branch with NO dock links its intents to the repo-scope dock
    //    (M5) or surfaces them as unlabeled commits.
    let repo_scope = views
        .iter()
        .find(|(_, v)| v.origin == "repo")
        .map(|(id, _)| id.clone());
    let mut unlabeled_commits = 0u64;
    let mut linked: Vec<String> = Vec::new();
    for (branch, count) in &branch_commits {
        let ids_on_branch: Vec<String> = docks
            .iter()
            .filter(|d| &d.branch == branch)
            .map(|d| d.dock_id.clone())
            .collect();
        let sum: u64 = docks
            .iter()
            .filter(|d| &d.branch == branch)
            .map(|d| d.cost_usd_micros)
            .sum();
        if sum > 0 {
            for id in &ids_on_branch {
                if let Some(d) = docks.iter_mut().find(|d| &d.dock_id == id) {
                    d.commit_count = *count;
                }
            }
        } else if ids_on_branch.is_empty() {
            if let Some(rs) = &repo_scope {
                if let Some(rd) = docks.iter_mut().find(|d| &d.dock_id == rs) {
                    rd.commit_count += count;
                    linked.push(branch.clone());
                }
            } else {
                unlabeled_commits += count;
            }
        } else {
            for id in &ids_on_branch {
                if let Some(d) = docks.iter_mut().find(|d| &d.dock_id == id) {
                    d.commit_count = *count;
                }
            }
        }
    }

    // 3) Finalize buckets (R1): cost without landing → investigated; cost with
    //    landing (or zero cost) → matched/unlabeled.
    for d in docks.iter_mut() {
        if d.cost_usd_micros > 0 {
            d.bucket = if d.commit_count > 0 {
                Bucket::Matched
            } else {
                Bucket::Investigated
            };
        } // else stays Unlabeled (work with zero measured cost — honest)
    }

    Ok(AttributionSummary {
        docks,
        reconciled_cost_usd_micros: reconciled,
        unlabeled_cost_usd_micros: unlabeled_cost,
        unlabeled_commit_count: unlabeled_commits,
        repo_scope_linked_branches: linked,
    })
}

/// The cheapest read used by the rest of the crate: does this +only* log have
/// any dock records at all (the "is this a docked repo" guard).
#[must_use]
pub fn any_docks(log: &Path) -> bool {
    load_records(log)
        .map(|rs| {
            rs.iter()
                .any(|(k, p)| k == DOCK_RECORD_KIND && !as_str(&p["dock_id"]).is_empty())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::authz::{Endpoint, PrincipalClass};
    use hugit_refstore::canonical_json;
    use serde_json::{Value, json};

    /// A scratch log builder that appends real hash-chained records through
    /// the SAME append path the hooks/dock use (authz'd, valid chain).
    struct LogBuilder {
        path: std::path::PathBuf,
    }

    impl LogBuilder {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("hugit-rec-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("log.json");
            std::fs::write(&path, b"[]\n").unwrap();
            Self { path }
        }

        fn record(&self, kind: &str, payload: Value) {
            let mut log = load_event_log(&self.path).unwrap();
            let payload_str = payload.to_string();
            let cap = canonical_json(&payload_str).unwrap_or(payload_str);
            log.append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Push,
                kind.to_string(),
                vec!["orchestrator:hugit-hook".to_string()],
                cap,
                1000,
            )
            .expect("append authorized");
            let bytes = serde_json::to_vec_pretty(log.records()).unwrap();
            crate::pr::filelock::atomic_write(&self.path, &bytes).unwrap();
        }

        fn dock(&self, id: &str, branch: &str, origin: &str, gitdir: &str) {
            self.record(
                DOCK_RECORD_KIND,
                json!({
                    "dock_id": id, "gitdir": gitdir, "branch": branch,
                    "charter": "test", "charter_derived": true, "state": "open",
                    "origin": origin, "created_ts": 1000, "pid": 1,
                }),
            );
        }

        fn sample(&self, dock_id: &str, cost: u64, run: &str) {
            self.record(
                COST_SAMPLE_KIND,
                json!({
                    "run_id": run, "dock_id": dock_id, "model": "m",
                    "input_tokens": 1, "output_tokens": 1,
                    "cost_usd_micros": cost, "ts_ms": 1000,
                    "content_hash": "deadbeef", "is_unlabeled": dock_id.is_empty(),
                }),
            );
        }

        fn commit(&self, branch: &str, target: &str) {
            self.record(
                "ref.update",
                json!({
                    "ref": format!("refs/heads/{branch}"),
                    "target": target, "branch": branch,
                }),
            );
        }
    }

    fn cost_of<'a>(att: &'a AttributionSummary, dock_id: &str) -> &'a DockAttribution {
        att.docks.iter().find(|d| d.dock_id == dock_id).unwrap()
    }

    #[test]
    fn r1_exact_once_every_sample_one_bucket() {
        let b = LogBuilder::new("r1");
        b.dock("d1", "feat/a", "worktree", "/tmp/wt-a"); // lands → matched
        b.dock("d2", "feat/b", "worktree", "/tmp/wt-b"); // no landing → investigated
        b.sample("d1", 100, "r1"); // matched
        b.sample("d2", 200, "r2"); // investigated
        b.sample("", 50, "r3"); // unlabeled (empty dock)
        b.sample("unknown-dock", 30, "r4"); // reconciled (no record)
        b.commit("feat/a", "aaaa");

        let att = attribute(&b.path).unwrap();
        assert_eq!(cost_of(&att, "d1").bucket, Bucket::Matched);
        assert_eq!(cost_of(&att, "d1").cost_usd_micros, 100);
        assert_eq!(cost_of(&att, "d2").bucket, Bucket::Investigated);
        assert_eq!(cost_of(&att, "d2").cost_usd_micros, 200);
        assert_eq!(att.unlabeled_cost_usd_micros, 50);
        assert_eq!(att.reconciled_cost_usd_micros, 30);
        // R1 — every sample counted exactly once, in exactly one place.
        let total = att.docks.iter().map(|d| d.cost_usd_micros).sum::<u64>()
            + att.unlabeled_cost_usd_micros
            + att.reconciled_cost_usd_micros;
        assert_eq!(total, 380);
    }

    #[test]
    fn r2_multi_head_worktrees_aggregate_never_duplicate() {
        let b = LogBuilder::new("r2");
        // Two worktrees on ONE branch (multi-head) — F5 aggregates, never dup.
        b.dock("wt-1", "feat/x", "worktree", "/tmp/wt-1");
        b.dock("wt-2", "feat/x", "worktree", "/tmp/wt-2");
        b.sample("wt-1", 100, "r1");
        b.sample("wt-2", 50, "r2");
        b.commit("feat/x", "aaaa");

        let att = attribute(&b.path).unwrap();
        let sum: u64 = att.docks.iter().map(|d| d.cost_usd_micros).sum();
        assert_eq!(sum, 150, "both docks counted, nothing duplicated");
        assert!(att.docks.iter().all(|d| d.bucket == Bucket::Matched));
        // Each dock's commit_count = the branch commits (aggregated view).
        assert!(att.docks.iter().all(|d| d.commit_count == 1));
    }

    #[test]
    fn r3_honest_gaps_cost_without_landing_investigated_landing_without_cost_unlabeled() {
        let b = LogBuilder::new("r3");
        b.dock("spent", "feat/noland", "worktree", "/tmp/wt-noland");
        b.dock("silent", "feat/nocost", "worktree", "/tmp/wt-nocost");
        b.sample("spent", 999_000, "r1"); // cost, no commits
        b.commit("feat/nocost", "cccc"); // commits, no cost

        let att = attribute(&b.path).unwrap();
        // R3 — spent-but-nothing-landed is INVESTIGATED, never hidden.
        assert_eq!(cost_of(&att, "spent").bucket, Bucket::Investigated);
        assert_eq!(cost_of(&att, "spent").cost_usd_micros, 999_000);
        // R3 — committed work with zero measured cost is UNLABELED, never a
        // silent zero (here with no repo-scope dock ⇒ surfaces as unlabeled).
        let silent = cost_of(&att, "silent");
        assert_eq!(silent.cost_usd_micros, 0);
        assert_eq!(silent.commit_count, 1);
        assert_eq!(silent.bucket, Bucket::Unlabeled);
    }

    #[test]
    fn m5_unbound_intents_link_to_repo_scope_dock() {
        let b = LogBuilder::new("m5");
        // A repo-scope dock (auto-coined, origin=="repo").
        b.dock("repo-scope", "main", "repo", "/repo/.git");
        // Unbound intent: a commit on a branch with NO per-branch dock.
        b.commit("feat/unbound", "eeee");

        let att = attribute(&b.path).unwrap();
        let rs = cost_of(&att, "repo-scope");
        assert_eq!(
            rs.commit_count, 1,
            "M5 — unbound intent links to repo-scope"
        );
        assert_eq!(att.repo_scope_linked_branches, vec!["feat/unbound"]);
        assert_eq!(
            att.unlabeled_commit_count, 0,
            "no unlabeled when a repo-scope dock exists"
        );
    }
}
