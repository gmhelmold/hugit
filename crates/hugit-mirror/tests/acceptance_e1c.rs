//! WP-E1c acceptance oracle — verified mirror: bootstrap + disaster recovery.
//!
//! Owned items:
//!   ⑧  `item_8_cold_seed_full_history_hash_verified`
//!   ⑧  `item_8_cold_seed_resumable_mid_seed`
//!   ⑨  `item_9_github_app_revocation_detected_incident`
//!   ⑨  `item_9_mirror_repo_deletion_rename_detected_incident`
//!   ⑨  `item_9_recovery_source_pinned_completeness_stated`
//!   ⑨  `item_9_resume_from_recovered_state`
//!   ⑪  `item_11_substrate_loss_mirror_is_working_git_repo`
//!   ⑪  `item_11_substrate_loss_full_recovery_resume_end_to_end`
//!   ⑪  `item_11_recovered_content_imports_as_change_events_not_fabricated`
//!
//! All items are local fixture proofs over E1c's bootstrap/DR logic.
//!
//! # Contract deps (consumed, never modified)
//! - `hugit_contracts::EventRecord` (frozen by D2/contracts)
//! - `hugit_mirror::bootstrap::{Seeder, SeedProgress, SeedResult}`
//! - `hugit_mirror::dr::{DrController, GithubLossKind, DrIncident, RecoverySource,
//!    CompletenessVerdict, ResumeResult, SubstrateLossResult, ChangeEventImport}`

use hugit_mirror::bootstrap::{SeedProgress, SeedResult, Seeder};
use hugit_mirror::dr::{
    ChangeEventImport, CompletenessVerdict, DrController, DrIncident, GithubLossKind,
    RecoverySource, ResumeResult, SubstrateLossResult,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_repo() -> &'static str {
    "humangr-labs/hugit-fleet-syn-1"
}

fn forge_hash(n: u8) -> String {
    format!("{:040x}", n as u64)
}

// ── ⑧ cold-seed: full history, hash-verified, resumable ───────────────────────
#[test]
fn item_8_cold_seed_full_history_hash_verified() {
    // A cold-seed must replicate the full existing history to a fresh mirror repo
    // via the E1a push+verify pipeline, producing a SeedResult with byte-identity
    // hash verification for every ref.

    let seeder = Seeder::new_fixture(test_repo());

    // Fixture history: 3 refs with known hashes.
    let refs: Vec<(&str, String)> = vec![
        ("refs/heads/main", forge_hash(0x01)),
        ("refs/heads/dev", forge_hash(0x02)),
        ("refs/tags/v0.1.0", forge_hash(0x03)),
    ];

    let seed_result: SeedResult = seeder
        .seed_fixture(&refs)
        .expect("cold-seed must succeed for fixture history");

    assert!(
        seed_result.all_verified,
        "cold-seed must hash-verify every replicated ref to byte-identity"
    );
    assert_eq!(
        seed_result.seeded_count,
        refs.len(),
        "cold-seed must replicate all {} refs",
        refs.len()
    );

    // Every ref in the result must carry a verified flag.
    for ref_result in &seed_result.ref_results {
        assert!(
            ref_result.verified,
            "ref {} must be byte-identity verified after cold-seed",
            ref_result.ref_name
        );
        assert!(
            !ref_result.verified_hash.is_empty(),
            "ref {} must carry the verified hash",
            ref_result.ref_name
        );
    }
}

// ── ⑧ cold-seed: resumable mid-seed ──────────────────────────────────────────
#[test]
fn item_8_cold_seed_resumable_mid_seed() {
    // If a cold-seed is interrupted mid-run, it must persist its progress
    // (last verified ref / pack offset) so a resumed run continues from where
    // it stopped without restarting from scratch.

    let seeder = Seeder::new_fixture(test_repo());

    // Fixture: 5 refs, interrupted after the 2nd.
    let refs: Vec<(&str, String)> = (1u8..=5)
        .map(|i| {
            let ref_name: &'static str = Box::leak(
                format!("refs/heads/branch-{i}").into_boxed_str(),
            );
            (ref_name, forge_hash(i))
        })
        .collect();

    // Simulate seed interrupted after 2 refs.
    let interrupted_progress: SeedProgress = seeder
        .seed_partial_fixture(&refs, /* stop_after */ 2)
        .expect("partial seed must succeed");

    assert_eq!(
        interrupted_progress.completed_count, 2,
        "interrupted seed must have completed exactly 2 refs"
    );
    assert!(
        interrupted_progress.last_verified_ref.is_some(),
        "interrupted seed must persist the last verified ref"
    );
    assert!(
        !interrupted_progress.is_complete(),
        "interrupted seed progress must not be marked complete"
    );

    // Resume from persisted progress — must complete the remaining 3 refs.
    let resume_result: SeedResult = seeder
        .resume_seed_fixture(&refs, &interrupted_progress)
        .expect("resumed seed must succeed");

    assert!(
        resume_result.all_verified,
        "resumed seed must verify all remaining refs"
    );
    assert_eq!(
        resume_result.seeded_count,
        refs.len(),
        "resumed seed must account for all {} refs in total",
        refs.len()
    );
    assert!(
        resume_result.was_resumed,
        "SeedResult must record that this was a resumed (not fresh) seed"
    );
}

// ── ⑨ GitHub App revocation detected → incident ───────────────────────────────
#[test]
fn item_9_github_app_revocation_detected_incident() {
    // App revocation (auth failure class) must be detected and emit an incident.
    // Recovery-source must be pinned to the hugit substrate (authoritative).

    let controller = DrController::new_fixture(test_repo());

    let incident: DrIncident = controller
        .handle_github_loss_fixture(GithubLossKind::AppRevocation)
        .expect("App revocation must produce a DrIncident");

    assert_eq!(incident.loss_kind, GithubLossKind::AppRevocation);
    assert!(
        incident.detected,
        "App revocation must be flagged as detected"
    );
    assert!(
        !incident.id.is_empty(),
        "incident must carry a non-empty identifier"
    );

    // Recovery source must be pinned.
    let source: &RecoverySource = incident
        .recovery_source
        .as_ref()
        .expect("incident must pin a recovery source");
    assert!(
        source.is_substrate_authoritative,
        "recovery source must be the hugit substrate (authoritative)"
    );
}

// ── ⑨ mirror-repo deletion/rename detected → incident ────────────────────────
#[test]
fn item_9_mirror_repo_deletion_rename_detected_incident() {
    // Mirror-repo deletion or rename (404/redirect class) must be detected
    // and emit an incident with a pinned recovery source.

    let controller = DrController::new_fixture(test_repo());

    for loss_kind in [GithubLossKind::RepoDeletion, GithubLossKind::RepoRename] {
        let incident: DrIncident = controller
            .handle_github_loss_fixture(loss_kind.clone())
            .expect("repo deletion/rename must produce a DrIncident");

        assert_eq!(incident.loss_kind, loss_kind);
        assert!(incident.detected, "loss must be flagged as detected");

        let source = incident
            .recovery_source
            .as_ref()
            .expect("incident must pin a recovery source");
        assert!(
            source.is_substrate_authoritative,
            "recovery source must be the hugit substrate for {:?}",
            loss_kind
        );
    }
}

// ── ⑨ recovery source pinned + completeness criterion stated ─────────────────
#[test]
fn item_9_recovery_source_pinned_completeness_stated() {
    // Recovery source must be pinned (hugit substrate is authoritative),
    // and the completeness criterion must be stated (byte-identity of re-seeded mirror).

    let controller = DrController::new_fixture(test_repo());

    let incident = controller
        .handle_github_loss_fixture(GithubLossKind::RepoDeletion)
        .expect("repo deletion must produce a DrIncident");

    let source = incident
        .recovery_source
        .as_ref()
        .expect("incident must pin a recovery source");

    // Completeness criterion must be stated.
    let verdict: &CompletenessVerdict = source
        .completeness_criterion
        .as_ref()
        .expect("recovery source must state its completeness criterion");

    assert!(
        verdict.is_byte_identity,
        "completeness criterion must be byte-identity of the re-seeded mirror"
    );
    assert!(
        !verdict.description.is_empty(),
        "completeness criterion must carry a human-readable description"
    );
}

// ── ⑨ resume from recovered state ────────────────────────────────────────────
#[test]
fn item_9_resume_from_recovered_state() {
    // After GitHub-side loss and recovery (re-seed via ⑧), continuous verified
    // sync must resume from the recovered mirror without data loss.

    let controller = DrController::new_fixture(test_repo());

    // Simulate recovery: re-seed fixture (3 refs) then resume sync.
    let refs: Vec<(&str, String)> = vec![
        ("refs/heads/main", forge_hash(0x10)),
        ("refs/heads/dev", forge_hash(0x11)),
    ];

    let resume_result: ResumeResult = controller
        .recover_and_resume_fixture(GithubLossKind::RepoDeletion, &refs)
        .expect("recover-and-resume must succeed");

    assert!(
        resume_result.recovery_verified,
        "recovery must be byte-identity verified before resuming sync"
    );
    assert!(
        resume_result.sync_resumed,
        "continuous verified sync must be re-established after recovery"
    );
    assert!(
        !resume_result.data_loss,
        "recovery must not incur data loss"
    );
}

// ── ⑪ substrate-loss: mirror is a byte-complete working git repo ───────────────
#[test]
fn item_11_substrate_loss_mirror_is_working_git_repo() {
    // After inducing forge/substrate loss, the continuously-verified mirror must
    // be a byte-complete, working git repository: clone, log, and checkout all
    // succeed from the mirror alone.

    let controller = DrController::new_fixture(test_repo());

    let refs: Vec<(&str, String)> = vec![
        ("refs/heads/main", forge_hash(0x20)),
        ("refs/tags/v1.0.0", forge_hash(0x21)),
    ];

    let result: SubstrateLossResult = controller
        .simulate_substrate_loss_fixture(&refs)
        .expect("substrate-loss simulation must succeed");

    // The mirror must be byte-complete.
    assert!(
        result.mirror_byte_complete,
        "mirror must be byte-complete after substrate loss"
    );

    // Git operations must succeed from the mirror alone.
    assert!(
        result.git_clone_ok,
        "git clone from mirror must succeed (working git repo)"
    );
    assert!(
        result.git_log_ok,
        "git log from mirror must succeed (working git repo)"
    );
    assert!(
        result.git_checkout_ok,
        "git checkout from mirror must succeed (working git repo)"
    );
}

// ── ⑪ substrate-loss: full recovery/resume end-to-end ─────────────────────────
#[test]
fn item_11_substrate_loss_full_recovery_resume_end_to_end() {
    // After substrate loss: recover a rebuilt substrate from the mirror and
    // resume continuous sync. The recovered substrate must pass byte-identity
    // verification before being declared recovered.

    let controller = DrController::new_fixture(test_repo());

    let refs: Vec<(&str, String)> = vec![
        ("refs/heads/main", forge_hash(0x30)),
        ("refs/heads/dev", forge_hash(0x31)),
    ];

    let result: SubstrateLossResult = controller
        .simulate_substrate_loss_fixture(&refs)
        .expect("substrate-loss simulation must succeed");

    assert!(
        result.full_recovery_ok,
        "full substrate recovery end-to-end must succeed"
    );
    assert!(
        result.recovery_byte_verified,
        "recovered substrate must be byte-identity verified"
    );
    assert!(
        result.sync_resumed,
        "continuous verified sync must resume after substrate recovery"
    );

    // Fail-CLOSED: an unverifiable recovery must be an incident, never silent.
    let unverifiable = controller
        .simulate_unverifiable_recovery_fixture()
        .expect("unverifiable recovery simulation must not crash");
    assert!(
        unverifiable.incident_emitted,
        "unverifiable recovery must emit an incident (fail-CLOSED, never silent recovered)"
    );
    assert!(
        !unverifiable.declared_recovered,
        "unverifiable recovery must not be declared recovered"
    );
}

// ── ⑪ recovered content imports as change-events, never fabricated intents ────
#[test]
fn item_11_recovered_content_imports_as_change_events_not_fabricated() {
    // Recovered content must flow through E2a's import boundary as opaque
    // change-events — never fabricated intents (cf. E2⑤, no-fake-intents law).

    let controller = DrController::new_fixture(test_repo());

    // Simulate recovery of 2 content items from the mirror.
    let recovered_hashes = vec![forge_hash(0x40), forge_hash(0x41)];

    let imports: Vec<ChangeEventImport> = controller
        .import_recovered_content_fixture(&recovered_hashes)
        .expect("content import must succeed");

    assert_eq!(
        imports.len(),
        recovered_hashes.len(),
        "must import one ChangeEventImport per recovered content item"
    );

    for (i, import) in imports.iter().enumerate() {
        // Must be an opaque change-event (not a fabricated intent).
        assert!(
            !import.is_fabricated_intent,
            "import {} must NOT be a fabricated intent (no-fake-intents law / E2⑤)",
            i
        );
        assert!(
            import.is_change_event,
            "import {} must be classified as a change-event",
            i
        );
        assert!(
            !import.content_hash.is_empty(),
            "import {} must carry the content hash of the recovered object",
            i
        );
        // The import must not carry an intent_id (intents are not fabricated).
        assert!(
            import.intent_id.is_none(),
            "import {} must not carry an intent_id (no fabricated intents)",
            i
        );
    }
}
