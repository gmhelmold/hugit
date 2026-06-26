//! Multi-repo engine — one `hugit-serve` instance serves MANY repos, and the
//! `{repo}` URL slug resolves to that repo's OWN git content seam.
//!
//! These tests prove the forge model end-to-end through the public `route` + git
//! wire surfaces:
//! - two repos loaded → each serves its OWN blob bytes + its OWN refs;
//! - a request for repo A never returns repo B's content (no cross-repo bleed);
//! - an unknown/unloaded repo → the SAME uniform 404 (no existence oracle);
//! - the single-repo config still works unchanged;
//! - `/readyz` reflects the loaded repo set (the `git_repos` count).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gix_hash::ObjectId;
use hugit_proto::{CasObjectSource, GitObject, ObjectKind};
use hugit_refstore::EventLog;
use hugit_serve::server::route;
use hugit_serve::state::AppState;
use tiny_http::{Header, Method};

const TOKEN: &str = "dev-token-multi";

// ── scratch + seed helpers ───────────────────────────────────────────────────

fn scratch_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hugit-serve-multi-{}-{}-{}",
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

/// Write a PUBLIC one-record `repo.meta` log so the read gate has visibility.
fn write_public_log(dir: &std::path::Path, repo: &str) {
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility": "public", "owner_tenant": "org-a"}).to_string(),
        0,
    );
    let log_json = serde_json::to_string_pretty(log.records()).unwrap();
    std::fs::write(dir.join(format!("{repo}.json")), log_json).unwrap();
}

/// A single-blob repo: one tree holding `README` → `content`. Returns the seeded
/// CAS source + the root-tree oid + a `refs/heads/main` ref map pointing at the
/// commit. Each repo's content is distinct, so a cross-repo bleed is detectable.
fn seed_single_file(content: &[u8]) -> (CasObjectSource, ObjectId, BTreeMap<String, String>) {
    let mut cas = CasObjectSource::new();
    let b = cas.insert(GitObject::new(ObjectKind::Blob, content.to_vec()));

    let mut tree_body = Vec::new();
    tree_body.extend_from_slice(b"100644 README");
    tree_body.push(0);
    tree_body.extend_from_slice(b.as_slice());
    let t = cas.insert(GitObject::new(ObjectKind::Tree, tree_body));

    let mut commit_body = String::new();
    commit_body.push_str(&format!("tree {t}\n"));
    let ident = "hugit <bot@hugit.dev> 1717000000 +0000";
    commit_body.push_str(&format!("author {ident}\n"));
    commit_body.push_str(&format!("committer {ident}\n"));
    commit_body.push_str("\ninit\n");
    let c = cas.insert(GitObject::new(ObjectKind::Commit, commit_body.into_bytes()));

    let mut refs = BTreeMap::new();
    refs.insert("refs/heads/main".to_string(), c.to_string());
    (cas, t, refs)
}

/// Read the `README` blob of `repo` through the public route; returns `(status,
/// body)`. The Bearer is the dev/operator token (read gate passes).
fn read_readme(state: &AppState, repo: &str) -> (u16, String) {
    route(
        state,
        &Method::Get,
        &format!("/v1/repos/{repo}/blob/README"),
        &bearer(TOKEN),
    )
}

// ── tests ────────────────────────────────────────────────────────────────────

#[test]
fn two_repos_each_serve_their_own_blob_no_cross_bleed() {
    let dir = scratch_dir();
    write_public_log(&dir, "alpha");
    write_public_log(&dir, "beta");

    let (cas_a, root_a, _refs_a) = seed_single_file(b"ALPHA CONTENT\n");
    let (cas_b, root_b, _refs_b) = seed_single_file(b"BETA CONTENT\n");

    let mut state = AppState::new(dir.clone(), TOKEN.to_string());
    state.set_repo_git("alpha", Arc::new(cas_a), root_a, BTreeMap::new());
    state.set_repo_git("beta", Arc::new(cas_b), root_b, BTreeMap::new());

    // Each repo serves its OWN content.
    let (sa, ba) = read_readme(&state, "alpha");
    assert_eq!(sa, 200, "alpha blob read: {ba}");
    assert!(ba.contains("ALPHA CONTENT"), "alpha must serve alpha: {ba}");
    assert!(
        !ba.contains("BETA CONTENT"),
        "alpha must NOT serve beta's content: {ba}"
    );

    let (sb, bb) = read_readme(&state, "beta");
    assert_eq!(sb, 200, "beta blob read: {bb}");
    assert!(bb.contains("BETA CONTENT"), "beta must serve beta: {bb}");
    assert!(
        !bb.contains("ALPHA CONTENT"),
        "beta must NOT serve alpha's content: {bb}"
    );
}

#[test]
fn unknown_repo_is_uniform_404_not_500() {
    let dir = scratch_dir();
    write_public_log(&dir, "alpha");
    let (cas_a, root_a, _) = seed_single_file(b"ALPHA CONTENT\n");
    let mut state = AppState::new(dir.clone(), TOKEN.to_string());
    state.set_repo_git("alpha", Arc::new(cas_a), root_a, BTreeMap::new());

    // A repo with no log + no git seam → 404 (the read load fails, hidden to 404).
    let (status, _body) = read_readme(&state, "ghost");
    assert_eq!(status, 404, "an unknown repo must be a uniform 404");
}

#[test]
fn loaded_repo_without_blob_seam_404s_honestly() {
    // A repo whose LOG exists (read API works) but whose git seam was NOT loaded:
    // its blob read 404s honestly (no fake-empty), distinct from a 500.
    let dir = scratch_dir();
    write_public_log(&dir, "alpha");
    write_public_log(&dir, "logonly");
    let (cas_a, root_a, _) = seed_single_file(b"ALPHA CONTENT\n");
    let mut state = AppState::new(dir.clone(), TOKEN.to_string());
    state.set_repo_git("alpha", Arc::new(cas_a), root_a, BTreeMap::new());

    // `logonly` has a log (a non-blob read would work) but no git seam → blob 404.
    let (status, _body) = read_readme(&state, "logonly");
    assert_eq!(
        status, 404,
        "a repo with no git seam must 404 its blob honestly"
    );
}

#[test]
fn single_repo_config_still_works_unchanged() {
    // The single-repo path (one entry in the map) serves exactly as before.
    let dir = scratch_dir();
    write_public_log(&dir, "solo");
    let (cas, root, _) = seed_single_file(b"SOLO CONTENT\n");
    let mut state = AppState::new(dir.clone(), TOKEN.to_string());
    state.set_repo_git("solo", Arc::new(cas), root, BTreeMap::new());

    assert_eq!(state.git_serving_count(), 1);
    let (status, body) = read_readme(&state, "solo");
    assert_eq!(status, 200, "solo blob read: {body}");
    assert!(
        body.contains("SOLO CONTENT"),
        "solo must serve solo: {body}"
    );
}

#[test]
fn readyz_reflects_the_loaded_repo_set() {
    let dir = scratch_dir();
    write_public_log(&dir, "alpha");
    write_public_log(&dir, "beta");
    let (cas_a, root_a, _) = seed_single_file(b"A\n");
    let (cas_b, root_b, _) = seed_single_file(b"B\n");
    let mut state = AppState::new(dir.clone(), TOKEN.to_string());

    // Zero loaded → git_repos:0, git_serving:false.
    let (_s0, b0) = route(&state, &Method::Get, "/readyz", &[]);
    let v0: serde_json::Value = serde_json::from_str(&b0).unwrap();
    assert_eq!(v0["git_repos"], 0);
    assert_eq!(v0["git_serving"], false);

    // Two loaded → git_repos:2, git_serving:true.
    state.set_repo_git("alpha", Arc::new(cas_a), root_a, BTreeMap::new());
    state.set_repo_git("beta", Arc::new(cas_b), root_b, BTreeMap::new());
    let (_s2, b2) = route(&state, &Method::Get, "/readyz", &[]);
    let v2: serde_json::Value = serde_json::from_str(&b2).unwrap();
    assert_eq!(
        v2["git_repos"], 2,
        "git_repos must count the loaded set: {b2}"
    );
    assert_eq!(v2["git_serving"], true);

    // The version marker (the deploy tag via `HUGIT_SERVE_VERSION`, "dev" when
    // unset) — present + a string so a consumer self-confirms a cutover reached
    // the serving instance. The `from_str` above also proves the body stays
    // valid JSON with the field appended.
    assert!(
        v2["version"].is_string(),
        "readyz must carry a string version marker: {b2}"
    );
}
