//! WP-X4 acceptance oracle — supply chain: image pinning + integrity +
//! fail-closed. Contract: `docs/plan/wp-contracts/WP-X4.md`.
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ① `item_1_image_content_pinned_verified` — a runner image is content
//!      (digest) pinned and integrity-verified against the live box at spawn;
//!      a digest mismatch is detectable. (box-dependent)
//!   ② `item_2_app_deps_pinned_ci` — the App/workspace dependencies are pinned
//!      (committed `Cargo.lock`) and verified by the CI audit gate; a floating
//!      dep is rejected by the gate. (workspace-file inspection)
//!   ③ `item_3_tampered_unpinned_fail_closed` — a tampered AND an unpinned
//!      image both fail CLOSED **before any tenant work**: the verify-before-
//!      spawn guard rejects with no container spawned and no tenant byte
//!      processed. The ordering is load-bearing. (box-dependent)
//!
//! Box-dependence: items ① and ③ drive the live runner box pinned by
//! `HUGIT_RUNNER_HOST` (the suite exports `91.99.11.196`). When the env is set
//! but the box is unreachable they **FAIL** (not skip), per contract. When the
//! env is entirely **unset** (the bare `cargo test --workspace` gate lane) the
//! box-dependent bodies short-circuit so the gate stays green — acceptance
//! completeness is owned by the suite that sets the env.

use hugit_contracts::{RunnerLease, RunnerState};
use hugit_invariants::pin::{GuardedSpawn, PinnedImage, VerifiedSpawn};
use hugit_runner::isolation::DockerEngine;
use hugit_runner::lease::{BoxExec, ContainerSpec, SshBox};
use hugit_runner::teardown::teardown;

/// Tag used only to *resolve* a real content digest from the box; never used as
/// a spawn reference (a tag is unpinned by definition).
const RESOLVE_TAG: &str = "alpine:3.20";

/// A syntactically-valid but content-wrong (tampered) digest: 64 hex zeros.
/// No registry can serve it, so it must fail integrity verification.
const TAMPERED_REF: &str =
    "alpine@sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// Whether the box-dependent acceptance lane is active (env set + non-empty).
/// Mirrors the WP-C2a convention: the suite sets the env; the bare gate does
/// not, so box bodies short-circuit there to keep `cargo test --workspace`
/// green.
fn box_lane_active() -> bool {
    std::env::var("HUGIT_RUNNER_HOST")
        .ok()
        .is_some_and(|h| !h.trim().is_empty())
}

/// Connect to the live box; FAIL (panic) if unreachable, per contract. Only
/// called inside the active box lane.
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

/// Resolve a real `repo@sha256:…` content pin from the box by pulling a tag and
/// reading its `RepoDigests`. This is how a pinned reference is obtained from a
/// floating tag in practice; the result is a genuine, servable content digest.
fn resolve_pin(boxx: &SshBox) -> PinnedImage {
    let pull = boxx
        .run(&["docker", "pull", RESOLVE_TAG])
        .expect("docker pull failed to spawn");
    assert!(
        pull.ok(),
        "docker pull {RESOLVE_TAG} failed: {}",
        pull.stderr.trim()
    );
    let inspect = boxx
        .run(&[
            "docker",
            "image",
            "inspect",
            RESOLVE_TAG,
            "--format",
            "{{range .RepoDigests}}{{.}}\n{{end}}",
        ])
        .expect("docker inspect failed to spawn");
    assert!(
        inspect.ok(),
        "docker inspect failed: {}",
        inspect.stderr.trim()
    );
    let reference = inspect
        .stdout
        .lines()
        .map(str::trim)
        .find(|l| l.contains("@sha256:"))
        .unwrap_or_else(|| {
            panic!(
                "no RepoDigest resolved for {RESOLVE_TAG}: {:?}",
                inspect.stdout
            )
        })
        .to_string();
    PinnedImage::parse(&reference)
        .unwrap_or_else(|e| panic!("box-resolved reference {reference:?} is not a valid pin: {e}"))
}

/// A fresh, unique, C2a-conformant lease for the given image reference.
fn fresh_lease(slug: &str) -> RunnerLease {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    RunnerLease {
        lease_id: format!("x4-{slug}-{nonce}"),
        principal_chain: vec!["agent:acceptance".to_string()],
        path_set: vec!["src/".to_string()],
        expiry: u64::MAX,
        net_policy: "none".to_string(),
        tmp_root: "/hugit/tmp".to_string(),
        state: RunnerState::Held,
    }
}

// ── ① image content-pinned + integrity-verified at spawn ─────────────────────
#[test]
fn item_1_image_content_pinned_verified() {
    if !box_lane_active() {
        return;
    }
    let boxx = live_box();

    // A genuine content digest resolved from the box is pinned-parseable.
    let pinned = resolve_pin(&boxx);
    assert!(
        PinnedImage::is_pinned(pinned.reference()),
        "resolved reference {} must be content-pinned",
        pinned.reference()
    );
    assert_eq!(
        pinned.digest_hex().len(),
        64,
        "sha256 digest is 64 hex chars"
    );

    // A floating tag is NOT content-pinned — pinning is digest, not tag.
    assert!(
        !PinnedImage::is_pinned(RESOLVE_TAG),
        "a floating tag ({RESOLVE_TAG}) must not count as content-pinned"
    );

    // Integrity verification against the live box passes for the true pin …
    pinned
        .verify_on_box(&boxx)
        .expect("a box-resolved content pin must integrity-verify at spawn");

    // … and a digest MISMATCH is detectable (the integrity check fails CLOSED).
    let tampered = PinnedImage::parse(TAMPERED_REF).expect("tampered ref is syntactically pinned");
    assert!(
        tampered.verify_on_box(&boxx).is_err(),
        "a tampered (content-wrong) digest must fail integrity verification"
    );
}

// ── ② App deps pinned + verified in CI ───────────────────────────────────────
#[test]
fn item_2_app_deps_pinned_ci() {
    // Locate the repo root from this crate's manifest dir.
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest
        .parent() // crates/
        .and_then(std::path::Path::parent) // repo root
        .expect("repo root above crates/hugit-invariants")
        .to_path_buf();

    // (a) Dependencies are PINNED: a committed root Cargo.lock exists and pins
    //     concrete versions (`version = "…"` entries under [[package]]).
    let lock = repo_root.join("Cargo.lock");
    let lock_body = std::fs::read_to_string(&lock).unwrap_or_else(|e| {
        panic!(
            "committed Cargo.lock must pin deps ({}): {e}",
            lock.display()
        )
    });
    assert!(
        lock_body.contains("[[package]]") && lock_body.contains("version = \""),
        "Cargo.lock must pin concrete dependency versions"
    );

    // The App crate's own deps must appear pinned in the lockfile.
    let app_pinned = lock_body
        .split("[[package]]")
        .any(|p| p.contains("name = \"hugit-app\"") && p.contains("version = \""));
    assert!(
        app_pinned,
        "the App crate (hugit-app) must be pinned in Cargo.lock"
    );

    // (b) Verified in CI: the existing audit gate runs `cargo audit` over the
    //     pinned graph. X4 rides this gate, it does not add a new one.
    let ci = std::fs::read_to_string(repo_root.join(".github/workflows/ci.yml"))
        .expect("CI workflow must exist");
    assert!(
        ci.contains("cargo audit"),
        "CI must verify the pinned dependency graph via `cargo audit`"
    );
    assert!(
        ci.contains("cargo test --workspace"),
        "CI must build/test the pinned workspace"
    );

    // (c) An unpinned/floating dep is REJECTED: a Cargo.toml requirement with no
    //     concrete version (e.g. `dep = \"*\"`) is the floating case the gate
    //     refuses. Assert the App manifest carries no wildcard requirement.
    let app_toml = std::fs::read_to_string(repo_root.join("crates/hugit-app/Cargo.toml"))
        .expect("hugit-app Cargo.toml must exist");
    for line in app_toml.lines() {
        let l = line.trim();
        if l.starts_with('#') || l.is_empty() {
            continue;
        }
        assert!(
            !l.contains("\"*\"") && !l.ends_with("= \"*\""),
            "App dependency must not be a floating wildcard: {l:?}"
        );
    }
}

// ── ③ tampered/unpinned image → fail CLOSED before any tenant work ───────────
// Load-bearing: verify-before-spawn ORDERING. A post-hoc detection is a FAIL.
#[test]
fn item_3_tampered_unpinned_fail_closed() {
    if !box_lane_active() {
        return;
    }
    let boxx = live_box();
    let engine = DockerEngine::new(boxx.clone());
    let guard = VerifiedSpawn::new(&engine, &boxx);

    // ── attack 1: UNPINNED image (floating tag). ─────────────────────────────
    let lease_unpinned = fresh_lease("unpinned");
    let spec_unpinned =
        ContainerSpec::from_lease(&lease_unpinned, RESOLVE_TAG).expect("derive spec (unpinned)");
    let r1 = guard
        .spawn_verified(&spec_unpinned)
        .expect("guard must not error on an unpinned image; it must reject CLOSED");
    assert!(
        r1.rejected_before_spawn(),
        "unpinned image must be REJECTED before spawn, got: {r1:?}"
    );
    // Prove fail-closed ordering: NO container by that name exists on the box.
    assert_no_container(&boxx, &spec_unpinned.name, "unpinned");

    // ── attack 2: TAMPERED image (valid-form digest, content-wrong). ─────────
    let lease_tampered = fresh_lease("tampered");
    let spec_tampered =
        ContainerSpec::from_lease(&lease_tampered, TAMPERED_REF).expect("derive spec (tampered)");
    let r2 = guard
        .spawn_verified(&spec_tampered)
        .expect("guard must not error on a tampered image; it must reject CLOSED");
    assert!(
        r2.rejected_before_spawn(),
        "tampered image must be REJECTED before spawn, got: {r2:?}"
    );
    assert_no_container(&boxx, &spec_tampered.name, "tampered");

    // ── positive control: a genuine content pin DOES spawn (proving the guard
    //    is not vacuously rejecting everything), then is torn down clean. ─────
    let pinned = resolve_pin(&boxx);
    let lease_ok = fresh_lease("pinned-ok");
    let spec_ok =
        ContainerSpec::from_lease(&lease_ok, pinned.reference()).expect("derive spec (pinned)");
    let r3 = guard
        .spawn_verified(&spec_ok)
        .expect("verified pinned spawn");
    match r3 {
        GuardedSpawn::Spawned(container) => {
            // It really ran: tear it down and forensic-clean the box.
            let report = teardown(&boxx, &container).expect("teardown");
            assert!(
                report.is_clean(),
                "forensic re-scan found residue after teardown: {report:#?}"
            );
        }
        GuardedSpawn::RejectedBeforeSpawn(why) => {
            panic!("a genuine content pin must spawn, but was rejected: {why}")
        }
    }
}

/// Assert no container with `name` exists on the box — proof that a rejected
/// spawn processed **no tenant work** (fail-closed BEFORE spawn).
fn assert_no_container(boxx: &SshBox, name: &str, label: &str) {
    let ps = boxx
        .run(&[
            "docker",
            "ps",
            "-a",
            "--filter",
            &format!("name=^{name}$"),
            "--format",
            "{{.Names}}",
        ])
        .expect("docker ps failed to spawn");
    assert!(
        ps.stdout.trim().is_empty(),
        "{label} image must leave NO container (fail-closed before tenant work); \
         found: {:?}",
        ps.stdout.trim()
    );
}
