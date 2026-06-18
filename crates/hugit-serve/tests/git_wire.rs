//! Git smart-HTTP wire serving — the FIRST live `git clone`/`git fetch` against
//! `hugit-serve` (previously a flat 404).
//!
//! ## The correctness oracle
//!
//! [`real_git_clone_succeeds`] spawns the actual `serve_on` loop on an ephemeral
//! port, seeds the same 7-object two-branch graph the proto's `acceptance_d2a`
//! uses (+ a `refs/heads/main`/`feature` ref map + a PUBLIC `repo.meta`), then
//! shells a REAL `git clone http://127.0.0.1:<port>/<repo>` and asserts it
//! SUCCEEDS and reconstructs the expected objects. `git` refuses to clone if the
//! wire bytes are wrong, so a green clone IS the proof the protocol is correct.
//!
//! A non-`git`-dependent fallback ([`direct_protocol_clone_bytes`]) drives the
//! HTTP envelope itself: GET info/refs → assert the advertisement Content-Type +
//! `# service` preamble + the tips; POST a want/have body → assert `NAK` + a `PACK`
//! header. So the protocol is covered even where `git` is unavailable.
//!
//! Push (`git-receive-pack`) is OUT of scope — [`receive_pack_is_404`] asserts it
//! is not served.

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
use tiny_http::Server;

const TOKEN: &str = "dev-token-git";

// ── the 7-object two-branch graph (mirrors acceptance_d2a) ───────────────────

fn blob(content: &[u8]) -> GitObject {
    GitObject::new(ObjectKind::Blob, content.to_vec())
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

/// The tip oids + ref map the assertions read (the CAS is owned by the state).
struct Seed {
    refs: BTreeMap<String, String>,
    c2: ObjectId,
    cf: ObjectId,
}

/// `main` = c1→c2, `feature` = c1→cf (reuses c1's tree). 7 distinct objects.
/// Returns the seeded CAS (the state's object source) alongside the tips/refs.
fn seed_repo() -> (CasObjectSource, Seed) {
    let mut cas = CasObjectSource::new();
    let b1 = cas.insert(blob(b"hello hugit\n"));
    let t1 = cas.insert(tree_one("README", &b1));
    let c1 = cas.insert(commit(&t1, None, "init"));
    let b2 = cas.insert(blob(b"hello hugit, again\n"));
    let t2 = cas.insert(tree_one("README", &b2));
    let c2 = cas.insert(commit(&t2, Some(&c1), "update readme"));
    let cf = cas.insert(commit(&t1, Some(&c1), "feature work"));

    let mut refs = BTreeMap::new();
    refs.insert("refs/heads/main".to_string(), c2.to_string());
    refs.insert("refs/heads/feature".to_string(), cf.to_string());
    (cas, Seed { refs, c2, cf })
}

// ── AppState assembly with a seeded git source + a public repo ───────────────

fn scratch_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hugit-serve-git-{}-{}-{}",
        std::process::id(),
        nanos,
        seq
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An `AppState` whose `repo` is PUBLIC (cloneable over the unauth git wire) and
/// whose git source/refs are the seeded 7-object graph.
fn state_with_git(repo: &str, visibility: &str) -> (AppState, PathBuf, Seed) {
    let (cas, seed) = seed_repo();
    let dir = scratch_dir();
    // A valid one-record chain carrying repo.meta (so the read gate has visibility).
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility": visibility, "owner_tenant": "org-a"}).to_string(),
        0,
    );
    let log_json = serde_json::to_string_pretty(log.records()).unwrap();
    std::fs::write(dir.join(format!("{repo}.json")), log_json).unwrap();

    let mut state = AppState::new(dir.clone(), TOKEN.to_string());
    // Re-seed the git source + refs (the `new()` ctor leaves them empty). The state
    // owns the seeded CAS; the returned `Seed` carries the refs + tip oids the
    // assertions read. root tree isn't exercised by the git wire; leave None.
    state.git_source = Some(Arc::new(cas));
    state.git_refs = seed.refs.clone();
    (state, dir, seed)
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

// ── raw HTTP helpers (no Bearer — git sends none) ────────────────────────────

/// Raw HTTP/1.1 GET → the full response bytes (headers + body).
fn http_get_raw(addr: &str, path: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let req = format!("GET {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).unwrap();
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).unwrap();
    resp
}

/// Raw HTTP/1.1 POST of `body` → the full response bytes.
fn http_post_raw(addr: &str, path: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: t\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).unwrap();
    resp
}

/// Split a raw HTTP response into (status_line, header_block, body_bytes).
fn split_response(resp: &[u8]) -> (String, String, Vec<u8>) {
    let sep = b"\r\n\r\n";
    let idx = resp
        .windows(sep.len())
        .position(|w| w == sep)
        .expect("response has a header/body separator");
    let head = String::from_utf8_lossy(&resp[..idx]).into_owned();
    let body = resp[idx + sep.len()..].to_vec();
    let status_line = head.lines().next().unwrap_or("").to_string();
    (status_line, head, body)
}

// ── UNIT: info/refs advertisement ────────────────────────────────────────────

#[test]
fn info_refs_advertisement_is_well_formed() {
    let (state, _d, seed) = state_with_git("acme", "public");
    let addr = spawn(state);
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-upload-pack");
    let (status, head, body) = split_response(&resp);
    assert!(status.starts_with("HTTP/1.1 200"), "status: {status}");
    assert!(
        head.contains("application/x-git-upload-pack-advertisement"),
        "Content-Type: {head}"
    );
    assert!(!body.is_empty(), "advertisement body must be non-empty");
    let text = String::from_utf8_lossy(&body);
    // v1 smart-HTTP preamble + the service name + both ref tips.
    assert!(
        text.contains("# service=git-upload-pack"),
        "preamble: {text}"
    );
    assert!(text.contains(&seed.c2.to_string()), "main tip advertised");
    assert!(
        text.contains(&seed.cf.to_string()),
        "feature tip advertised"
    );
    assert!(text.contains("refs/heads/main"), "main ref advertised");
    // The first ref line carries the capability list after a NUL.
    assert!(text.contains('\0'), "first ref carries NUL-sep caps");
    assert!(text.contains("object-format=sha1"), "caps include sha1");
    // We must NOT advertise side-band (the proto sends a bare packfile).
    assert!(
        !text.contains("side-band"),
        "no side-band advertised: {text}"
    );
}

#[test]
fn info_refs_wrong_service_is_404() {
    // receive-pack (push) is out of scope — never advertised.
    let (state, _d, _s) = state_with_git("acme", "public");
    let addr = spawn(state);
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-receive-pack");
    let (status, _h, _b) = split_response(&resp);
    assert!(status.starts_with("HTTP/1.1 404"), "status: {status}");
}

// ── UNIT: upload-pack result framing ─────────────────────────────────────────

#[test]
fn upload_pack_result_is_nak_then_pack() {
    let (state, _d, seed) = state_with_git("acme", "public");
    let addr = spawn(state);
    // A clone want body: want both tips, done (mirrors what a v1 client sends).
    let body = clone_want_body(&[seed.c2, seed.cf]);
    let resp = http_post_raw(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        &body,
    );
    let (status, head, out) = split_response(&resp);
    assert!(status.starts_with("HTTP/1.1 200"), "status: {status}");
    assert!(
        head.contains("application/x-git-upload-pack-result"),
        "Content-Type: {head}"
    );
    // First the NAK pkt-line (`0008NAK\n`), then the packfile.
    assert!(out.starts_with(b"0008NAK\n"), "leads with NAK pkt-line");
    let pack = &out[8..];
    assert_eq!(&pack[0..4], b"PACK", "packfile section starts with PACK");
    let version = u32::from_be_bytes(pack[4..8].try_into().unwrap());
    assert_eq!(version, 2, "pack version 2");
    let count = u32::from_be_bytes(pack[8..12].try_into().unwrap());
    assert_eq!(count, 7, "the full 7-object closure");
}

/// Build a v1 `want`/`done` request body as pkt-lines (what a client POSTs).
fn clone_want_body(wants: &[ObjectId]) -> Vec<u8> {
    let mut out = Vec::new();
    for w in wants {
        // A bare `want <oid>` line. Real git appends capabilities to the first want,
        // but the proto parser ignores trailing args and our envelope advertises a
        // side-band-free set, so a bare want is sufficient and faithful.
        pkt(&mut out, format!("want {w}\n").as_bytes());
    }
    out.extend_from_slice(b"0000"); // flush ends the want list
    pkt(&mut out, b"done\n");
    out
}

fn pkt(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len() + 4;
    out.extend_from_slice(format!("{len:04x}").as_bytes());
    out.extend_from_slice(data);
}

// ── UNIT: not-live + non-public 404s ─────────────────────────────────────────

#[test]
fn git_serving_404s_when_no_git_dir() {
    // `new()` leaves git_source None + git_refs empty → git serving not live.
    let dir = scratch_dir();
    std::fs::write(dir.join("acme.json"), "[]").unwrap();
    let state = AppState::new(dir, TOKEN.to_string());
    let addr = spawn(state);
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-upload-pack");
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "not-live → 404: {status}"
    );
}

#[test]
fn non_public_repo_is_404_over_git_wire() {
    // A PRIVATE repo is not cloneable over the unauthenticated git wire (no Bearer,
    // so no operator bypass) → 404, no existence oracle.
    let (state, _d, _s) = state_with_git("acme", "private");
    let addr = spawn(state);
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-upload-pack");
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "private repo → 404 over git wire: {status}"
    );
    // And the upload-pack POST is equally closed.
    let resp = http_post_raw(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        b"0000",
    );
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "upload-pack → 404: {status}"
    );
}

#[test]
fn receive_pack_is_404_push_out_of_scope() {
    let (state, _d, _s) = state_with_git("acme", "public");
    let addr = spawn(state);
    // Even on a public repo, push (git-receive-pack) is not served.
    let resp = http_post_raw(
        &addr,
        "/acme/git-receive-pack",
        "application/x-git-receive-pack-request",
        b"0000",
    );
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "receive-pack (push) out of scope → 404: {status}"
    );
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-receive-pack");
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "receive-pack advert → 404"
    );
}

// ── FALLBACK: direct-protocol clone (no `git` binary needed) ──────────────────

#[test]
fn direct_protocol_clone_bytes() {
    let (state, _d, seed) = state_with_git("acme", "public");
    let addr = spawn(state);

    // 1. info/refs → advertisement with both tips.
    let adv = http_get_raw(&addr, "/acme/info/refs?service=git-upload-pack");
    let (status, head, body) = split_response(&adv);
    assert!(status.starts_with("HTTP/1.1 200"));
    assert!(head.contains("application/x-git-upload-pack-advertisement"));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains(&seed.c2.to_string()) && text.contains(&seed.cf.to_string()));

    // 2. POST want both tips → NAK + a valid V2 packfile of the full closure.
    let body = clone_want_body(&[seed.c2, seed.cf]);
    let resp = http_post_raw(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        &body,
    );
    let (status, _h, out) = split_response(&resp);
    assert!(status.starts_with("HTTP/1.1 200"));
    assert!(out.starts_with(b"0008NAK\n"));
    assert_eq!(&out[8..12], b"PACK");
    let count = u32::from_be_bytes(out[16..20].try_into().unwrap());
    assert_eq!(count, 7, "full closure of the public repo");
}

// ── THE ORACLE: a real `git clone` ───────────────────────────────────────────

#[test]
fn real_git_clone_succeeds() {
    if !have_git() {
        eprintln!("SKIP real_git_clone_succeeds: `git` not on PATH");
        return;
    }
    let (state, _d, seed) = state_with_git("acme", "public");
    let addr = spawn(state);

    let dst = scratch_dir().join("clone");
    let url = format!("http://{addr}/acme");
    let out = Command::new("git")
        // Force v0/v1 smart-HTTP (the protocol our envelope speaks); avoid a v2
        // attempt that would expect packfile-section framing we do not emit.
        .arg("-c")
        .arg("protocol.version=0")
        .arg("clone")
        .arg("-q")
        .arg(&url)
        .arg(&dst)
        .output()
        .expect("spawn git clone");
    assert!(
        out.status.success(),
        "real git clone of hugit-serve failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The cloned repo must hold the full 7-object closure.
    let listing = git_in(
        &dst,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
    );
    let cloned: std::collections::BTreeSet<String> =
        listing.split_whitespace().map(str::to_string).collect();
    assert_eq!(cloned.len(), 7, "cloned 7 objects, got {}", cloned.len());
    assert!(cloned.contains(&seed.c2.to_string()), "main tip cloned");
    assert!(cloned.contains(&seed.cf.to_string()), "feature tip cloned");

    // And the default branch checks out with the expected file content.
    let head = git_in(&dst, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), seed.c2.to_string(), "HEAD is main@c2");
    let readme = std::fs::read_to_string(dst.join("README")).expect("README checked out");
    assert_eq!(readme, "hello hugit, again\n", "working tree content");
}

fn have_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git_in(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
