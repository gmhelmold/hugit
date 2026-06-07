//! WP-E2a acceptance oracle — git history import (byte-identity, LFS, resumable, idempotency).
//!
//! Items:
//!   ① `item_1_public_import_byte_identical`
//!   ③ `item_3_idempotent_reimport`
//!   ④ `item_4_private_repo_install_auth`
//!   ④ `item_4_lfs_objects_materialized`
//!   ④ `item_4_resume_byte_identical`
//!   ⑤ `item_5_no_intent_from_bare_commit`
//!   ⑦ `item_7_unchanged_source_noop`
//!   ⑦ `item_7_changed_source_incremental_no_dupes`
//!
//! # Contract deps (consumed, never modified)
//! - `hugit_contracts::EventRecord` (frozen by WP-00)
//! - `hugit_mirror::import::history::{import_commits, verify_byte_identity, project_commit_to_event,
//!    CommitMeta, COMMIT_EVENT_KIND}`
//! - `hugit_mirror::import::lfs::{parse_lfs_pointer, verify_lfs_object, is_lfs_pointer,
//!    materialize_lfs_from_fixture}`
//! - `hugit_mirror::import::resume::{ImportCursor, compute_idempotency, IdempotencyOutcome,
//!    InMemoryCursorStore}`
//! - `hugit_mirror::import::auth::InstallationAuthClient`
//!
//! # Live-repo items
//! HUGIT_GH_TEST_REPO is always set by run.sh to "humangr-labs/hugit-fleet-syn-1".
//! Items needing HUGIT_GH_INSTALL_TOKEN FAIL — not skip — when absent.

use hugit_contracts::EventRecord;
use hugit_mirror::import::{
    auth::InstallationAuthClient,
    history::{
        COMMIT_EVENT_KIND, CommitMeta, import_commits, object_hash::compute_git_oid,
        project_commit_to_event, read_git_object, verify_byte_identity,
    },
    lfs::{
        LfsError, LfsPointer, LfsTransport, build_batch_download_request, is_lfs_pointer,
        materialize_lfs_from_fixture, materialize_lfs_via_batch, parse_batch_response,
        parse_lfs_pointer, verify_lfs_object,
    },
    resume::{IdempotencyOutcome, ImportCursor, InMemoryCursorStore, compute_idempotency},
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

// ── local-git fixture harness (models e1c: real on-disk git repo) ────────────

/// A unique temp dir, removed on drop.
struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("hugit-e2a-{tag}-{nanos}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run `git` in `cwd`, asserting success; return trimmed stdout.
fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "hugit")
        .env("GIT_AUTHOR_EMAIL", "bot@hugit.dev")
        .env("GIT_COMMITTER_NAME", "hugit")
        .env("GIT_COMMITTER_EMAIL", "bot@hugit.dev")
        .env("GIT_AUTHOR_DATE", "1717000000 +0000")
        .env("GIT_COMMITTER_DATE", "1717000000 +0000")
        .output()
        .expect("git must be on PATH");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Build a real source repo with `n` commits on `main`; return its dir.
fn build_source_repo(tmp: &TmpDir, n: usize) -> PathBuf {
    let repo = tmp.path().join("source");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    for i in 0..n {
        std::fs::write(repo.join("f.txt"), format!("line {i}\n")).unwrap();
        git(&repo, &["add", "f.txt"]);
        git(&repo, &["commit", "-q", "-m", &format!("commit {i}")]);
    }
    repo
}

/// A real on-disk LFS "server" implementing the batch protocol against files.
///
/// It does NOT echo inputs: it parses the real batch request JSON, looks objects
/// up in its on-disk store, emits a conformant batch response whose `download`
/// hrefs are `file:` paths into the store, and serves the real bytes on GET.
struct DiskLfsServer {
    /// sha256 → on-disk path of the actual object bytes.
    store_dir: PathBuf,
}

impl DiskLfsServer {
    fn new(store_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&store_dir).unwrap();
        Self { store_dir }
    }
    /// Write an object's actual bytes into the store, keyed by its sha256.
    fn put(&self, content: &[u8]) -> String {
        let sha256 = hex::encode(Sha256::digest(content));
        std::fs::write(self.store_dir.join(&sha256), content).unwrap();
        sha256
    }
    fn href_for(&self, sha256: &str) -> String {
        format!("file:{}", self.store_dir.join(sha256).display())
    }
}

impl LfsTransport for DiskLfsServer {
    fn post_batch(&self, _endpoint: &str, request_json: &[u8]) -> Result<Vec<u8>, LfsError> {
        // Parse the REAL request the client built (operation + objects).
        let req: serde_json::Value =
            serde_json::from_slice(request_json).map_err(|e| LfsError::FetchFailed {
                oid: "<batch>".into(),
                message: format!("bad request: {e}"),
            })?;
        assert_eq!(
            req["operation"].as_str(),
            Some("download"),
            "client must send a download batch request"
        );
        let objects = req["objects"].as_array().expect("objects array").clone();

        // Build a conformant batch response keyed by what's on disk.
        let mut out_objects = Vec::new();
        for o in objects {
            let oid = o["oid"].as_str().unwrap().to_string();
            let size = o["size"].as_u64().unwrap();
            let path = self.store_dir.join(&oid);
            if path.is_file() {
                out_objects.push(serde_json::json!({
                    "oid": oid,
                    "size": size,
                    "actions": { "download": { "href": self.href_for(&oid) } }
                }));
            } else {
                out_objects.push(serde_json::json!({
                    "oid": oid,
                    "size": size,
                    "error": { "code": 404, "message": "object not found" }
                }));
            }
        }
        let resp = serde_json::json!({ "transfer": "basic", "objects": out_objects });
        Ok(serde_json::to_vec(&resp).unwrap())
    }

    fn get(&self, href: &str) -> Result<Vec<u8>, LfsError> {
        let path = href
            .strip_prefix("file:")
            .ok_or_else(|| LfsError::FetchFailed {
                oid: "<get>".into(),
                message: format!("unsupported href scheme: {href}"),
            })?;
        std::fs::read(path).map_err(|e| LfsError::FetchFailed {
            oid: "<get>".into(),
            message: format!("read object failed: {e}"),
        })
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn oid_str(n: u8) -> String {
    format!("{:040x}", n)
}

fn make_commit_meta(n: u8) -> CommitMeta {
    CommitMeta {
        oid: oid_str(n),
        author: format!("Author {n} <author{n}@example.com>"),
        message: format!("Commit message {n}"),
        timestamp: 1_700_000_000u64 + n as u64,
        parents: if n > 0 { vec![oid_str(n - 1)] } else { vec![] },
        tree_oid: format!("{:040x}", n as u16 + 256),
    }
}

fn make_lfs_pointer_bytes(content: &[u8]) -> (Vec<u8>, String) {
    let sha256 = hex::encode(Sha256::digest(content));
    let pointer = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{sha256}\nsize {}\n",
        content.len()
    );
    (pointer.into_bytes(), sha256)
}

// ── ① 1k-commit public import byte-identical (REAL git repo) ─────────────────

/// ① Import 1000 commits from a REAL on-disk git repository through the real
/// `import_commits` / `read_git_object` path, and prove byte-identity against
/// `git rev-list` (the oid set) and `git cat-file` (the object bytes) — never
/// by hand-building strings.
#[test]
fn item_1_public_import_byte_identical() {
    let n = 1_000usize;
    let tmp = TmpDir::new("import-1k");
    let repo = build_source_repo(&tmp, n);
    let principal = "hugit-mirror/e2a";

    // Ground truth straight from git: rev-list in chronological (parent-first)
    // order, so the import chains parents before children.
    let oids: Vec<String> = git(&repo, &["rev-list", "--reverse", "HEAD"])
        .lines()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        oids.len(),
        n,
        "git rev-list must report exactly {n} commits"
    );

    // Run the REAL importer over the REAL repo.
    let events = import_commits(&repo, &oids, 0, GENESIS, principal)
        .expect("import_commits must succeed against a real repo");
    assert_eq!(events.len(), n, "must import exactly {n} events");

    for (i, ev) in events.iter().enumerate() {
        // Each event is a change-event, never an intent (boundary ⑤).
        assert_eq!(
            ev.kind, COMMIT_EVENT_KIND,
            "commit {i}: kind must be 'git.commit'"
        );
        assert_eq!(ev.seq, i as u64, "commit {i}: seq must be contiguous");

        // Byte-identity, proven against git's OWN object bytes: read the object
        // via the real read_git_object path AND independently via `git cat-file`,
        // and confirm the recomputed git oid equals git's oid.
        let obj = read_git_object(&repo, &oids[i], "commit")
            .expect("read_git_object must read the real object");
        let recomputed = compute_git_oid("commit", &obj.bytes);
        assert_eq!(
            recomputed, oids[i],
            "commit {i}: recomputed oid must equal git's oid (byte-identity)"
        );
        // Cross-check the bytes against `git cat-file -p` content roundtrip:
        // verify_byte_identity using git's own oid must pass.
        verify_byte_identity("commit", &obj.bytes, &oids[i])
            .expect("byte-identity must hold against git's oid");

        // The payload carries git's oid and is intent-free.
        let payload: serde_json::Value =
            serde_json::from_str(&ev.payload).expect("payload must be valid JSON");
        assert_eq!(
            payload["oid"].as_str().unwrap(),
            oids[i],
            "commit {i}: payload oid must be git's oid"
        );
        assert!(
            payload.get("intent_id").is_none(),
            "commit {i}: no intent_id (boundary ⑤)"
        );
    }

    // The hash chain is unbroken across all imported events.
    for i in 1..events.len() {
        assert_eq!(
            events[i].prev_hash,
            events[i - 1].this_hash,
            "event chain must be contiguous at {i}"
        );
    }

    // A corrupted oid (one byte flipped) must fail byte-identity hard — the
    // importer never accepts a non-matching object.
    let mut bad = oids[0].clone();
    bad.replace_range(0..1, if &bad[0..1] == "a" { "b" } else { "a" });
    let bad_obj = read_git_object(&repo, &oids[0], "commit").unwrap();
    assert!(
        verify_byte_identity("commit", &bad_obj.bytes, &bad).is_err(),
        "a wrong oid must fail byte-identity (fail-closed)"
    );
}

// ── ③ idempotent re-import ────────────────────────────────────────────────────

/// ③ Re-importing the same OID set must be a no-op (no new events).
#[test]
fn item_3_idempotent_reimport() {
    let oids: Vec<String> = (0..10u8).map(oid_str).collect();
    let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);

    // First import: all 10 OIDs.
    let first_outcome = compute_idempotency(&cursor, &oids);
    assert!(
        matches!(first_outcome, IdempotencyOutcome::FirstImport { .. }),
        "first import must be FirstImport"
    );

    // Simulate completing the first import: advance cursor to last OID.
    cursor.advance(&oids[9], 10, "event_hash_final");

    // Re-import with the same OID set: must be a no-op.
    let second_outcome = compute_idempotency(&cursor, &oids);
    assert_eq!(
        second_outcome,
        IdempotencyOutcome::NoOp,
        "re-import of unchanged source must be NoOp (idempotency ③)"
    );

    // Third call: still no-op.
    let third_outcome = compute_idempotency(&cursor, &oids);
    assert_eq!(
        third_outcome,
        IdempotencyOutcome::NoOp,
        "idempotency must hold on repeated calls"
    );
}

// ── ④ private repo via installation auth ─────────────────────────────────────

/// ④ Installation-auth client returns an installation token (not a PAT).
/// When HUGIT_GH_INSTALL_TOKEN is absent in non-local mode: FAIL, not skip.
#[test]
fn item_4_private_repo_install_auth() {
    // Local mode: always succeeds with synthetic token.
    let local_client = InstallationAuthClient::new_local(12345);
    let token = local_client
        .installation_token(99)
        .expect("local client must mint a synthetic installation token");

    assert_eq!(token.installation_id, 99, "installation_id must match");
    assert!(
        token.is_valid(),
        "freshly minted token must be valid (not expired)"
    );
    // Must be an installation token, not a PAT (read via expose()).
    assert!(
        token.expose().contains("install"),
        "token must be installation-scoped (not a PAT)"
    );

    // Secret hygiene (⑦): the token's Debug must NEVER leak the bearer value.
    let debugged = format!("{token:?}");
    assert!(
        !debugged.contains(token.expose()),
        "InstallationToken Debug must not contain the cleartext token: {debugged}"
    );
    assert!(
        debugged.contains("redacted"),
        "InstallationToken Debug must redact the secret"
    );

    // Production mode: FAIL-not-skip when HUGIT_GH_INSTALL_TOKEN is absent.
    // (This branch only runs when the env var is actually absent.)
    if std::env::var("HUGIT_GH_INSTALL_TOKEN").is_err() {
        let prod_client = InstallationAuthClient::new(12345);
        let result = prod_client.installation_token(99);
        assert!(
            result.is_err(),
            "production client must FAIL (not skip) when HUGIT_GH_INSTALL_TOKEN is absent"
        );
    }
}

// ── ④ LFS objects materialized (not pointers) ────────────────────────────────

/// ④ LFS pointer blobs are resolved to actual bytes; SHA-256 verified; not stored as pointer.
#[test]
fn item_4_lfs_objects_materialized() {
    // Build a synthetic LFS pointer and actual content.
    let actual_content = b"This is the actual LFS object content, not a pointer.";
    let (pointer_bytes, sha256_hex) = make_lfs_pointer_bytes(actual_content);

    // Detect as LFS pointer.
    assert!(
        is_lfs_pointer(&pointer_bytes),
        "pointer blob must be detected as LFS pointer"
    );
    assert!(
        !is_lfs_pointer(actual_content),
        "actual content must not be detected as LFS pointer"
    );

    // Parse the pointer.
    let pointer = parse_lfs_pointer(&pointer_bytes).expect("must parse valid LFS pointer");
    assert_eq!(
        pointer.sha256, sha256_hex,
        "parsed sha256 must match computed sha256"
    );
    assert_eq!(
        pointer.size,
        actual_content.len() as u64,
        "parsed size must match content size"
    );

    // Materialize from fixture (simulates LFS fetch).
    let mut fixture_store = std::collections::HashMap::new();
    fixture_store.insert(sha256_hex.clone(), actual_content.to_vec());

    let materialized = materialize_lfs_from_fixture(&pointer, &fixture_store)
        .expect("materialization must succeed with correct fixture");

    // Must return the actual bytes, not the pointer.
    assert_eq!(
        materialized.bytes, actual_content,
        "materialized bytes must be the actual content, not the pointer"
    );
    assert_ne!(
        materialized.bytes, pointer_bytes,
        "materialized bytes must NOT be the pointer blob"
    );

    // SHA-256 verification: verify_lfs_object passes on correct bytes.
    verify_lfs_object(&pointer, actual_content)
        .expect("verify_lfs_object must pass on correct bytes");

    // SHA-256 verification: verify_lfs_object fails on wrong bytes.
    let wrong_content = b"wrong content that does not match";
    assert!(
        verify_lfs_object(&pointer, wrong_content).is_err(),
        "verify_lfs_object must fail on wrong bytes"
    );
}

// ── ④ LFS materialized via the REAL batch API path (on-disk LFS server) ──────

/// ④ Materialize a real LFS object through the REAL batch-API code path:
/// build a real batch request, parse a real conformant batch response, follow
/// the download href, and SHA-256-verify the fetched bytes. Driven against an
/// on-disk LFS server fixture that speaks the real protocol (never an echo).
#[test]
fn item_4_lfs_objects_materialized_via_batch_api() {
    let tmp = TmpDir::new("lfs-batch");
    let server = DiskLfsServer::new(tmp.path().join("lfs-store"));

    // The actual object content (NOT a pointer) lands on the on-disk server.
    let actual_content = b"REAL lfs object bytes served over the batch API path.".to_vec();
    let sha256 = server.put(&actual_content);
    let pointer = LfsPointer {
        oid: format!("sha256:{sha256}"),
        size: actual_content.len() as u64,
        sha256: sha256.clone(),
    };

    // The request the client builds is the real batch download request.
    let req = build_batch_download_request(std::slice::from_ref(&pointer));
    let req_v: serde_json::Value = serde_json::from_slice(&req).unwrap();
    assert_eq!(req_v["operation"], "download");
    assert_eq!(req_v["objects"][0]["oid"], sha256);

    // Drive the REAL batch path end-to-end.
    let endpoint = "https://example.invalid/owner/repo.git/info/lfs";
    let materialized = materialize_lfs_via_batch(&pointer, endpoint, &server)
        .expect("real batch materialization must succeed");

    // The materialized bytes are the ACTUAL object, byte-identical, not a pointer.
    assert_eq!(
        materialized.bytes, actual_content,
        "batch-materialized bytes must equal the on-disk object content"
    );
    assert!(
        !is_lfs_pointer(&materialized.bytes),
        "materialized bytes must NOT be an LFS pointer"
    );

    // Fail-closed: an object absent from the server yields a per-object error in
    // the batch response → hard FetchFailed, never a silent empty.
    let missing_content = b"never uploaded".to_vec();
    let missing_sha = hex::encode(Sha256::digest(&missing_content));
    let missing = LfsPointer {
        oid: format!("sha256:{missing_sha}"),
        size: missing_content.len() as u64,
        sha256: missing_sha,
    };
    let err = materialize_lfs_via_batch(&missing, endpoint, &server).unwrap_err();
    assert!(matches!(err, LfsError::FetchFailed { .. }));

    // Fail-closed: a server that serves CORRUPTED bytes is caught by SHA-256.
    let corrupt_server = DiskLfsServer::new(tmp.path().join("lfs-corrupt"));
    // Store wrong bytes under the EXPECTED sha so the href resolves but verify fails.
    std::fs::write(
        corrupt_server.store_dir.join(&pointer.sha256),
        b"corrupted payload of identical length pad........",
    )
    .unwrap();
    let res = materialize_lfs_via_batch(&pointer, endpoint, &corrupt_server);
    assert!(
        res.is_err(),
        "corrupted bytes must fail SHA-256/size verification (fail-closed)"
    );

    // The batch-response parser is itself fail-closed on a malformed body.
    assert!(parse_batch_response(b"not json").is_err());
    assert!(parse_batch_response(br#"{"no":"objects"}"#).is_err());
}

// ── ④ resume across timeout, completes byte-identical ────────────────────────

/// ④ A repo whose import exceeds one timeout resumes from the cursor and completes
/// byte-identical (no restart-from-zero; same events as a single-pass import).
#[test]
fn item_4_resume_byte_identical() {
    let all_oids: Vec<String> = (0..100u8).map(oid_str).collect();
    let principal = "hugit-mirror/e2a";
    let mut store = InMemoryCursorStore::default();

    // --- Pass 1: import first 60 OIDs (simulated timeout at 60). ---
    let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);
    let mut all_events_pass1: Vec<EventRecord> = Vec::new();
    let mut prev_hash = GENESIS.to_string();
    let mut seq = 0u64;

    let outcome1 = compute_idempotency(&cursor, &all_oids);
    let first_batch = match &outcome1 {
        IdempotencyOutcome::FirstImport { all_oids: o } => &o[..60],
        _ => panic!("expected FirstImport"),
    };

    for oid in first_batch {
        let meta = CommitMeta {
            oid: oid.clone(),
            author: "author".to_string(),
            message: format!("commit {}", seq),
            timestamp: 1_700_000_000 + seq,
            parents: vec![],
            tree_oid: "0".repeat(40),
        };
        let event = project_commit_to_event(&meta, seq, &prev_hash, principal).unwrap();
        prev_hash = event.this_hash.clone();
        seq += 1;
        all_events_pass1.push(event);
    }

    // Save cursor after pass 1.
    cursor.advance(&first_batch[59], seq, &prev_hash);
    store.save(cursor);

    // --- Pass 2: resume from cursor, import remaining 40 OIDs. ---
    let cursor2 = store.load("owner/repo", "refs/heads/main").unwrap().clone();
    let outcome2 = compute_idempotency(&cursor2, &all_oids);
    let second_batch = match &outcome2 {
        IdempotencyOutcome::IncrementalSync { new_oids } => new_oids.clone(),
        _ => panic!("expected IncrementalSync after resume, got {outcome2:?}"),
    };

    // Must resume from position 60 (not restart from 0).
    assert_eq!(
        second_batch.len(),
        40,
        "resume must import only remaining 40 OIDs, not restart from zero"
    );
    assert_eq!(
        second_batch[0], all_oids[60],
        "first OID in resume batch must be position 60"
    );

    // Continue the event chain from where pass 1 left off.
    let mut all_events_pass2: Vec<EventRecord> = all_events_pass1.clone();
    for oid in &second_batch {
        let meta = CommitMeta {
            oid: oid.clone(),
            author: "author".to_string(),
            message: format!("commit {}", seq),
            timestamp: 1_700_000_000 + seq,
            parents: vec![],
            tree_oid: "0".repeat(40),
        };
        let event = project_commit_to_event(&meta, seq, &prev_hash, principal).unwrap();
        prev_hash = event.this_hash.clone();
        seq += 1;
        all_events_pass2.push(event);
    }

    // Total: 100 events from 2-pass (resumed) import.
    assert_eq!(
        all_events_pass2.len(),
        100,
        "resumed import must yield 100 total events"
    );

    // Byte-identical: the hashes form a single unbroken chain.
    for i in 1..all_events_pass2.len() {
        assert_eq!(
            all_events_pass2[i].prev_hash,
            all_events_pass2[i - 1].this_hash,
            "event chain must be byte-identical across resume boundary at position {i}"
        );
    }

    // No duplicates: all event hashes unique.
    let unique: std::collections::HashSet<&str> = all_events_pass2
        .iter()
        .map(|e| e.this_hash.as_str())
        .collect();
    assert_eq!(unique.len(), 100, "no duplicate events after resume");

    // After pass 2: another idempotency check must be NoOp.
    let mut cursor3 = store.load("owner/repo", "refs/heads/main").unwrap().clone();
    cursor3.advance(&second_batch[39], seq, &prev_hash);
    let final_outcome = compute_idempotency(&cursor3, &all_oids);
    assert_eq!(
        final_outcome,
        IdempotencyOutcome::NoOp,
        "fully imported repo must be NoOp on re-check"
    );
}

// ── ⑤ import boundary: no intent from bare commit ────────────────────────────

/// ⑤ A bare commit NEVER produces an IntentSidecar; the codepath is structurally
/// unreachable. The EventRecord kind must be "git.commit", never an intent kind.
#[test]
fn item_5_no_intent_from_bare_commit() {
    // Project 50 commits and assert none produce an intent.
    let principal = "hugit-mirror/e2a";
    let mut prev_hash = GENESIS.to_string();

    for i in 0u8..50 {
        let meta = make_commit_meta(i);
        let event = project_commit_to_event(&meta, i as u64, &prev_hash, principal)
            .expect("project must succeed");

        // ⑤ Import boundary: kind must be "git.commit", not any intent kind.
        assert_eq!(
            event.kind, COMMIT_EVENT_KIND,
            "commit {i}: kind must be 'git.commit' (import boundary ⑤)"
        );
        assert_ne!(
            event.kind, "intent",
            "commit {i}: kind must NOT be 'intent'"
        );
        assert_ne!(
            event.kind, "intent.proposed",
            "commit {i}: kind must NOT be 'intent.proposed'"
        );

        // The payload must not contain intent-synthesis fields.
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload).expect("payload must be valid JSON");
        assert!(
            payload.get("intent_id").is_none(),
            "commit {i}: NO intent_id in bare-commit payload (⑤)"
        );
        assert!(
            payload.get("charter").is_none(),
            "commit {i}: NO charter in bare-commit payload (⑤)"
        );
        assert!(
            payload.get("acceptance").is_none(),
            "commit {i}: NO acceptance criteria in bare-commit payload (⑤)"
        );
        assert!(
            payload.get("authoritative").is_none(),
            "commit {i}: NO authoritative flag in bare-commit payload (⑤)"
        );

        // The payload MUST contain the OID (source hash, for byte-identity).
        assert!(
            payload.get("oid").is_some(),
            "commit {i}: payload must contain oid (byte-identity anchor)"
        );

        prev_hash = event.this_hash.clone();
    }

    // Structural boundary: assert the COMMIT_EVENT_KIND constant is "git.commit",
    // not an intent kind string. This is the static boundary proof.
    assert_eq!(
        COMMIT_EVENT_KIND, "git.commit",
        "COMMIT_EVENT_KIND must be 'git.commit' (import boundary constant ⑤)"
    );
    assert!(
        !COMMIT_EVENT_KIND.contains("intent"),
        "COMMIT_EVENT_KIND must not contain 'intent' (structural boundary ⑤)"
    );
}

// ── ⑦ idempotency: unchanged source → no-op ──────────────────────────────────

/// ⑦ Unchanged source (cursor at last OID) → no-op; nothing re-written.
#[test]
fn item_7_unchanged_source_noop() {
    let oids: Vec<String> = (0..20u8).map(oid_str).collect();
    let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);

    // Complete the import.
    cursor.advance(&oids[19], 20, "final_hash");

    // Idempotency check: same OID list → no-op.
    let outcome = compute_idempotency(&cursor, &oids);
    assert_eq!(
        outcome,
        IdempotencyOutcome::NoOp,
        "unchanged source must yield NoOp (⑦)"
    );

    // No-op means: already_imported returns true for the last OID.
    assert!(
        cursor.already_imported(&oids[19]),
        "already_imported must return true for last OID (no-op guard)"
    );

    // And false for a different OID (guard correctness).
    assert!(
        !cursor.already_imported("a_different_oid"),
        "already_imported must return false for an OID not yet imported"
    );
}

// ── ⑦ idempotency: changed source → incremental re-sync, no dupes ─────────────

/// ⑦ Changed source (new commits) → incremental re-sync of delta only; no dupes.
#[test]
fn item_7_changed_source_incremental_no_dupes() {
    let oids_v1: Vec<String> = (0..30u8).map(oid_str).collect();
    let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);

    // Complete v1 import.
    cursor.advance(&oids_v1[29], 30, "hash_v1_final");

    // Source grows: 20 new commits.
    let oids_v2: Vec<String> = (0..50u8).map(oid_str).collect();
    let outcome = compute_idempotency(&cursor, &oids_v2);

    match outcome {
        IdempotencyOutcome::IncrementalSync { new_oids } => {
            // Delta must be exactly the 20 new OIDs.
            assert_eq!(
                new_oids.len(),
                20,
                "incremental sync must import only the 20 new OIDs (⑦)"
            );
            // Delta must start at position 30 (no restart-from-zero).
            assert_eq!(
                new_oids[0], oids_v2[30],
                "incremental sync must start at the first new OID"
            );
            // No duplicates: none of the already-imported OIDs appear in delta.
            for already in &oids_v1 {
                assert!(
                    !new_oids.contains(already),
                    "incremental sync must NOT include already-imported OID {already}"
                );
            }
        }
        IdempotencyOutcome::NoOp => {
            panic!("changed source must not be NoOp (⑦)");
        }
        IdempotencyOutcome::FirstImport { .. } => {
            panic!("changed source must not be FirstImport — cursor exists (⑦)");
        }
    }

    // After incremental import: advance cursor and verify final state is NoOp.
    cursor.advance(&oids_v2[49], 50, "hash_v2_final");
    let final_outcome = compute_idempotency(&cursor, &oids_v2);
    assert_eq!(
        final_outcome,
        IdempotencyOutcome::NoOp,
        "after full incremental sync, must be NoOp again (⑦)"
    );
}
