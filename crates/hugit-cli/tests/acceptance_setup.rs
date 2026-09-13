//! `hugit setup --repo` attaches capture hooks to an existing repository.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-setup-repo-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git_in(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", cwd.join(".hugit-test-gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn setup_in(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args(["setup", "--repo", root.to_str().unwrap()])
        .env("GIT_CONFIG_GLOBAL", root.join(".hugit-test-gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("hugit setup --repo runs")
}

fn wait_for_capture(log: &Path, timeout_ms: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if std::fs::read(log)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).ok())
            .is_some_and(|records| records.iter().any(|record| record["kind"] == "ref.update"))
        {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

#[test]
fn setup_repo_installs_hooks_preserves_git_data_and_reports_conflicts() {
    let root = scratch("install");
    git_in(&root, &["init", "-q", "-b", "main"]);
    git_in(&root, &["config", "user.name", "hugit-test"]);
    git_in(&root, &["config", "user.email", "hugit-test@example.com"]);
    std::fs::write(root.join("seed.txt"), "seed\n").unwrap();
    git_in(&root, &["add", "seed.txt"]);
    git_in(&root, &["commit", "-q", "-m", "seed", "--no-gpg-sign"]);
    let original_head = git_in(&root, &["rev-parse", "HEAD"]);
    let original_count = git_in(&root, &["rev-list", "--count", "HEAD"]);

    let conflict = root.join(".git/hooks/post-merge");
    let conflict_bytes = b"#!/bin/sh\necho user-hook\n";
    std::fs::write(&conflict, conflict_bytes).unwrap();

    let out = setup_in(&root);
    assert!(out.status.success(), "setup exits 0");
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        value["hooks_installed"],
        serde_json::json!(["post-commit", "post-checkout", "pre-push"])
    );
    assert_eq!(value["hooks_noop"], serde_json::json!([]));
    assert_eq!(value["hooks_conflict"], serde_json::json!(["post-merge"]));
    assert_eq!(std::fs::read(&conflict).unwrap(), conflict_bytes);
    for kind in ["post-commit", "post-checkout", "pre-push"] {
        assert!(
            root.join(".git/hooks").join(kind).is_file(),
            "{kind} installed"
        );
    }
    assert_eq!(git_in(&root, &["rev-parse", "HEAD"]), original_head);
    assert_eq!(
        git_in(&root, &["rev-list", "--count", "HEAD"]),
        original_count
    );

    std::fs::write(root.join("first.txt"), "first\n").unwrap();
    git_in(&root, &["add", "first.txt"]);
    let commit = Command::new("git")
        .args(["commit", "-q", "-m", "first", "--no-gpg-sign"])
        .current_dir(&root)
        .env("HUGIT_BIN", env!("CARGO_BIN_EXE_hugit"))
        .status()
        .expect("git commit runs");
    assert!(commit.success(), "installed hook never blocks commit");
    assert!(wait_for_capture(
        &root.join(".git/hugit/event-log.json"),
        20_000
    ));

    let rerun = setup_in(&root);
    let value: Value = serde_json::from_slice(&rerun.stdout).unwrap();
    assert_eq!(value["hooks_installed"], serde_json::json!([]));
    assert_eq!(
        value["hooks_noop"],
        serde_json::json!(["post-commit", "post-checkout", "pre-push"])
    );
    assert_eq!(value["hooks_conflict"], serde_json::json!(["post-merge"]));
    assert_eq!(std::fs::read(&conflict).unwrap(), conflict_bytes);
}

#[test]
fn setup_repo_refuses_custom_hooks_path() {
    let root = scratch("custom-hooks-path");
    git_in(&root, &["init", "-q", "-b", "main"]);
    git_in(&root, &["config", "core.hooksPath", "custom-hooks"]);

    let out = setup_in(&root);

    assert!(
        !out.status.success(),
        "setup refuses inactive hook destination"
    );
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["error"]["kind"], "custom_hooks_path");
    assert!(!root.join(".git/hooks/post-commit").exists());
}

#[cfg(unix)]
#[test]
fn setup_repo_refuses_dangling_hook_symlink() {
    use std::os::unix::fs::symlink;

    let root = scratch("symlink");
    git_in(&root, &["init", "-q", "-b", "main"]);
    let outside = root.join("outside-target");
    symlink(&outside, root.join(".git/hooks/post-commit")).unwrap();

    let out = setup_in(&root);

    assert!(!out.status.success(), "setup refuses symlink hook path");
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["error"]["kind"], "unsafe_hook_path");
    assert!(
        !outside.exists(),
        "setup must not follow dangling hook symlink"
    );
}
