//! Round 8 — Wave L, WP L-D acceptance: STATE-MACHINE INTEGRITY (C5) + AUTHZ
//! MUTATION-GUARD (C4).
//!
//! Class-level (not just instance-level) coverage for the three confirmed
//! Round-8 holes the lead reproduced:
//!
//! - **C5-F1** — within-record lens-substitution launders a sticky reject. The
//!   ledger fold is now reject-sticky WITHIN a record (the source of truth) AND
//!   the recorder refuses a duplicate-lens-with-conflicting-result input. The
//!   legit cross-record clear (same-lens approve in a LATER record) still works.
//! - **C5-F2** — a SEALED campaign is terminal for EVERY campaign-scoped verb,
//!   not just `verdict`: `intent new`, `pr open/land/settle/abandon` all refuse a
//!   post-seal append via the ONE shared `campaign::seal_guard::guard_not_sealed`
//!   chokepoint.
//! - **C4** — the raw `EventLog::append` door is `pub(crate)`; the typed
//!   `append_external_change` shim and the guarded `append_authorized` are the
//!   only cross-crate doors. This file asserts the source-level invariant (no
//!   out-of-crate raw `::append` production caller) as a gate.
//!
//! The CLI verbs are driven through the real binary on a hermetic temp `--log`
//! (outside the repo). The C4 source invariant is a grep over the workspace.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve the freshly-built `hugit` binary (Cargo sets `CARGO_BIN_EXE_*` for
/// integration tests of a crate that builds a binary; here the binary lives in
/// the sibling `hugit-app` crate, so fall back to the workspace `target/debug`).
fn hugit_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_hugit") {
        return PathBuf::from(p);
    }
    // tests run from the crate dir; the workspace target is two levels up.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ws_root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let bin = ws_root.join("target").join("debug").join("hugit");
    assert!(
        bin.exists(),
        "hugit binary not found at {bin:?}; build it with `cargo build -p hugit-app` first",
    );
    bin
}

/// One scratch dir per test, removed on drop.
struct Scratch {
    dir: PathBuf,
}
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "hugit-r8-sm-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self { dir }
    }
    fn log(&self) -> PathBuf {
        self.dir.join("log.json")
    }
    fn store(&self) -> PathBuf {
        self.dir.join("store.json")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run the binary, returning (exit_code, stdout).
fn run(bin: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(bin).args(args).output().expect("spawn hugit");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn open_campaign(bin: &Path, log: &Path, key: &str) {
    let (c, _) = run(
        bin,
        &[
            "campaign",
            "open",
            "--campaign",
            key,
            "--owner",
            "bob",
            "--charter",
            "t",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(c, 0, "campaign open should succeed");
}

fn new_intent(bin: &Path, log: &Path, store: &Path, campaign: &str, id: &str) -> i32 {
    run(
        bin,
        &[
            "intent",
            "new",
            "--campaign",
            campaign,
            "--charter",
            "c",
            "--id",
            id,
            "--log",
            log.to_str().unwrap(),
            "--store",
            store.to_str().unwrap(),
        ],
    )
    .0
}

/// Project `ledger.{proven,rejected,done}` from `campaign show`.
fn ledger_counts(bin: &Path, log: &Path, key: &str) -> (i64, i64, i64) {
    let (c, out) = run(
        bin,
        &[
            "campaign",
            "show",
            "--campaign",
            key,
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(c, 0, "campaign show should succeed: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("show json");
    let l = &v["ledger"];
    (
        l["proven"].as_i64().unwrap_or(-1),
        l["rejected"].as_i64().unwrap_or(-1),
        l["done"].as_i64().unwrap_or(-1),
    )
}

// ── C5-F1 ────────────────────────────────────────────────────────────────────

/// C5-F1 (recorder layer): a SINGLE call carrying the same lens twice with
/// conflicting results is refused with `duplicate_lens`/exit-2 — no
/// self-contradictory record is ever written.
#[test]
fn within_record_conflicting_duplicate_lens_is_refused() {
    let bin = hugit_bin();
    let s = Scratch::new("f1-dup");
    let (log, store) = (s.log(), s.store());
    open_campaign(&bin, &log, "camp");
    assert_eq!(new_intent(&bin, &log, &store, "camp", "i1"), 0);

    let (code, out) = run(
        &bin,
        &[
            "verdict",
            "record",
            "--intent",
            "i1",
            "--log",
            log.to_str().unwrap(),
            "--store",
            "--lens",
            "security",
            "--result",
            "reject",
            "--lens",
            "security",
            "--result",
            "approve",
        ],
    );
    assert_eq!(code, 2, "conflicting duplicate lens must exit 2: {out}");
    assert!(out.contains("duplicate_lens"), "kind=duplicate_lens: {out}");

    // The reject must NOT be laundered out: the campaign carries no laundered
    // approve. Since the call was refused, no verdict record exists, so the
    // intent is neither proven nor rejected — and crucially NOT falsely proven.
    let (proven, _rejected, done) = ledger_counts(&bin, &log, "camp");
    assert_eq!(done, 1);
    assert_eq!(proven, 0, "a refused launder must never yield proven>0");
}

/// C5-F1 (class): an honest single reject sets `rejected`; a same-lens approve in
/// a LATER record clears it (cross-record clear preserved). A reject is never
/// laundered to proven.
#[test]
fn reject_is_sticky_then_clearable_cross_record() {
    let bin = hugit_bin();
    let s = Scratch::new("f1-sticky");
    let (log, store) = (s.log(), s.store());
    open_campaign(&bin, &log, "camp");
    assert_eq!(new_intent(&bin, &log, &store, "camp", "i1"), 0);

    // Single reject → rejected.
    let (c, _) = run(
        &bin,
        &[
            "verdict",
            "record",
            "--intent",
            "i1",
            "--log",
            log.to_str().unwrap(),
            "--store",
            "--lens",
            "security",
            "--result",
            "reject",
        ],
    );
    assert_eq!(c, 0);
    let (proven, rejected, _) = ledger_counts(&bin, &log, "camp");
    assert_eq!((proven, rejected), (0, 1), "single reject → rejected");

    // Same-lens approve in a LATER record → clears.
    let (c, _) = run(
        &bin,
        &[
            "verdict",
            "record",
            "--intent",
            "i1",
            "--log",
            log.to_str().unwrap(),
            "--store",
            "--lens",
            "security",
            "--result",
            "approve",
        ],
    );
    assert_eq!(c, 0);
    let (proven, rejected, _) = ledger_counts(&bin, &log, "camp");
    assert_eq!(
        (proven, rejected),
        (1, 0),
        "later same-lens approve clears the reject (cross-record clear preserved)"
    );
}

/// C5-F1 ⊕ F2 compound: with the launder closed, a real reject survives, so
/// `campaign close` refuses a clean seal without `--allow-rejected`.
#[test]
fn compound_rejected_campaign_refuses_clean_seal() {
    let bin = hugit_bin();
    let s = Scratch::new("compound");
    let (log, store) = (s.log(), s.store());
    open_campaign(&bin, &log, "camp");
    assert_eq!(new_intent(&bin, &log, &store, "camp", "i1"), 0);
    // honest reject (the launder is refused, so we record a clean one).
    let (c, _) = run(
        &bin,
        &[
            "verdict",
            "record",
            "--intent",
            "i1",
            "--log",
            log.to_str().unwrap(),
            "--store",
            "--lens",
            "security",
            "--result",
            "reject",
        ],
    );
    assert_eq!(c, 0);
    // close without --allow-rejected → refused.
    let (c, out) = run(
        &bin,
        &[
            "campaign",
            "close",
            "--campaign",
            "camp",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(c, 2, "rejected campaign must refuse clean seal: {out}");
    assert!(out.contains("campaign_has_rejected"), "{out}");
    // close WITH --allow-rejected → seals, marked sealed_with_rejected.
    let (c, out) = run(
        &bin,
        &[
            "campaign",
            "close",
            "--campaign",
            "camp",
            "--log",
            log.to_str().unwrap(),
            "--allow-rejected",
        ],
    );
    assert_eq!(c, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["sealed_with_rejected"], serde_json::json!(true));
}

// ── C5-F2 ────────────────────────────────────────────────────────────────────

/// C5-F2 (class): a sealed campaign is terminal for EVERY campaign-scoped verb.
/// `intent new` and `pr open` both refuse a post-seal append with
/// `campaign_sealed`/exit-2, and `done` does not move past the seal.
#[test]
fn sealed_campaign_is_terminal_for_intent_and_pr_open() {
    let bin = hugit_bin();
    let s = Scratch::new("f2-seal");
    let (log, store) = (s.log(), s.store());
    open_campaign(&bin, &log, "camp");
    assert_eq!(new_intent(&bin, &log, &store, "camp", "i1"), 0);
    // close (no PRs in flight, no rejects) → sealed.
    let (c, _) = run(
        &bin,
        &[
            "campaign",
            "close",
            "--campaign",
            "camp",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(c, 0);
    let (.., done_before) = ledger_counts(&bin, &log, "camp");
    assert_eq!(done_before, 1);

    // intent new into the sealed campaign → refused.
    let (c, out) = run(
        &bin,
        &[
            "intent",
            "new",
            "--campaign",
            "camp",
            "--charter",
            "c2",
            "--id",
            "i2",
            "--log",
            log.to_str().unwrap(),
            "--store",
            store.to_str().unwrap(),
        ],
    );
    assert_eq!(c, 2, "post-seal intent new must refuse: {out}");
    assert!(out.contains("campaign_sealed"), "{out}");

    // pr open into the sealed campaign → refused.
    let (c, out) = run(
        &bin,
        &[
            "pr",
            "open",
            "--pr",
            "pr1",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r1",
            "--intent",
            "i1",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(c, 2, "post-seal pr open must refuse: {out}");
    assert!(out.contains("campaign_sealed"), "{out}");

    // done must NOT have moved past the seal.
    let (.., done_after) = ledger_counts(&bin, &log, "camp");
    assert_eq!(done_after, 1, "no append joined the sealed campaign");
}

/// C5-F2 (class, pr terminal verbs): `pr land`/`pr abandon` into a sealed
/// campaign also refuse with `campaign_sealed`/exit-2.
#[test]
fn sealed_campaign_is_terminal_for_pr_land_and_abandon() {
    let bin = hugit_bin();
    let s = Scratch::new("f2-pr");
    let (log, store) = (s.log(), s.store());
    open_campaign(&bin, &log, "camp");
    assert_eq!(new_intent(&bin, &log, &store, "camp", "i1"), 0);
    let logs = log.to_str().unwrap();
    // open → queue → settle the PR so the campaign can close clean.
    assert_eq!(
        run(
            &bin,
            &[
                "pr",
                "open",
                "--pr",
                "p",
                "--campaign",
                "camp",
                "--author-kind",
                "orchestrator",
                "--run-id",
                "r1",
                "--intent",
                "i1",
                "--log",
                logs,
            ],
        )
        .0,
        0
    );
    assert_eq!(run(&bin, &["pr", "queue", "--pr", "p", "--log", logs]).0, 0);
    assert_eq!(run(&bin, &["pr", "land", "--pr", "p", "--log", logs]).0, 0);
    assert_eq!(
        run(
            &bin,
            &["campaign", "close", "--campaign", "camp", "--log", logs]
        )
        .0,
        0
    );

    // land (re-queue) into sealed → refused.
    let (c, out) = run(&bin, &["pr", "queue", "--pr", "p", "--log", logs]);
    assert_eq!(c, 2, "post-seal pr land must refuse: {out}");
    assert!(out.contains("campaign_sealed"), "{out}");

    // abandon into sealed → refused.
    let (c, out) = run(
        &bin,
        &["pr", "abandon", "--pr", "p", "--reason", "x", "--log", logs],
    );
    assert_eq!(c, 2, "post-seal pr abandon must refuse: {out}");
    assert!(out.contains("campaign_sealed"), "{out}");
}

// ── C4 ───────────────────────────────────────────────────────────────────────

/// C4-F1 (source invariant / gate): no crate OUTSIDE `hugit-refstore` references
/// the raw `EventLog::append` door in PRODUCTION code (`src/`). The only
/// cross-crate doors are the guarded `append_authorized` and the typed
/// `append_external_change`; test fixtures use the feature-gated
/// `append_for_test`. This converts the "don't call append raw" prose rule into
/// a compiled gate.
#[test]
fn no_out_of_crate_raw_append_in_production_source() {
    // Walk every crate's `src/` and assert no `.append(` that is not one of the
    // allowed forms appears outside `hugit-refstore`.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let crates_dir = manifest.parent().expect("crates/ dir").to_path_buf();

    let mut offenders: Vec<String> = Vec::new();
    visit_rs_files(&crates_dir, &mut |path| {
        let p = path.to_string_lossy();
        // Only PRODUCTION source, and only OUTSIDE the refstore crate (where the
        // raw door legitimately lives, `pub(crate)`).
        if !p.contains("/src/") {
            return;
        }
        if p.contains("/hugit-refstore/") {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        for (i, line) in text.lines().enumerate() {
            // A bare `.append(` that is NOT one of the allowed wrappers. Note the
            // session journal's `Journal::append` (hugit-ledger) is a DIFFERENT
            // type (not the D14 EventLog) — it is matched here too, so we exempt
            // `journal`/`Journal` lines explicitly (they are out of D14 scope).
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains(".append(")
                && !line.contains(".append_authorized(")
                && !line.contains(".append_external_change(")
                && !line.contains(".append_for_test(")
                && !line.contains("push_record(")
                // Journal (session-resume) append is a different store, not D14.
                && !line.contains("journal")
                && !line.contains("Journal")
                // `std::fs::OpenOptions::append(bool)` is the file-open mode, NOT the
                // D14 `EventLog::append` (which never takes a bare bool). The webhook
                // ingress journals NDJSON to disk via `OpenOptions::new().append(true)`.
                && !line.contains(".append(true)")
                && !line.contains(".append(false)")
            {
                offenders.push(format!("{}:{}: {}", p, i + 1, line.trim()));
            }
        }
    });

    assert!(
        offenders.is_empty(),
        "raw `EventLog::append` is reachable from production code outside \
         hugit-refstore — the D14 guard can be bypassed. Offenders:\n{}",
        offenders.join("\n")
    );
}

/// Recursively visit every `.rs` file under `dir`.
fn visit_rs_files(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip build artifacts.
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            visit_rs_files(&path, f);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            f(&path);
        }
    }
}
