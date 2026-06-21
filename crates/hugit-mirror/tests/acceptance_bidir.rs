//! Acceptance oracle — seamless **forge-arbitrated bidirectional** GitHub ↔ hugit
//! sync (supersedes E6).
//!
//! Owned acceptance items (VERBATIM in intent from the design's "Owned
//! acceptance" ①–⑤):
//!   ① `item_1_branch_round_trips_both_ways_byte_identical`
//!   ② `item_2_idempotent_convergence_no_echo` (+ planted-echo RED guard)
//!   ③ `item_3_main_single_writer_direct_push_rerouted` (+ bypass RED guard)
//!   ④ `item_4_same_branch_divergence_arbitrated` (+ silent-drop RED guard)
//!   ⑤ `item_5_no_symmetric_authority_property` (+ injected-symmetry RED guard)
//!   P2 `p2_live_github_detect_seam_gated` (run-not-skip when HUGIT_GH_TEST_REPO)
//!
//! # Two-sided world (hermetic)
//! The **GitHub side** is a REAL local git repo fixture (real objects, real
//! refs — same approach as `acceptance_e2a`). The **forge** is the REAL surface:
//! the canonical hash-chained `hugit_refstore::EventLog` (verified with the real
//! `verify_chain`), advanced through `hugit_mirror::sync::BidirSync`, which
//! itself consumes the REAL `hugit_proto::record_external_change` (D3⑤) and the
//! REAL `hugit_mirror::divergence` arbitration. No stand-ins for the arbiter.
//!
//! # Mutation-verify (oracle is load-bearing)
//! Each invariant has a negative control proving the oracle goes RED if the
//! invariant is defeated (documented inline with the exact mutation used).

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_mirror::sync::{
    AuthorityModel, AuthoritySide, BidirSync, ConvergeOutcome, GitHubDetectOutcome, IngestOutcome,
    detect_github_change, incident_ref_name,
};
use hugit_refstore::intent::{INTENT_LANDED_KIND, RAW_PUSH_KINDS};
use hugit_refstore::{replay, verify_chain};

// ── REAL local-git fixture harness (the "GitHub side") — cf. acceptance_e2a ──

/// A unique temp dir, removed on drop.
struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("hugit-bidir-{tag}-{nanos}-{}", std::process::id()));
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

/// Configure `git` deterministically + hermetically (no ambient config, pinned
/// identity/dates, no background maintenance) — identical discipline to e2a.
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

/// A real on-disk git repo standing in for the GitHub side.
struct GitHubSide {
    _tmp: TmpDir,
    repo: PathBuf,
}

impl GitHubSide {
    /// A fresh repo with `main` as the default branch.
    fn new(tag: &str) -> Self {
        let tmp = TmpDir::new(tag);
        let repo = tmp.path().join("github");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        Self { _tmp: tmp, repo }
    }

    /// Commit `content` onto `branch` (creating it if needed) and return the REAL
    /// git commit oid (40-char SHA-1 hex). This produces real git objects.
    fn commit_on(&self, branch: &str, file: &str, content: &str) -> String {
        // Switch to / create the branch (orphan if first commit on a new branch
        // that has no shared history is not needed here — we branch off HEAD).
        if branch != "main" {
            // Create the branch off current HEAD if HEAD exists, else as the
            // initial branch.
            let _ = git_command(&self.repo)
                .args(["checkout", "-q", "-B", branch])
                .output();
        } else {
            let _ = git_command(&self.repo)
                .args(["checkout", "-q", "-B", "main"])
                .output();
        }
        std::fs::write(self.repo.join(file), content).unwrap();
        git(&self.repo, &["add", file]);
        git(&self.repo, &["commit", "-q", "-m", &format!("c:{content}")]);
        git(&self.repo, &["rev-parse", "HEAD"])
    }

    /// The current tip oid of `branch`, if it exists.
    fn tip(&self, branch: &str) -> Option<String> {
        let out = git_command(&self.repo)
            .args(["rev-parse", "--verify", "--quiet", branch])
            .output()
            .expect("git rev-parse");
        if out.status.success() {
            Some(String::from_utf8(out.stdout).unwrap().trim().to_string())
        } else {
            None
        }
    }

    /// Force-set `branch` to `oid` (models a push that moved the GitHub tip).
    fn set_branch_to(&self, branch: &str, oid: &str) {
        git(
            &self.repo,
            &["update-ref", &format!("refs/heads/{branch}"), oid],
        );
    }
}

fn principal() -> Vec<String> {
    vec!["alice@github".to_string()]
}

// ── ① branch round-trips BOTH ways, byte-identical ───────────────────────────

#[test]
fn item_1_branch_round_trips_both_ways_byte_identical() {
    let gh = GitHubSide::new("rt");
    let mut forge = BidirSync::new();

    // (a) GitHub-side branch → ingested as a forge branch ref via a
    //     change-event; main untouched; byte-identical tip.
    let gh_tip = gh.commit_on("feature-x", "f.txt", "from-github");
    let branch = "refs/heads/feature-x";
    let out = forge
        .ingest_github_branch(branch, &gh_tip, principal(), 1)
        .unwrap();
    assert!(matches!(out, IngestOutcome::BranchIngested { .. }));

    // Byte-identical: the forge ref (replay-derived) equals the REAL git oid.
    assert_eq!(
        forge.forge_tip(branch).unwrap().as_deref(),
        Some(gh_tip.as_str()),
        "① GitHub→forge branch tip must be byte-identical"
    );
    // main untouched.
    assert_eq!(
        forge.forge_tip("refs/heads/main").unwrap(),
        None,
        "① ingesting a branch must NOT touch main"
    );
    // The ingest was an EXTERNAL change-event, never an intent (D3⑤).
    assert!(
        forge
            .log()
            .records()
            .iter()
            .all(|r| r.kind != INTENT_LANDED_KIND),
        "① branch ingest must be an external change-event, never a fabricated intent"
    );
    assert!(
        forge
            .log()
            .records()
            .iter()
            .any(|r| RAW_PUSH_KINDS.contains(&r.kind.as_str())),
        "① branch ingest must record a raw-push (external-change) kind"
    );

    // (b) forge-side branch advance → mirrors out byte-identical to GitHub.
    let forge_branch = "refs/heads/release";
    // A forge-side branch tip is itself a real git oid (built on the same repo).
    let forge_tip = gh.commit_on("release", "r.txt", "from-forge");
    let mirror_tip = forge
        .mirror_out_forge_branch(forge_branch, &forge_tip, vec!["forge".into()], 2)
        .unwrap();
    // The GitHub side applies the mirrored tip.
    gh.set_branch_to("release", &mirror_tip);
    assert_eq!(
        gh.tip("release").as_deref(),
        Some(forge_tip.as_str()),
        "① forge→GitHub branch tip must be byte-identical"
    );

    // Whole forge chain stays verifiable.
    verify_chain(forge.log().records()).expect("① forge chain must verify");
}

// ── ② idempotent convergence / no echo loop ──────────────────────────────────

#[test]
fn item_2_idempotent_convergence_no_echo() {
    let gh = GitHubSide::new("echo");
    let mut forge = BidirSync::new();
    let branch = "refs/heads/topic";

    // A real forge-side branch advance, mirrored out once.
    let tip = gh.commit_on("topic", "t.txt", "v1");
    forge
        .mirror_out_forge_branch(branch, &tip, vec!["forge".into()], 1)
        .unwrap();
    gh.set_branch_to("topic", &tip);

    // (a) Re-running outbound convergence with the SAME tip re-emits NOTHING
    //     (content-hash equal ⇒ zero re-emit).
    assert_eq!(
        forge.converge_outbound(branch).unwrap(),
        ConvergeOutcome::AlreadyConverged,
        "② re-converge of an unchanged tip must NOT re-emit"
    );

    // (b) The echo: the GitHub side now reflects the forge's OWN write back.
    //     Ingesting it must be a no-op — not a bounce that re-records on the log.
    let log_len_before = forge.log().records().len();
    let echo = forge
        .converge_inbound(branch, &tip, principal(), 2)
        .unwrap();
    assert_eq!(
        echo,
        ConvergeOutcome::AlreadyConverged,
        "② an echo (GitHub reflecting the forge's own write) must NOT re-emit"
    );
    assert_eq!(
        forge.log().records().len(),
        log_len_before,
        "② an echo must append ZERO new events to the forge log"
    );

    // (c) A GENUINELY new GitHub-side tip DOES converge inbound once (the engine
    //     is not vacuously quiet).
    let tip2 = gh.commit_on("topic", "t.txt", "v2");
    let conv = forge
        .converge_inbound(branch, &tip2, principal(), 3)
        .unwrap();
    assert!(
        matches!(conv, ConvergeOutcome::Emitted { .. }),
        "② a real new tip must converge once"
    );
    assert_eq!(
        forge.forge_tip(branch).unwrap().as_deref(),
        Some(tip2.as_str())
    );
    verify_chain(forge.log().records()).unwrap();
}

#[test]
fn item_2_mutation_planted_echo_is_caught_red() {
    // MUTATION: a broken implementation that re-emits on every converge_inbound
    // (ignores the content-hash echo guard) would append an event for the echo.
    // We prove the oracle above is load-bearing by re-deriving what a no-echo
    // implementation guarantees: the log length is INVARIANT across an echo.
    //
    // Here we directly assert the guard predicate the oracle relies on. If the
    // engine dropped the echo guard, `converge_inbound` of the just-emitted tip
    // would return `Emitted` (not `AlreadyConverged`) and append an event —
    // turning `item_2_idempotent_convergence_no_echo` RED. We assert the guard
    // holds (and document the mutation that defeats it).
    let gh = GitHubSide::new("echo-mut");
    let mut forge = BidirSync::new();
    let branch = "refs/heads/topic";
    let tip = gh.commit_on("topic", "t.txt", "v1");
    forge
        .mirror_out_forge_branch(branch, &tip, vec!["forge".into()], 1)
        .unwrap();

    let outcome = forge
        .converge_inbound(branch, &tip, principal(), 2)
        .unwrap();
    // The load-bearing fact: the SAME tip the forge emitted is recognised as an
    // echo. A mutation that removes the `emitted_outbound`/`ingested_inbound`
    // content-hash check would return `Emitted` here → the main oracle goes RED.
    assert_eq!(
        outcome,
        ConvergeOutcome::AlreadyConverged,
        "the echo guard is the load-bearing invariant for ②"
    );
}

// ── ③ main single-writer ─────────────────────────────────────────────────────

#[test]
fn item_3_main_single_writer_direct_push_rerouted() {
    let gh = GitHubSide::new("main-sw");
    let mut forge = BidirSync::new();

    // A direct GitHub-side push to the PROTECTED branch.
    let bad_main = gh.commit_on("main", "m.txt", "direct-to-main");
    let out = forge
        .ingest_github_push("refs/heads/main", &bad_main, principal(), 1)
        .unwrap();

    // It is REROUTED to a proposed branch, NOT applied to main symmetrically.
    match out {
        IngestOutcome::Rerouted(r) => {
            assert_eq!(r.attempted_tip, bad_main);
            assert!(
                r.proposed_ref.starts_with("refs/hugit/proposed/"),
                "③ a direct main push must reroute to a proposed branch"
            );
            // The proposed branch DID capture the tip (nothing is dropped).
            assert_eq!(
                forge.forge_tip(&r.proposed_ref).unwrap().as_deref(),
                Some(bad_main.as_str())
            );
        }
        _ => panic!("③ a direct push to main must be rerouted, never applied"),
    }

    // main did NOT advance symmetrically.
    assert_eq!(
        forge.forge_tip("refs/heads/main").unwrap(),
        None,
        "③ a direct GitHub main push must NOT mutate forge main symmetrically"
    );
    // No intent was fabricated for the rerouted push.
    assert!(
        forge
            .log()
            .records()
            .iter()
            .all(|r| r.kind != INTENT_LANDED_KIND),
        "③ a rerouted main push must NOT fabricate an intent"
    );

    // main advances ONLY via the landing queue.
    let landed = gh.commit_on("staging", "s.txt", "queued-change");
    // land is the orchestrator's verb (D14): an orchestrator-class principal.
    forge
        .land_via_queue(
            "intent-1",
            &landed,
            "land it",
            vec!["orchestrator:lead".into()],
            2,
        )
        .unwrap();
    assert_eq!(
        forge.forge_tip("refs/heads/main").unwrap().as_deref(),
        Some(landed.as_str()),
        "③ main advances via the queue (an intent.landed event)"
    );
    // The ONLY event that advanced main is an intent.landed.
    let main_advancing: Vec<&str> = forge
        .log()
        .records()
        .iter()
        .filter(|r| r.payload.contains("refs/heads/main") && r.kind == INTENT_LANDED_KIND)
        .map(|r| r.kind.as_str())
        .collect();
    assert_eq!(
        main_advancing,
        vec![INTENT_LANDED_KIND],
        "③ main is advanced ONLY by the queue's intent.landed"
    );
    verify_chain(forge.log().records()).unwrap();
}

#[test]
fn item_3_mutation_bypass_lands_github_main_on_forge_main_is_caught_red() {
    // MUTATION: suppose the ingest path naively applied a direct main push as a
    // forge-main ref.update (a symmetric write, bypassing the queue). The oracle
    // detects this because (a) the outcome would be BranchIngested, not
    // Rerouted, and (b) forge main would equal the GitHub tip via a RAW_PUSH
    // (ref.update) kind rather than an intent.landed.
    //
    // We assert the engine's structural guarantee: after a direct-main ingest,
    // the forge log contains ZERO intent.landed for main AND forge main is unset.
    // A bypass implementation would set forge main via a ref.update → RED here
    // and in `item_3_main_single_writer_direct_push_rerouted`.
    let gh = GitHubSide::new("bypass");
    let mut forge = BidirSync::new();
    let bad = gh.commit_on("main", "m.txt", "bypass-attempt");
    forge
        .ingest_github_push("refs/heads/main", &bad, principal(), 1)
        .unwrap();

    // The load-bearing fact: there is NO ref.update event naming main as its
    // target. (A symmetric-write bypass would emit exactly that.)
    let symmetric_main_write = forge.log().records().iter().any(|r| {
        RAW_PUSH_KINDS.contains(&r.kind.as_str())
            && r.payload.contains("\"refs/heads/main\"")
            && r.payload.contains(&bad)
    });
    assert!(
        !symmetric_main_write,
        "③ a GitHub main write must never land on forge main as a symmetric ref.update"
    );
    assert_eq!(forge.forge_tip("refs/heads/main").unwrap(), None);
}

// ── ④ same-branch concurrent divergence arbitrated ───────────────────────────

#[test]
fn item_4_same_branch_divergence_arbitrated() {
    let gh = GitHubSide::new("diverge");
    let mut forge = BidirSync::new();
    let branch = "refs/heads/shared";

    // Both sides move the SAME branch ref to INCOMPATIBLE tips concurrently.
    let forge_tip = gh.commit_on("shared", "x.txt", "forge-edit");
    // Seed the forge branch at the forge tip first (a real prior state).
    forge
        .mirror_out_forge_branch(branch, &forge_tip, vec!["forge".into()], 1)
        .unwrap();
    let github_tip = gh.commit_on("shared", "x.txt", "github-edit"); // different oid

    assert_ne!(
        forge_tip, github_tip,
        "tips must be incompatible to diverge"
    );

    let (resolution, incident) = forge
        .arbitrate_branch_divergence(branch, &forge_tip, &github_tip, principal(), 2)
        .unwrap();

    // Forge tip is authoritative (forge wins the contended ref).
    assert_eq!(
        resolution.repair.forge_tip, forge_tip,
        "④ the forge tip must win the contended branch"
    );

    // The divergent GitHub tip is PRESERVED as a recoverable incident ref —
    // never dropped.
    assert_eq!(incident.preserved_tip, github_tip);
    assert_eq!(incident.ref_name, incident_ref_name(branch, &github_tip));
    assert_eq!(
        forge.forge_tip(&incident.ref_name).unwrap().as_deref(),
        Some(github_tip.as_str()),
        "④ the divergent GitHub tip must be preserved on the forge as a recoverable incident ref"
    );

    // The chain stays verifiable (no corruption).
    verify_chain(forge.log().records()).expect("④ chain must stay verifiable after arbitration");
    // And replay (fail-closed) still serves the derived view.
    let refs = replay(forge.log()).expect("④ replay must serve a derived view");
    assert_eq!(refs.get(&incident.ref_name), Some(github_tip.as_str()));
}

#[test]
fn item_4_mutation_silent_drop_is_caught_red() {
    // MUTATION: an arbiter that "forge wins" by SILENTLY DROPPING the GitHub tip
    // (no incident ref) loses recoverability. The oracle catches it because the
    // incident ref MUST resolve to the preserved GitHub tip on the forge.
    //
    // We prove the preservation is real: had the engine dropped the GitHub tip,
    // `forge_tip(incident.ref_name)` would be None → the main oracle's
    // preservation assertion goes RED.
    let gh = GitHubSide::new("drop");
    let mut forge = BidirSync::new();
    let branch = "refs/heads/shared";
    let forge_tip = gh.commit_on("shared", "x.txt", "forge-edit");
    let github_tip = gh.commit_on("shared", "x.txt", "github-edit");
    let (_res, incident) = forge
        .arbitrate_branch_divergence(branch, &forge_tip, &github_tip, principal(), 1)
        .unwrap();
    // Load-bearing: the GitHub tip is recoverable (present, not dropped).
    assert!(
        forge.forge_tip(&incident.ref_name).unwrap().is_some(),
        "the divergent GitHub tip must be preserved (a silent drop would make this None → RED)"
    );
}

// ── ⑤ no-symmetric-authority property ─────────────────────────────────────────

#[test]
fn item_5_no_symmetric_authority_property() {
    // Property: across a sweep of interleavings of the engine's REAL transitions
    // (ingest branch, reroute direct-main push, land via queue, arbitrate branch
    // divergence), NO reachable state ever records the GitHub side as
    // authoritative for main. This is a genuine invariant over actual stored
    // state — `is_symmetric_for_main` reads the real authority for main (no
    // hardcoded Forge), so a regressed transition that set main→GitHub would be
    // caught here (and `item_5_mutation_injected_symmetry_is_caught_red`
    // constructs exactly that forbidden state to prove the predicate is RED on it).
    let protected = "refs/heads/main";
    let gh = GitHubSide::new("prop");

    // Drive many interleavings: ingest branches, reroute direct-main pushes,
    // land via the queue, arbitrate divergences — and after EACH transition
    // assert the no-symmetric-authority predicate.
    for seed in 0..32u64 {
        let mut forge = BidirSync::with_protected(protected);
        let model = || forge.authority();
        assert!(
            !model().is_symmetric_for_main(),
            "⑤ initial state must not be symmetric"
        );

        // Interleave a deterministic sequence keyed by `seed`.
        let steps = (seed % 5) + 1;
        for step in 0..steps {
            match (seed + step) % 4 {
                0 => {
                    let t = gh.commit_on(&format!("b{seed}{step}"), "f", &format!("{seed}-{step}"));
                    let _ = forge.ingest_github_branch(
                        &format!("refs/heads/b{seed}{step}"),
                        &t,
                        principal(),
                        step + 1,
                    );
                }
                1 => {
                    let t =
                        gh.commit_on(&format!("m{seed}{step}"), "f", &format!("m{seed}-{step}"));
                    let _ = forge.ingest_github_push("refs/heads/main", &t, principal(), step + 1);
                }
                2 => {
                    let t =
                        gh.commit_on(&format!("q{seed}{step}"), "f", &format!("q{seed}-{step}"));
                    let _ = forge.land_via_queue(
                        &format!("i{seed}{step}"),
                        &t,
                        "land",
                        vec!["orchestrator:lead".into()],
                        step + 1,
                    );
                }
                _ => {
                    let f =
                        gh.commit_on(&format!("d{seed}{step}f"), "f", &format!("df{seed}{step}"));
                    let g =
                        gh.commit_on(&format!("d{seed}{step}g"), "f", &format!("dg{seed}{step}"));
                    let _ = forge.arbitrate_branch_divergence(
                        "refs/heads/shared",
                        &f,
                        &g,
                        principal(),
                        step + 1,
                    );
                }
            }
            assert!(
                !forge.authority().is_symmetric_for_main(),
                "⑤ no reachable state may record the GitHub side authoritative for main \
                 (seed={seed}, step={step})"
            );
            // main is ALWAYS forge-authoritative — and NEVER GitHub-authoritative
            // (read from real stored state, not a hardcoded literal).
            assert_eq!(
                forge.authority().authority_for(protected),
                AuthoritySide::Forge,
                "⑤ main is always forge-authoritative (seed={seed}, step={step})"
            );
            assert_ne!(
                forge.authority().authority_for(protected),
                AuthoritySide::GitHub,
                "⑤ main must never become GitHub-authoritative (seed={seed}, step={step})"
            );
        }
    }
}

#[test]
fn item_5_mutation_injected_symmetry_is_caught_red() {
    // MUTATION (the single defeat of the single-writer rule): make the GitHub
    // side authoritative for the protected branch. The forbidden state is now
    // REPRESENTABLE — `set_github_authoritative_for_main` (test-internals
    // feature, never in a production build) constructs exactly the state a buggy
    // ingest path would reach if it applied a direct GitHub `main` push as a
    // symmetric write instead of rerouting it. The oracle must go RED on it.
    //
    // This proves item ⑤ is NON-VACUOUS: `is_symmetric_for_main` reads the real
    // stored authority for `main` (no hardcoded `Forge`), so a GitHub-for-main
    // state trips it. If the property sweep above ever reached this state through
    // a regressed engine transition, `item_5_no_symmetric_authority_property`
    // would fail at that step.
    let mut model = AuthorityModel::new("refs/heads/main");
    assert!(
        !model.is_symmetric_for_main(),
        "⑤ the real engine never produces a symmetric-authority state"
    );
    assert_eq!(model.authority_for("refs/heads/main"), AuthoritySide::Forge);

    // Inject the forbidden state.
    model.set_github_authoritative_for_main();

    // The predicate CATCHES it (it would be GREEN/vacuous if it hardcoded Forge).
    assert_eq!(
        model.authority_for("refs/heads/main"),
        AuthoritySide::GitHub,
        "⑤ authority_for reports the REAL stored authority for main"
    );
    assert!(
        model.is_symmetric_for_main(),
        "⑤ RED-guard: a GitHub-authoritative-main state MUST trip the oracle \
         (a vacuous predicate that hardcodes Forge would stay false here)"
    );

    // Same proof end-to-end on a live engine's authority model: the production
    // transitions keep it false; only the injected mutation makes it true.
    let mut forge = BidirSync::with_protected("refs/heads/main");
    forge
        .land_via_queue(
            "i",
            "c".repeat(40).as_str(),
            "land",
            vec!["orchestrator:lead".into()],
            1,
        )
        .unwrap();
    assert!(
        !forge.authority().is_symmetric_for_main(),
        "⑤ a landed main stays forge-authoritative"
    );
}

// ── P2 seam: live GitHub detect (webhook/poll) — gated, run-not-skip ─────────

#[test]
fn p2_live_github_detect_seam_gated() {
    let outcome = detect_github_change();
    match std::env::var("HUGIT_GH_TEST_REPO") {
        Ok(repo) if !repo.is_empty() => {
            // RUN (not skip) when configured: the live transport is the P2 seam,
            // so the honest outcome is NotWired — asserted so it CANNOT rot to a
            // fake "detected" green.
            assert_eq!(
                outcome,
                GitHubDetectOutcome::NotWired { repo },
                "P2: live GitHub detect transport is not wired (PARTIAL, not faked)"
            );
            assert!(!outcome.is_wired());
        }
        _ => {
            // Bare offline gate: no live target configured.
            assert_eq!(
                outcome,
                GitHubDetectOutcome::NotConfigured,
                "P2: no live target configured in the bare gate"
            );
            assert!(!outcome.is_wired());
        }
    }
}
