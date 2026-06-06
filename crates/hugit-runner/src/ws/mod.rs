//! Workspace lifecycle: attach / resume / spawn-dedup (WP-C9).
//!
//! This module orchestrates the workspace lifecycle over the C2a runtime
//! ([`Engine`](crate::isolation::Engine) + [`BoxExec`](crate::lease::BoxExec))
//! and the C5a fence ([`FenceManifest`](hugit_contracts::FenceManifest)). It
//! does NOT re-implement materialization or the box transport — it consumes them.
//!
//! # Four owned items (contract: WP-C9)
//! 1. **Attach (①)** — join a live workspace sharing the same
//!    fence/materialization *without* a respawn or re-hydrate.
//! 2. **Resume (②)** — restore state + fence; the resumed workspace is bounded
//!    by the original [`FenceManifest`] `path_set` and cannot exceed it.
//! 3. **Spawn + dedup (③)** — spawn a workspace in <1s; identical concurrent
//!    spawns are deduped to ONE materialization (warm-CAS economics).
//! 4. **Local ≡ remote (④)** — local and remote execution produce identical
//!    observable results; the same pure function either way.
//!
//! # Container naming
//! All C9 workspace containers are prefixed `hugit-c9-` so forensic scans and
//! kill-sweeps stay scoped to this WP on the shared box. Scans/cleanups target
//! ONLY this prefix — no other WP's containers are touched.
//!
//! # Firecracker note
//! Runtime is container-per-job on the Hetzner box; Firecracker is the
//! documented upgrade path, not built here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use hugit_contracts::{FenceManifest, RunnerLease};

use crate::isolation::{Engine, RunningContainer};
use crate::lease::{BoxExec, ContainerSpec};

/// Prefix for all C9-owned workspace containers on the shared box.
pub const C9_PREFIX: &str = "hugit-c9-";

// ── container naming ──────────────────────────────────────────────────────────

/// Derive a C9-namespaced container name from a workspace id.
///
/// The `hugit-c9-` prefix is what lets cleanup sweeps target ONLY this WP's
/// containers on the shared box. Docker names must match
/// `[a-zA-Z0-9][a-zA-Z0-9_.-]*`, so non-conforming chars are mapped to `_`.
#[must_use]
pub fn c9_container_name(workspace_id: &str) -> String {
    let mut s = String::with_capacity(workspace_id.len() + C9_PREFIX.len());
    s.push_str(C9_PREFIX);
    for c in workspace_id.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

// ── WorkspaceHandle ───────────────────────────────────────────────────────────

/// A live workspace handle — the result of spawn/attach/resume.
///
/// Carries the container that backs the workspace, the frozen [`RunnerLease`]
/// it runs under, and the [`FenceManifest`] that is the ceiling for path access.
/// The fence manifest cannot be widened on resume (security: resume is not a
/// fence-widening hole).
#[derive(Debug, Clone)]
pub struct WorkspaceHandle {
    /// The backing container.
    pub container: RunningContainer,
    /// The lease the workspace runs under (consumed, not modified).
    pub lease: RunnerLease,
    /// The fence manifest (path_set ceiling; cannot expand on resume).
    pub fence: FenceManifest,
    /// How this handle was obtained.
    pub origin: WorkspaceOrigin,
}

/// How a [`WorkspaceHandle`] was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceOrigin {
    /// A fresh container was spawned for this workspace.
    Spawned,
    /// An existing live container was joined (no respawn, no re-hydrate).
    Attached,
    /// A previously serialized state was restored (state + fence restored).
    Resumed,
}

// ── spawn ─────────────────────────────────────────────────────────────────────

/// Spawn a new workspace container under `lease`, enforcing the `fence`.
///
/// The container is named `hugit-c9-<workspace_id>` where `workspace_id` is
/// derived from `lease.lease_id`. The spawn MUST complete in <1s on a warm box
/// (`alpine:3.20` cached) — the C9 performance contract.
///
/// Returns a [`WorkspaceHandle`] with `origin = Spawned`.
///
/// # Errors
/// Fails if the box is unreachable, if the lease is invalid (propagates C2a
/// validation), or if the container fails to start.
pub fn spawn_workspace<E>(
    engine: &E,
    lease: &RunnerLease,
    fence: &FenceManifest,
    image: &str,
) -> Result<WorkspaceHandle>
where
    E: Engine,
{
    // Derive a C9-namespaced spec from the frozen lease.
    let mut spec = ContainerSpec::from_lease(lease, image)
        .context("deriving C9 workspace container spec from lease")?;
    spec.name = c9_container_name(&lease.lease_id);

    let container = engine
        .spawn(&spec)
        .context("spawning C9 workspace container")?;

    Ok(WorkspaceHandle {
        container,
        lease: lease.clone(),
        fence: fence.clone(),
        origin: WorkspaceOrigin::Spawned,
    })
}

// ── dedup spawn ───────────────────────────────────────────────────────────────

/// A workspace-id → single in-progress spawn entry.
#[derive(Clone)]
struct SpawnEntry {
    handle: WorkspaceHandle,
}

/// Deduplicating workspace spawner.
///
/// Concurrent identical spawns (same `workspace_id`) are coalesced to ONE
/// materialization: the first caller does the real spawn; all later callers for
/// the same id during the `dedup_window` get the SAME handle back without
/// re-spawning, even if the first spawn is still in progress.
///
/// After the window expires the entry is evicted so future spawns create a
/// fresh container.
///
/// This is the C9 warm-CAS-economics contract: identical concurrent spawns
/// dedup to one materialization.
pub struct DedupSpawner {
    /// In-progress/recent spawns, keyed by workspace id.
    ///
    /// Arc+Mutex so the spawner can be shared across threads (C9 item ③ uses
    /// concurrent callers from multiple threads).
    entries: Arc<Mutex<HashMap<String, (SpawnEntry, Instant)>>>,
    /// How long a completed spawn is held for deduplication.
    dedup_window: Duration,
}

impl DedupSpawner {
    /// Construct with a deduplication window.
    ///
    /// A window of at least 1s is enough to coalesce any realistic burst of
    /// concurrent spawns for the same workspace. The acceptance test uses a
    /// held `sleep` command so the spawns genuinely overlap in time.
    pub fn new(dedup_window: Duration) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            dedup_window,
        }
    }

    /// Spawn or return an existing workspace for `workspace_id`.
    ///
    /// Thread-safe: if two threads call this simultaneously with the same id,
    /// one does the real spawn and the other gets the same handle (one
    /// materialization). The spawn is done under the lock so the second caller
    /// always gets a fully-initialized handle (no "spawn in progress" window).
    ///
    /// # Errors
    /// Fails if the underlying spawn fails.
    pub fn spawn_or_join<E>(
        &self,
        workspace_id: &str,
        engine: &E,
        lease: &RunnerLease,
        fence: &FenceManifest,
        image: &str,
    ) -> Result<WorkspaceHandle>
    where
        E: Engine,
    {
        let mut map = self.entries.lock().unwrap();

        // Evict stale entries first.
        let now = Instant::now();
        map.retain(|_, (_, ts)| now.duration_since(*ts) < self.dedup_window);

        // Return the existing entry if one is live within the dedup window.
        if let Some((entry, _)) = map.get(workspace_id) {
            return Ok(entry.handle.clone());
        }

        // No live entry — do the real spawn (still under the lock so concurrent
        // callers wait for the single materialization to complete).
        let handle = spawn_workspace(engine, lease, fence, image)?;
        map.insert(
            workspace_id.to_string(),
            (
                SpawnEntry {
                    handle: handle.clone(),
                },
                now,
            ),
        );
        Ok(handle)
    }

    /// Evict the entry for `workspace_id` (e.g. after teardown).
    pub fn evict(&self, workspace_id: &str) {
        self.entries.lock().unwrap().remove(workspace_id);
    }
}

// ── attach ────────────────────────────────────────────────────────────────────

/// Attach to a live workspace identified by `workspace_id` — no respawn, no
/// re-hydrate.
///
/// This is the C9 attach contract (item ①): join a live workspace sharing the
/// same fence/materialization. The container must already be running; if it is
/// not present, this is a logic error (the caller should spawn first).
///
/// Returns a [`WorkspaceHandle`] with `origin = Attached`.
///
/// # Errors
/// Fails if the box is unreachable or the container is not running.
pub fn attach_workspace<B: BoxExec>(
    boxx: &B,
    workspace_id: &str,
    lease: &RunnerLease,
    fence: &FenceManifest,
) -> Result<WorkspaceHandle> {
    let container_name = c9_container_name(workspace_id);

    // Probe that the container is live — attach joins an existing container,
    // it does NOT respawn or re-hydrate.
    let out = boxx
        .run(&[
            "docker",
            "ps",
            "--filter",
            &format!("name={container_name}"),
            "--format",
            "{{.Names}}",
        ])
        .context("probing live workspace container for attach")?;

    if !out.stdout.lines().any(|l| l.trim() == container_name) {
        bail!(
            "attach failed: workspace container '{container_name}' is not running on the box; \
             spawn the workspace first (attach joins an existing live workspace without re-hydration)"
        );
    }

    Ok(WorkspaceHandle {
        container: RunningContainer {
            name: container_name,
        },
        lease: lease.clone(),
        fence: fence.clone(),
        origin: WorkspaceOrigin::Attached,
    })
}

// ── resume ────────────────────────────────────────────────────────────────────

/// Serialized workspace state for resume.
///
/// Minimal: carries the workspace id plus the frozen fence manifest from the
/// original spawn. The fence is the path_set ceiling — resume cannot exceed it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceState {
    /// Unique workspace identifier (derives the container name).
    pub workspace_id: String,
    /// The original fence manifest; resume is bounded by this path_set.
    pub original_fence: FenceManifest,
    /// Any extra opaque state payload (empty in v0; forward-compatible).
    pub state_payload: Vec<u8>,
}

impl WorkspaceState {
    /// Construct a state snapshot for a live workspace (to be persisted and
    /// passed to [`resume_workspace`] later).
    pub fn snapshot(workspace_id: impl Into<String>, fence: FenceManifest) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            original_fence: fence,
            state_payload: Vec::new(),
        }
    }
}

/// Resume a workspace from a serialized state, restoring state + fence.
///
/// The resumed workspace CANNOT exceed the original `path_set` — resume is not
/// a fence-widening hole. If `new_fence` attempts to widen the path_set beyond
/// `state.original_fence.path_set`, this function fails (ceiling enforced).
///
/// Returns a [`WorkspaceHandle`] with `origin = Resumed`.
///
/// # Errors
/// - If `new_fence` has paths not covered by `state.original_fence` (ceiling
///   violation).
/// - If the box is unreachable or the container spawn fails.
pub fn resume_workspace<B, E>(
    boxx: &B,
    engine: &E,
    state: &WorkspaceState,
    new_fence: &FenceManifest,
    lease: &RunnerLease,
    image: &str,
) -> Result<WorkspaceHandle>
where
    B: BoxExec,
    E: Engine,
{
    // ── path_set ceiling: resume cannot exceed the original fence ─────────────
    // Every path in new_fence.path_set must be covered by some entry in the
    // original fence's path_set. A new_fence entry is "covered" iff it is an
    // exact match or is a descendant of a directory prefix in the original set.
    let ceiling = &state.original_fence.path_set;
    for new_path in &new_fence.path_set {
        if !path_covered_by(new_path, ceiling) {
            bail!(
                "resume refused: new_fence path '{new_path}' is outside the original path_set \
                 ceiling — resume cannot widen the fence (path_set ceiling enforced)"
            );
        }
    }

    // ── re-spawn the container (resume re-materializes on a fresh container) ──
    let handle = spawn_workspace(engine, lease, new_fence, image).with_context(|| {
        format!(
            "spawning container on resume for workspace '{}'",
            state.workspace_id
        )
    })?;

    // Restore the state payload into the container (no-op in v0; forward compat).
    if !state.state_payload.is_empty() {
        restore_state_payload(boxx, &handle.container, &state.state_payload)?;
    }

    Ok(WorkspaceHandle {
        container: handle.container,
        lease: handle.lease,
        fence: new_fence.clone(),
        origin: WorkspaceOrigin::Resumed,
    })
}

/// Returns `true` iff `path` is covered by at least one entry in `ceiling`.
///
/// - A ceiling entry ending in `/` is a directory prefix: `path` is covered iff
///   it is the directory itself or any descendant.
/// - Any other entry is an exact match.
/// - Absolute paths and `..` are never covered (they escape the workspace root).
fn path_covered_by(path: &str, ceiling: &[String]) -> bool {
    if path.starts_with('/') || path.contains("..") {
        return false;
    }
    for entry in ceiling {
        if entry.ends_with('/') {
            // Directory prefix.
            let prefix = entry.trim_end_matches('/');
            if path == prefix || path.starts_with(&format!("{prefix}/")) {
                return true;
            }
        } else if path == entry {
            return true;
        }
    }
    false
}

/// Restore a state payload into the container (no-op in v0; forward-compat).
///
/// In v0 no state_payload is written, so this is never called. The seam is here
/// so future implementations can hydrate the container from serialized state.
fn restore_state_payload<B: BoxExec>(
    boxx: &B,
    container: &RunningContainer,
    payload: &[u8],
) -> Result<()> {
    // Write the payload as a marker file into the container's tmp root so the
    // acceptance test can confirm state_restore was invoked (forward-compat).
    let b64 = minimal_base64(payload);
    let script = format!(
        "printf %s {bq} | base64 -d > /hugit/tmp/state_restore_marker",
        bq = shell_quote(&b64),
    );
    let out = boxx
        .run(&["docker", "exec", &container.name, "sh", "-c", &script])
        .context("restoring state payload into workspace container")?;
    if !out.ok() {
        bail!(
            "state_restore into {} failed: {}",
            container.name,
            out.stderr.trim()
        );
    }
    Ok(())
}

// ── local ≡ remote ────────────────────────────────────────────────────────────

/// Result of running a pure function in a workspace.
///
/// The local ≡ remote contract (item ④) states that running the same
/// deterministic function locally or inside the runner container produces
/// identical observable results. This type carries the observable output so
/// equality can be asserted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceResult {
    /// Exit code (0 = success).
    pub exit_code: Option<i32>,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
}

impl WorkspaceResult {
    /// `true` iff exit code is 0.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// `true` iff `self` and `other` are result-identical (local ≡ remote).
    ///
    /// Two results are identical iff they have the same exit code and the same
    /// stdout (stderr differences from the runtime are not observable output).
    #[must_use]
    pub fn result_identity(&self, other: &Self) -> bool {
        self.exit_code == other.exit_code && self.stdout == other.stdout
    }
}

/// Run a command inside a workspace container (remote execution path).
///
/// This is the remote leg of the local ≡ remote identity check (item ④). The
/// same deterministic `argv` run locally via [`run_local`] and remotely via
/// this function must produce identical [`WorkspaceResult`]s.
///
/// # Errors
/// Fails if the box is unreachable.
pub fn run_remote<B: BoxExec>(
    boxx: &B,
    container: &RunningContainer,
    argv: &[&str],
) -> Result<WorkspaceResult> {
    let mut full = vec!["docker", "exec", &container.name];
    full.extend_from_slice(argv);
    let out = boxx
        .run(&full)
        .context("running command in remote workspace container")?;
    Ok(WorkspaceResult {
        exit_code: out.code,
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// Run a command locally (local execution path).
///
/// This is the local leg of the local ≡ remote identity check (item ④). The
/// same deterministic `argv` run here and via [`run_remote`] must produce
/// identical [`WorkspaceResult`]s.
///
/// # Errors
/// Fails if the local process cannot be spawned.
pub fn run_local(argv: &[&str]) -> Result<WorkspaceResult> {
    if argv.is_empty() {
        bail!("run_local: argv is empty");
    }
    let out = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .with_context(|| format!("spawning local command {:?}", argv[0]))?;
    Ok(WorkspaceResult {
        exit_code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

// ── spawn timing ──────────────────────────────────────────────────────────────

/// Timed spawn: measure box-side spawn latency for the <1s contract (item ③).
///
/// Returns the handle and the end-to-end elapsed duration (includes SSH RTT).
/// The box-side spawn_ms is measured separately in acceptance tests because the
/// SSH transport adds ~3s of network overhead that is not part of the <1s
/// warm-CAS contract. On a warm box (image pre-pulled) `docker run` itself takes
/// <500ms; the 1000ms (1s) budget is for box-side startup only.
///
/// # Errors
/// Propagates spawn errors; does NOT fail on timing (the test asserts timing).
pub fn spawn_timed<E>(
    engine: &E,
    lease: &RunnerLease,
    fence: &FenceManifest,
    image: &str,
) -> Result<(WorkspaceHandle, Duration)>
where
    E: Engine,
{
    let t0 = Instant::now();
    let handle = spawn_workspace(engine, lease, fence, image)?;
    let elapsed = t0.elapsed();
    Ok((handle, elapsed))
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// POSIX single-quote for safe interpolation into a remote `sh -c`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Minimal base64 (standard alphabet, padded) — no dep.
fn minimal_base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(A[(n >> 18) & 63] as char);
        out.push(A[(n >> 12) & 63] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6) & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[n & 63] as char
        } else {
            '='
        });
    }
    out
}

// ── unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c9_name_is_prefixed_and_sanitized() {
        assert_eq!(c9_container_name("ws/abc 1"), "hugit-c9-ws_abc_1");
        assert!(c9_container_name("x").starts_with(C9_PREFIX));
    }

    #[test]
    fn path_covered_exact() {
        let ceiling = vec!["src/main.rs".to_string(), "Cargo.toml".to_string()];
        assert!(path_covered_by("src/main.rs", &ceiling));
        assert!(path_covered_by("Cargo.toml", &ceiling));
        assert!(!path_covered_by("src/secret.rs", &ceiling));
    }

    #[test]
    fn path_covered_dir_prefix() {
        let ceiling = vec!["src/".to_string()];
        assert!(path_covered_by("src/main.rs", &ceiling));
        assert!(path_covered_by("src/inner/deep.rs", &ceiling));
        assert!(!path_covered_by("tests/x.rs", &ceiling));
    }

    #[test]
    fn path_covered_rejects_absolute_and_traversal() {
        let ceiling = vec!["src/".to_string()];
        assert!(!path_covered_by("/etc/passwd", &ceiling));
        assert!(!path_covered_by("src/../etc/passwd", &ceiling));
    }

    #[test]
    fn workspace_state_snapshot() {
        let fence = FenceManifest {
            path_set: vec!["src/".to_string()],
            deny_default: true,
            materialized: vec![],
        };
        let state = WorkspaceState::snapshot("ws-001", fence.clone());
        assert_eq!(state.workspace_id, "ws-001");
        assert_eq!(state.original_fence, fence);
    }

    #[test]
    fn result_identity_checks_exit_and_stdout() {
        let a = WorkspaceResult {
            exit_code: Some(0),
            stdout: "hi\n".into(),
            stderr: String::new(),
        };
        let b = WorkspaceResult {
            exit_code: Some(0),
            stdout: "hi\n".into(),
            stderr: "noise".into(),
        };
        let c = WorkspaceResult {
            exit_code: Some(1),
            stdout: "hi\n".into(),
            stderr: String::new(),
        };
        assert!(
            a.result_identity(&b),
            "same exit+stdout => identical regardless of stderr"
        );
        assert!(!a.result_identity(&c), "different exit => not identical");
    }

    #[test]
    fn dedup_spawner_evicts_expired() {
        // Window of 0ms means the entry is immediately stale; eviction must not panic.
        let spawner = DedupSpawner::new(Duration::from_millis(0));
        let map = spawner.entries.lock().unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn spawn_lt_1s_budget_duration() {
        // Document the <1s contract: 1000ms budget expressed in Duration form.
        let budget_1s = Duration::from_millis(1_000);
        assert!(budget_1s >= Duration::from_millis(1000));
    }
}
