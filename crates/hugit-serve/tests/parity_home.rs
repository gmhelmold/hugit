//! Parity test for `build_home` (Wave 1 home handler).
//!
//! Verifies:
//!   1. `build_home` compiles and returns without panicking on an empty log.
//!   2. The result serializes via `serde_json` and round-trips losslessly back
//!      into `hugit_http_contracts::RepoHomeVm`.
//!   3. STUB fields equal their honest defaults — no faked data.

use hugit_http_contracts::RepoHomeVm;
use hugit_refstore::EventLog;
use hugit_serve::handlers::build_home;

/// Build a fresh empty `EventLog` — the log is already "chain-verified" (an
/// empty log has a trivially valid chain) per the frozen handler contract.
fn empty_log() -> EventLog {
    EventLog::new()
}

#[test]
fn empty_log_round_trips() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");

    // 1. Serializes without error.
    let json = serde_json::to_string(&vm).expect("RepoHomeVm serializes");

    // 2. Re-parses losslessly.
    let reparsed: RepoHomeVm = serde_json::from_str(&json).expect("JSON re-parses into RepoHomeVm");
    assert_eq!(vm, reparsed, "round-trip must be lossless");
}

#[test]
fn repo_field_is_exact() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(vm.repo, "hugit");
}

#[test]
fn stub_files_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert!(
        vm.files.is_empty(),
        "files must be [] (no git-tree API — honest STUB)"
    );
}

#[test]
fn stub_readme_html_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(
        vm.readme_html, "",
        "readme_html must be \"\" (no local rendered README — honest STUB)"
    );
}

#[test]
fn stub_about_description_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(
        vm.about.description, "",
        "about.description must be \"\" (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_topics_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert!(
        vm.about.topics.is_empty(),
        "about.topics must be [] (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_release_none() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert!(
        vm.about.release.is_none(),
        "about.release must be None (P2 — honest STUB)"
    );
}

#[test]
fn stub_about_stars_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(
        vm.about.stars, "",
        "about.stars must be \"\" (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_forks_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(
        vm.about.forks, "",
        "about.forks must be \"\" (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_license_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(
        vm.about.license, "",
        "about.license must be \"\" (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_releases_count_zero() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(
        vm.about.releases_count, 0,
        "about.releases_count must be 0 (P2 — honest STUB)"
    );
}

#[test]
fn stub_about_languages_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert!(
        vm.about.languages.is_empty(),
        "about.languages must be [] (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_synergy_lines_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert!(
        vm.synergy.lines.is_empty(),
        "synergy.lines must be [] (no live AC seam — honest STUB)"
    );
}

#[test]
fn empty_log_branch_count_zero() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(vm.branch_count, 0, "empty log → branch_count 0");
}

#[test]
fn empty_log_tag_count_zero() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(vm.tag_count, 0, "empty log → tag_count 0");
}

#[test]
fn empty_log_commit_count_zero_string() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(vm.commit_count, "0", "empty log → commit_count \"0\"");
}

#[test]
fn empty_log_branches_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert!(vm.branches.is_empty(), "empty log → branches []");
}

#[test]
fn empty_log_last_commit_is_default() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert_eq!(
        vm.last_commit.author, "",
        "empty log → last_commit.author \"\""
    );
    assert_eq!(
        vm.last_commit.intent_id, "",
        "empty log → last_commit.intent_id \"\""
    );
    assert_eq!(
        vm.last_commit.message, "",
        "empty log → last_commit.message \"\""
    );
    assert_eq!(
        vm.last_commit.short_sha, "",
        "empty log → last_commit.short_sha \"\""
    );
}

#[test]
fn empty_log_contributors_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit");
    assert!(
        vm.about.contributors.is_empty(),
        "empty log → about.contributors []"
    );
}
