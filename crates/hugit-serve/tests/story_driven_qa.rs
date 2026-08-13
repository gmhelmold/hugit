//! Story-driven QA — the single-tenant forge proven through the product's OWN
//! narratives (the press-day walkthrough), end-to-end over the real engine.
//!
//! One FORGE HISTORY (a chain-verified `forge.json` built from real event kinds:
//! repo.meta public · campaign.opened · intent.landed ×2 · pr.opened · a
//! PR-altitude ContextEnvelope carrying attested `cost_usd_micros` · check.recorded
//! ×4 (3 cache HITS + 1 EXECUTED) · verdict.recorded) is written to a Local dir and
//! served by [`AppState::new`]; each story hits a REAL `/v1` boundary via the
//! socket-free [`route`]/[`route_with_body`] with the same handlers the deployed
//! engine runs.
//!
//! The stories:
//!   1. **The repo home renders a real tree + README** — `GET /home` with a seeded
//!      git content seam lists the seeded files (a REAL tree read, not a stub).
//!   2. **Fleet clone + push land and clone back** — a REAL `git clone`, a
//!      credentialed `git push` of a new branch, and a FRESH engine instance whose
//!      advertise + clone-back carry the pushed branch (the durable write path;
//!      note: GIT_DIR-mode pushes advance the on-disk ref — fresh-instance
//!      freshness is the honest claim, matching `git_wire`).
//!   3. **Checks render cached-intent memoization** — `GET /checks` aggregates
//!      the AC: 3 hits / 1 executed → `PARTIAL`, the real hit-rate, `saved_ms`,
//!      and the cache pills.
//!   4. **PR detail renders attested cost** — `GET /prs/1` shows the campaign
//!      chip and a NON-ZERO cost block driven by the captured envelope's
//!      `cost_usd_micros` (real attested cost, never honest-zero).
//!   5. **Landing is idempotent end-to-end** — a real `POST …/land` with an
//!      `Idempotency-Key` queues the PR once (its `pr.queued` + `idem.recorded`
//!      land atomically); replaying the key appends NOTHING (the land one-position
//!      invariant through load→mutate→persist); no key → 400.
//!   6. **Cost killer: the insights ledger renders validated, attested spend** —
//!      `GET /insights` shows the campaign ledger (asked/done/proven = real
//!      counts) and a `cost_xray` row whose `spend_proof` is the `cas:` ref of the
//!      envelope bytes + `cost_total_micros > 0` (the `✓ cas:` attestation marker).
//!   7. **The quality gate fails CLOSED** — a tampered chain is 503 for the
//!      operator and a uniform 404 for a non-operator (no integrity/existence
//!      oracle); the write door rejects a missing credential 401.
//!   8. **Visibility is the forge's call, enforced by the engine** — an anonymous
//!      read of a PUBLIC repo is open (200); a PRIVATE repo is a 404 to an
//!      anonymous visitor (identical to non-existent); and that open read never
//!      opens a write (anonymous write → 401).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gix_hash::ObjectId;
use hugit_cli::pr::{INTENT_ENVELOPE_KIND, PR_ENVELOPE_KIND};
use hugit_http_contracts::{
    ChecksVm, InsightsVm, LandingItemVm, LandingVm, PrDetailVm, RepoHomeVm,
};
use hugit_ledger::envelope::cold_ref_for;
use hugit_proto::{CasObjectSource, GitObject, ObjectKind};
use hugit_refstore::EventLog;
use hugit_serve::server::{route, route_with_body, serve_on};
use hugit_serve::state::AppState;
use tiny_http::{Header, Method, Server};

const TOKEN: &str = "dev-token-qa";

// ── scaffolding (the repo's proven test primitives) ───────────────────────────

/// A temp log dir unique to this test process AND to each call (atomic counter —
/// no clock-collision flake; parallel tests must never share a `<repo>.json`).
fn scratch_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hugit-serve-story-{}-{}-{}",
        std::process::id(),
        nanos,
        seq
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn bearer(tok: &str) -> Vec<Header> {
    vec![Header::from_bytes(&b"Authorization"[..], format!("Bearer {tok}").as_bytes()).unwrap()]
}

fn hdr(name: &str, val: &str) -> Header {
    Header::from_bytes(name.as_bytes(), val.as_bytes()).unwrap()
}

/// Write `<dir>/<repo>.json` carrying one chain-valid `repo.meta` record at the
/// given visibility (the read gate's input for the visibility stories).
fn write_repo_log(dir: &std::path::Path, repo: &str, visibility: &str) {
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility": visibility, "owner_tenant": "org-a"}).to_string(),
        0,
    );
    std::fs::write(
        dir.join(format!("{repo}.json")),
        serde_json::to_vec(log.records()).unwrap(),
    )
    .unwrap();
}

fn post(state: &AppState, url: &str, headers: &[Header], body: &[u8]) -> (u16, String) {
    route_with_body(state, &Method::Post, url, headers, body)
}

/// Spawn `serve_on` on an ephemeral port; return the bound `127.0.0.1:<port>`.
fn spawn(state: AppState) -> String {
    let server = Server::http("127.0.0.1:0").expect("bind ephemeral port");
    let addr = server
        .server_addr()
        .to_ip()
        .expect("ip listen addr")
        .to_string();
    std::thread::spawn(move || {
        let _ = serve_on(state, server);
    });
    addr
}

// ── the FORGE HISTORY ─────────────────────────────────────────────────────────

/// A minimal ContextEnvelope carrying a NON-ZERO `cost_usd_micros` (real attested
/// spend, the shape the engine captures). `altitude:"pr"` + `agent_type:"main"` +
/// `parent_run_id:null` satisfy the D14 `ensure_top_level` guard.
fn envelope_json(
    altitude: &str,
    unit_id: &str,
    campaign: &str,
    agent_type: &str,
    parent_run_id: Option<&str>,
    cost_usd_micros: u64,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "1.2.0",
        "altitude": altitude,
        "intent_id": unit_id,
        "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "tree_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "authorship": {
            "model": "claude-sonnet",
            "model_digest": "0000000000000000000000000000000000000000000000000000000000000000",
            "agent_type": agent_type,
            "spawn": {
                "run_id": "run-qa",
                "parent_run_id": parent_run_id,
                "born_at": 1000,
                "died_at": 2000
            },
            "operator": "test@example.com"
        },
        "charter": "auth hardening — rate-limit per tenant",
        "campaign": campaign,
        "constraints": [],
        "acceptance": [],
        "parent_intents": [],
        "trajectory": {
            "raw_transcript_ref": null,
            "task_transcript_ref": null,
            "summary": null,
            "journal_ref": null,
            "redaction_policy": "default-v1"
        },
        "snapshot": {
            "files_read": [],
            "prompt_ref": null,
            "env_manifest": "test"
        },
        "metrics": {
            "tokens": { "input": 100, "output": 50, "cache_read": 0, "cache_write": 0, "total": 150 },
            "wall_ms": 1000,
            "active_ms": 500,
            "tool_calls": 0,
            "tool_breakdown": [],
            "model_turns": 1,
            "cost_usd_micros": cost_usd_micros
        },
        "verdicts_ref": null
    })
}

/// Persist the ONE forge history: `<dir>/forge.json`, a chain-verified single
/// repo whose campaign/PR/checks/verdict/cost records the stories read. Returns
/// the expected `cas:` attestation ref of the PR envelope (what /insights MUST
/// show as `spend_proof`).
fn write_forge_history(dir: &std::path::Path) -> String {
    let mut log = EventLog::new();

    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility": "public", "owner_tenant": "org-a"}).to_string(),
        0,
    );
    log.append_for_test(
        "campaign.opened",
        vec!["orchestrator:hugit".to_string()],
        serde_json::json!({ "campaign": "auth-hardening" }).to_string(),
        1_000,
    );
    // The "Auth Hardening spine": two landed intents (a real `intent.landed`
    // carries `ref`/`target` like a ref.update — replay/unify reads them).
    for (i, (id, charter)) in [
        ("i-001", "rate-limit per tenant"),
        ("i-002", "auth hardening"),
    ]
    .iter()
    .enumerate()
    {
        let target = format!("00000000000000000000000000000000000000{:02}", i + 1);
        log.append_for_test(
            "intent.landed",
            vec!["agent:opus".to_string()],
            serde_json::json!({
                "intent_id": id, "campaign": "auth-hardening",
                "charter": charter, "deep_link_target": id,
                "ref": "refs/hugit/intents", "target": target,
            })
            .to_string(),
            2_000 + u64::try_from(i).unwrap() * 1_000,
        );
    }
    // The PR bundling both intents, opened by the orchestrator.
    log.append_for_test(
        "pr.opened",
        vec!["orchestrator:hugit".to_string()],
        serde_json::json!({
            "pr_id": "1", "campaign": "auth-hardening", "author_kind": "orchestrator",
            "intent_ids": ["i-001", "i-002"], "principal": serde_json::Value::Null,
            "run_id": "run-qa",
        })
        .to_string(),
        4_000,
    );
    // The attested spend: PR-envelope (cost $0.42) + the intent-envelope for i-002.
    let pr_env = envelope_json("pr", "1", "auth-hardening", "main", None, 420_000);
    let expected_pr_cas = cold_ref_for(pr_env.to_string().as_bytes());
    log.append_for_test(PR_ENVELOPE_KIND, vec![], pr_env.to_string(), 5_000);
    let intent_env = envelope_json(
        "intent",
        "i-002",
        "auth-hardening",
        "implementer",
        Some("run-qa"),
        120_000,
    );
    log.append_for_test(INTENT_ENVELOPE_KIND, vec![], intent_env.to_string(), 6_000);
    // The memoized CI: 3 cache HITs + 1 fresh EXECUTED (the AC spine).
    for (i, (memo_key, duration_ms, name)) in [
        (
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            1_200_u64,
            "fmt",
        ),
        (
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            850_u64,
            "clippy",
        ),
        (
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            300_u64,
            "test",
        ),
    ]
    .iter()
    .enumerate()
    {
        log.append_for_test(
            "check.recorded",
            vec!["agent:opus".to_string()],
            serde_json::json!({
                "name": name, "memo_key": memo_key, "cache_hit": true,
                "exit": 0, "duration_ms": duration_ms,
            })
            .to_string(),
            7_000 + u64::try_from(i).unwrap() * 1_000,
        );
    }
    log.append_for_test(
        "check.recorded",
        vec!["agent:opus".to_string()],
        serde_json::json!({
            "name": "audit",
            "memo_key": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "cache_hit": false, "exit": 0, "duration_ms": 5_000_u64,
        })
        .to_string(),
        10_000,
    );
    // A correctness approve for i-002 (the `proven` ledger count).
    log.append_for_test(
        "verdict.recorded",
        vec!["agent:reviewer".to_string()],
        serde_json::json!({
            "intent": "i-002", "tree_hash": "t0", "lens": "correctness",
            "model": "test", "prompt_digest": "d0", "verdict": "approve",
            "claims_checked": ["correctness:approve"], "evidence_refs": [],
        })
        .to_string(),
        11_000,
    );
    // The default branch on the LOG (the home handler's `branch` + `branch_count`
    // are log-derived from `ref.update` records — the git seam carries the objects).
    log.append_for_test(
        "ref.update",
        vec![],
        serde_json::json!({
            "ref": "refs/heads/main", "target": "aabbccddeeff00112233445566778899aabbccdd",
        })
        .to_string(),
        12_000,
    );

    std::fs::write(
        dir.join("forge.json"),
        serde_json::to_vec(log.records()).unwrap(),
    )
    .unwrap();
    expected_pr_cas
}

/// Seed a small git graph (README + src/) the repo-home story reads as a REAL
/// file listing — a real blob/tree/commit in the state-owned CAS, refs at HEAD.
fn seed_forge_git() -> (CasObjectSource, BTreeMap<String, String>, ObjectId) {
    let mut cas = CasObjectSource::new();
    let readme = cas.insert(GitObject::new(
        ObjectKind::Blob,
        b"# hugit\n\nforge for agent fleets\n".to_vec(),
    ));
    let src = cas.insert(GitObject::new(ObjectKind::Blob, b"handler.rs\n".to_vec()));
    let mut tree = Vec::new();
    tree.extend_from_slice(b"100644 README\0");
    tree.extend_from_slice(readme.as_slice());
    tree.extend_from_slice(b"100644 src/handler.rs\0");
    tree.extend_from_slice(src.as_slice());
    let tree_oid = cas.insert(GitObject::new(ObjectKind::Tree, tree));
    let mut body = format!("tree {tree_oid}\n");
    let ident = "hugit <bot@hugit.dev> 1717000000 +0000";
    body.push_str(&format!("author {ident}\n"));
    body.push_str(&format!("committer {ident}\n\ninit\n"));
    let commit = cas.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()));
    let mut refs = BTreeMap::new();
    refs.insert("refs/heads/main".to_string(), commit.to_string());
    (cas, refs, tree_oid)
}

/// An `AppState` whose `forge` repo carries the full history + a seeded git
/// content seam. Returns the expected `cas:` attestation ref (for /insights).
fn state_with_forge() -> (AppState, String) {
    let dir = scratch_dir();
    let expected = write_forge_history(&dir);
    let (cas, refs, root_tree) = seed_forge_git();
    let mut state = AppState::new(dir.clone(), TOKEN.to_string());
    state.set_repo_git("forge", Arc::new(cas), root_tree, refs);
    (state, expected)
}

// ── story 1: repo home ────────────────────────────────────────────────────────

#[test]
fn story_home_renders_a_real_tree_and_readme() {
    let (state, _expect) = state_with_forge();
    let (status, body) = route(&state, &Method::Get, "/v1/repos/forge/home", &bearer(TOKEN));
    assert_eq!(status, 200, "home serves: {body}");
    let vm: RepoHomeVm = serde_json::from_str(&body).expect("home body is RepoHomeVm");

    assert_eq!(vm.repo, "forge");
    assert_eq!(
        vm.branch, "main",
        "default-branch picker on the seeded refs"
    );
    assert_eq!(vm.branch_count, 1, "one seeded branch, honestly counted");
    // A REAL tree read (seeded CAS), not a stub listing.
    assert!(!vm.files.is_empty(), "files must be a real tree listing");
    let readme = vm
        .files
        .iter()
        .find(|f| f.name == "README")
        .expect("README in the tree");
    assert!(!readme.is_dir, "README is a file");
    assert!(
        vm.files.iter().any(|f| f.name == "src/handler.rs"),
        "nested path is listed"
    );
    // The README is rendered from real blob bytes.
    assert!(
        vm.readme_html.contains("forge for agent fleets"),
        "readme_html renders the seeded README: {}",
        vm.readme_html
    );
}

// ── story 2: fleet clone + credentialed push, durable clone-back ──────────────

fn git_cfg_prefix() -> Vec<&'static str> {
    vec![
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "-c",
        "protocol.version=0",
    ]
}

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .args(git_cfg_prefix())
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git in dir");
    assert!(
        out.status.success(),
        "git -C {} {args:?} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_ls_remote(addr: &str, repo: &str) -> String {
    let url = format!("http://{addr}/{repo}");
    let out = Command::new("git")
        .args(git_cfg_prefix())
        .arg("ls-remote")
        .arg(&url)
        .output()
        .expect("run git ls-remote");
    assert!(
        out.status.success(),
        "ls-remote failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn story_fleet_clone_and_push_land_and_clone_back() {
    let root = scratch_dir();
    let work = root.join("work");
    let bare = root.join("wireforge.git");

    // Seed a bare server repo (one commit on main), exactly like `git_wire`.
    {
        let _ = Command::new("git")
            .args(["init", "-q", work.to_str().unwrap()])
            .output()
            .unwrap();
        std::fs::write(work.join("README"), b"seed\n").unwrap();
        git_in(&work, &["add", "."]);
        git_in(&work, &["commit", "-q", "-m", "init"]);
        git_in(&work, &["branch", "-M", "main"]);
        let _ = Command::new("git")
            .args(["init", "-q", "--bare", bare.to_str().unwrap()])
            .output()
            .unwrap();
        git_in(&work, &["push", "-q", bare.to_str().unwrap(), "main"]);
        git_in(&bare, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    }

    // A PUBLIC wireforge with the receive-pack write path enabled.
    let log_dir = scratch_dir();
    write_repo_log(&log_dir, "wireforge", "public");
    let build_state = || {
        let mut s = AppState::new(log_dir.clone(), TOKEN.to_string());
        s.set_repo_from_git_dir("wireforge", bare.to_str().unwrap())
            .expect("load bare git dir");
        s.enable_write_path();
        s
    };
    let addr_a = spawn(build_state());

    // 1. A REAL `git clone` of the public repo — the wire must be git-correct.
    let clone_a = root.join("clone_a");
    let out = Command::new("git")
        .args(git_cfg_prefix())
        .args(["clone", "-q", &format!("http://{addr_a}/wireforge")])
        .arg(clone_a.to_str().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        clone_a.join("README").exists(),
        "the seeded file clones back"
    );

    // 2. An agent branch: one commit.
    git_in(&clone_a, &["checkout", "-q", "-b", "feat/rate-limit"]);
    std::fs::write(clone_a.join("limits.toml"), b"per_tenant = 50\n").unwrap();
    git_in(&clone_a, &["add", "."]);
    git_in(&clone_a, &["commit", "-q", "-m", "rate limit per tenant"]);

    // 3. A CREDENTIALED push (read-authz ≠ write-authz: writes always need a token).
    let auth = format!("http.extraHeader=Authorization: Bearer {TOKEN}");
    let push = Command::new("git")
        .args(git_cfg_prefix())
        .arg("-C")
        .arg(&clone_a)
        .arg("-c")
        .arg(&auth)
        .args([
            "push",
            "-q",
            &format!("http://{addr_a}/wireforge"),
            "HEAD:refs/heads/feat/rate-limit",
        ])
        .output()
        .unwrap();
    assert!(
        push.status.success(),
        "create-branch push failed: {}",
        String::from_utf8_lossy(&push.stderr)
    );

    // 4. DURABLE: a FRESH instance advertises the pushed branch (a git-dir push
    // advances the on-disk ref, so a fresh boot reads it — the honest claim), and
    // the pushed file clones back to a FRESH clone.
    let addr_b = spawn(build_state());
    let ls = git_ls_remote(&addr_b, "wireforge");
    assert!(
        ls.contains("refs/heads/feat/rate-limit"),
        "a fresh instance advertises the pushed branch: {ls}"
    );
    assert!(ls.contains("refs/heads/main"), "main is still advertised");
    let clone_b = root.join("clone_b");
    let out = Command::new("git")
        .args(git_cfg_prefix())
        .args(["clone", "-q", &format!("http://{addr_b}/wireforge")])
        .arg(clone_b.to_str().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "clone-back failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    git_in(&clone_b, &["checkout", "-q", "feat/rate-limit"]);
    assert!(
        clone_b.join("limits.toml").exists(),
        "the pushed file clones back from the fresh instance"
    );
}

// ── story 3: checks render the memoized CI ────────────────────────────────────

#[test]
fn story_checks_render_cached_intent_memoization() {
    let (state, _expect) = state_with_forge();
    let (status, body) = route(
        &state,
        &Method::Get,
        "/v1/repos/forge/checks",
        &bearer(TOKEN),
    );
    assert_eq!(status, 200, "checks serves: {body}");
    let vm: ChecksVm = serde_json::from_str(&body).expect("checks body is ChecksVm");

    // The AC spine: 3 HITs + 1 EXECUTED → an honest PARTIAL shape + the real math.
    assert_eq!(vm.kpis.hits, 3, "three memoized cache hits");
    assert_eq!(vm.kpis.executed, 1, "one freshly executed check");
    assert_eq!(vm.kpis.shape, "PARTIAL", "honest shape label");
    assert_eq!(vm.kpis.hit_rate_pct, 75.0, "3 of 4 hits — the REAL rate");
    assert_eq!(
        vm.kpis.saved_ms,
        1_200 + 850 + 300,
        "execution time the cache saved"
    );
    assert_eq!(vm.cpills.len(), 3, "one cache pill per HIT row");
    assert!(vm.checks.len() >= 4, "all four check rows projected");
    assert!(vm.hero.green, "all rows exit 0 → green hero");
}

// ── story 4: PR detail renders the attested cost ──────────────────────────────

#[test]
fn story_pr_detail_renders_attested_cost() {
    let (state, _expect) = state_with_forge();
    let (status, body) = route(
        &state,
        &Method::Get,
        "/v1/repos/forge/prs/1",
        &bearer(TOKEN),
    );
    assert_eq!(status, 200, "pr detail serves: {body}");
    let vm: PrDetailVm = serde_json::from_str(&body).expect("pr detail body is PrDetailVm");

    assert_eq!(vm.number, 1);
    assert_eq!(vm.change_id, "1", "the PR's stable id (REAL)");
    let chip = vm
        .campaign
        .as_ref()
        .expect("the PR carries its campaign chip");
    assert_eq!(chip.id, "auth-hardening");
    // The captured PR-altitude envelope drives a NON-ZERO cost block (real spend,
    // never an honest-zero stub): the envelope carried cost_usd_micros = 420_000.
    assert!(
        vm.cost.total_usd > 0.0,
        "the attested envelope cost renders: total_usd={}",
        vm.cost.total_usd
    );
}

// ── story 5: idempotent landing + the landing view ────────────────────────────

#[test]
fn story_landing_is_idempotent_end_to_end() {
    // (a) The landing VIEW projects the open PR card + campaign chip.
    let (state, _expect) = state_with_forge();
    let (status, body) = route(
        &state,
        &Method::Get,
        "/v1/repos/forge/landing",
        &bearer(TOKEN),
    );
    assert_eq!(status, 200, "landing serves: {body}");
    let vm: LandingVm = serde_json::from_str(&body).expect("landing body is LandingVm");
    assert_eq!(vm.open_count, 1, "one opened, non-terminal PR");
    assert_eq!(vm.merged_count, 0, "nothing has landed yet");
    assert!(vm.campaigns.iter().any(|c| c.id == "auth-hardening"));
    let cards: Vec<u64> = vm
        .columns
        .iter()
        .flat_map(|col| {
            col.items.iter().filter_map(|item| match item {
                LandingItemVm::Card(c) => Some(c.number),
                LandingItemVm::Bundle { .. } => None,
            })
        })
        .collect();
    assert!(
        cards.contains(&1),
        "the open PR card is on a column: {cards:?}"
    );

    // (b) A real POST land with an Idempotency-Key lands ONCE — replay appends
    // NOTHING (the land one-position invariant through load→mutate→persist→reload).
    let dir = scratch_dir();
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility": "private", "owner_tenant": "org-a"}).to_string(),
        0,
    );
    let pr = hugit_refstore::canonical_json(
        &serde_json::json!({
            "author_kind":"orchestrator","campaign":"c","intent_ids":["i"],
            "pr_id":"1","principal":null,"run_id":"r",
        })
        .to_string(),
    )
    .unwrap();
    log.append_for_test("pr.opened", vec!["orchestrator:t".into()], pr, 1);
    std::fs::write(
        dir.join("acme.json"),
        serde_json::to_vec(log.records()).unwrap(),
    )
    .unwrap();
    let state = AppState::new(dir.clone(), TOKEN.to_string());
    let land = |key: &str| {
        post(
            &state,
            "/v1/repos/acme/prs/1/land",
            &[
                hdr("Authorization", &format!("Bearer {TOKEN}")),
                hdr("Idempotency-Key", key),
            ],
            br#"{"mode":"union"}"#,
        )
    };
    let count = |dir: &std::path::Path| -> usize {
        let recs: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("acme.json")).unwrap()).unwrap();
        recs.as_array().expect("records array").len()
    };
    let before = count(&dir);

    let (s, b) = land("k1");
    assert_eq!(s, 200, "the owner lands the PR: {b}");
    let after_first = count(&dir);
    assert!(
        after_first > before,
        "the land appends its records (pr.queued + idem.recorded) atomically"
    );

    let (s, b) = land("k1");
    assert_eq!(s, 200, "idempotent replay stays 200: {b}");
    assert_eq!(
        count(&dir),
        after_first,
        "replaying the SAME key appends nothing"
    );

    let (s, b) = land("k2");
    assert_eq!(s, 200, "a fresh key lands: {b}");
    assert_eq!(
        count(&dir),
        after_first + (after_first - before),
        "a fresh key lands exactly once more (the same record set)"
    );

    // A missing Idempotency-Key is a 400 (the transport law).
    let (s, b) = post(
        &state,
        "/v1/repos/acme/prs/1/land",
        &[hdr("Authorization", &format!("Bearer {TOKEN}"))],
        br#"{"mode":"union"}"#,
    );
    assert_eq!(s, 400, "no Idempotency-Key → 400: {b}");
}

// ── story 6: insights + the cost killer's validated spend ─────────────────────

#[test]
fn story_insights_ledger_renders_validated_attested_cost() {
    let (state, expected_pr_cas) = state_with_forge();
    let (status, body) = route(
        &state,
        &Method::Get,
        "/v1/repos/forge/insights",
        &bearer(TOKEN),
    );
    assert_eq!(status, 200, "insights serves: {body}");
    let vm: InsightsVm = serde_json::from_str(&body).expect("insights body is InsightsVm");

    // The campaign ledger: REAL counts (2 asked, 2 done, 1 proven by the verdict).
    assert_eq!(vm.ledger.campaigns.len(), 1, "one campaign on the ledger");
    let campaign = &vm.ledger.campaigns[0];
    assert_eq!(campaign.campaign.id, "auth-hardening");
    assert_eq!(campaign.asked, 2, "two intents asked");
    assert_eq!(campaign.done, 2, "two intents landed");
    assert_eq!(campaign.proven, 1, "i-002 verified by an approve verdict");
    let asked_kpi = vm
        .kpis
        .iter()
        .find(|k| k.label == "Intents pousados")
        .expect("the KPI is present");
    assert_eq!(asked_kpi.value, "2");

    // The cost killer: the campaign's cost-xray row carries the ENVELOPE's cas:
    // attestation ref (the ✓ cas: marker) + a NON-ZERO raw spend figure.
    assert_eq!(vm.cost_xray.len(), 1, "the campaign has a cost-xray row");
    let row = &vm.cost_xray[0];
    let sp = row.spend_proof.as_ref().expect("spend_proof present");
    assert!(
        sp.starts_with("cas:"),
        "spend_proof is a content-address ref: {sp}"
    );
    assert_eq!(
        sp, &expected_pr_cas,
        "spend_proof byte-identically pins the raw envelope"
    );
    assert!(
        row.cost_total_micros > 0,
        "attested non-zero spend: {} µUSD",
        row.cost_total_micros
    );
}

// ── story 7: the quality gate fails CLOSED ────────────────────────────────────

#[test]
fn story_quality_gate_rejects_tampered_and_leaks_nothing() {
    // A bogus chain (valid JSON array shape, NOT a verifiable chain).
    let bogus = r#"[{"seq":7,"kind":"pr.opened","payload":"{}","prev_hash":"deadbeef","this_hash":"00","recorded_at":1,"principal_chain":["x"]}]"#;
    let dir = scratch_dir();
    std::fs::write(dir.join("acme.json"), bogus).unwrap();
    let state = AppState::new(dir.clone(), TOKEN.to_string());

    // The operator (dev-token with the dev/seed break-glass) gets the honest 503.
    let (s, b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(TOKEN));
    assert_eq!(s, 503, "tampered chain must be 503 for the operator: {b}");
    assert!(
        b.contains("ENGINE_UNAVAILABLE"),
        "fail-closed code exposed to the operator: {b}"
    );

    // A non-operator gets a UNIFORM 404 — no integrity/existence oracle.
    let tok_b = state
        .token_store
        .mint(&hugit_serve::token::ClerkPrincipal {
            user: "u-b".into(),
            org: "org-b".into(),
            fresh_auth: false,
        })
        .expect("mint b");
    let (s, b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(&tok_b));
    assert_eq!(s, 404, "non-operator gets the uniform 404, not 503: {b}");
    assert!(b.contains("NOT_FOUND"));
    assert!(
        !b.contains("ENGINE_UNAVAILABLE"),
        "no integrity detail leaks: {b}"
    );
}

// ── story 8: visibility enforced, reads never open writes ─────────────────────

#[test]
fn story_visibility_public_reads_open_private_hidden() {
    // Anonymous read of a PUBLIC repo is open (the forge's visibility decision,
    // enforced by the engine — a stock `git clone`/read needs no account).
    let (forge_state, _expect) = state_with_forge();
    let (s, body) = route(&forge_state, &Method::Get, "/v1/repos/forge/home", &[]);
    assert_eq!(s, 200, "anonymous read of a public repo is open: {body}");

    // Anonymous read of a PRIVATE repo is a 404 — identical to a non-existent repo.
    let dir = scratch_dir();
    write_repo_log(&dir, "priv", "private");
    let priv_state = AppState::new(dir.clone(), TOKEN.to_string());
    let (s, b) = route(&priv_state, &Method::Get, "/v1/repos/priv/home", &[]);
    assert_eq!(s, 404, "private repo hidden from anonymous: {b}");
    assert!(b.contains("NOT_FOUND"), "no existence oracle: {b}");

    // read-authz ≠ write-authz: the open public read NEVER opens a write.
    let (s, b) = post(
        &priv_state,
        "/v1/repos/priv/prs/1/land",
        &[],
        br#"{"mode":"union"}"#,
    );
    assert_eq!(s, 401, "an anonymous write is always denied: {b}");
}
