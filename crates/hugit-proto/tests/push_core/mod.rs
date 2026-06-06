//! WP-D3a owned acceptance fixtures + the D3 red-team write-path helpers.
//!
//! Builds real git repositories and packfiles with the system git binary (the
//! libgit2-class engine the write path itself wires to), so the round-trip is
//! exercised against genuine git artifacts, not hand-rolled bytes.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A built push fixture: a packfile carrying a tiny repo's objects plus the ref
/// the push wants set and the head commit's byte-exact object.
pub struct Fixture {
    /// The packfile bytes (as a real `git push` would deliver).
    pub pack: Vec<u8>,
    /// The full ref name the push targets, e.g. `refs/heads/main`.
    pub ref_name: String,
    /// The head commit oid.
    pub head_oid: String,
    /// The raw `git cat-file commit` bytes of the head — the byte-identity
    /// oracle for the clone-back.
    pub head_commit_bytes: Vec<u8>,
    // keeps the source repo dir alive for the fixture's lifetime.
    _src: ScratchDir,
}

impl Fixture {
    /// Build a one-commit repo containing `files`, then pack all reachable
    /// objects into a single packfile.
    pub fn build_repo(files: &[(&str, &str)]) -> Self {
        let src = ScratchDir::new("hugit-fx-src");
        let p = src.path();
        git(p, &["init", "-q", "-b", "main", "."]);
        git(p, &["config", "user.email", "test@hugit.dev"]);
        git(p, &["config", "user.name", "hugit-test"]);
        git(p, &["config", "commit.gpgsign", "false"]);
        for (name, body) in files {
            std::fs::write(p.join(name), body).expect("write fixture file");
            git(p, &["add", name]);
        }
        // deterministic author/committer dates → stable oid across machines.
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
        let head_commit_bytes = git_bytes(p, &["cat-file", "commit", &head_oid]);

        // pack every reachable object from HEAD into one pack on stdout.
        let pack = git_bytes_stdin(
            p,
            &["pack-objects", "--revs", "--stdout"],
            format!("{head_oid}\n").as_bytes(),
        );

        Fixture {
            pack,
            ref_name: "refs/heads/main".to_string(),
            head_oid,
            head_commit_bytes,
            _src: src,
        }
    }
}

/// The head of a clone: its oid and its raw commit object bytes.
pub struct ClonedHead {
    pub oid: String,
    pub commit_bytes: Vec<u8>,
}

/// Clone `source` (a bare repo materialized from CAS) and read back its head.
pub fn clone_and_head(source: &Path) -> ClonedHead {
    let dst = ScratchDir::new("hugit-fx-clone");
    // clone into a subdir of the scratch dir.
    let target = dst.path().join("work");
    let status = Command::new("git")
        .arg("clone")
        .arg("-q")
        .arg(source)
        .arg(&target)
        .status()
        .expect("spawn git clone");
    assert!(status.success(), "git clone of materialized repo failed");
    let oid = git(&target, &["rev-parse", "HEAD"]).trim().to_string();
    let commit_bytes = git_bytes(&target, &["cat-file", "commit", &oid]);
    // keep dst alive until after we read.
    std::mem::forget(dst);
    ClonedHead { oid, commit_bytes }
}

fn git(cwd: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(cwd, args)).expect("git stdout utf8")
}

fn git_bytes(cwd: &Path, args: &[&str]) -> Vec<u8> {
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
    out.stdout
}

fn git_env(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn git_bytes_stdin(cwd: &Path, args: &[&str], stdin: &[u8]) -> Vec<u8> {
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
