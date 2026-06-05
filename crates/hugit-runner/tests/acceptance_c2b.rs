//! WP-C2b acceptance oracle — ephemeral runner: concurrency/throughput +
//! expiry hard-kill + crash recovery.
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ③ `item_3_expiry_hard_kill` — an expired lease's container is hard-killed
//!      (SIGKILL, not graceful) and torn down; the forensic re-scan is clean.
//!   ④ `item_4_concurrent_ge8_per_box` — ≥8 `hugit-c2b-*` containers run
//!      simultaneously on the one box, every job exits 0, and all tear down to
//!      zero residue.
//!   ⑤ `item_5_crash_recovery_lost_detected` (R2) — a box crash mid-job is
//!      simulated by reaping the container out-of-band; the runner detects the
//!      job **lost**, surfaces/requeues it (never silent-drop, never false
//!      green), marks the lease `Crashed`, and cleans lease/fence to zero
//!      residue. This is the ⑤ kill-test.
//!
//! These are **box-dependent**: they drive the live runner box pinned by
//! `HUGIT_RUNNER_HOST` (the suite exports `91.99.11.196`). When the box is
//! unreachable they **FAIL** (not skip) — box-dependent tests must fail, never
//! silently pass. When `HUGIT_RUNNER_HOST` is **entirely unset** (the bare
//! cargo gate lane) the box-dependent body short-circuits.
//!
//! # Box-sharing (CRITICAL)
//! WP-C5a runs on the SAME box concurrently. Every container/label this oracle
//! creates is under the `hugit-c2b-` prefix; the concurrency census and the
//! best-effort kill-sweep are scoped to that prefix only — never to containers
//! owned by other WPs.

use hugit_contracts::{RunnerLease, RunnerState};
use hugit_runner::concurrency::{C2B_PREFIX, Scheduler, c2b_container_name, c2b_spec};
use hugit_runner::expiry::{enforce_expiry, is_expired};
use hugit_runner::isolation::{DockerEngine, Engine, RunningContainer};
use hugit_runner::lease::{BoxExec, SshBox};
use hugit_runner::recovery::{LostDisposition, detect_and_recover};
use hugit_runner::teardown::teardown;

const IMAGE: &str = "alpine:3.20";

/// Build a fresh, unique, C2b-conformant lease with the given expiry.
fn fresh_lease(slug: &str, expiry: u64) -> RunnerLease {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    RunnerLease {
        lease_id: format!("c2b-{slug}-{nonce}"),
        principal_chain: vec!["agent:acceptance".to_string()],
        path_set: vec!["src/".to_string()],
        expiry,
        net_policy: "none".to_string(),
        tmp_root: "/hugit/tmp".to_string(),
        state: RunnerState::Held,
    }
}

/// Whether the box-dependent acceptance lane is active. Mirrors C2a: present &
/// non-empty `HUGIT_RUNNER_HOST` ⇒ run + FAIL on unreachable; entirely unset ⇒
/// short-circuit (bare cargo gate lane, which does not provision the box).
fn box_lane_active() -> bool {
    std::env::var("HUGIT_RUNNER_HOST")
        .ok()
        .is_some_and(|h| !h.trim().is_empty())
}

/// Connect to the live box; FAIL (panic) if unreachable, per contract.
fn live_box() -> SshBox {
    let boxx = SshBox::from_env().expect("HUGIT_RUNNER_HOST must be set inside the box lane");
    let ping = boxx
        .run(&["docker", "version", "--format", "{{.Server.Version}}"])
        .expect("ssh to runner box failed to spawn");
    assert!(
        ping.ok() && !ping.stdout.trim().is_empty(),
        "runner box {} unreachable or docker down (code={:?} stderr={:?}); \
         box-dependent acceptance must FAIL, not skip",
        boxx.target,
        ping.code,
        ping.stderr.trim(),
    );
    boxx
}

/// Ensure the job image is present on the box.
fn ensure_image(boxx: &SshBox) {
    let pull = boxx
        .run(&["docker", "pull", IMAGE])
        .expect("docker pull failed to spawn");
    assert!(
        pull.ok(),
        "docker pull {IMAGE} failed: {}",
        pull.stderr.trim()
    );
}

/// Best-effort sweep of **only** `hugit-c2b-*` containers, so a prior aborted
/// run never poisons a census. Never touches other WPs' containers.
fn sweep_c2b(boxx: &SshBox) {
    let _ = boxx.run(&[
        "sh",
        "-c",
        &format!("docker ps -aq --filter name={C2B_PREFIX} | xargs -r docker rm -f"),
    ]);
}

// ── ③ expiry hard-kill ──────────────────────────────────────────────────────
#[test]
fn item_3_expiry_hard_kill() {
    // Pure-logic guard: expiry is deterministic from the lease field, even off
    // the box. (Real kill happens on the box below.)
    assert!(is_expired(&fresh_lease("x", 0), 1));
    assert!(!is_expired(&fresh_lease("x", u64::MAX), u64::MAX - 1));

    if !box_lane_active() {
        return;
    }
    let boxx = live_box();
    ensure_image(&boxx);
    sweep_c2b(&boxx);

    // An already-expired lease (expiry in the past relative to "now").
    let lease = fresh_lease("expiry", 1);
    let spec = c2b_spec(&lease, IMAGE).expect("derive C2b spec");
    let engine = DockerEngine::new(boxx.clone());
    let container = engine.spawn(&spec).expect("spawn container for expiry");

    // Prove it was really running before the kill.
    let live = boxx
        .run(&[
            "docker",
            "ps",
            "--filter",
            &format!("name={}", spec.name),
            "--format",
            "{{.Names}}",
        ])
        .unwrap();
    assert!(
        live.stdout.contains(&spec.name),
        "container {} must be running before expiry kill",
        spec.name
    );

    // Enforce expiry at a "now" well past the lease's expiry → hard-kill.
    let now_ms = u64::MAX; // far past expiry=1
    let outcome = enforce_expiry(&boxx, &lease, &container, now_ms).expect("enforce expiry");
    assert!(outcome.killed, "expired lease must be hard-killed");
    assert!(
        outcome.was_running,
        "the live container should have been SIGKILLed (was running)"
    );
    assert!(
        outcome.residue.is_clean(),
        "forensic re-scan after expiry kill must be clean: {:#?}",
        outcome.residue
    );

    // And the container must be gone from the box.
    let gone = boxx
        .run(&[
            "docker",
            "ps",
            "-a",
            "--filter",
            &format!("name={}", spec.name),
            "--format",
            "{{.Names}}",
        ])
        .unwrap();
    assert!(
        !gone.stdout.contains(&spec.name),
        "container {} must be gone after expiry hard-kill",
        spec.name
    );
}

// ── ④ ≥8 concurrent / box ───────────────────────────────────────────────────
#[test]
fn item_4_concurrent_ge8_per_box() {
    if !box_lane_active() {
        return;
    }
    let boxx = live_box();
    ensure_image(&boxx);
    sweep_c2b(&boxx);

    const N: usize = 8;
    let leases: Vec<(RunnerLease, String)> = (0..N)
        .map(|i| {
            (
                fresh_lease(&format!("conc{i}"), u64::MAX),
                IMAGE.to_string(),
            )
        })
        .collect();

    let scheduler = Scheduler::new(boxx.clone(), DockerEngine::new(boxx.clone()));
    let report = scheduler
        .run_batch(&leases, &["true"])
        .expect("run concurrent batch");

    assert!(
        report.peak_concurrency >= N,
        "must reach ≥{N} concurrent hugit-c2b-* containers; peak was {}",
        report.peak_concurrency
    );
    assert_eq!(
        report.ok_count(),
        N,
        "all {N} concurrent jobs must exit 0; outcomes: {:#?}",
        report.outcomes
    );
    assert!(
        report.residue.is_clean(),
        "forensic re-scan after concurrent teardown must be clean: {:#?}",
        report.residue
    );

    // Census must be back to zero of *our* prefix (never counting other WPs).
    let census = scheduler.running_census().expect("post-batch census");
    assert_eq!(
        census, 0,
        "no hugit-c2b-* containers may remain after batch"
    );
}

// ── ⑤ box crash mid-job: lost → requeued/surfaced, no silent drop / false green
#[test]
fn item_5_crash_recovery_lost_detected() {
    if !box_lane_active() {
        return;
    }
    let boxx = live_box();
    ensure_image(&boxx);
    sweep_c2b(&boxx);

    // Spawn a long-running job (the "in-flight" job).
    let lease = fresh_lease("crash", u64::MAX);
    let spec = c2b_spec(&lease, IMAGE).expect("derive C2b spec");
    let engine = DockerEngine::new(boxx.clone());
    let container = engine.spawn(&spec).expect("spawn in-flight container");
    let _ = c2b_container_name(&lease.lease_id); // exercised name helper

    // Sanity: job is live before the crash.
    assert!(
        detect_and_recover(&boxx, &lease, &container, LostDisposition::Requeued)
            .expect("liveness probe while alive")
            .is_none(),
        "no recovery while the job is still alive (no false-positive loss)"
    );

    // ── KILL-TEST: simulate a box crash mid-job ──────────────────────────────
    // A box crash reaps the container out-of-band (reboot / OOM / daemon
    // restart). We emulate that by force-removing the container directly, NOT
    // via the runner's lifecycle — the runner did not initiate this.
    let crash = boxx
        .run(&["docker", "rm", "-f", &spec.name])
        .expect("simulate box crash (out-of-band container reaping)");
    assert!(
        crash.code == Some(0) || crash.stderr.contains("No such container"),
        "crash simulation should reap the container: {:?}",
        crash
    );

    // Now the runner must DETECT the job lost and SURFACE it — never green.
    let lost = detect_and_recover(&boxx, &lease, &container, LostDisposition::Requeued)
        .expect("detect_and_recover after crash")
        .expect("a crashed job MUST be detected lost, not silently dropped");

    assert_eq!(
        lost.lease_id, lease.lease_id,
        "the surfaced lost job must name the lost lease"
    );
    assert_eq!(
        lost.disposition,
        LostDisposition::Requeued,
        "lost job must be requeued/surfaced (not dropped)"
    );
    assert_eq!(
        lost.lease_state,
        RunnerState::Crashed,
        "lost lease must be marked Crashed (distinct from Expired/Released) — \
         never false-green"
    );
    assert!(
        lost.is_surfaced_not_green(),
        "degradation invariant (§9 lock 5): lost job surfaced+cleaned, not green"
    );
    assert!(
        lost.residue.is_clean(),
        "lease/fence cleanup after crash must leave zero residue: {:#?}",
        lost.residue
    );

    // Defensive cleanup of our own prefix only.
    let _ = teardown(&boxx, &RunningContainer { name: spec.name });
}
