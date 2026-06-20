//! Acceptance — `hugit symbol` (W6 local symbol outline).
//!
//! Drives the REAL binary over a fixture source file. Pins: the outline is the
//! `hugit-symbols` projection (kind/name/line), an unsupported extension is an
//! honest empty outline (not an error), a missing file obeys the WB0
//! one-error/one-exit law (exit 2), and a secret-shaped symbol name is scrubbed.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-symbol-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

#[test]
fn symbol_outlines_a_rust_file_with_kind_name_line() {
    let dir = scratch("rust");
    let file = dir.join("m.rs");
    std::fs::write(
        &file,
        "pub struct Point;\n\npub fn area() -> u32 {\n    0\n}\n",
    )
    .unwrap();

    let (code, v) = run(&["symbol", "--file", file.to_str().unwrap()]);
    assert_eq!(code, 0, "symbol exits 0 on a readable file: {v}");
    assert_eq!(v["lang"], "rust");

    let outline = v["outline"].as_array().expect("outline array");
    let rows: Vec<(&str, &str, u64)> = outline
        .iter()
        .map(|o| {
            (
                o["kind"].as_str().unwrap(),
                o["name"].as_str().unwrap(),
                o["line"].as_u64().unwrap(),
            )
        })
        .collect();
    assert!(rows.contains(&("struct", "Point", 1)), "rows: {rows:?}");
    assert!(rows.contains(&("fn", "area", 3)), "rows: {rows:?}");
}

#[test]
fn symbol_unsupported_extension_is_honest_empty_not_an_error() {
    let dir = scratch("txt");
    let file = dir.join("notes.txt");
    std::fs::write(&file, "just some prose, no language").unwrap();

    let (code, v) = run(&["symbol", "--file", file.to_str().unwrap()]);
    assert_eq!(code, 0, "unsupported lang is not an error: {v}");
    assert_eq!(v["lang"], Value::Null);
    assert!(
        v["outline"].as_array().unwrap().is_empty(),
        "unsupported lang → empty outline: {v}"
    );
}

#[test]
fn symbol_missing_file_obeys_the_error_law() {
    let (code, v) = run(&["symbol", "--file", "/no/such/source/file.rs"]);
    assert_eq!(code, 2, "missing file is a structured error, exit 2: {v}");
    assert_eq!(v["error"]["kind"], "file_not_found");
    // The error envelope carries the remediation key `fix` (never `suggested_fix`).
    assert!(v["error"]["fix"].is_string(), "error carries a fix: {v}");
}

#[test]
fn symbol_name_is_scrubbed_at_the_read_boundary() {
    // A secret-shaped identifier name must be redacted in the outline — the
    // outline must not be a redaction bypass.
    let dir = scratch("secret");
    let file = dir.join("s.rs");
    std::fs::write(
        &file,
        "const gho_16C7e42F292c6912E7710c838347Ae178B4a: u32 = 1;\n",
    )
    .unwrap();

    let (code, v) = run(&["symbol", "--file", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    let raw = serde_json::to_string(&v).unwrap();
    assert!(
        !raw.contains("gho_16C7e42F292c6912E7710c838347Ae178B4a"),
        "a secret-shaped symbol name must be scrubbed in the outline: {raw}"
    );
}
