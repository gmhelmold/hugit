//! WP-D4 acceptance oracle — intents native + two-altitude projection.
//!
//! Owned items (VERBATIM from decomposition v2.0 — D4):
//!   ① commits embed intent_id, reproducible from log
//!   ② two altitudes consistent (50-intent fixture)
//!   ③ sidecar corpus importable
//!   ④(+) mixed fixture (intents + raw pushes interleaved on one ref):
//!         altitudes stay provably consistent, externals as external-change
//!
//! Driven by `tests/acceptance/wp-d4/run.sh`. One `#[test] item_<n>_<slug>` per
//! owned item.

mod intents_projection;

use hugit_contracts::intent_sidecar::IntentSidecar;
use hugit_refstore::intent::{
    GitCommit, ProjectionRow, import_sidecar, intents_from_log, project, project_machine,
};
use hugit_refstore::log::EventLog;

use intents_projection::{build_50_intent_log, build_mixed_log};

/// ① commits embed intent_id, reproducible from log.
///
/// Every landed intent projects to a generated git commit whose message EMBEDS
/// the `intent_id`, and the whole commit set is byte-for-byte reproducible by
/// re-projecting the same log (the projection is deterministic / one-directional).
#[test]
fn item_1_commits_embed_intent_id() {
    let log = build_50_intent_log();
    let intents = intents_from_log(&log).expect("intent altitude reads clean");

    let machine = project_machine(&log).expect("machine projection succeeds");
    let commits: Vec<&GitCommit> = machine.commits().collect();
    assert_eq!(commits.len(), 50, "one generated commit per landed intent");

    // Each generated commit message embeds its intent_id, recoverable from the
    // message bytes alone.
    for (commit, intent) in commits.iter().zip(intents.intents()) {
        assert!(
            commit.message.contains(&intent.intent_id),
            "commit message must embed the intent_id"
        );
        assert_eq!(
            GitCommit::intent_id_from_message(&commit.message),
            Some(intent.intent_id.as_str()),
            "intent_id must be recoverable from the commit message"
        );
        assert_eq!(commit.intent_id, intent.intent_id);
        assert_eq!(commit.seq, intent.seq, "commit ties back to the log event");
    }

    // Reproducible from the log: re-projecting the same log yields an identical
    // history (deterministic, no clock / no randomness).
    let machine_again = project_machine(&log).expect("re-projection succeeds");
    assert_eq!(
        machine, machine_again,
        "the commit set is reproducible from the log"
    );
}

/// ② two altitudes consistent (50-intent fixture).
///
/// `hugit log` (intent altitude) and `git log` (machine altitude) are both folds
/// of the same event log and are PROVABLY consistent — one is derived from the
/// other, so they cannot disagree.
#[test]
fn item_2_two_altitudes_consistent() {
    let log = build_50_intent_log();

    let intent_altitude = project(&log).expect("intent altitude projects");
    let machine_altitude = project_machine(&log).expect("machine altitude projects");

    assert_eq!(
        intent_altitude.len(),
        50,
        "50 intents at the intent altitude"
    );
    assert_eq!(
        machine_altitude.commits().count(),
        50,
        "50 generated commits at the machine altitude"
    );
    // No external changes in the pure-intent fixture.
    assert_eq!(
        machine_altitude.external_changes().count(),
        0,
        "no external-change rows in a pure-intent log"
    );

    // The proof: the two zooms are consistent.
    assert!(
        machine_altitude.is_consistent_with(&intent_altitude),
        "intent altitude and machine altitude must be provably consistent"
    );

    // And they order identically by log seq.
    let intent_seqs: Vec<u64> = intent_altitude.intents().iter().map(|i| i.seq).collect();
    let commit_seqs: Vec<u64> = machine_altitude.commits().map(|c| c.seq).collect();
    assert_eq!(intent_seqs, commit_seqs, "altitudes share one total order");
}

/// ③ sidecar corpus importable.
///
/// A B6 `IntentSidecar` corpus imports into the native intent model BY
/// `intent_id` — one lifecycle, one id. After import it is visible at both
/// altitudes under the same id.
#[test]
fn item_3_sidecar_corpus_importable() {
    let mut log = EventLog::new();

    // A small sidecar corpus (B6 produces these; non-authoritative by design).
    let corpus = [
        IntentSidecar {
            intent_id: "sidecar-aaa".into(),
            charter: "import the first corpus intent".into(),
            acceptance: vec!["does the thing".into()],
            context_ref: "cas://ctx/aaa".into(),
            authoritative: false,
        },
        IntentSidecar {
            intent_id: "sidecar-bbb".into(),
            charter: "import the second corpus intent".into(),
            acceptance: vec!["does the other thing".into()],
            context_ref: "cas://ctx/bbb".into(),
            authoritative: false,
        },
    ];

    for (i, sidecar) in corpus.iter().enumerate() {
        import_sidecar(
            &mut log,
            sidecar,
            "refs/heads/main",
            &format!("oid-import-{i}"),
            vec!["agent:importer".into()],
            1_717_000_100_000 + i as u64,
        )
        .expect("sidecar imports onto the log");
    }

    // Visible at the intent altitude, keyed by the SAME intent_id.
    let intent_altitude = intents_from_log(&log).expect("intent altitude reads clean");
    assert_eq!(intent_altitude.len(), 2, "both corpus intents landed");
    assert!(
        intent_altitude.by_id("sidecar-aaa").is_some(),
        "imported corpus is addressable by its intent_id"
    );
    assert_eq!(
        intent_altitude.by_id("sidecar-bbb").unwrap().charter,
        "import the second corpus intent",
        "charter carried through the import"
    );

    // And at the machine altitude, as generated commits embedding the same id.
    let machine = project_machine(&log).expect("machine projects");
    let ids: Vec<&str> = machine.commits().map(|c| c.intent_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["sidecar-aaa", "sidecar-bbb"],
        "one lifecycle, one id"
    );

    // Re-importing the same id is refused (one identity, never duplicated).
    let dup = import_sidecar(
        &mut log,
        &corpus[0],
        "refs/heads/main",
        "oid-dup",
        vec!["agent:importer".into()],
        1_717_000_200_000,
    );
    assert!(
        dup.is_err(),
        "re-importing the same intent_id must be refused"
    );
}

/// ④(+) mixed fixture: intents + raw pushes interleaved on one ref — altitudes
/// stay provably consistent, externals stay external-change, NO synthetic intent.
#[test]
fn item_4_mixed_fixture_externals_stay_external() {
    let (log, expected_intents, expected_raw_pushes) = build_mixed_log();
    assert!(
        expected_intents > 0 && expected_raw_pushes > 0,
        "fixture interleaves both"
    );

    let intent_altitude = project(&log).expect("intent altitude projects");
    let machine_altitude = project_machine(&log).expect("machine altitude projects");

    // The intent altitude counts ONLY intents — no raw push leaks in as an intent.
    assert_eq!(
        intent_altitude.len(),
        expected_intents,
        "intent altitude holds exactly the landed intents — no fabricated intents"
    );

    // The machine altitude has one commit per intent and one external-change row
    // per raw push.
    assert_eq!(
        machine_altitude.commits().count(),
        expected_intents,
        "one generated commit per landed intent"
    );
    assert_eq!(
        machine_altitude.external_changes().count(),
        expected_raw_pushes,
        "one external-change row per raw push — every raw push accounted for"
    );

    // Externals stay external: every non-intent row is an ExternalChange of a
    // raw-push kind, NEVER a synthesised intent.
    for row in machine_altitude.external_changes() {
        match row {
            ProjectionRow::ExternalChange { kind, .. } => {
                assert!(
                    kind == "ref.update" || kind == "ref.delete",
                    "external change must be a raw-push kind, got {kind}"
                );
            }
            ProjectionRow::Intent(_) => {
                panic!("a raw push was fabricated into an intent (④ violated)")
            }
        }
    }

    // The two altitudes stay provably consistent even with externals interleaved.
    assert!(
        machine_altitude.is_consistent_with(&intent_altitude),
        "altitudes must stay consistent on the mixed fixture"
    );

    // No raw-push oid ever appears as an intent target (externals never become
    // intents).
    for intent in intent_altitude.intents() {
        assert!(
            !intent.target.starts_with("rawoid-"),
            "a raw-push target leaked into the intent altitude (④ violated)"
        );
    }
}
