//! WP-E1c acceptance oracle — verified mirror: bootstrap + disaster recovery.
//!
//! Owned items (one `#[test] item_<n>_<slug>` per acceptance item):
//!   ⑧(R3) cold-seed bootstrap — full history → fresh repo, hash-verified,
//!         resumable mid-seed:
//!           `item_8_cold_seed_full_history_hash_verified`
//!           `item_8_cold_seed_resumable_mid_seed`
//!   ⑨🔧   GitHub-side loss DR — App revocation / repo deletion-rename →
//!         incident → recoverable; recovery-source pinned, completeness stated,
//!         resume-from-recovered-state asserted:
//!           `item_9_github_app_revocation_detected_incident`
//!           `item_9_mirror_repo_deletion_rename_detected_incident`
//!           `item_9_recovery_source_pinned_completeness_stated`
//!           `item_9_resume_from_recovered_state`
//!   ⑪(R6) SUBSTRATE-LOSS DR — induce forge loss → mirror is byte-complete
//!         WORKING git repo; full recovery/resume proven e2e; recovered content
//!         imports as change-events, never fabricated intents:
//!           `item_11_substrate_loss_mirror_is_working_git_repo`
//!           `item_11_substrate_loss_full_recovery_resume_end_to_end`
//!           `item_11_recovered_content_imports_as_change_events_not_fabricated`
//!
//! Byte-identity is proven against **local `git`** (real object SHAs), never by
//! fiat: fixtures build a genuine git repo on disk, read its refs + content
//! hashes via `git`, seed/recover those exact hashes, and clone/log/checkout
//! off the mirror dir to prove it is a working repo.
//!
//! # Contract deps (consumed, never modified)
//! - `hugit_contracts::EventRecord` (frozen by WP-00)
//! - `hugit_mirror::bootstrap::{cold_seed, SeedUnit, SeedProgress, SeedOutcome,
//!    MockSeedTransport}`
//! - `hugit_mirror::dr::{detect_github_loss, recover_github_loss,
//!    recover_substrate_loss, GithubProbe, Incident, LossKind, RecoverySource,
//!    CompletenessCriterion, RecoveryOutcome, WorkingGitRepo,
//!    project_recovered_commit, import_as_change_event, CHANGE_EVENT_KIND,
//!    substrate_loss_incident}`

use hugit_contracts::EventRecord;
use hugit_mirror::bootstrap::{MockSeedTransport, SeedOutcome, SeedProgress, SeedUnit, cold_seed};
use hugit_mirror::dr::{
    CHANGE_EVENT_KIND, CompletenessCriterion, GithubProbe, LossKind, RecoveryOutcome,
    RecoverySource, WorkingGitRepo, detect_github_loss, import_as_change_event,
    project_recovered_commit, recover_github_loss, recover_substrate_loss, substrate_loss_incident,
};
use std::path::{Path, PathBuf};
use std::process::Command;

// ── local-git fixture harness ───────────────────────────────────────────────

/// A unique temp dir, removed on drop.
struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("hugit-e1c-{tag}-{nanos}-{}", std::process::id()));
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

/// Configure a `git` invocation deterministically and hermetically.
///
/// CI runners have NO global/system git config and may run a different git
/// version than a dev box, so the test must never depend on ambient config
/// (e.g. a global `init.defaultBranch`, checkout/LFS filters, or auto-gc). We
/// pin identity + dates, neutralise global/system config, and disable every
/// background maintenance path (auto-gc, the commit-graph).
fn git_command(cwd: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "hugit")
        .env("GIT_AUTHOR_EMAIL", "bot@hugit.dev")
        .env("GIT_COMMITTER_NAME", "hugit")
        .env("GIT_COMMITTER_EMAIL", "bot@hugit.dev")
        .env("GIT_AUTHOR_DATE", "1717000000 +0000")
        .env("GIT_COMMITTER_DATE", "1717000000 +0000")
        .args([
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "core.commitGraph=false",
        ]);
    cmd
}

/// Run `git` in `cwd`, asserting success; return trimmed stdout.
fn git(cwd: &Path, args: &[&str]) -> String {
    let out = git_command(cwd)
        .args(args)
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

/// `git` allowing failure; return (success, stdout, stderr).
fn git_try(cwd: &Path, args: &[&str]) -> (bool, String, String) {
    let out = git_command(cwd)
        .args(args)
        .output()
        .expect("git must be on PATH");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    )
}

/// Build a real source repo with `n` commits on `main`; return its dir.
///
/// All `git` calls go through `git_command`, so the build is hermetic (no
/// ambient global/system config) and has no background auto-gc/maintenance that
/// could race object writes. `git fsck` proves the object DB and parent chain
/// are complete before any traversal/clone runs.
fn build_source_repo(tmp: &TmpDir, n: usize) -> PathBuf {
    let repo = tmp.path().join("source");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    for i in 0..n {
        std::fs::write(repo.join("f.txt"), format!("line {i}\n")).unwrap();
        git(&repo, &["add", "f.txt"]);
        git(&repo, &["commit", "-q", "-m", &format!("commit {i}")]);
    }
    git(&repo, &["fsck", "--no-progress", "--strict"]);
    repo
}

/// Read the real refs of `repo` as seed units, with the byte-identity object
/// hash = git's own SHA for each ref's tip. Pack offset is the unit index.
fn seed_units_from(repo: &Path) -> Vec<SeedUnit> {
    let listing = git(repo, &["show-ref"]);
    listing
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let mut parts = line.split_whitespace();
            let object_hash = parts.next().unwrap().to_string();
            let refname = parts.next().unwrap().to_string();
            SeedUnit {
                refname,
                object_hash,
                pack_offset: i as u64,
            }
        })
        .collect()
}

/// Clone `--mirror` of `src` into a sibling dir — the byte-complete mirror.
fn make_mirror(tmp: &TmpDir, src: &Path) -> PathBuf {
    let mirror = tmp.path().join("mirror.git");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            "--mirror",
            src.to_str().unwrap(),
            mirror.to_str().unwrap(),
        ],
    );
    mirror
}

// ════════════════════════════════ ⑧ cold-seed ═══════════════════════════════

#[test]
fn item_8_cold_seed_full_history_hash_verified() {
    let tmp = TmpDir::new("seed-full");
    let src = build_source_repo(&tmp, 5);
    let units = seed_units_from(&src);
    assert!(!units.is_empty(), "source must have refs");

    // Cold-seed full history into a fresh mirror; mock transport echoes the
    // git-true object hash, so the byte-identity gate compares real SHAs.
    let mut transport = MockSeedTransport::default();
    let mut progress = SeedProgress::default();
    let outcome = cold_seed(&units, &mut transport, &mut progress).unwrap();

    match outcome {
        SeedOutcome::Complete { refs_verified } => {
            assert_eq!(refs_verified, units.len(), "every ref must verify");
        }
        other => panic!("expected Complete, got {other:?}"),
    }
    // Every unit was actually delivered, in order — byte-identity by readback.
    assert_eq!(transport.delivered, vec![units[0].refname.clone()]);
    assert!(progress.is_verified(&units[0].refname));

    // Fail-CLOSED: a mirror that reports a wrong hash is never declared seeded.
    let mut bad = MockSeedTransport::with_corruption(&units[0].refname, "0".repeat(40));
    let mut p2 = SeedProgress::default();
    let err = cold_seed(&units, &mut bad, &mut p2).unwrap_err();
    assert!(
        matches!(
            err,
            hugit_mirror::bootstrap::SeedError::ByteIdentityMismatch { .. }
        ),
        "byte-identity mismatch must be fail-closed, got {err:?}"
    );
}

#[test]
fn item_8_cold_seed_resumable_mid_seed() {
    let tmp = TmpDir::new("seed-resume");
    let src = build_source_repo(&tmp, 4);
    // Add a second branch so there are >=2 refs to interrupt between.
    git(&src, &["branch", "feature"]);
    let units = seed_units_from(&src);
    assert!(
        units.len() >= 2,
        "need >=2 refs to prove resume; got {}",
        units.len()
    );

    // First pass: transport fails once at the 2nd ref → Interrupted, cursor set.
    let mut transport = MockSeedTransport::fail_once_at(units[1].refname.clone());
    let mut progress = SeedProgress::default();
    let first = cold_seed(&units, &mut transport, &mut progress).unwrap();
    match &first {
        SeedOutcome::Interrupted { at, progress: p } => {
            assert_eq!(at, &units[1].refname);
            // Cursor persisted exactly the units already byte-identity verified.
            assert!(p.is_verified(&units[0].refname));
            assert!(!p.is_verified(&units[1].refname));
            assert_eq!(p.last_verified.as_deref(), Some(units[0].refname.as_str()));
        }
        other => panic!("expected Interrupted, got {other:?}"),
    }

    // Resume from the SAME cursor: completes without re-seeding verified refs.
    let second = cold_seed(&units, &mut transport, &mut progress).unwrap();
    match second {
        SeedOutcome::Complete { refs_verified } => {
            assert_eq!(refs_verified, units.len());
        }
        other => panic!("expected Complete on resume, got {other:?}"),
    }
    // The already-verified first ref was NOT re-delivered on resume.
    assert_eq!(
        transport
            .delivered
            .iter()
            .filter(|r| **r == units[0].refname)
            .count(),
        1,
        "resume must not re-seed an already-verified ref"
    );
}

// ════════════════════════════════ ⑨ GitHub-side loss DR ═════════════════════

#[test]
fn item_9_github_app_revocation_detected_incident() {
    let probe = GithubProbe::AuthFailure {
        detail: "installation token rejected (401)".into(),
    };
    let incident = detect_github_loss(&probe).expect("auth failure must raise an incident");
    assert_eq!(incident.kind, LossKind::AppRevocation);
    // Fail-CLOSED: a clean probe raises nothing.
    assert!(detect_github_loss(&GithubProbe::Ok).is_none());
}

#[test]
fn item_9_mirror_repo_deletion_rename_detected_incident() {
    let probe = GithubProbe::NotFoundOrRedirect {
        detail: "mirror repo 404 / redirected (deleted or renamed)".into(),
    };
    let incident = detect_github_loss(&probe).expect("404/redirect must raise an incident");
    assert_eq!(incident.kind, LossKind::RepoDeletionOrRename);
}

#[test]
fn item_9_recovery_source_pinned_completeness_stated() {
    // Both GitHub-loss classes pin the substrate as authoritative + byte-identity.
    for probe in [
        GithubProbe::AuthFailure { detail: "x".into() },
        GithubProbe::NotFoundOrRedirect { detail: "y".into() },
    ] {
        let incident = detect_github_loss(&probe).unwrap();
        assert_eq!(
            incident.recovery_source,
            RecoverySource::HugitSubstrate,
            "recovery-source must be explicitly pinned to the authoritative substrate"
        );
        assert_eq!(
            incident.completeness,
            CompletenessCriterion::ByteIdentity,
            "completeness criterion must be stated as byte-identity"
        );
    }
}

#[test]
fn item_9_resume_from_recovered_state() {
    let tmp = TmpDir::new("gh-recover");
    let src = build_source_repo(&tmp, 3);
    let units = seed_units_from(&src);

    let incident = detect_github_loss(&GithubProbe::NotFoundOrRedirect {
        detail: "mirror deleted".into(),
    })
    .unwrap();

    // Re-seed from pinned substrate; rebuilt mirror reads back git-true hashes.
    let outcome = recover_github_loss(&incident, &units, |u| u.object_hash.clone());
    match outcome {
        RecoveryOutcome::Recovered {
            resume_from,
            refs_verified,
        } => {
            assert!(outcome_recovered_sane(
                &resume_from,
                refs_verified,
                units.len()
            ));
            // resume-from-recovered-state: cursor names the last verified ref so
            // continuous verified sync resumes without data loss.
            assert_eq!(
                resume_from.last_verified.as_deref(),
                Some(units.last().unwrap().refname.as_str())
            );
        }
        other => panic!("expected Recovered, got {other:?}"),
    }

    // Fail-CLOSED: a non-byte-identical re-seed is Unverifiable, never recovered.
    let bad = recover_github_loss(&incident, &units, |_| "deadbeef".repeat(8));
    assert!(!bad.is_recovered());
}

fn outcome_recovered_sane(p: &SeedProgress, refs_verified: usize, expected: usize) -> bool {
    refs_verified == expected && p.verified_refs.len() == expected
}

// ════════════════════════════════ ⑪ substrate-loss DR ═══════════════════════

#[test]
fn item_11_substrate_loss_mirror_is_working_git_repo() {
    let tmp = TmpDir::new("subloss-working");
    let src = build_source_repo(&tmp, 5);
    let mirror = make_mirror(&tmp, &src);

    // Induce substrate loss: the source/forge is gone.
    std::fs::remove_dir_all(&src).unwrap();
    assert!(!src.exists(), "substrate must be lost");

    // The mirror alone must be a byte-complete WORKING git repo: clone, log,
    // checkout all succeed off the mirror — proven by local git, not by fiat.
    let (clone_ok, _, clone_err) = git_try(
        tmp.path(),
        &[
            "clone",
            "-q",
            mirror.to_str().unwrap(),
            tmp.path().join("recovered-clone").to_str().unwrap(),
        ],
    );
    assert!(clone_ok, "clone off mirror must succeed: {clone_err}");
    let clone_dir = tmp.path().join("recovered-clone");
    let log = git(&clone_dir, &["log", "--oneline"]);
    assert_eq!(
        log.lines().count(),
        5,
        "all 5 commits recovered from mirror"
    );
    let (co_ok, _, co_err) = git_try(&clone_dir, &["checkout", "-q", "main"]);
    assert!(co_ok, "checkout off mirror clone must succeed: {co_err}");
    assert_eq!(
        std::fs::read_to_string(clone_dir.join("f.txt")).unwrap(),
        "line 4\n"
    );

    // And the controller's working-repo invariant agrees.
    let refs = git(&mirror, &["show-ref"])
        .lines()
        .map(|l| l.split_whitespace().nth(1).unwrap().to_string())
        .collect::<Vec<_>>();
    let model = WorkingGitRepo {
        path: mirror.clone(),
        refs,
        head_resolves: true,
    };
    assert!(model.is_working());
}

#[test]
fn item_11_substrate_loss_full_recovery_resume_end_to_end() {
    let tmp = TmpDir::new("subloss-e2e");
    let src = build_source_repo(&tmp, 4);
    let mirror = make_mirror(&tmp, &src);
    let units = seed_units_from(&src);

    // Induce substrate loss.
    std::fs::remove_dir_all(&src).unwrap();
    let incident = substrate_loss_incident("forge/substrate lost");
    assert_eq!(incident.kind, LossKind::SubstrateLoss);
    assert_eq!(incident.recovery_source, RecoverySource::VerifiedMirror);
    assert_eq!(incident.completeness, CompletenessCriterion::ByteIdentity);

    // Rebuild the substrate from the surviving mirror, end-to-end. Byte-identity
    // is read back from a REAL clone of the mirror (git-true SHAs).
    let rebuilt = tmp.path().join("rebuilt-substrate");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            "--mirror",
            mirror.to_str().unwrap(),
            rebuilt.to_str().unwrap(),
        ],
    );
    let rebuilt_hashes: std::collections::HashMap<String, String> = git(&rebuilt, &["show-ref"])
        .lines()
        .map(|l| {
            let mut p = l.split_whitespace();
            let h = p.next().unwrap().to_string();
            let r = p.next().unwrap().to_string();
            (r, h)
        })
        .collect();

    let mirror_model = WorkingGitRepo {
        path: mirror.clone(),
        refs: units.iter().map(|u| u.refname.clone()).collect(),
        head_resolves: true,
    };
    let outcome = recover_substrate_loss(&mirror_model, &units, |u| {
        rebuilt_hashes.get(&u.refname).cloned().unwrap_or_default()
    });
    match outcome {
        RecoveryOutcome::Recovered {
            resume_from,
            refs_verified,
        } => {
            assert_eq!(refs_verified, units.len());
            // resume-from-recovered-state asserted.
            assert!(resume_from.last_verified.is_some());
        }
        other => panic!("expected Recovered e2e, got {other:?}"),
    }

    // Fail-CLOSED: a non-working mirror cannot recover.
    let dead = WorkingGitRepo {
        path: mirror,
        refs: vec![],
        head_resolves: false,
    };
    assert!(!recover_substrate_loss(&dead, &units, |u| u.object_hash.clone()).is_recovered());
}

#[test]
fn item_11_recovered_content_imports_as_change_events_not_fabricated() {
    let tmp = TmpDir::new("subloss-events");
    let src = build_source_repo(&tmp, 3);
    let units = seed_units_from(&src);

    // Recovered commit history projects into opaque change-events (EventRecord),
    // never a synthesized intent — the E2⑤ import boundary.
    let mut prev = "0".repeat(64);
    let mut events: Vec<EventRecord> = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let ev = project_recovered_commit(
            i as u64,
            &prev,
            &u.object_hash,
            vec!["mirror-recovery".into()],
            format!("{{\"ref\":\"{}\"}}", u.refname),
            1_717_000_000_000,
        );
        assert_eq!(
            ev.kind, CHANGE_EVENT_KIND,
            "must be a change-event, not an intent"
        );
        // Boundary accepts change-events.
        import_as_change_event(&ev).expect("change-event must import");
        prev = ev.this_hash.clone();
        events.push(ev);
    }
    assert_eq!(events.len(), units.len());

    // Fail-CLOSED: importing recovered content as a fabricated intent is rejected.
    let fabricated = EventRecord {
        seq: 99,
        prev_hash: prev,
        this_hash: "f".repeat(64),
        kind: "intent".to_string(),
        principal_chain: vec!["mirror-recovery".into()],
        payload: "{}".into(),
        recorded_at: 1_717_000_000_000,
    };
    assert!(
        import_as_change_event(&fabricated).is_err(),
        "fabricating an intent from a bare recovered commit must be rejected (E2⑤)"
    );
}
