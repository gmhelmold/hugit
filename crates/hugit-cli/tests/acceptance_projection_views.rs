use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::{Value, json};

fn scratch() -> PathBuf {
    let path = std::env::temp_dir().join(format!("hugit-projection-views-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn log(path: &Path) -> EventLog {
    let mut log = EventLog::new();
    log.append_for_test(
        "ref.update",
        vec!["orchestrator:hugit-hook".into()],
        json!({
            "receipt_id": "receipt-1",
            "ref": "refs/heads/main",
            "target": "abc123",
            "branch": "main",
        })
        .to_string(),
        1,
    );
    std::fs::write(path, serde_json::to_vec(log.records()).unwrap()).unwrap();
    log
}

fn run(root: &Path, args: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}

#[test]
fn fleet_renders_only_source_verified_projection_facts() {
    let root = scratch();
    let log_path = root.join("log.json");
    log(&log_path);

    let log_arg = log_path.to_str().unwrap();
    let (code, value) = run(&root, &["fleet", "--log", log_arg]);
    assert_eq!(code, 0, "{value}");
    assert_eq!(value["projection"]["state"], "complete");
    assert_eq!(value["projection"]["applied"].as_array().unwrap().len(), 3);
    assert!(
        value["projection"]["applied"]
            .as_array()
            .unwrap()
            .iter()
            .all(|applied| applied["source"]["receipt_id"] == "receipt-1")
    );
    let status: Value = serde_json::from_slice(
        &std::fs::read(root.join(hugit_cli::runtime_store::STATUS)).unwrap(),
    )
    .unwrap();
    assert_eq!(status["projection"]["summary"]["completed"], 3);
    assert_eq!(
        status["projection"]["completed"].as_array().unwrap().len(),
        3
    );

    std::fs::write(&log_path, b"[]").unwrap();
    let (code, value) = run(&root, &["watch", "--log", log_arg]);
    assert_eq!(code, 2, "{value}");
    assert_eq!(value["error"]["kind"], "projection_invalid");
}
