//! Parity test for `build_home` (Wave 1 home handler).
//!
//! Verifies:
//!   1. `build_home` compiles and returns without panicking on an empty log.
//!   2. The result serializes via `serde_json` and round-trips losslessly back
//!      into `hugit_http_contracts::RepoHomeVm`.
//!   3. STUB fields equal their honest defaults — no faked data.

use hugit_contracts::IntentSidecar;
use hugit_http_contracts::RepoHomeVm;
use hugit_refstore::EventLog;
use hugit_refstore::intent::ExternalChangeKind;
use hugit_refstore::intent::import_sidecar;
use hugit_serve::handlers::build_home;

/// Build a fresh empty `EventLog` — the log is already "chain-verified" (an
/// empty log has a trivially valid chain) per the frozen handler contract.
fn empty_log() -> EventLog {
    EventLog::new()
}

/// A minimal `IntentSidecar` with the given id + charter (free-text).
fn sidecar(intent_id: &str, charter: &str) -> IntentSidecar {
    IntentSidecar {
        intent_id: intent_id.to_string(),
        charter: charter.to_string(),
        acceptance: vec![],
        context_ref: String::new(),
        authoritative: false,
    }
}

#[test]
fn empty_log_round_trips() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);

    // 1. Serializes without error.
    let json = serde_json::to_string(&vm).expect("RepoHomeVm serializes");

    // 2. Re-parses losslessly.
    let reparsed: RepoHomeVm = serde_json::from_str(&json).expect("JSON re-parses into RepoHomeVm");
    assert_eq!(vm, reparsed, "round-trip must be lossless");
}

#[test]
fn repo_field_is_exact() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(vm.repo, "hugit");
}

#[test]
fn stub_files_empty() {
    let log = empty_log();
    // No git content seam (None/None) → the honest-empty file listing (the REAL
    // tree read is covered by the handler's own `root_tree_is_listed_*` unit test).
    let vm = build_home(&log, "hugit", None, None);
    assert!(
        vm.files.is_empty(),
        "files must be [] with no git seam threaded (honest default)"
    );
}

#[test]
fn stub_readme_html_empty() {
    let log = empty_log();
    // No git content seam → no README to render (honest-empty default).
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(
        vm.readme_html, "",
        "readme_html must be \"\" with no git seam threaded (honest default)"
    );
}

#[test]
fn stub_about_description_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(
        vm.about.description, "",
        "about.description must be \"\" (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_topics_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert!(
        vm.about.topics.is_empty(),
        "about.topics must be [] (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_release_none() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert!(
        vm.about.release.is_none(),
        "about.release must be None (P2 — honest STUB)"
    );
}

#[test]
fn stub_about_stars_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(
        vm.about.stars, "",
        "about.stars must be \"\" (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_forks_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(
        vm.about.forks, "",
        "about.forks must be \"\" (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_license_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(
        vm.about.license, "",
        "about.license must be \"\" (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_about_releases_count_zero() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(
        vm.about.releases_count, 0,
        "about.releases_count must be 0 (P2 — honest STUB)"
    );
}

#[test]
fn stub_about_languages_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert!(
        vm.about.languages.is_empty(),
        "about.languages must be [] (GitHub-mirror P2 — honest STUB)"
    );
}

#[test]
fn stub_synergy_lines_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert!(
        vm.synergy.lines.is_empty(),
        "synergy.lines must be [] (no live AC seam — honest STUB)"
    );
}

#[test]
fn empty_log_branch_count_zero() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(vm.branch_count, 0, "empty log → branch_count 0");
}

#[test]
fn empty_log_tag_count_zero() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(vm.tag_count, 0, "empty log → tag_count 0");
}

#[test]
fn empty_log_commit_count_zero_string() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert_eq!(vm.commit_count, "0", "empty log → commit_count \"0\"");
}

#[test]
fn empty_log_branches_empty() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
    assert!(vm.branches.is_empty(), "empty log → branches []");
}

#[test]
fn empty_log_last_commit_is_default() {
    let log = empty_log();
    let vm = build_home(&log, "hugit", None, None);
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
    let vm = build_home(&log, "hugit", None, None);
    assert!(
        vm.about.contributors.is_empty(),
        "empty log → about.contributors []"
    );
}

/// REQUIRED secret-matrix test (P0 read-path redaction).
///
/// A landed intent whose charter embeds a GitHub PAT, landed by a principal
/// chain carrying a secret-shaped token. EVERY free-text field echoed from the
/// log into the VM (`last_commit.message`, `last_commit.author`, every
/// `about.contributors` entry) MUST pass through `crate::fmt::scrub`, so the
/// raw secret never reaches the browser — it surfaces as `[REDACTED]`.
#[test]
fn secret_matrix_message_author_contributors_redacted() {
    const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    // The raw secret carried inside a principal entry (a known-prefix PAT — a
    // canonical secret-shaped token, structural so the test is deterministic).
    const PRINCIPAL_RAW: &str = "ghp_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    const PRINCIPAL_SECRET: &str = "agent:ghp_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    let mut log = EventLog::new();
    let charter = format!("land the feature using {PAT} as the token");
    import_sidecar(
        &mut log,
        &sidecar("intent-secret", &charter),
        "refs/hugit/intents",
        "abc123def456abc123def456abc123def456abcd",
        vec![PRINCIPAL_SECRET.to_string()],
        1_000,
    )
    .expect("intent authorship (Push) is allowed for every class");

    let vm = build_home(&log, "hugit", None, None);

    // last_commit.message: the charter carried the PAT → redacted.
    assert!(
        !vm.last_commit.message.contains(PAT),
        "last_commit.message must NOT echo the raw PAT"
    );
    assert_eq!(
        vm.last_commit.message, "[REDACTED]",
        "secret-shaped message scrubs to the sentinel"
    );

    // last_commit.author: the principal entry was secret-shaped → redacted.
    assert!(
        !vm.last_commit.author.contains(PRINCIPAL_RAW),
        "last_commit.author must NOT echo the raw secret token"
    );
    assert_eq!(
        vm.last_commit.author, "[REDACTED]",
        "secret-shaped author scrubs to the sentinel"
    );

    // about.contributors: every entry is scrubbed; none echoes the raw secret.
    assert!(
        vm.about
            .contributors
            .iter()
            .all(|c| !c.contains(PRINCIPAL_RAW)),
        "no contributor entry may echo the raw secret"
    );
    assert!(
        vm.about.contributors.contains(&"[REDACTED]".to_string()),
        "the secret-shaped contributor surfaces as [REDACTED]"
    );

    // Structural fields are NOT scrubbed — provenance survives.
    assert_eq!(
        vm.last_commit.intent_id, "intent-secret",
        "intent_id is structural, never scrubbed"
    );
    assert_eq!(
        vm.last_commit.short_sha, "abc123",
        "short_sha is structural, never scrubbed"
    );
}

/// Populated-log test — proves the REAL path produces non-default values.
///
/// A real `refs/heads/main` branch (external change) plus a landed intent on
/// the intent namespace. Asserts the engine-sourced fields reflect the data,
/// not the empty defaults.
#[test]
fn populated_log_real_values() {
    let mut log = EventLog::new();

    // A real branch ref (the only cross-crate door to refs/heads/* is the typed
    // external-change shim).
    log.append_external_change(
        ExternalChangeKind::RefUpdate,
        vec!["agent:opus".to_string()],
        serde_json::json!({
            "ref": "refs/heads/main",
            "target": "0123456789abcdef0123456789abcdef01234567",
        })
        .to_string(),
        1_000,
    );

    // A landed intent with a clean charter + a clean principal chain.
    import_sidecar(
        &mut log,
        &sidecar("intent-1", "Add the login screen"),
        "refs/hugit/intents",
        "fedcba9876543210fedcba9876543210fedcba98",
        vec!["orchestrator:lead".to_string(), "agent:sonnet".to_string()],
        2_000,
    )
    .expect("intent authorship is allowed");

    let vm = build_home(&log, "hugit", None, None);

    // Branch (REAL: replay → first refs/heads/* sorted).
    assert_eq!(vm.branch, "main", "real primary branch");
    assert_eq!(vm.branch_count, 1, "exactly one refs/heads/* ref");
    assert_eq!(vm.tag_count, 0, "no tags");
    assert_eq!(vm.branches, vec!["main".to_string()], "branch list");

    // commit_count (REAL: project_machine rows = ref.update + intent.landed).
    assert_eq!(vm.commit_count, "2", "two ref-mutating rows projected");

    // last_commit (REAL: last machine row = the landed intent).
    assert_eq!(vm.last_commit.intent_id, "intent-1");
    assert_eq!(vm.last_commit.message, "Add the login screen");
    assert_eq!(
        vm.last_commit.author, "lead",
        "author is the FIRST principal-chain entry, prefix-stripped"
    );
    assert_eq!(vm.last_commit.short_sha, "fedcba");

    // contributors (REAL: deduped, scrubbed, sorted principal-chain names).
    assert!(
        vm.about.contributors.contains(&"lead".to_string()),
        "orchestrator:lead → lead"
    );
    assert!(
        vm.about.contributors.contains(&"sonnet".to_string()),
        "agent:sonnet → sonnet"
    );
}
