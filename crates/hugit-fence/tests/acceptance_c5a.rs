//! WP-C5a acceptance oracle — sparse fence materialization + path enforcement.
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ① `item_1_outside_path_set_enoent` — a claim-fenced workspace is sparsely
//!      materialized from a [`FenceManifest`] (only the in-fence path_set is
//!      written into a live per-job container); an access to a path *outside*
//!      the path_set returns **ENOENT** — the file physically isn't there —
//!      while the in-fence file is readable.
//!
//! This is **box-dependent**: it drives the live runner box pinned by
//! `HUGIT_RUNNER_HOST` (the suite exports `91.99.11.196`). When the box is
//! unreachable it **FAILS** (not skip) — per contract, box-dependent tests must
//! fail, never silently pass. It skips only when `HUGIT_RUNNER_HOST` is unset
//! (the bare cargo gate lane, which does not provision the box).
//!
//! Box-sharing: WP-C2b runs concurrently on the same box. Everything here is
//! namespaced with the prefix `hugit-c5a-`; spawn/probe/teardown touch only
//! that prefix and never `hugit-c2b-*` / `hugit-job-*`.

use hugit_contracts::FenceManifest;
use hugit_fence::enforce::{FenceVerdict, classify, probe_outside_enoent};
use hugit_fence::materialize::{CandidateEntry, materialize_sparse};
use hugit_runner::isolation::RunningContainer;
use hugit_runner::lease::{BoxExec, SshBox};

const IMAGE: &str = "alpine:3.20";
/// All box artifacts for this WP carry this prefix (box-sharing isolation).
const PREFIX: &str = "hugit-c5a-";
/// In-container workspace root the fence materializes into.
const WORKSPACE_ROOT: &str = "/hugit-c5a-ws";

/// Whether the box-dependent acceptance lane is active.
///
/// The WP-C5a suite always exports `HUGIT_RUNNER_HOST`; inside that lane the
/// test runs and FAILS if the box is unreachable. When the var is absent the
/// file is being collected by the bare cargo gate lane (`cargo test
/// --workspace`), which must stay green and does not provision the box — so the
/// body short-circuits. Acceptance completeness is owned by the suite that sets
/// the env, never by the bare gate.
fn box_lane_active() -> bool {
    std::env::var("HUGIT_RUNNER_HOST")
        .ok()
        .is_some_and(|h| !h.trim().is_empty())
}

/// Connect to the live box; FAIL (panic) if it is unreachable, per contract.
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

/// A unique, prefix-namespaced container name for this run.
fn fresh_name(slug: &str) -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{PREFIX}{slug}-{nonce}")
}

/// Spawn a `hugit-c5a-`-prefixed, network-isolated idle container directly via
/// the box transport (NOT the C2a `DockerEngine`, whose `hugit-job-` name +
/// `hugit.job=1` label are shared with C2b). Returns a [`RunningContainer`]
/// consumable by the fence API.
fn spawn_fenced(boxx: &SshBox, name: &str) -> RunningContainer {
    let out = boxx
        .run(&[
            "docker",
            "run",
            "-d",
            "--rm",
            "--name",
            name,
            "--network",
            "none",
            "--label",
            "hugit.wp=c5a",
            IMAGE,
            "sleep",
            "3600",
        ])
        .expect("docker run failed to spawn");
    assert!(out.ok(), "docker run {name} failed: {}", out.stderr.trim());
    RunningContainer {
        name: name.to_string(),
    }
}

/// Force-remove a `hugit-c5a-`-prefixed container (idempotent, prefix-scoped).
fn teardown_fenced(boxx: &SshBox, name: &str) {
    assert!(
        name.starts_with(PREFIX),
        "refusing to tear down non-c5a container {name}"
    );
    let _ = boxx.run(&["docker", "rm", "-f", name]);
}

// ── ① outside path_set → ENOENT ─────────────────────────────────────────────
#[test]
fn item_1_outside_path_set_enoent() {
    if !box_lane_active() {
        return;
    }
    let boxx = live_box();
    ensure_image(&boxx);
    let name = fresh_name("enoent");
    let container = spawn_fenced(&boxx, &name);

    // The fence: only `src/` is in the path_set. `secret.env` and an out-of-fence
    // sibling are offered as candidates but MUST NOT be materialized.
    let manifest = FenceManifest {
        path_set: vec!["src/".to_string()],
        deny_default: true,
        materialized: vec![],
    };
    let candidates = vec![
        CandidateEntry::new("src/main.rs", b"fn main() {}\n".to_vec()),
        CandidateEntry::new("secret.env", b"TOKEN=must-not-materialize\n".to_vec()),
        CandidateEntry::new("config/prod.toml", b"key=outside\n".to_vec()),
    ];

    // Pure classification agrees with the intended fence before we touch the box.
    assert_eq!(classify(&manifest, "src/main.rs"), FenceVerdict::Inside);
    assert_eq!(classify(&manifest, "secret.env"), FenceVerdict::Outside);
    assert_eq!(
        classify(&manifest, "config/prod.toml"),
        FenceVerdict::Outside
    );

    // Sparse-materialize into the live container, then prove ENOENT on the box.
    let result = (|| {
        let filled = materialize_sparse(&boxx, &container, WORKSPACE_ROOT, &manifest, &candidates)
            .map_err(|e| format!("materialize: {e}"))?;

        // Only the in-fence file was recorded as materialized.
        let mat_paths: Vec<&str> = filled
            .materialized
            .iter()
            .map(|e| e.path.as_str())
            .collect();
        if mat_paths != vec!["src/main.rs"] {
            return Err(format!(
                "expected only src/main.rs materialized, got {mat_paths:?}"
            ));
        }

        // The in-fence file is present and readable inside the container.
        let read = boxx
            .run(&[
                "docker",
                "exec",
                &container.name,
                "cat",
                &format!("{WORKSPACE_ROOT}/src/main.rs"),
            ])
            .map_err(|e| format!("read in-fence: {e}"))?;
        if !(read.ok() && read.stdout.contains("fn main")) {
            return Err(format!(
                "in-fence src/main.rs should be readable (code={:?} stderr={:?})",
                read.code,
                read.stderr.trim()
            ));
        }

        // THE FENCE: out-of-fence paths return ENOENT — physically absent.
        for outside in ["secret.env", "config/prod.toml"] {
            let proof = probe_outside_enoent(&boxx, &container, WORKSPACE_ROOT, outside)
                .map_err(|e| format!("probe {outside}: {e}"))?;
            if !proof.enoent {
                return Err(format!(
                    "out-of-fence {outside} must return ENOENT; observed {:?} at {}",
                    proof.observed, proof.path
                ));
            }
        }

        // Traversal-escape access is also ENOENT (no break-out via `..`).
        let escape = probe_outside_enoent(&boxx, &container, WORKSPACE_ROOT, "src/../secret.env")
            .map_err(|e| format!("probe escape: {e}"))?;
        if !escape.enoent {
            return Err(format!(
                "traversal escape must return ENOENT; observed {:?}",
                escape.observed
            ));
        }
        Ok(())
    })();

    // Always clean up the box, regardless of outcome (prefix-scoped).
    teardown_fenced(&boxx, &name);
    result.expect("fence ENOENT acceptance");
}
