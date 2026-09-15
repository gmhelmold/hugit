//! Safe foreign-hook adoption: preview first, hash-bound mutation, reversible only when unchanged.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn scratch(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("hugit-attach-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn git(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}
fn hugit(root: &Path, args: &[&str]) -> (i32, Value) {
    let o = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    (
        o.status.code().unwrap_or(-1),
        serde_json::from_slice(&o.stdout).unwrap(),
    )
}
fn repo(tag: &str) -> PathBuf {
    let root = scratch(tag);
    assert!(git(&root, &["init", "-q"]).status.success());
    root
}

#[cfg(unix)]
#[test]
fn preview_is_read_only_and_adoption_preserves_foreign_exit() {
    let root = repo("adopt");
    let hook = root.join(".git/hooks/pre-push");
    let foreign = b"#!/bin/sh\necho foreign-ran >&2\nexit 17\n";
    std::fs::write(&hook, foreign).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (_, preview) = hugit(&root, &["attach", "--preview"]);
    let token = preview["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "pre-push")
        .unwrap()["preview_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        std::fs::read(&hook).unwrap(),
        foreign,
        "preview writes nothing"
    );
    let (_, attached) = hugit(&root, &["attach", "--adopt-managed-dispatcher", &token]);
    assert_eq!(attached["attached"], true);
    let out = Command::new(&hook).current_dir(&root).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(17),
        "foreign hook status remains Git status"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("foreign-ran"));
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(attached["manifest"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(manifest["version"], 1);
    assert_eq!(hugit(&root, &["attach"]).0, 0, "owned dispatcher upgrades");
    assert_eq!(hugit(&root, &["attach", "--detach"]).0, 0);
    assert_eq!(
        std::fs::read(&hook).unwrap(),
        foreign,
        "upgrade retains adopted backup"
    );
}

#[test]
fn detach_restores_only_manifest_matching_dispatcher() {
    let root = repo("detach");
    let hook = root.join(".git/hooks/post-commit");
    let foreign = b"#!/bin/sh\necho original\n";
    std::fs::write(&hook, foreign).unwrap();
    let (_, preview) = hugit(&root, &["attach", "--preview"]);
    let token = preview["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "post-commit")
        .unwrap()["preview_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        hugit(&root, &["attach", "--adopt-managed-dispatcher", &token]).0,
        0
    );
    std::fs::write(&hook, b"#!/bin/sh\necho user changed\n").unwrap();
    let (_, detached) = hugit(&root, &["attach", "--detach"]);
    assert!(
        detached["preserved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "post-commit")
    );
    assert!(
        std::fs::read_to_string(&hook)
            .unwrap()
            .contains("user changed")
    );
}

#[test]
fn attach_uses_active_core_hooks_path() {
    let root = repo("hooks-path");
    assert!(
        git(&root, &["config", "core.hooksPath", "custom-hooks"])
            .status
            .success()
    );
    let (_, preview) = hugit(&root, &["attach", "--preview"]);
    assert!(
        preview["hooks_path"]
            .as_str()
            .unwrap()
            .ends_with("custom-hooks")
    );
    assert_eq!(
        preview["changes"].as_array().unwrap().len(),
        hugit_cli::init::HOOK_KINDS.len()
    );
}
