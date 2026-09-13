//! Acceptance — `hugit ledger` (Phase-D forge history view, graduated from
//! RESERVED with REAL wiring).
//!
//! Drives the REAL `hugit` binary (`CARGO_BIN_EXE_hugit`) over a REAL chained
//! event log built through the engine's own `EventLog::append_for_test` — the
//! same primitive every porcelain verb writes through, so the hash chain is
//! real, never forged. What is pinned:
//!   - the per-campaign rollup (`asked` / `done` / `proven` / `rejected`) matches
//!     the shared `hugit_ledger::Ledger` fold — so the ledger view AGREES with
//!     `campaign show` / `queue show` by construction (one projection);
//!   - one entry per `intent.landed`, `proven`/`rejected` driven by the
//!     `verdict.recorded` events (approve ⇒ proven, reject ⇒ rejected);
//!   - `--campaign` scopes the projection to one campaign;
//!   - honest-null on a log with no `intent.landed` (entries `[]`, `note`
//!     present) — never a fabricated history;
//!   - the WB0 one-error/one-exit law (missing log ⇒ exit 2, `{"error":…}`).

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::{Value, json};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-ledger-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>` and return `(exit_code, parsed_stdout_json)`.
fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

/// Write a `[EventRecord, …]` log to `path`, appending each `(kind, payload)`
/// through the engine's canonical `EventLog::append_for_test` (real hash chain).
fn write_log(path: &Path, events: &[(&str, Value)]) {
    let mut log = EventLog::new();
    for (kind, payload) in events {
        log.append_for_test(*kind, vec![], payload.to_string(), 0);
    }
    let json = serde_json::to_string_pretty(log.records()).unwrap();
    std::fs::write(path, json).unwrap();
}

/// A canonical `verdict.recorded` payload (a `VerdictObject`), as `hugit verdict
/// --store` writes for a single-lens panel.
fn vo(intent: &str, verdict: &str) -> Value {
    json!({
        "intent": intent,
        "tree_hash": "",
        "lens": "panel",
        "model": "porcelain",
        "prompt_digest": "",
        "verdict": verdict,
        "claims_checked": [],
        "evidence_refs": [],
    })
}

/// Find the rollup row for `campaign` in the `campaigns` array.
fn campaign_row<'a>(v: &'a Value, campaign: &str) -> &'a Value {
    v["campaigns"]
        .as_array()
        .expect("campaigns is an array")
        .iter()
        .find(|c| c["campaign"] == campaign)
        .unwrap_or_else(|| panic!("campaign {campaign} present"))
}

#[test]
fn ledger_projects_asked_done_proven_rejected_per_campaign() {
    let dir = scratch("rollup");
    let log = dir.join("log.json");
    write_log(
        &log,
        &[
            (
                "intent.landed",
                json!({"intent_id":"i1","campaign":"auth","charter":"a"}),
            ),
            (
                "intent.landed",
                json!({"intent_id":"i2","campaign":"billing","charter":"b"}),
            ),
            (
                "intent.landed",
                json!({"intent_id":"i3","campaign":"auth","charter":"c"}),
            ),
            ("verdict.recorded", vo("i1", "approve")),
            ("verdict.recorded", vo("i3", "reject")),
            ("verdict.recorded", vo("i2", "approve")),
        ],
    );

    let (code, v) = run(&["ledger", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "ledger exits 0 on a valid log: {v}");

    // One entry per intent.landed.
    assert_eq!(v["entries"].as_array().unwrap().len(), 3);

    // auth: i1 approve (proven), i3 reject (rejected) → 2 asked/done, 1 proven, 1 rejected.
    let auth = campaign_row(&v, "auth");
    assert_eq!(auth["asked"], 2);
    assert_eq!(auth["done"], 2);
    assert_eq!(auth["proven"], 1);
    assert_eq!(auth["rejected"], 1);

    // billing: i2 approve → 1 asked/done, 1 proven, 0 rejected.
    let billing = campaign_row(&v, "billing");
    assert_eq!(billing["asked"], 1);
    assert_eq!(billing["proven"], 1);
    assert_eq!(billing["rejected"], 0);
}

#[test]
fn ledger_scopes_to_one_campaign() {
    let dir = scratch("scope");
    let log = dir.join("log.json");
    write_log(
        &log,
        &[
            (
                "intent.landed",
                json!({"intent_id":"i1","campaign":"auth","charter":"a"}),
            ),
            (
                "intent.landed",
                json!({"intent_id":"i2","campaign":"billing","charter":"b"}),
            ),
            (
                "intent.landed",
                json!({"intent_id":"i3","campaign":"auth","charter":"c"}),
            ),
        ],
    );

    let (code, v) = run(&[
        "ledger",
        "--log",
        log.to_str().unwrap(),
        "--campaign",
        "auth",
    ]);
    assert_eq!(code, 0, "scoped ledger exits 0: {v}");

    // Only the auth campaign is in the rollup, and only its 2 entries.
    let campaigns = v["campaigns"].as_array().unwrap();
    assert_eq!(campaigns.len(), 1);
    assert_eq!(campaigns[0]["campaign"], "auth");
    assert_eq!(campaigns[0]["asked"], 2);
    assert_eq!(v["entries"].as_array().unwrap().len(), 2);
}

#[test]
fn ledger_projection_redacts_secret_shaped_fields() {
    let dir = scratch("redaction");
    let log = dir.join("log.json");
    let secret = "SECRET:ledger-projection";
    write_log(
        &log,
        &[(
            "intent.landed",
            json!({"intent_id":"i1","campaign":"safe","charter":secret}),
        )],
    );

    let (code, v) = run(&["ledger", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "ledger exits 0: {v}");
    let rendered = v.to_string();
    assert!(
        !rendered.contains(secret),
        "projection must not leak secret: {v}"
    );
    assert_eq!(v["entries"][0]["charter"], "[REDACTED]");
}

#[test]
fn ledger_is_honest_null_on_log_without_intents() {
    let dir = scratch("empty");
    let log = dir.join("log.json");
    // A valid, chained log that carries no intent.landed event.
    write_log(
        &log,
        &[("journal.note", json!({"note": "nothing landed yet"}))],
    );

    let (code, v) = run(&["ledger", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "empty-history ledger exits 0: {v}");
    assert!(v["campaigns"].as_array().unwrap().is_empty());
    assert!(v["entries"].as_array().unwrap().is_empty());
    assert!(
        v["note"].is_string(),
        "an empty ledger discloses the honest-null with a note: {v}"
    );
}

#[test]
fn ledger_missing_log_obeys_the_error_law() {
    let (code, v) = run(&["ledger", "--log", "/no/such/ledger/log.json"]);
    assert_eq!(code, 2, "a structured error is exit 2");
    assert_eq!(
        v["error"]["kind"], "log_not_found",
        "the canonical nested {{\"error\":{{…}}}} envelope: {v}"
    );
}
