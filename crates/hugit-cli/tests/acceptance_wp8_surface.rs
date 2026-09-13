//! WP-8: public onboarding uses setup/attach, normal Git, then health.

use std::path::PathBuf;
use std::process::Command;

const CAPTURE_HELP: &str = include_str!("fixtures/wp8-capture-help.golden");
const README: &str = include_str!("../../../README.md");
const INSTALL: &str = include_str!("../../../docs/installation.md");
const QUICKSTART: &str = include_str!("../../../docs/quickstart-hooks.md");
const LOCAL_QUICKSTART: &str = include_str!("../../../docs/quickstart-local.md");
const GO_LIVE: &str = include_str!("../../../docs/quickstart-golive.md");

#[test]
fn capture_help_is_internal_hook_only() {
    let out = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .arg("--help")
        .output()
        .expect("hugit --help runs");
    assert!(out.status.success(), "top-level help exits 0");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(CAPTURE_HELP.trim()),
        "capture help matches WP-8 golden: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn onboarding_docs_keep_wp8_truth_labels_and_runtime_path() {
    for doc in [README, INSTALL, QUICKSTART, LOCAL_QUICKSTART, GO_LIVE] {
        for label in [
            "observed locally",
            "attempted push",
            "explicit declaration",
            "unsupported",
        ] {
            assert!(doc.contains(label), "missing strength label {label:?}");
        }
        assert!(
            doc.contains("<git-common-dir>/hugit/event-log.json"),
            "missing canonical runtime path"
        );
        assert!(
            !doc.contains(".hugit/log.json"),
            "stale tracked-worktree log path"
        );
        for step in [
            "hugit setup",
            "hugit attach",
            "git commit",
            "git checkout",
            "git push",
            "hugit health",
        ] {
            assert!(doc.contains(step), "missing normal-workflow step {step:?}");
        }
    }
    assert!(
        README.contains("partial coverage"),
        "README preserves partial coverage"
    );
    assert!(
        INSTALL.contains("partial coverage"),
        "install preserves partial coverage"
    );
}

#[test]
fn clean_machine_journey_uses_setup_git_then_health() {
    let root = std::env::temp_dir().join(format!("hugit-wp8-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let xdg = root.join("xdg");
    let repo = root.join("repo");
    let remote = root.join("remote.git");
    std::fs::create_dir_all(&home).expect("create isolated home");
    std::fs::create_dir_all(&xdg).expect("create isolated XDG");

    let git_config = home.join(".gitconfig");
    let run = |cmd: &mut Command, step: &str| {
        let out = cmd
            .output()
            .unwrap_or_else(|error| panic!("{step} starts: {error}"));
        assert!(
            out.status.success(),
            "{step} exits 0: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run(
        Command::new(env!("CARGO_BIN_EXE_hugit"))
            .arg("setup")
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &xdg)
            .env("GIT_CONFIG_GLOBAL", &git_config)
            .env("GIT_CONFIG_NOSYSTEM", "1"),
        "hugit setup",
    );
    run(
        Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .arg(&repo)
            .env("HOME", &home)
            .env("GIT_CONFIG_GLOBAL", &git_config)
            .env("GIT_CONFIG_NOSYSTEM", "1"),
        "git init",
    );
    for (key, value) in [("user.name", "WP8"), ("user.email", "wp8@example.test")] {
        run(
            Command::new("git")
                .args(["config", key, value])
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", &git_config)
                .env("GIT_CONFIG_NOSYSTEM", "1"),
            "git config",
        );
    }
    std::fs::write(repo.join("README.md"), "# WP-8\n").expect("write commit input");
    run(
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", &git_config)
            .env("GIT_CONFIG_NOSYSTEM", "1"),
        "git add",
    );
    run(
        Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", &git_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HUGIT_BIN", PathBuf::from(env!("CARGO_BIN_EXE_hugit"))),
        "git commit",
    );
    run(
        Command::new("git")
            .args(["checkout", "-q", "-b", "feature/example"])
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", &git_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HUGIT_BIN", PathBuf::from(env!("CARGO_BIN_EXE_hugit"))),
        "git checkout",
    );
    run(
        Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&remote),
        "git init --bare",
    );
    run(
        Command::new("git")
            .args(["remote", "add", "origin"])
            .arg(&remote)
            .current_dir(&repo),
        "git remote add",
    );
    run(
        Command::new("git")
            .args(["push", "-q", "-u", "origin", "feature/example"])
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", &git_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HUGIT_BIN", PathBuf::from(env!("CARGO_BIN_EXE_hugit"))),
        "git push",
    );
    run(
        Command::new(env!("CARGO_BIN_EXE_hugit"))
            .arg("health")
            .current_dir(&repo),
        "hugit health",
    );

    let _ = std::fs::remove_dir_all(root);
}
