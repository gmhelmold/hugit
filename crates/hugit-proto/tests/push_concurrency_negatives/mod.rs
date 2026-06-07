//! Shared fixtures + the D3 **write-path red-team** for the WP-D3b acceptance
//! suite (push concurrency / total order + external-change + flag/negatives).
//!
//! This module is included by `tests/acceptance_d3b.rs` via `#[path]`. It holds:
//!
//! - deterministic git-object-id fixtures (real-shaped 40-hex oids),
//! - a barrier-synchronized concurrent-push driver that forces **real overlap**
//!   (all writer threads are held at a barrier and released together, so the
//!   single-writer point is genuinely contended — not instant/serial), and
//! - the red-team assertions: stale-tip forgery, attribution spoofing, and
//!   intent-fabrication attempts, all of which the write path must defeat.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use hugit_proto::write::receive::{Receipt, ReceiveError, ReceiveRequest, SerializedReceiver};
use hugit_proto::write::store::InMemoryCas;
use hugit_proto::{PushOutcome, RefUpdate, SerializedWriter};

/// A deterministic, real-shaped 40-hex git object id from a short seed.
///
/// Not a real reachable object — the D3b write path treats targets as opaque
/// object ids (item ③: externals are opaque), so a well-formed oid suffices to
/// exercise total order / stale rejection without standing up a CAS.
pub fn oid(seed: &str) -> String {
    let mut s = String::new();
    for b in seed.bytes() {
        s.push_str(&format!("{:02x}", b));
    }
    while s.len() < 40 {
        s.push('0');
    }
    s.truncate(40);
    s
}

/// A compare-and-append create (`expected = None`) of `ref_name → target` by
/// `who`, recorded at `at`.
pub fn create(ref_name: &str, target: &str, who: &str, at: u64) -> RefUpdate {
    RefUpdate {
        ref_name: ref_name.to_string(),
        expected: None,
        target: target.to_string(),
        principal_chain: vec![who.to_string()],
        recorded_at: at,
    }
}

/// A compare-and-append move of `ref_name` from `from` to `to` by `who`.
pub fn advance(ref_name: &str, from: &str, to: &str, who: &str, at: u64) -> RefUpdate {
    RefUpdate {
        ref_name: ref_name.to_string(),
        expected: Some(from.to_string()),
        target: to.to_string(),
        principal_chain: vec![who.to_string()],
        recorded_at: at,
    }
}

/// Drive `n` pushes through one [`SerializedWriter`] from `n` real threads held
/// at a [`Barrier`] so they are released simultaneously — genuine overlap on the
/// single-writer point, not instant serial calls. Returns the per-thread outcomes
/// in thread-spawn order alongside the writer for post-hoc assertions.
pub fn drive_overlapping(
    writer: Arc<SerializedWriter>,
    updates: Vec<RefUpdate>,
) -> Vec<PushOutcome> {
    let n = updates.len();
    let barrier = Arc::new(Barrier::new(n));
    let mut handles = Vec::with_capacity(n);

    for update in updates {
        let w = Arc::clone(&writer);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            // Hold every thread here until all are ready, then race the writer.
            b.wait();
            w.push(update)
        }));
    }

    handles
        .into_iter()
        .map(|h| h.join().expect("push thread panicked"))
        .collect()
}

// ─── DEFECT-2/3: concurrency over the REAL receive-pack ingest ────────────────
//
// The order-module `SerializedWriter` above proves total order *in isolation*.
// The defect was that the real ingest (`receive_pack`) never routed through
// compare-and-append, so two concurrent REAL pushes lost-update. These helpers
// drive `SerializedReceiver` — the single-writer point the real ingest uses —
// from genuinely overlapping threads with REAL packs.

/// A built real packfile + the head oid it delivers, for a one-commit repo whose
/// single file carries `seed` (so distinct seeds ⇒ distinct head oids).
pub struct RealPack {
    /// The packfile bytes a real `git push` would deliver.
    pub pack: Vec<u8>,
    /// The head commit oid the pack delivers.
    pub head_oid: String,
}

/// Build a real one-commit pack whose content is keyed by `seed`.
pub fn real_pack(seed: &str) -> RealPack {
    let src = ScratchDir::new("hugit-recv-fx");
    let p = src.path();
    git(p, &["init", "-q", "-b", "main", "."]);
    git(p, &["config", "user.email", "test@hugit.dev"]);
    git(p, &["config", "user.name", "hugit-test"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    std::fs::write(p.join("f"), format!("content {seed}\n")).expect("write fixture file");
    git(p, &["add", "f"]);
    let env_date = "2026-06-05T00:00:00 +0000";
    git_env(
        p,
        &["commit", "-q", "-m", "fixture commit"],
        &[
            ("GIT_AUTHOR_DATE", env_date),
            ("GIT_COMMITTER_DATE", env_date),
        ],
    );
    let head_oid = git(p, &["rev-parse", "HEAD"]).trim().to_string();
    let pack = git_stdin(
        p,
        &["pack-objects", "--revs", "--stdout"],
        head_oid.as_bytes(),
    );
    RealPack { pack, head_oid }
}

/// Build a real follow-on pack that advances `main` from `base_seed` to a new
/// commit keyed by `seed` (the parent is the `base_seed` commit). Returns the
/// pack plus the new head oid. Both racers in the stale test share the same base
/// commit so they genuinely contend on the one ref from the same expected tip.
pub fn real_advance_pack(base_seed: &str, seed: &str) -> RealPack {
    let src = ScratchDir::new("hugit-recv-adv");
    let p = src.path();
    git(p, &["init", "-q", "-b", "main", "."]);
    git(p, &["config", "user.email", "test@hugit.dev"]);
    git(p, &["config", "user.name", "hugit-test"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    let env_date = "2026-06-05T00:00:00 +0000";
    std::fs::write(p.join("f"), format!("content {base_seed}\n")).expect("write base file");
    git(p, &["add", "f"]);
    git_env(
        p,
        &["commit", "-q", "-m", "base commit"],
        &[
            ("GIT_AUTHOR_DATE", env_date),
            ("GIT_COMMITTER_DATE", env_date),
        ],
    );
    std::fs::write(p.join("g"), format!("advance {seed}\n")).expect("write advance file");
    git(p, &["add", "g"]);
    git_env(
        p,
        &["commit", "-q", "-m", "advance commit"],
        &[
            ("GIT_AUTHOR_DATE", env_date),
            ("GIT_COMMITTER_DATE", env_date),
        ],
    );
    let head_oid = git(p, &["rev-parse", "HEAD"]).trim().to_string();
    // Pack the whole reachable closure so the follow-on push is self-contained.
    let pack = git_stdin(
        p,
        &["pack-objects", "--revs", "--stdout"],
        head_oid.as_bytes(),
    );
    RealPack { pack, head_oid }
}

/// Drive `requests` through one [`SerializedReceiver`] from real threads held at a
/// [`Barrier`] — genuine overlap on the single-writer ingest point. Returns the
/// per-thread ingest results in spawn order.
pub fn drive_receivers(
    receiver: Arc<SerializedReceiver<InMemoryCas>>,
    requests: Vec<ReceiveRequest>,
) -> Vec<Result<Receipt, ReceiveError>> {
    let n = requests.len();
    let barrier = Arc::new(Barrier::new(n));
    let mut handles = Vec::with_capacity(n);
    for req in requests {
        let r = Arc::clone(&receiver);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            r.receive(&req)
        }));
    }
    handles
        .into_iter()
        .map(|h| h.join().expect("receive thread panicked"))
        .collect()
}

// ── minimal git plumbing (self-contained; no extra crate dep) ────────────────

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn git_env(cwd: &Path, args: &[&str], env: &[(&str, &str)]) {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_stdin(cwd: &Path, args: &[&str], stdin: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    child
        .stdin
        .take()
        .expect("git stdin")
        .write_all(stdin)
        .expect("write git stdin");
    let out = child.wait_with_output().expect("git wait");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// A unique self-cleaning scratch directory (no extra crate dependency).
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(prefix: &str) -> Self {
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{n}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create scratch dir");
        ScratchDir { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
