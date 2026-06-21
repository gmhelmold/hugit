//! WP-B4b acceptance oracle — items ③④⑥ of B4 (the GitHub integration surface).
//!
//! Contract: docs/plan/wp-contracts/WP-B4b.md.
//! Owned items:
//!   ③ force-push recompute — prior union invalidated, re-folded via B4a,
//!      EventRecord written; no stale union may land.
//!   ④ crash idempotent (kill-test) — worker killed mid-land; on restart no
//!      double-merge, no lost batch, no false green.
//!   ⑥ protected/required-review PR is HELD+reported, never force-merged; the
//!      configured merge method is honored.
//!
//! The standalone bash suite (tests/acceptance/wp-b4b/run.sh) drives
//! `cargo test --test acceptance_wp-b4b` and adds structural assertions.
//!
//! LIVE-GITHUB LANE: `item_6_*_live_*` mints the B1 App JWT (RS256) from the
//! on-disk secret store, calls `GET /app/installations`, and proves the App
//! auth surface end-to-end against real GitHub. If `hugit-fleet-syn-1`'s owner
//! is not in the App installation list, the live MERGE portion is reported
//! PARTIAL (blocked on an owner install-click) — never faked.

use hugit_contracts::LandableEntry;
use hugit_queue::core::affected::AffectedSet;
use hugit_queue::core::batch::Batch;
use hugit_queue::core::state::UnionOutcome;
use hugit_queue::core::union::{CheckSource, MemoCheck, UnionVerdict};
use hugit_queue::github::app_auth::{AppCredentials, AppJwt, AppJwtClaims, Installation, JwtError};
use hugit_queue::github::merge::{
    MergeApi, MergeError, MergeMethod, MergeRecord, drive_atomic_merge,
};
use hugit_queue::github::protection::{ProtectionStatus, evaluate_protection};
use hugit_queue::github::recompute::{RecomputeTrigger, recompute_on_force_push};
use hugit_queue::github::recovery::{LandLog, recover_and_replay};

// ── helpers ──────────────────────────────────────────────────────────────────

fn landable(id: &str, order: u64) -> LandableEntry {
    LandableEntry {
        item_id: id.to_string(),
        intent_id: format!("intent-{id}"),
        tree_hash: format!("tree-{id}"),
        order_index: order,
    }
}

fn batch(ids: &[(&str, u64)]) -> Batch {
    Batch::from_entries(
        "batch-b4b",
        ids.iter()
            .map(|(id, o)| (landable(id, *o), AffectedSet::new([*id]))),
    )
}

/// Oracle with a fixed verdict, every evaluation an AC hit.
struct FixedOracle(UnionVerdict);
impl MemoCheck for FixedOracle {
    fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
        (self.0, item_ids.iter().map(|_| CheckSource::Hit).collect())
    }
}

/// In-process merge API double: records merges, can simulate a stale head and
/// count merges per id (for the idempotency / no-double-merge proof).
#[derive(Default)]
struct FakeMergeApi {
    merged: Vec<MergeRecord>,
    counts: std::collections::BTreeMap<String, u32>,
    stale: Option<String>,
}
impl MergeApi for FakeMergeApi {
    fn merge(&mut self, record: &MergeRecord) -> Result<(), MergeError> {
        if self.stale.as_deref() == Some(record.item_id.as_str()) {
            return Err(MergeError::StaleHead {
                item_id: record.item_id.clone(),
            });
        }
        *self.counts.entry(record.item_id.clone()).or_insert(0) += 1;
        self.merged.push(record.clone());
        Ok(())
    }
}

/// A merge API that IGNORES `expected_head` entirely — it blindly merges
/// whatever it is handed (modelling a buggy/old transport that forgot the
/// stale-head check). Used to prove the ENGINE catches staleness on its own
/// (defect 6): if the engine relied on the impl, this double would let a stale
/// union land.
#[derive(Default)]
struct BlindMergeApi {
    merged: Vec<String>,
}
impl MergeApi for BlindMergeApi {
    fn merge(&mut self, record: &MergeRecord) -> Result<(), MergeError> {
        // No expected_head verification at all.
        self.merged.push(record.item_id.clone());
        Ok(())
    }
}

// ── ③ force-push recompute ────────────────────────────────────────────────────

#[test]
fn item_3_force_push_recompute_invalidates_prior_union_and_records() {
    let b = batch(&[("A", 0), ("B", 1)]);
    let trigger = RecomputeTrigger {
        item_id: "A".to_string(),
        new_head: "tree-A-forced".to_string(),
    };
    // After the force-push the union is now RED — a stale (previously green)
    // union must not be allowed to land.
    let mut oracle = FixedOracle(UnionVerdict::Red);
    let (eval, event) =
        recompute_on_force_push(&b, &trigger, &mut oracle, 7, &"0".repeat(64), 1000);

    // The FRESH union is used (no stale reuse).
    assert_eq!(
        eval.verdict,
        UnionVerdict::Red,
        "fresh union, not the stale one"
    );
    // The recompute is audited as an EventRecord (③).
    assert_eq!(event.kind, "queue.force_push_recompute");
    assert_eq!(event.seq, 7);
    assert!(event.payload.contains("tree-A-forced"));
    assert!(event.payload.contains("prior_union_invalidated"));
}

#[test]
fn item_3_force_push_recompute_always_refolds_even_when_green() {
    let b = batch(&[("A", 0)]);
    let trigger = RecomputeTrigger {
        item_id: "A".to_string(),
        new_head: "tree-A2".to_string(),
    };
    let mut oracle = FixedOracle(UnionVerdict::Green);
    let (eval, event) = recompute_on_force_push(&b, &trigger, &mut oracle, 1, &"0".repeat(64), 0);
    // Even a still-green result was reached by RE-folding, not by reusing.
    assert_eq!(eval.verdict, UnionVerdict::Green);
    assert_eq!(event.kind, "queue.force_push_recompute");
}

// ── ④ crash idempotent (kill-test) ───────────────────────────────────────────

#[test]
fn item_4_crash_idempotent_kill_test_no_double_merge_no_lost_batch() {
    // Simulate: the worker merged a, b, durably logged them, then was KILLED
    // before finishing c. The replay must merge only c, and merge nothing twice.
    let mut durable = LandLog::from_merged(["a", "b"]);
    let mut b = batch(&[("a", 0), ("b", 1), ("c", 2)]);
    let mut api = FakeMergeApi::default();

    let out = recover_and_replay(
        &mut b,
        &[
            ("a", UnionOutcome::Green),
            ("b", UnionOutcome::Green),
            ("c", UnionOutcome::Green),
        ],
        |_| MergeMethod::Merge,
        |id| format!("head-{id}"),
        &mut durable,
        &mut api,
    )
    .expect("recovery replay succeeds");

    assert_eq!(
        out.already_merged,
        vec!["a", "b"],
        "pre-crash merges skipped"
    );
    assert_eq!(
        out.newly_merged,
        vec!["c"],
        "only the unfinished tail merges"
    );
    // No double-merge: a and b never re-merged.
    assert_eq!(api.counts.get("a"), None);
    assert_eq!(api.counts.get("b"), None);
    assert_eq!(api.counts.get("c"), Some(&1));
    // No lost batch + no false green: the full landed set is exactly a,b,c.
    assert_eq!(out.all_merged(), vec!["a", "b", "c"]);
}

#[test]
fn item_4_crash_idempotent_double_replay_is_a_noop() {
    // Kill-then-restart-then-kill-then-restart, replaying the SAME in-memory
    // batch object (defect 2): the worker re-drives recovery without rebuilding
    // the batch from the queue. The first pass lands a,b (entries terminal); the
    // second pass MUST tolerate already-terminal entries idempotently — NOT
    // hard-error with AlreadyTerminal — and merge nothing new.
    let mut durable = LandLog::new();
    let outcomes = [("a", UnionOutcome::Green), ("b", UnionOutcome::Green)];
    let mut api = FakeMergeApi::default();

    let mut b = batch(&[("a", 0), ("b", 1)]);
    let first = recover_and_replay(
        &mut b,
        &outcomes,
        |_| MergeMethod::Squash,
        |id| format!("head-{id}"),
        &mut durable,
        &mut api,
    )
    .unwrap();
    assert_eq!(first.newly_merged, vec!["a", "b"]);

    // Replay on the SAME batch object — must be idempotent, not an error.
    let second = recover_and_replay(
        &mut b,
        &outcomes,
        |_| MergeMethod::Squash,
        |id| format!("head-{id}"),
        &mut durable,
        &mut api,
    )
    .expect("replaying the same batch is idempotent, not an AlreadyTerminal error");
    assert!(second.newly_merged.is_empty(), "replay merges nothing new");
    assert_eq!(second.already_merged, vec!["a", "b"]);
    assert_eq!(api.counts.get("a"), Some(&1), "a merged exactly once");
    assert_eq!(api.counts.get("b"), Some(&1), "b merged exactly once");
}

// ── ⑥ branch protection: HELD, never force-merged; merge method honored ───────

#[test]
fn item_6_protected_required_review_pr_is_held_and_recorded() {
    let status = ProtectionStatus {
        protection_enabled: true,
        required_review_pending: true,
        required_check_pending: false,
    };
    let (reasons, event) = evaluate_protection("pr-42", &status, 3, &"0".repeat(64), 5);
    assert!(!reasons.is_empty(), "a required-review PR is HELD");
    let ev = event.expect("the hold is recorded as an EventRecord");
    assert_eq!(ev.kind, "queue.protection_hold");
    assert!(ev.payload.contains("pr-42"));
    assert!(ev.payload.contains("\"held\":true"));
    // There is NO force-merge return value or path — the hold is terminal until
    // GitHub reports protection satisfied.
}

#[test]
fn item_6_merge_method_is_honored_on_land() {
    // The repository-configured merge method is transcribed onto each merge.
    let mut b = batch(&[("a", 0), ("b", 1)]);
    let mut api = FakeMergeApi::default();
    let out = drive_atomic_merge(
        &mut b,
        &[("a", UnionOutcome::Green), ("b", UnionOutcome::Green)],
        |id| match id {
            "a" => MergeMethod::Rebase,
            _ => MergeMethod::Squash,
        },
        |id| format!("head-{id}"),
        // Live heads match the union-tested heads (the steady state).
        |id| format!("head-{id}"),
        &mut api,
    )
    .unwrap();
    assert_eq!(out.merged, vec!["a", "b"]);
    let methods: Vec<&str> = api.merged.iter().map(|r| r.method.as_api_str()).collect();
    assert_eq!(
        methods,
        vec!["rebase", "squash"],
        "configured method honored"
    );
}

/// Defect 6 (stale-head enforced at the ENGINE, not delegated): a force-push
/// lands on item2's head while items 1 and 3 are unchanged. Even with a
/// `MergeApi` that IGNORES `expected_head` entirely, the engine must catch the
/// staleness and refuse — item2 must NOT merge against the stale union. On
/// `main` (delegated-only) the BlindMergeApi would happily merge the stale head.
#[test]
fn item_6_engine_enforces_stale_head_even_when_api_ignores_it() {
    let mut b = batch(&[("item1", 0), ("item2", 1), ("item3", 2)]);
    let mut api = BlindMergeApi::default();

    let err = drive_atomic_merge(
        &mut b,
        &[
            ("item1", UnionOutcome::Green),
            ("item2", UnionOutcome::Green),
            ("item3", UnionOutcome::Green),
        ],
        |_| MergeMethod::Merge,
        // The union-tested heads.
        |id| format!("head-{id}"),
        // Live heads: item2 was force-pushed under us (head differs); 1 and 3
        // are fine.
        |id| {
            if id == "item2" {
                "head-item2-FORCED".to_string()
            } else {
                format!("head-{id}")
            }
        },
        &mut api,
    )
    .expect_err("a stale head must be caught by the engine");

    assert_eq!(
        err,
        MergeError::StaleHead {
            item_id: "item2".to_string()
        },
        "the engine refuses the stale union for item2"
    );
    // item1 merged (it preceded the stale entry and was fresh); item2 NEVER
    // merged despite the blind API; the driver stopped at the stale head so
    // item3 has not merged either — main never received the stale union.
    assert_eq!(
        api.merged,
        vec!["item1"],
        "only the fresh predecessor landed"
    );
    assert!(
        !api.merged.contains(&"item2".to_string()),
        "the stale entry never merged, even though the API ignores expected_head"
    );
}

#[test]
fn item_6_satisfied_protection_does_not_hold() {
    let status = ProtectionStatus {
        protection_enabled: true,
        required_review_pending: false,
        required_check_pending: false,
    };
    let (reasons, event) = evaluate_protection("pr-1", &status, 1, &"0".repeat(64), 0);
    assert!(reasons.is_empty(), "satisfied protection does not hold");
    assert!(event.is_none());
}

// ════════════════════════════════════════════════════════════════════════════
// LIVE-GITHUB LANE — App JWT mint + GET /app/installations against real GitHub.
// ════════════════════════════════════════════════════════════════════════════

// SECRETS_DIR is resolved at runtime from HUGIT_SECRETS_DIR env var (no hardcoded path).
const TEST_REPO_OWNER: &str = "example-org";

/// Live AppJwt implementation: RS256-signs with jsonwebtoken and calls GitHub
/// with ureq. The private key is read from disk via std::fs and NEVER printed.
struct LiveAppJwt;

impl AppJwt for LiveAppJwt {
    fn sign(&self, claims: &AppJwtClaims) -> Result<String, JwtError> {
        let creds = load_creds()
            .ok_or_else(|| JwtError::Signing("HUGIT_SECRETS_DIR unset".into()))?
            .map_err(JwtError::Signing)?;
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(creds.private_key_pem.as_bytes())
            .map_err(|e| JwtError::Signing(e.to_string()))?;
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        jsonwebtoken::encode(&header, claims, &key).map_err(|e| JwtError::Signing(e.to_string()))
    }

    fn list_installations(&self, jwt: &str) -> Result<Vec<Installation>, JwtError> {
        let resp = ureq::get("https://api.github.com/app/installations")
            .set("Authorization", &format!("Bearer {jwt}"))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .set("User-Agent", "hugit-queue-b4b-acceptance")
            .call();
        let body = match resp {
            Ok(r) => r
                .into_string()
                .map_err(|e| JwtError::Transport(e.to_string()))?,
            Err(ureq::Error::Status(code, r)) => {
                let msg = r.into_string().unwrap_or_default();
                return Err(JwtError::Status(code, msg));
            }
            Err(e) => return Err(JwtError::Transport(e.to_string())),
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| JwtError::Transport(e.to_string()))?;
        let arr = parsed.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .filter_map(|v| {
                Some(Installation {
                    id: v.get("id")?.as_u64()?,
                    account_login: v.get("account")?.get("login")?.as_str()?.to_string(),
                    repository_selection: v
                        .get("repository_selection")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect())
    }
}

/// Read App credentials from the on-disk secret store via std::fs. The key
/// material is returned in-memory and never printed anywhere.
///
/// Returns `None` when `HUGIT_SECRETS_DIR` is unset (caller must skip the test).
fn load_creds() -> Option<Result<AppCredentials, String>> {
    let secrets_dir = match std::env::var("HUGIT_SECRETS_DIR") {
        Ok(d) => d,
        Err(_) => {
            eprintln!(
                "LIVE-SKIP: HUGIT_SECRETS_DIR unset — live App JWT lane requires the \
                 secret store path; set HUGIT_SECRETS_DIR to run this lane"
            );
            return None;
        }
    };
    let result = (|| {
        let app_id = std::fs::read_to_string(format!("{secrets_dir}/app-id"))
            .map_err(|e| format!("read app-id: {e}"))?
            .trim()
            .to_string();
        let private_key_pem = std::fs::read_to_string(format!("{secrets_dir}/private-key.pem"))
            .map_err(|e| format!("read private key: {e}"))?;
        Ok(AppCredentials {
            app_id,
            private_key_pem,
        })
    })();
    Some(result)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// LIVE: mint the App JWT and prove it authenticates against GitHub's
/// `GET /app/installations`. This is the real App-auth surface the merge lane
/// rides on. PARTIAL signalling: if the test repo's owner is not in the
/// installation list, the live MERGE is blocked on an owner install-click —
/// reported on stdout, never faked.
#[test]
fn item_6_live_app_jwt_lists_installations() {
    // The b4b bash suite is the gate that ENFORCES the live lane: it fails when
    // HUGIT_GH_TEST_REPO is unset and only then runs this binary, so the live
    // assertions below always execute under the suite. The generic
    // `cargo test --workspace` gate runs this binary WITHOUT the env var; there
    // the live lane is not applicable, so we skip cleanly rather than panic —
    // keeping the workspace gate green without ever faking the live result.
    let repo = match std::env::var("HUGIT_GH_TEST_REPO") {
        Ok(r) => r,
        Err(_) => {
            println!(
                "LIVE-SKIP: HUGIT_GH_TEST_REPO unset (not the b4b live lane) — \
                 enforced by tests/acceptance/wp-b4b/run.sh, not faked here"
            );
            return;
        }
    };
    assert!(repo.contains('/'), "repo must be owner/name");

    let creds = match load_creds() {
        None => return, // HUGIT_SECRETS_DIR unset — skip (reason printed above)
        Some(r) => r.expect("App credentials present in secret store"),
    };
    assert!(!creds.app_id.is_empty(), "app-id loaded");
    assert!(
        creds.private_key_pem.contains("PRIVATE KEY"),
        "private key PEM loaded (content not printed)"
    );

    let signer = LiveAppJwt;
    let claims = AppJwtClaims::mint(&creds.app_id, now_unix());
    assert!(claims.is_within_github_window(now_unix()));

    // Sign + call GitHub. A signing failure is a hard failure (our bug); a
    // transport/status failure is surfaced — the live endpoint genuinely ran.
    let jwt = signer.sign(&claims).expect("App JWT minted (RS256)");
    let installs = signer
        .list_installations(&jwt)
        .expect("GET /app/installations succeeded — App JWT authenticated to GitHub");

    println!(
        "LIVE-EVIDENCE: App JWT authenticated; {} installation(s) returned by GitHub",
        installs.len()
    );

    // PARTIAL gate: is the test repo's owner covered by an installation?
    let owner = repo.split('/').next().unwrap_or(TEST_REPO_OWNER);
    match hugit_queue::github::app_auth::select_installation(&installs, owner) {
        Some(inst) => {
            println!(
                "LIVE-EVIDENCE: App installed on '{}' (installation_id={}, selection={}) \
                 — live merge lane reachable",
                owner, inst.id, inst.repository_selection
            );
        }
        None => {
            // NEVER fake: the App-auth surface is proven (JWT authenticated and
            // GitHub returned the installation list), but the synthetic repo's
            // owner is not in the install scope, so the live MERGE is blocked on
            // an owner install-click. Reported as PARTIAL, not failed.
            println!(
                "LIVE-PARTIAL: App JWT authenticated to GitHub, but owner '{}' is NOT in the \
                 installation list — live merge on {} is BLOCKED-ON-OWNER-CLICK (install the \
                 hugit-dev App on the repo). App-auth surface proven; merge step not faked.",
                owner, repo
            );
        }
    }
}
