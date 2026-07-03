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
//! Push (`git-receive-pack`) is served, GATED: with the write flag OFF it is the
//! honest 403 (the existing forbidden-path tests); [`real_git_push_succeeds`] flips
//! the flag on a git-dir-backed serve and proves a real `git push` lands + clones
//! back from a fresh instance.

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
use hugit_serve::clone_pack::{
    CloneCacheSeam, CurrentPointer, clone_pack_current_key, clone_pack_key, refset_sha,
};
use hugit_serve::server::serve_on;
use hugit_serve::state::{AppState, R2Config};
use tiny_http::{Method, Response, Server};

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
    // Wire this repo's git seam (the `new()` ctor leaves the repo map empty). The
    // state owns the seeded CAS; the returned `Seed` carries the refs + tip oids the
    // assertions read. The root tree isn't exercised by the git wire; seed the
    // empty-tree oid as a placeholder.
    state.set_repo_git(
        repo,
        Arc::new(cas),
        ObjectId::empty_tree(gix_hash::Kind::Sha1),
        seed.refs.clone(),
    );
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
    // An unknown service (not upload-pack, not receive-pack) is a 404.
    let (state, _d, _s) = state_with_git("acme", "public");
    let addr = spawn(state);
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-unknown-service");
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "unknown service → 404: {status}"
    );
    // No service param at all is also 404.
    let resp = http_get_raw(&addr, "/acme/info/refs");
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "no service → 404: {status}"
    );
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

// ── AUTHENTICATED CLONE-BACK (the go-live cross-tenant READ gate) ────────────
//
// A real user clones THEIR private repo with THEIR engine token. The principal is
// derived from `Authorization: Bearer <token>` validated against the SAME Tier-1
// token store the receive-pack write side uses, then fed to `authorize_read`.

/// Mint a real engine token (the Tier-1 path: a Clerk session exchange would have
/// minted this) for tenant `org` in `state`'s token store, returning the raw token
/// a client presents as `Authorization: Bearer <raw>`. This is the SAME store
/// `clone_principal`/`two_tier_auth` look up.
fn mint_tenant_token(state: &AppState, org: &str, user: &str) -> String {
    state
        .token_store
        .mint(&hugit_serve::token::ClerkPrincipal {
            user: user.to_string(),
            org: org.to_string(),
            fresh_auth: false,
        })
        .expect("mint engine token")
}

/// GET info/refs?service=git-upload-pack with an explicit `Authorization: Bearer`.
fn http_get_authed(addr: &str, path: &str, bearer: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {bearer}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).unwrap();
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).unwrap();
    resp
}

/// POST git-upload-pack with an explicit `Authorization: Bearer`.
fn http_post_authed(
    addr: &str,
    path: &str,
    content_type: &str,
    bearer: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {bearer}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).unwrap();
    resp
}

#[test]
fn authed_clone_of_private_repo_succeeds() {
    // A real `git clone` carrying the OWNER tenant's Bearer of a PRIVATE repo
    // (owner_tenant=org-a) SUCCEEDS — the advertise serves the tips and the
    // upload-pack POST serves the pack. (state_with_git sets owner_tenant=org-a.)
    if !have_git() {
        eprintln!("SKIP authed_clone_of_private_repo_succeeds: `git` not on PATH");
        return;
    }
    let (state, _d, _seed) = state_with_git("acme", "private");
    let token = mint_tenant_token(&state, "org-a", "user-1");
    let addr = spawn(state);

    let dst = scratch_dir().join("authed_clone");
    let out = Command::new("git")
        .args(["-c", "protocol.version=0"])
        .arg("-c")
        .arg(format!("http.extraHeader=Authorization: Bearer {token}"))
        .args([
            "clone",
            "-q",
            &format!("http://{addr}/acme"),
            dst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "owner-tenant clone of its private repo must succeed:\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The working tree was reconstructed (the README blob from the seed graph).
    assert!(
        dst.join("README").exists(),
        "the clone reconstructed the README from the private repo's closure"
    );
}

#[test]
fn anon_clone_of_private_repo_is_404() {
    // No Bearer at all → anonymous → a private repo is a uniform 404 (unchanged).
    let (state, _d, _s) = state_with_git("acme", "private");
    let addr = spawn(state);
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-upload-pack");
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "anon clone of private → 404: {status}"
    );
}

#[test]
fn foreign_tenant_clone_of_private_repo_is_404() {
    // THE crown jewel: a VALID Bearer for the WRONG tenant (org-b) cloning org-a's
    // private repo gets the SAME 404 as a non-existent repo — a foreign tenant must
    // not even learn the repo exists. Both the advertise and the POST.
    let (state, _d, _s) = state_with_git("acme", "private");
    let foreign = mint_tenant_token(&state, "org-b", "user-x");
    let addr = spawn(state);

    let resp = http_get_authed(&addr, "/acme/info/refs?service=git-upload-pack", &foreign);
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "foreign-tenant advertise → 404 (cross-tenant isolation): {status}"
    );

    let resp = http_post_authed(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        &foreign,
        b"0000",
    );
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "foreign-tenant upload-pack POST → 404: {status}"
    );
}

#[test]
fn invalid_bearer_falls_back_to_anonymous() {
    // An invalid/garbage Bearer is treated as ANONYMOUS (fail-closed, NOT
    // authenticated): a PUBLIC repo still clones (anon-public gate), a PRIVATE repo
    // is 404. Same for an unknown token shape.
    let garbage = "deadbeefnotarealtoken";

    // Public repo + garbage Bearer → still advertises (treated anon).
    let (state, _d, seed) = state_with_git("pubrepo", "public");
    let addr = spawn(state);
    let resp = http_get_authed(&addr, "/pubrepo/info/refs?service=git-upload-pack", garbage);
    let (status, _h, body) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 200"),
        "garbage Bearer on a PUBLIC repo → still clones (anon): {status}"
    );
    assert!(
        String::from_utf8_lossy(&body).contains(&seed.c2.to_string()),
        "public advertise still serves the tip under a garbage Bearer"
    );

    // Private repo + garbage Bearer → 404 (NOT authenticated, fail-closed).
    let (state2, _d2, _s2) = state_with_git("privrepo", "private");
    let addr2 = spawn(state2);
    let resp = http_get_authed(
        &addr2,
        "/privrepo/info/refs?service=git-upload-pack",
        garbage,
    );
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "garbage Bearer on a PRIVATE repo → 404 (fail-closed to anon): {status}"
    );
}

#[test]
#[allow(non_snake_case)] // the spec'd test name uses `POST` for legibility
fn upload_pack_POST_gates_identically_to_advertise() {
    // No split-route bypass: the upload-pack POST must 404 for EXACTLY the cases
    // the advertise hides. Checked for both anon-on-private and foreign-tenant.
    let (state, _d, _s) = state_with_git("acme", "private");
    let foreign = mint_tenant_token(&state, "org-b", "user-x");
    let addr = spawn(state);

    // (a) anon POST on a private repo → 404 (advertise also 404s — proven above).
    let resp = http_post_raw(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        b"0000",
    );
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "anon upload-pack POST on private → 404: {status}"
    );

    // (b) foreign-tenant POST → 404 (the advertise hid it; the POST must not leak
    //     a pack the advertise concealed).
    let resp = http_post_authed(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        &foreign,
        b"0000",
    );
    let (status, _h, _b) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 404"),
        "foreign-tenant upload-pack POST → 404 (gates like the advertise): {status}"
    );
}

#[test]
fn receive_pack_post_is_403_with_human_message() {
    // Push (git-receive-pack POST) → 403 with a clear human-readable body,
    // NOT a silent 404. The message tells the developer to use `hugit land`.
    let (state, _d, _s) = state_with_git("acme", "public");
    let addr = spawn(state);
    let resp = http_post_raw(
        &addr,
        "/acme/git-receive-pack",
        "application/x-git-receive-pack-request",
        b"0000",
    );
    let (status, _h, body) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "receive-pack POST → 403, got: {status}"
    );
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("git push is not yet supported"),
        "body must explain push is unsupported: {body_str}"
    );
    assert!(
        body_str.contains("hugit land"),
        "body must mention 'hugit land': {body_str}"
    );
}

#[test]
fn receive_pack_info_refs_is_403_with_human_message() {
    // GET info/refs?service=git-receive-pack (push discovery) → 403.
    // Previously this was a 404; now it returns a clear 403 + message.
    let (state, _d, _s) = state_with_git("acme", "public");
    let addr = spawn(state);
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-receive-pack");
    let (status, _h, body) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "receive-pack advert → 403, got: {status}"
    );
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("git push is not yet supported"),
        "body must explain push is unsupported: {body_str}"
    );
}

#[test]
fn upload_pack_clone_still_works_after_push_403_change() {
    // Regression: the push→403 routing change must not break clone/fetch.
    let (state, _d, seed) = state_with_git("acme", "public");
    let addr = spawn(state);
    let resp = http_get_raw(&addr, "/acme/info/refs?service=git-upload-pack");
    let (status, _h, body) = split_response(&resp);
    assert!(
        status.starts_with("HTTP/1.1 200"),
        "clone still works: {status}"
    );
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains(&seed.c2.to_string()),
        "main tip still advertised after push-403 change"
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

/// End-to-end `git push` (receive-pack): a real `git push` to a flag-enabled,
/// git-dir-backed serve SUCCEEDS, and the pushed commit + ref clone back from a
/// FRESH serve instance over the same git dir (proving durable persistence:
/// objects in the dir, ref written, no same-process hot-swap relied on).
#[test]
fn real_git_push_succeeds() {
    if !have_git() {
        eprintln!("SKIP real_git_push_succeeds: `git` not on PATH");
        return;
    }
    let git_cfg = |c: &mut Command| {
        c.args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "-c",
            "protocol.version=0",
        ]);
    };

    // 1. Seed a bare "server" repo on disk with one commit on main (so the wire has
    //    a ref to advertise + objects to clone).
    let root = scratch_dir();
    let work = root.join("work");
    let bare = root.join("repo.git");
    {
        let mut c = Command::new("git");
        git_cfg(&mut c);
        c.args(["init", "-q", work.to_str().unwrap()])
            .output()
            .unwrap();
        std::fs::write(work.join("README"), b"seed\n").unwrap();
        for args in [
            vec!["add", "."],
            vec!["commit", "-q", "-m", "init"],
            vec!["branch", "-M", "main"],
        ] {
            let mut c = Command::new("git");
            git_cfg(&mut c);
            c.arg("-C").arg(&work).args(&args).output().unwrap();
        }
        Command::new("git")
            .args(["init", "-q", "--bare", bare.to_str().unwrap()])
            .output()
            .unwrap();
        let mut c = Command::new("git");
        git_cfg(&mut c);
        let push = c
            .arg("-C")
            .arg(&work)
            .args(["push", "-q", bare.to_str().unwrap(), "main"])
            .output()
            .unwrap();
        assert!(push.status.success(), "seed push to bare failed");
        // The bare's default HEAD may be `master` (init default) while we pushed
        // `main` → point HEAD at main so the engine resolves HEAD's tree at load.
        Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["symbolic-ref", "HEAD", "refs/heads/main"])
            .output()
            .unwrap();
    }

    // 2. Public-meta log (clone is anon-public; push authorizes via the dev-token →
    //    operator). Serve instance A: git-dir seam + the receive-pack flag ON.
    let log_dir = scratch_dir();
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility": "public", "owner_tenant": "org-a"}).to_string(),
        0,
    );
    std::fs::write(
        log_dir.join("pushrepo.json"),
        serde_json::to_string_pretty(log.records()).unwrap(),
    )
    .unwrap();
    let build_state = || {
        let mut s = AppState::new(log_dir.clone(), TOKEN.to_string());
        s.set_repo_from_git_dir("pushrepo", bare.to_str().unwrap())
            .expect("load bare git dir");
        s.enable_write_path();
        s
    };
    let addr_a = spawn(build_state());

    // 3. Clone from A, make a new commit, PUSH it to a NEW branch (a create).
    let clone_a = root.join("clone_a");
    let mut c = Command::new("git");
    git_cfg(&mut c);
    let out = c
        .args([
            "clone",
            "-q",
            &format!("http://{addr_a}/pushrepo"),
            clone_a.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "clone from A failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Build the new ref on an ORPHAN branch so the pushed pack is SELF-CONTAINED
    // (no base objects already on the server). v0 receive_pack verifies the target
    // is reachable within the delivered pack; incremental/thin-pack reachability
    // (consulting the CAS for server-side bases) is the documented W3 follow-up.
    let mut c = Command::new("git");
    git_cfg(&mut c);
    c.arg("-C")
        .arg(&clone_a)
        .args(["checkout", "-q", "--orphan", "pushed"])
        .output()
        .unwrap();
    let mut c = Command::new("git");
    git_cfg(&mut c);
    c.arg("-C")
        .arg(&clone_a)
        .args(["rm", "-rfq", "."])
        .output()
        .ok();
    std::fs::write(clone_a.join("NEWFILE"), b"pushed content\n").unwrap();
    for args in [vec!["add", "."], vec!["commit", "-q", "-m", "push me"]] {
        let mut c = Command::new("git");
        git_cfg(&mut c);
        c.arg("-C").arg(&clone_a).args(&args).output().unwrap();
    }
    let mut c = Command::new("git");
    git_cfg(&mut c);
    let push = c
        .arg("-C")
        .arg(&clone_a)
        .arg("-c")
        .arg(format!("http.extraHeader=Authorization: Bearer {TOKEN}"))
        .args([
            "push",
            "-q",
            &format!("http://{addr_a}/pushrepo"),
            "HEAD:refs/heads/pushed",
        ])
        .output()
        .unwrap();
    assert!(
        push.status.success(),
        "git push (receive-pack) failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&push.stdout),
        String::from_utf8_lossy(&push.stderr)
    );

    // 4. The push durably landed: a FRESH serve instance B over the same git dir
    //    advertises + serves the new ref's commit (no same-process hot-swap used).
    let addr_b = spawn(build_state());
    let clone_b = root.join("clone_b");
    let mut c = Command::new("git");
    git_cfg(&mut c);
    let out = c
        .args([
            "clone",
            "-q",
            "--branch",
            "pushed",
            &format!("http://{addr_b}/pushrepo"),
            clone_b.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "clone of the pushed branch from a fresh serve failed:\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let content = std::fs::read_to_string(clone_b.join("NEWFILE"))
        .expect("pushed file present after clone-back");
    assert_eq!(
        content, "pushed content\n",
        "pushed commit's content round-trips"
    );
}

/// End-to-end `git push --delete <branch>` (receive-pack delete): create a branch over
/// the wire, then DELETE it. After the delete a fresh serve over the same git dir no
/// longer advertises the branch (durable removal), while `main` keeps serving.
/// Deleting the default branch (`main`) is REFUSED.
#[test]
fn real_git_push_delete_ref_succeeds() {
    if !have_git() {
        eprintln!("SKIP real_git_push_delete_ref_succeeds: `git` not on PATH");
        return;
    }
    let git_cfg = |c: &mut Command| {
        c.args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "-c",
            "protocol.version=0",
        ]);
    };

    // Seed a bare server repo with one commit on main.
    let root = scratch_dir();
    let work = root.join("work");
    let bare = root.join("repo.git");
    {
        let mut c = Command::new("git");
        git_cfg(&mut c);
        c.args(["init", "-q", work.to_str().unwrap()])
            .output()
            .unwrap();
        std::fs::write(work.join("README"), b"seed\n").unwrap();
        for args in [
            vec!["add", "."],
            vec!["commit", "-q", "-m", "init"],
            vec!["branch", "-M", "main"],
        ] {
            let mut c = Command::new("git");
            git_cfg(&mut c);
            c.arg("-C").arg(&work).args(&args).output().unwrap();
        }
        Command::new("git")
            .args(["init", "-q", "--bare", bare.to_str().unwrap()])
            .output()
            .unwrap();
        let mut c = Command::new("git");
        git_cfg(&mut c);
        c.arg("-C")
            .arg(&work)
            .args(["push", "-q", bare.to_str().unwrap(), "main"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["symbolic-ref", "HEAD", "refs/heads/main"])
            .output()
            .unwrap();
    }

    let log_dir = scratch_dir();
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility": "public", "owner_tenant": "org-a"}).to_string(),
        0,
    );
    std::fs::write(
        log_dir.join("pushrepo.json"),
        serde_json::to_string_pretty(log.records()).unwrap(),
    )
    .unwrap();
    let build_state = || {
        let mut s = AppState::new(log_dir.clone(), TOKEN.to_string());
        s.set_repo_from_git_dir("pushrepo", bare.to_str().unwrap())
            .expect("load bare git dir");
        s.enable_write_path();
        s
    };
    let addr_a = spawn(build_state());

    // Clone, create an orphan branch, push it (a create).
    let clone_a = root.join("clone_a");
    let mut c = Command::new("git");
    git_cfg(&mut c);
    c.args([
        "clone",
        "-q",
        &format!("http://{addr_a}/pushrepo"),
        clone_a.to_str().unwrap(),
    ])
    .output()
    .unwrap();
    let mut c = Command::new("git");
    git_cfg(&mut c);
    c.arg("-C")
        .arg(&clone_a)
        .args(["checkout", "-q", "--orphan", "doomed"])
        .output()
        .unwrap();
    let mut c = Command::new("git");
    git_cfg(&mut c);
    c.arg("-C")
        .arg(&clone_a)
        .args(["rm", "-rfq", "."])
        .output()
        .ok();
    std::fs::write(clone_a.join("F"), b"x\n").unwrap();
    for args in [vec!["add", "."], vec!["commit", "-q", "-m", "doomed"]] {
        let mut c = Command::new("git");
        git_cfg(&mut c);
        c.arg("-C").arg(&clone_a).args(&args).output().unwrap();
    }
    let auth = format!("http.extraHeader=Authorization: Bearer {TOKEN}");
    let mut c = Command::new("git");
    git_cfg(&mut c);
    let push = c
        .arg("-C")
        .arg(&clone_a)
        .arg("-c")
        .arg(&auth)
        .args([
            "push",
            "-q",
            &format!("http://{addr_a}/pushrepo"),
            "HEAD:refs/heads/doomed",
        ])
        .output()
        .unwrap();
    assert!(
        push.status.success(),
        "create-branch push failed:\nstderr={}",
        String::from_utf8_lossy(&push.stderr)
    );

    // Read back the tips a delete must carry as its `old_oid` (git sends the real
    // current tip). `doomed` was just created; `main` is the seeded tip.
    let doomed_oid = git_in(&bare, &["rev-parse", "refs/heads/doomed"])
        .trim()
        .to_string();
    let main_oid = git_in(&bare, &["rev-parse", "refs/heads/main"])
        .trim()
        .to_string();

    // The GIT_DIR create-push lands the ref on disk but does NOT hot-swap instance A's
    // boot-time `git_refs` snapshot (only the CAS path hot-swaps). The delete's
    // stale-check reads that authoritative snapshot, so it must run against a FRESH
    // instance whose boot snapshot already includes `doomed` from the on-disk dir.
    // (The git client refuses to delete a remote's HEAD branch + has version-specific
    // remote-tracking quirks, so the delete is driven by a RAW receive-pack POST — it
    // exercises the exact serve handler the git client would hit, deterministically.)
    let addr_b = spawn(build_state());

    // 1. Deleting the DEFAULT branch (main) → `ng refuse-delete-default-branch`, ref kept.
    let report = post_receive_delete(&addr_b, "pushrepo", &main_oid, "refs/heads/main");
    assert!(
        report.contains("unpack ok") && report.contains("ng refs/heads/main"),
        "default-branch delete is refused per-ref: {report}"
    );
    assert!(
        report.contains("refuse-delete-default-branch"),
        "the refusal reason is the frozen token: {report}"
    );

    // 2. A stale old_oid on `doomed` → `ng non-fast-forward`, ref kept.
    let stale = "0".repeat(39) + "1";
    let report = post_receive_delete(&addr_b, "pushrepo", &stale, "refs/heads/doomed");
    assert!(
        report.contains("ng refs/heads/doomed") && report.contains("non-fast-forward"),
        "a stale-tip delete is a non-fast-forward: {report}"
    );

    // 3. The valid delete of `doomed` (correct current tip) → `ok refs/heads/doomed`.
    let report = post_receive_delete(&addr_b, "pushrepo", &doomed_oid, "refs/heads/doomed");
    assert!(
        report.contains("unpack ok") && report.contains("ok refs/heads/doomed"),
        "the delete succeeds: {report}"
    );
    assert!(
        !report.contains("ng "),
        "no ng line on the successful delete: {report}"
    );

    // The same instance B (live hot-swap) no longer advertises `doomed`; main remains.
    let advert_b = http_get_raw(&addr_b, "/pushrepo/info/refs?service=git-upload-pack");
    let (_, _, body_b) = split_response(&advert_b);
    let body_b = String::from_utf8_lossy(&body_b);
    assert!(
        !body_b.contains("refs/heads/doomed"),
        "the deleted ref drops from B's advertise with no reboot: {body_b}"
    );
    assert!(
        body_b.contains("refs/heads/main"),
        "main still advertised on B"
    );

    // And it is DURABLE: a FRESH serve C over the same git dir also lacks `doomed`.
    let addr_c = spawn(build_state());
    let advert = http_get_raw(&addr_c, "/pushrepo/info/refs?service=git-upload-pack");
    let (_, _, body_bytes) = split_response(&advert);
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(
        !body.contains("refs/heads/doomed"),
        "the deleted ref is durably gone from a fresh serve's advertise"
    );
    assert!(
        body.contains("refs/heads/main"),
        "main still advertised after the delete"
    );
}

/// POST a delete-only `git-receive-pack` body (one `<old> 00..0 <ref>` command +
/// `report-status` cap + a flush, NO packfile) to `/<repo>/git-receive-pack` with the
/// dev-token bearer, and return the decoded report-status text. Drives the exact serve
/// handler a `git push --delete` would hit, without the git client's remote-view quirks.
fn post_receive_delete(addr: &str, repo: &str, old_oid: &str, ref_name: &str) -> String {
    let zero = "0".repeat(40);
    let mut first = format!("{old_oid} {zero} {ref_name}").into_bytes();
    first.push(0);
    first.extend_from_slice(b"report-status");
    let mut body = Vec::new();
    body.extend_from_slice(format!("{:04x}", first.len() + 4).as_bytes());
    body.extend_from_slice(&first);
    body.extend_from_slice(b"0000"); // flush — no pack follows (a delete)

    let mut stream = TcpStream::connect(addr).expect("connect");
    let head = format!(
        "POST /{repo}/git-receive-pack HTTP/1.1\r\nHost: t\r\nAuthorization: Bearer {TOKEN}\r\n\
         Content-Type: application/x-git-receive-pack-request\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).unwrap();
    let (_, _, body_bytes) = split_response(&resp);
    // Decode the pkt-line report into readable text (strip 4-hex length prefixes).
    let mut out = String::new();
    let mut b = &body_bytes[..];
    while b.len() >= 4 {
        let len =
            usize::from_str_radix(std::str::from_utf8(&b[..4]).unwrap_or("zzzz"), 16).unwrap_or(0);
        if len < 4 {
            b = &b[4..];
            continue;
        }
        if len > b.len() {
            break;
        }
        out.push_str(&String::from_utf8_lossy(&b[4..len]));
        b = &b[len..];
    }
    out
}

// ── WP-BC: the cached full-clone pack served over the wire ───────────────────
//
// A pre-assembled full-clone pack is stored as ONE R2 object; a FULL clone streams
// it instead of walking the whole object closure. These tests spin a MOCK R2 (a
// tiny_http server that serves seeded keys and REJECTS PUTs) and point a seeded
// repo's `clone_cache` at it. The mock rejecting PUTs makes a background bootstrap
// rebuild inert (it cannot populate the served map), so the fall-open cases stay
// deterministic. The clone-cache seam's `tenant`/`repo_slug` match the seeded keys.

const CACHE_TENANT: &str = "test-tenant";
const CACHE_BUCKET: &str = "b";

/// A recognizable SENTINEL pack blob — impossible to produce by walking the 7-object
/// seed graph, so a response carrying it PROVES the cached bytes were served (a
/// cache HIT), not a re-walk. Starts with `PACK` for realism (never parsed here).
const SENTINEL_PACK: &[u8] = b"PACK\x00\x00\x00\x02SENTINEL-CACHED-CLONE-PACK";

/// Spawn a MOCK R2 HTTP server that answers `GET /<bucket>/<key>` from `objects`
/// (keyed by R2 KEY) and returns **403** for any PUT (so a background clone-pack
/// rebuild can never populate the served map → the fall-open tests are
/// deterministic). Returns the bound `127.0.0.1:<port>`.
fn spawn_mock_r2(objects: std::collections::BTreeMap<String, Vec<u8>>) -> String {
    let server = Server::http("127.0.0.1:0").expect("bind mock R2");
    let addr = server
        .server_addr()
        .to_ip()
        .expect("mock R2 ip addr")
        .to_string();
    let prefix = format!("/{CACHE_BUCKET}/");
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let is_put = *request.method() == Method::Put;
            let path = request.url().split('?').next().unwrap_or("").to_string();
            if is_put {
                // Reject writes: a rebuild's `store_pack_and_flip` PUT 403s (like a
                // read-only cred), so the served map is immutable for the test.
                let _ = request.respond(Response::from_string("").with_status_code(403));
                continue;
            }
            let key = path.strip_prefix(&prefix).unwrap_or("");
            match objects.get(key) {
                Some(bytes) => {
                    let _ = request.respond(Response::from_data(bytes.clone()));
                }
                None => {
                    let _ = request.respond(Response::from_string("").with_status_code(404));
                }
            }
        }
    });
    addr
}

/// Build the clone-cache seam pointing at a mock R2 at `mock_addr`, scoped to
/// `CACHE_TENANT`/`repo` (matching the keys the tests seed).
fn cache_seam(mock_addr: &str, repo: &str) -> CloneCacheSeam {
    CloneCacheSeam {
        r2: R2Config::for_test_endpoint(format!("http://{mock_addr}"), CACHE_BUCKET.to_string()),
        tenant: CACHE_TENANT.to_string(),
        repo_slug: repo.to_string(),
    }
}

/// Serialize a `current.json` pointer for `refset_sha` naming a pack at `pack_key`.
fn current_json(refset: &str, pack_key: &str) -> Vec<u8> {
    serde_json::to_vec(&CurrentPointer {
        refset_sha: refset.to_string(),
        pack_key: pack_key.to_string(),
        object_count: 1,
        built_at_ms: 0,
    })
    .unwrap()
}

#[test]
fn full_clone_serves_cached_pack_bytes() {
    // A FULL clone (wants == the whole advertised tip-set, no haves) of a repo with a
    // MATCHING cached pack streams the cached bytes, framed `0008NAK\n` + raw pack —
    // byte-identical framing to the walk path, but ONE R2 GET instead of a closure walk.
    let (mut state, _d, seed) = state_with_git("acme", "public");
    let sha = refset_sha(&seed.refs);
    let pack_key = clone_pack_key(CACHE_TENANT, "acme", &sha);
    let current_key = clone_pack_current_key(CACHE_TENANT, "acme");
    let mut objects = std::collections::BTreeMap::new();
    objects.insert(current_key, current_json(&sha, &pack_key));
    objects.insert(pack_key, SENTINEL_PACK.to_vec());
    let mock = spawn_mock_r2(objects);
    state.set_repo_clone_cache("acme", cache_seam(&mock, "acme"));
    let addr = spawn(state);

    // Full clone: want BOTH advertised tips, no haves.
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
        "Content-Type (same as the walk path): {head}"
    );
    // The response is EXACTLY the NAK pkt-line + the cached (sentinel) pack bytes.
    let mut expected = b"0008NAK\n".to_vec();
    expected.extend_from_slice(SENTINEL_PACK);
    assert_eq!(
        out, expected,
        "a full clone streams the cached pack verbatim after the NAK pkt-line"
    );
}

#[test]
fn full_clone_absent_cache_falls_open_to_walk() {
    // No `current.json` in R2 → the cache lookup returns None → the serve FALLS OPEN
    // to the slow walk, which still produces a correct full 7-object clone.
    let (mut state, _d, seed) = state_with_git("acme", "public");
    let mock = spawn_mock_r2(std::collections::BTreeMap::new()); // empty: GET 404, PUT 403
    state.set_repo_clone_cache("acme", cache_seam(&mock, "acme"));
    let addr = spawn(state);

    let body = clone_want_body(&[seed.c2, seed.cf]);
    let resp = http_post_raw(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        &body,
    );
    let (status, _h, out) = split_response(&resp);
    assert!(status.starts_with("HTTP/1.1 200"), "status: {status}");
    assert!(out.starts_with(b"0008NAK\n"), "NAK then a real walked pack");
    assert_eq!(&out[8..12], b"PACK", "a real packfile from the walk");
    let count = u32::from_be_bytes(out[16..20].try_into().unwrap());
    assert_eq!(count, 7, "the full 7-object closure (walk, not the cache)");
}

#[test]
fn full_clone_stale_cache_falls_open_to_walk() {
    // A `current.json` whose `refset_sha` does NOT match the live refs (a moved ref)
    // is REFUSED — the serve recomputes refset_sha and falls open to the walk. Even
    // though a (sentinel) pack object is present, it is NEVER served on a mismatch.
    let (mut state, _d, seed) = state_with_git("acme", "public");
    let wrong_sha = "0".repeat(64); // deliberately != refset_sha(live refs)
    let pack_key = clone_pack_key(CACHE_TENANT, "acme", &wrong_sha);
    let current_key = clone_pack_current_key(CACHE_TENANT, "acme");
    let mut objects = std::collections::BTreeMap::new();
    objects.insert(current_key, current_json(&wrong_sha, &pack_key));
    objects.insert(pack_key, SENTINEL_PACK.to_vec());
    let mock = spawn_mock_r2(objects);
    state.set_repo_clone_cache("acme", cache_seam(&mock, "acme"));
    let addr = spawn(state);

    let body = clone_want_body(&[seed.c2, seed.cf]);
    let resp = http_post_raw(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        &body,
    );
    let (_s, _h, out) = split_response(&resp);
    assert!(out.starts_with(b"0008NAK\n"), "NAK prefix");
    assert_ne!(
        &out[8..],
        SENTINEL_PACK,
        "a refset_sha mismatch must NEVER serve the stale cached pack"
    );
    assert_eq!(
        &out[8..12],
        b"PACK",
        "the fall-open walk produced a real pack"
    );
    let count = u32::from_be_bytes(out[16..20].try_into().unwrap());
    assert_eq!(
        count, 7,
        "the full closure from the walk, not the stale cache"
    );
}

#[test]
fn fetch_with_haves_never_serves_cached_pack() {
    // A fetch (NON-empty `have` list) is NOT a full clone → the cached pack is NEVER
    // consulted, even though a matching cache is present. It walks (want-minus-have).
    let (mut state, _d, seed) = state_with_git("acme", "public");
    let sha = refset_sha(&seed.refs);
    let pack_key = clone_pack_key(CACHE_TENANT, "acme", &sha);
    let current_key = clone_pack_current_key(CACHE_TENANT, "acme");
    let mut objects = std::collections::BTreeMap::new();
    objects.insert(current_key, current_json(&sha, &pack_key)); // a MATCHING cache
    objects.insert(pack_key, SENTINEL_PACK.to_vec());
    let mock = spawn_mock_r2(objects);
    state.set_repo_clone_cache("acme", cache_seam(&mock, "acme"));
    let addr = spawn(state);

    // want both tips, but HAVE one of them (c2) → a fetch, not a full clone.
    let mut body = Vec::new();
    pkt(&mut body, format!("want {}\n", seed.c2).as_bytes());
    pkt(&mut body, format!("want {}\n", seed.cf).as_bytes());
    body.extend_from_slice(b"0000");
    pkt(&mut body, format!("have {}\n", seed.c2).as_bytes());
    pkt(&mut body, b"done\n");
    let resp = http_post_raw(
        &addr,
        "/acme/git-upload-pack",
        "application/x-git-upload-pack-request",
        &body,
    );
    let (_s, _h, out) = split_response(&resp);
    assert!(out.starts_with(b"0008NAK\n"), "NAK prefix");
    assert_ne!(
        &out[8..],
        SENTINEL_PACK,
        "a fetch WITH haves must never serve the cached full-clone pack"
    );
    assert_eq!(&out[8..12], b"PACK", "a real walked delta pack");
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
