//! # Harness — the live engine + the real user surfaces
//!
//! Exactly the plumbing `ultimate_qa.rs` proves: a REAL `serve_on` accept loop on a
//! REAL TCP port, a REAL `git` client over the smart-HTTP wire, and the REAL
//! `hugit` CLI binary pointed at the SAME canonical `<log_dir>/<repo>.json` the
//! server serves from. The world is BUILT by the product (`POST /v1/repos`, `git
//! push`, `hugit` verbs) — never by internal builders. This file owns BOOTING the
//! engine and the low-level surface verbs; the journey executor ([`crate::journey`])
//! orchestrates them into goal-driven steps and captures [`crate::evidence`].

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gix_hash::ObjectId;
use hugit_proto::{CasObjectSource, GitObject, ObjectKind};
use hugit_refstore::EventLog;
use hugit_serve::server::serve_on;
use hugit_serve::state::AppState;
use hugit_serve::token::{ClerkPrincipal, TokenStore};
use tiny_http::Server;

use crate::evidence::{RepoWorld, WorldSnapshot};

/// The dev/operator credential (the break-glass tier; NOT a tenant identity).
pub const DEV: &str = "dev-token-hugit-qa";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch world dir (created with the `_accounts` seam the CLI expects).
pub fn scratch_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("hugit-qa-{}-{}-{}", std::process::id(), nanos, seq));
    std::fs::create_dir_all(dir.join("_accounts")).expect("accounts dir");
    dir
}

/// The engine under test, on a live socket, with the surfaces a user holds.
pub struct Harness {
    /// `127.0.0.1:<port>` of the live accept loop.
    pub addr: String,
    /// The shared canonical world (server `Local` source ↔ CLI `HUGIT_LOG`).
    pub log_dir: PathBuf,
    /// A per-run scratch root for `git` worktrees (unique per journey run, so a
    /// re-run never collides with a prior run's clone destination).
    pub work_dir: PathBuf,
    /// The engine's token store — issues exactly what a CoreLink exchange would.
    tokens: Arc<TokenStore>,
}

/// Seeds the canonical world for a repo WITHOUT a git seam (provision-managed log
/// only) — used by the identity/repo-lifecycle journeys.
pub fn seed_meta(dir: &Path, repo: &str, visibility: &str, owner_tenant: &str) {
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility": visibility, "owner_tenant": owner_tenant}).to_string(),
        0,
    );
    let path = dir.join(format!("{repo}.json"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("nest repo dir");
    }
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap())
        .expect("write seed log");
}

/// Append well-formed records onto `repo`'s canonical log, composing with a prior
/// `seed_meta` (the existing file's records become the chain head). Uses the SAME
/// `EventLog::append_for_test` seam the QA suites use — real hash chaining and
/// monotonic seq, never hand-faked bytes.
pub fn seed_records(dir: &Path, repo: &str, records: &[crate::journey::SeedRecord]) {
    let path = dir.join(format!("{repo}.json"));
    let mut log = load_log(&path);
    let mut ts = 10_000;
    for r in records {
        log.append_for_test(
            &r.kind,
            vec![],
            serde_json::to_string(&r.payload).expect("seed payload serializable"),
            ts,
        );
        ts += 1_000;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("nest repo dir");
    }
    std::fs::write(&path, serde_json::to_string_pretty(log.records()).unwrap())
        .expect("write seed records log");
}

/// Seed the full attested-cost world in one step — campaign.opened + intent.landed
/// (one per `intent_ids`) + pr.opened + the `pr.envelope`/`intent.envelope`
/// ContextEnvelope records the `/insights` cost X-ray + spend_proof rows read.
/// Envelope JSON shape mirrors `parity_insights.rs::make_envelope_json`
/// (`schema_version 1.2.0`, altitude `pr`/`intent`, `agent_type main`/`implementer`
/// with `parent_run_id`, integer micro-USD metrics) — byte-consistent with what
/// the QA suite proves renders `spend_proof: cas:`.
pub fn seed_env_world(
    dir: &Path,
    repo: &str,
    campaign: &str,
    pr_id: &str,
    intent_ids: &[String],
    cost_usd_micros: u64,
) {
    let path = dir.join(format!("{repo}.json"));
    let mut log = load_log(&path);
    let mut ts = 10_000;
    let run_id = format!("run-{pr_id}");

    log.append_for_test(
        "campaign.opened",
        vec![],
        serde_json::json!({ "campaign": campaign, "title": campaign }).to_string(),
        ts,
    );
    ts += 1_000;

    for (n, intent) in intent_ids.iter().enumerate() {
        log.append_for_test(
            "intent.landed",
            vec![],
            serde_json::json!({
                "intent_id": intent,
                "campaign": campaign,
                "charter": format!("land intent {n}"),
                "deep_link_target": intent,
            })
            .to_string(),
            ts,
        );
        ts += 1_000;
    }

    log.append_for_test(
        "pr.opened",
        vec![],
        serde_json::json!({
            "pr_id": pr_id,
            "campaign": campaign,
            "author_kind": "orchestrator",
            "run_id": run_id,
            "principal": null,
            "intent_ids": intent_ids,
        })
        .to_string(),
        ts,
    );
    ts += 1_000;

    let pr_env = envelope_json("pr", pr_id, campaign, "main", None, cost_usd_micros);
    log.append_for_test("pr.envelope", vec![], pr_env.to_string(), ts);
    ts += 1_000;

    // `implementer` subagents atomically feed through pr_record with the PR run id.
    for intent in intent_ids {
        let intent_env = envelope_json(
            "intent",
            intent,
            campaign,
            "implementer",
            Some(&run_id),
            cost_usd_micros,
        );
        log.append_for_test("intent.envelope", vec![], intent_env.to_string(), ts);
        ts += 1_000;
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("nest repo dir");
    }
    std::fs::write(&path, serde_json::to_string_pretty(log.records()).unwrap())
        .expect("write envelope seed log");
}

/// A `ContextEnvelope` JSON value with the exact shape `pr_record`/`pr_record`
/// read-time fold expects (mirrors `parity_insights.rs::make_envelope_json`).
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
                "run_id": "run-seed",
                "parent_run_id": parent_run_id,
                "born_at": 1000,
                "died_at": 2000
            },
            "operator": "test@example.com"
        },
        "charter": "test charter for spend proof",
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

/// Load a repo's canonical log file, or a fresh log when absent/invalid.
fn load_log(path: &Path) -> EventLog {
    if let Ok(bytes) = std::fs::read(path)
        && let Ok(recs) =
            serde_json::from_slice::<Vec<hugit_contracts::event_record::EventRecord>>(&bytes)
    {
        let mut log = EventLog::new();
        for r in recs {
            // Re-append through the monotonic, gap-free `seq` invariant so the
            // seeded chain is byte-identical to what the server verifies.
            log.append_for_test(r.kind, r.principal_chain, r.payload, r.recorded_at);
        }
        return log;
    }
    EventLog::new()
}

/// Build the seed git object graph (`main` = c1→c2, `feature` = c1→cf — the same
/// shape `git_wire.rs` / `ultimate_qa.rs` prove readable) and wire it onto the
/// state for `repo`.
pub fn wire_git(state: &mut AppState, repo: &str) {
    let mut cas = CasObjectSource::new();
    let b1 = cas.insert(GitObject::new(ObjectKind::Blob, b"hello hugit\n".to_vec()));
    let t1 = cas.insert(tree_one("README", &b1));
    let c1 = cas.insert(commit(&t1, None, "init"));
    let b2 = cas.insert(GitObject::new(
        ObjectKind::Blob,
        b"hello hugit, again\n".to_vec(),
    ));
    let t2 = cas.insert(tree_one("README", &b2));
    let c2 = cas.insert(commit(&t2, Some(&c1), "update readme"));
    let cf = cas.insert(commit(&t1, Some(&c1), "feature work"));
    let mut refs = BTreeMap::new();
    refs.insert("refs/heads/main".to_string(), c2.to_string());
    refs.insert("refs/heads/feature".to_string(), cf.to_string());
    state.set_repo_git(
        repo,
        Arc::new(cas),
        ObjectId::empty_tree(gix_hash::Kind::Sha1),
        refs,
    );
}

fn tree_one(name: &str, blob_oid: &ObjectId) -> GitObject {
    let mut body = Vec::new();
    body.extend_from_slice(b"100644 ");
    body.extend_from_slice(name.as_bytes());
    body.push(0);
    body.extend_from_slice(blob_oid.as_slice());
    GitObject::new(ObjectKind::Tree, body)
}

fn commit(tree: &ObjectId, parent: Option<&ObjectId>, message: &str) -> GitObject {
    let mut body = String::new();
    body.push_str(&format!("tree {tree}\n"));
    if let Some(p) = parent {
        body.push_str(&format!("parent {p}\n"));
    }
    let ident = "hugit <bot@hugit.dev> 1717000000 +0000";
    body.push_str(&format!("author {ident}\n"));
    body.push_str(&format!("committer {ident}\n"));
    body.push('\n');
    body.push_str(message);
    body.push('\n');
    GitObject::new(ObjectKind::Commit, body.into_bytes())
}

impl Harness {
    /// Boot a Local engine with the write path enabled (dev/operator break-glass
    /// ON — `AppState::new` semantics; PAT auth OFF by default).
    pub fn spawn() -> Harness {
        Self::spawn_with(|_, _| {})
    }

    /// Boot with a per-journey world mutation (`dir` = the harness's world; the
    /// closure seeds git seams / flips gates / writes seed logs BEFORE serving).
    pub fn spawn_with(mutate: impl FnOnce(&Path, &mut AppState)) -> Harness {
        let log_dir = scratch_dir();
        let work_dir = scratch_dir().join("work");
        std::fs::create_dir_all(&work_dir).expect("work dir");
        let mut state = AppState::new(log_dir.clone(), DEV.to_string());
        state.enable_write_path();
        mutate(&log_dir, &mut state);
        let tokens = state.token_store.clone();
        let addr = serve(state);
        Harness {
            addr,
            log_dir,
            work_dir,
            tokens,
        }
    }

    /// Mint the identity seam: the bearer credential a real user holds.
    pub fn mint(&self, org: &str, user: &str, fresh: bool) -> String {
        self.tokens
            .mint_with_ttl(
                &ClerkPrincipal {
                    user: user.to_string(),
                    org: org.to_string(),
                    fresh_auth: fresh,
                },
                3600,
            )
            .expect("mint tenant token")
    }

    /// The canonical world file a repo lives in (`<log_dir>/<repo>.json`).
    pub fn world(&self, repo: &str) -> PathBuf {
        self.log_dir.join(format!("{repo}.json"))
    }

    /// Snapshot the observable world the journey sees: every repo's canonical log,
    /// chain-verified shape included (records + head seq + kind histogram).
    pub fn snapshot(&self) -> WorldSnapshot {
        let mut repos = BTreeMap::new();
        let mut entries: Vec<_> = std::fs::read_dir(&self.log_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let world = read_repo_world(&path);
            repos.insert(name, world);
        }
        WorldSnapshot { repos }
    }
}

fn read_repo_world(path: &Path) -> RepoWorld {
    let Ok(bytes) = std::fs::read(path) else {
        return RepoWorld {
            records: 0,
            head_seq: None,
            kinds: BTreeMap::new(),
            present: false,
        };
    };
    let Ok(recs) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) else {
        return RepoWorld {
            records: 0,
            head_seq: None,
            kinds: BTreeMap::new(),
            present: true,
        };
    };
    let mut kinds = BTreeMap::new();
    let mut head_seq = None;
    for r in &recs {
        if let Some(k) = r.get("kind").and_then(|k| k.as_str()) {
            *kinds.entry(k.to_string()).or_insert(0) += 1;
        }
        if let Some(s) = r.get("seq").and_then(|s| s.as_u64()) {
            head_seq = Some(s);
        }
    }
    RepoWorld {
        records: recs.len() as u64,
        head_seq,
        kinds,
        present: true,
    }
}

/// Spawn the accept loop on an ephemeral port; the state is owned by the loop.
pub fn serve(state: AppState) -> String {
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

/// A raw HTTP response (the wire the user hits).
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// Raw HTTP/1.1 exchange → the parsed response (headers + body). Mirrors
/// `ultimate_qa.rs`'s `http_req` exactly — the one codegen the QA suites share.
pub fn http_req(
    addr: &str,
    method: &str,
    path: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResponse {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: t\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in extra_headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    if !body.is_empty() {
        stream.write_all(body).unwrap();
    }
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).unwrap();
    let sep = b"\r\n\r\n";
    let idx = resp
        .windows(sep.len())
        .position(|w| w == sep)
        .expect("response separator");
    let raw_head = String::from_utf8_lossy(&resp[..idx]).into_owned();
    let status = raw_head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    HttpResponse {
        status,
        body: resp[idx + sep.len()..].to_vec(),
    }
}

/// Configure a `git` Command with the repo-local identity the suites use.
pub fn git_cfg(c: &mut Command) {
    c.args([
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "-c",
        "protocol.version=1",
    ]);
}

/// Whether `git` is on PATH (journey steps that need it report a skip, not a 500).
pub fn have_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Locate the REAL `hugit` CLI binary. Resolution order:
///   1. `HUGIT_QA_HUGIT_BIN` env override (CI / cross-build setups);
///   2. a sibling `hugit` next to THIS binary (`target/{debug,release}/hugit`),
///      which `cargo run -p hugit-qa` produces when `hugit-cli` is built;
///   3. walking UP from this binary's dir into the cargo `target/` layout
///      (`deps/…` → `debug/hugit`), which covers `cargo test` harnesses;
///   4. `hugit` on `PATH`.
///
/// Returns `None` when unresolvable — a CLI step then records a skip (the harness
/// never fabricates a fake CLI surface).
pub fn find_hugit_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HUGIT_QA_HUGIT_BIN") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
    {
        let sibling = dir.join("hugit");
        if sibling.is_file() {
            return Some(sibling);
        }
        // `cargo test` puts the harness in `target/{debug,release}/deps/`; the
        // CLI lands at `target/{debug,release}/hugit` — walk up to find it.
        if let Some(target) = dir.ancestors().find(|d| {
            d.file_name()
                .is_some_and(|n| n == "debug" || n == "release")
        }) {
            for cand in [target.join("hugit"), target.join("deps").join("hugit")] {
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join("hugit");
        #[cfg(windows)]
        let cand = dir.join("hugit.exe");
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Whether the real `hugit` binary is resolvable.
pub fn have_hugit() -> bool {
    find_hugit_bin().is_some()
}

/// A CLI verb surfaced to a single-private test. Result carries the trace.
pub struct CliRun {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}
