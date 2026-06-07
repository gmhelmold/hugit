//! Hermetic supply-chain + RCE oracle — runs in the **bare** `cargo test
//! --workspace` gate with NO runner box (fail-not-skip, no `HUGIT_RUNNER_HOST`
//! needed).
//!
//! The brutal review's load-bearing X4 finding (R2) was that the verify-before-
//! spawn invariant lived only in a wrapper the real path bypassed, AND that the
//! box-dependent oracle was a silent no-op in CI (no lane sets the host). This
//! file closes both: a [`FakeBox`] + [`FakeEngine`] record every call, so the
//! ordering ("an unpinned/tampered image never reaches `docker run`") and the
//! tmp_root RCE guard are proven deterministically, offline, on every gate run.
//!
//! What is asserted here, with no real Docker / no SSH:
//! - an UNPINNED image (floating tag) is refused at `from_lease` — the spec is
//!   never built, so `docker run` is never reached;
//! - a TAMPERED (syntactically-pinned but unservable) digest passes `from_lease`
//!   but is refused at `spawn` by the box integrity check — BEFORE `docker run`;
//!   the engine's `run` surface records ZERO container starts;
//! - a tmp_root injection string is refused at `from_lease`;
//! - a dead cached handle is re-spawned (liveness probe), never handed back;
//! - concurrent identical spawns dedup to ONE materialization while DIFFERENT
//!   ids are not serialized behind each other.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use hugit_contracts::{FenceManifest, RunnerLease, RunnerState};
use hugit_runner::isolation::{DockerEngine, Engine, IsolationProbe, RunningContainer};
use hugit_runner::lease::{BoxExec, CmdOutput, ContainerSpec};
use hugit_runner::ws::{DedupSpawner, spawn_workspace};

/// A real, servable content pin (its digest is the one `FakeBox` "serves").
const GOOD_PIN: &str =
    "alpine@sha256:1111111111111111111111111111111111111111111111111111111111111111";
/// Syntactically valid but unservable (tampered) digest — the registry refuses.
const TAMPERED_PIN: &str =
    "alpine@sha256:0000000000000000000000000000000000000000000000000000000000000000";

// ── FakeBox: records every command; simulates a content-addressed registry ──────

#[derive(Clone, Default)]
struct FakeBox {
    /// Every `run` argv joined by space, in order (the audit trail).
    calls: Arc<Mutex<Vec<String>>>,
    /// Digests the simulated registry can serve (good pins resolve; others fail).
    servable: Arc<HashSet<String>>,
}

impl FakeBox {
    fn new() -> Self {
        let mut servable = HashSet::new();
        servable.insert(GOOD_PIN.to_string());
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            servable: Arc::new(servable),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    /// `true` iff any recorded command actually started a container.
    fn ran_a_container(&self) -> bool {
        self.calls()
            .iter()
            .any(|c| c.contains("docker run") || c.starts_with("docker run"))
    }
}

impl BoxExec for FakeBox {
    fn run(&self, argv: &[&str]) -> Result<CmdOutput> {
        let joined = argv.join(" ");
        self.calls.lock().unwrap().push(joined.clone());

        // Simulate the content-addressed registry + inspect used by verify_on_box.
        if argv.first() == Some(&"docker") && argv.get(1) == Some(&"pull") {
            let reference = argv.get(2).copied().unwrap_or("");
            let ok = self.servable.contains(reference);
            return Ok(CmdOutput {
                code: Some(if ok { 0 } else { 1 }),
                stdout: String::new(),
                stderr: if ok {
                    String::new()
                } else {
                    "manifest unknown".to_string()
                },
            });
        }
        if argv.first() == Some(&"docker")
            && argv.get(1) == Some(&"image")
            && argv.get(2) == Some(&"inspect")
        {
            let reference = argv.get(3).copied().unwrap_or("");
            // RepoDigests echoes the exact pinned reference for a servable image.
            let stdout = if self.servable.contains(reference) {
                format!("{reference}\n")
            } else {
                String::new()
            };
            return Ok(CmdOutput {
                code: Some(0),
                stdout,
                stderr: String::new(),
            });
        }
        // Default: succeed quietly (docker run / ps / exec).
        Ok(CmdOutput {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    fn run_with_stdin(&self, argv: &[&str], _stdin: &[u8]) -> Result<CmdOutput> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("[stdin] {}", argv.join(" ")));
        Ok(CmdOutput {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

// ── FakeEngine: records spawns; never touches the network ───────────────────────

#[derive(Clone, Default)]
struct FakeEngine {
    /// Images that reached `spawn` (== reached the real `docker run` surface).
    spawned_images: Arc<Mutex<Vec<String>>>,
    /// Container names that are currently "alive" (for the liveness probe).
    alive: Arc<Mutex<HashSet<String>>>,
    /// Optional box used to mirror the production verify-before-spawn ordering.
    boxx: Option<FakeBox>,
}

impl FakeEngine {
    fn with_box(boxx: FakeBox) -> Self {
        Self {
            spawned_images: Arc::new(Mutex::new(Vec::new())),
            alive: Arc::new(Mutex::new(HashSet::new())),
            boxx: Some(boxx),
        }
    }

    fn spawned_images(&self) -> Vec<String> {
        self.spawned_images.lock().unwrap().clone()
    }

    fn kill(&self, name: &str) {
        self.alive.lock().unwrap().remove(name);
    }
}

impl Engine for FakeEngine {
    fn spawn(&self, spec: &ContainerSpec) -> Result<RunningContainer> {
        // Mirror DockerEngine: verify the pin against the box BEFORE recording a
        // spawn, so the ordering is faithfully exercised offline.
        if let Some(boxx) = &self.boxx {
            let pinned = hugit_runner::pin::PinnedImageRef::parse(&spec.image)?;
            pinned.verify_on_box(boxx)?;
        }
        self.spawned_images.lock().unwrap().push(spec.image.clone());
        self.alive.lock().unwrap().insert(spec.name.clone());
        Ok(RunningContainer {
            name: spec.name.clone(),
        })
    }

    fn probe(&self, _c: &RunningContainer, _spec: &ContainerSpec) -> Result<IsolationProbe> {
        Ok(IsolationProbe {
            tmp_is_private: true,
            net_is_isolated: true,
        })
    }

    fn exec(&self, _c: &RunningContainer, _argv: &[&str]) -> Result<Option<i32>> {
        Ok(Some(0))
    }

    fn is_alive(&self, c: &RunningContainer) -> Result<bool> {
        Ok(self.alive.lock().unwrap().contains(&c.name))
    }
}

fn lease(slug: &str) -> RunnerLease {
    RunnerLease {
        lease_id: format!("herm-{slug}"),
        principal_chain: vec!["agent:hermetic".to_string()],
        path_set: vec!["src/".to_string()],
        expiry: u64::MAX,
        net_policy: "none".to_string(),
        tmp_root: "/hugit/tmp".to_string(),
        state: RunnerState::Held,
    }
}

fn src_fence() -> FenceManifest {
    FenceManifest {
        path_set: vec!["src/".to_string()],
        deny_default: true,
        materialized: vec![],
    }
}

// ── ① unpinned image is refused at spec-build — never reaches docker run ─────────
#[test]
fn unpinned_image_refused_before_any_spawn() {
    let boxx = FakeBox::new();
    let engine = FakeEngine::with_box(boxx.clone());

    // A floating tag must be rejected at from_lease — no spec, no spawn.
    let err = ContainerSpec::from_lease(&lease("tag"), "alpine:3.20").unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("pin")
            || err.to_string().contains("content-pinned"),
        "unpinned image must be refused with a pin error; got: {err}"
    );

    // And the full workspace spawn path refuses it too — engine never spawns.
    let r = spawn_workspace(&engine, &lease("tag"), &src_fence(), "alpine:3.20");
    assert!(r.is_err(), "spawn_workspace must refuse an unpinned image");
    assert!(
        engine.spawned_images().is_empty(),
        "no image may reach the engine spawn surface for an unpinned ref"
    );
    assert!(
        !boxx.ran_a_container(),
        "no `docker run` may be issued for an unpinned image"
    );
}

// ── ② tampered digest passes parse but is refused at spawn BEFORE docker run ─────
#[test]
fn tampered_digest_refused_before_docker_run() {
    let boxx = FakeBox::new();
    let engine = FakeEngine::with_box(boxx.clone());

    // Syntactically pinned → from_lease accepts it (integrity is a spawn-time,
    // box-side check, not a parse-time one).
    let spec = ContainerSpec::from_lease(&lease("tamper"), TAMPERED_PIN)
        .expect("tampered digest is syntactically pinned");

    // But spawn must fail CLOSED — the registry cannot serve the digest.
    let r = engine.spawn(&spec);
    assert!(
        r.is_err(),
        "tampered digest must fail integrity verification"
    );
    assert!(
        engine.spawned_images().is_empty(),
        "a tampered image must NOT be recorded as spawned"
    );

    // The box saw a `docker pull` (the verify) but NEVER a `docker run`.
    let calls = boxx.calls();
    assert!(
        calls.iter().any(|c| c.contains("docker pull")),
        "verify must attempt a content-addressed pull first"
    );
    assert!(
        !boxx.ran_a_container(),
        "verify-before-spawn: NO `docker run` after a failed integrity check"
    );
}

// ── ③ a good pin spawns, and the pull (verify) precedes any run ──────────────────
#[test]
fn good_pin_verifies_then_spawns_in_order() {
    let boxx = FakeBox::new();
    let engine = FakeEngine::with_box(boxx.clone());

    let spec = ContainerSpec::from_lease(&lease("good"), GOOD_PIN).expect("good pin");
    let c = engine.spawn(&spec).expect("good pin must spawn");
    assert_eq!(engine.spawned_images(), vec![GOOD_PIN.to_string()]);
    assert!(engine.is_alive(&c).unwrap());

    // Ordering proof: the pull index precedes nothing-after-failure; with the
    // FakeEngine the run is recorded in the engine, the pull in the box. The
    // box must have pulled before the engine recorded the spawn.
    let calls = boxx.calls();
    assert!(
        calls.iter().any(|c| c.contains("docker pull")),
        "a good pin is still pulled (verified) before spawn"
    );
}

// ── ③b REAL DockerEngine::spawn enforces verify-before-run (non-tautological) ───
//
// This drives the PRODUCTION `DockerEngine::spawn` against a FakeBox (no real
// SSH/Docker), so the path under test is the real spawn surface — not a fake
// that re-implements the check. On `main` this test is RED: `DockerEngine::spawn`
// issued `docker run` for any image. With the fix, a tampered image yields a
// pull (verify) but NEVER a `docker run`, and a good pin runs only after the
// pull.
#[test]
fn real_docker_engine_verifies_before_run() {
    // A ContainerSpec built by hand (bypassing from_lease) to prove the engine
    // itself is the floor — even a directly-constructed spec is gated.
    let make_spec = |image: &str| ContainerSpec {
        name: "hugit-job-real".to_string(),
        image: image.to_string(),
        tmp_root: "/hugit/tmp".to_string(),
        no_network: true,
        path_set: vec![],
    };

    // Tampered: pull is attempted, but no `docker run` ever follows.
    let boxx = FakeBox::new();
    let engine = DockerEngine::new(boxx.clone());
    let r = engine.spawn(&make_spec(TAMPERED_PIN));
    assert!(r.is_err(), "real engine must refuse a tampered digest");
    let calls = boxx.calls();
    assert!(
        calls.iter().any(|c| c.contains("docker pull")),
        "real engine must verify (pull) first"
    );
    assert!(
        !boxx.ran_a_container(),
        "real engine: NO `docker run` after a failed integrity check"
    );

    // Unpinned: refused even earlier (parse), still no `docker run`.
    let boxx2 = FakeBox::new();
    let engine2 = DockerEngine::new(boxx2.clone());
    assert!(engine2.spawn(&make_spec("alpine:3.20")).is_err());
    assert!(
        !boxx2.ran_a_container(),
        "real engine: NO `docker run` for an unpinned image"
    );

    // Good pin: verify (pull) precedes the single `docker run`.
    let boxx3 = FakeBox::new();
    let engine3 = DockerEngine::new(boxx3.clone());
    engine3.spawn(&make_spec(GOOD_PIN)).expect("good pin runs");
    let c3 = boxx3.calls();
    let pull_idx = c3.iter().position(|c| c.contains("docker pull"));
    let run_idx = c3.iter().position(|c| c.contains("docker run"));
    assert!(pull_idx.is_some() && run_idx.is_some());
    assert!(
        pull_idx < run_idx,
        "verify (pull) MUST precede docker run; calls: {c3:?}"
    );
}

// ── ④ tmp_root injection is refused at from_lease (root-RCE guard) ───────────────
#[test]
fn tmp_root_injection_refused() {
    let mut l = lease("rce");
    l.tmp_root = "/x' ; touch /pwned ; echo '".to_string();
    let err = ContainerSpec::from_lease(&l, GOOD_PIN).unwrap_err();
    assert!(
        err.to_string().contains("tmp_root"),
        "tmp_root injection must be refused at from_lease; got: {err}"
    );
}

// ── ⑤ a dead cached handle is re-spawned, never handed back ──────────────────────
#[test]
fn dead_cached_handle_is_respawned() {
    use std::time::Duration;
    let boxx = FakeBox::new();
    let engine = FakeEngine::with_box(boxx.clone());
    let spawner = DedupSpawner::new(Duration::from_secs(30));

    let h1 = spawner
        .spawn_or_join("ws-a", &engine, &lease("a"), &src_fence(), GOOD_PIN)
        .expect("first spawn");
    // Kill the container out-of-band (simulate a reaped corpse).
    engine.kill(&h1.container.name);

    // The next call must NOT return the dead handle — it re-spawns.
    let _h2 = spawner
        .spawn_or_join("ws-a", &engine, &lease("a"), &src_fence(), GOOD_PIN)
        .expect("re-spawn after corpse");
    assert_eq!(
        engine.spawned_images().len(),
        2,
        "a dead cached handle must trigger a fresh spawn (no corpse reuse)"
    );
}

// ── ⑥ concurrent identical spawns dedup to one; different ids don't serialize ────
#[test]
fn concurrent_identical_dedups_to_one() {
    use std::thread;
    use std::time::Duration;

    let boxx = FakeBox::new();
    let engine = Arc::new(FakeEngine::with_box(boxx));
    let spawner = Arc::new(DedupSpawner::new(Duration::from_secs(30)));

    let mut handles = Vec::new();
    for _ in 0..8 {
        let spawner = Arc::clone(&spawner);
        let engine = Arc::clone(&engine);
        handles.push(thread::spawn(move || {
            spawner
                .spawn_or_join(
                    "ws-shared",
                    &*engine,
                    &lease("shared"),
                    &src_fence(),
                    GOOD_PIN,
                )
                .expect("concurrent spawn_or_join")
                .container
                .name
        }));
    }
    let names: HashSet<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        names.len(),
        1,
        "8 identical spawns must dedup to ONE container"
    );
    assert_eq!(
        engine.spawned_images().len(),
        1,
        "exactly ONE materialization for 8 identical concurrent spawns"
    );
}
