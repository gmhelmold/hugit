//! `hugit journal note --log <path> --note <text> [--principal <p>] [--workspace
//! <id>] [--intent <id>]` — append a session note onto the canonical event log.
//!
//! Mirrors the `issue.transition` write path exactly (the freshly-merged
//! template): append through `append_authorized(Orchestrator, Land, …)` so the
//! D14 matrix gates the mutation and denials are audited (the `(Orchestrator,
//! Land)` cell is the one `land` Allow), payload built as canonical scrubbed
//! JSON via [`crate::porcelain::scrub_to_canonical`], persisted atomically via
//! `persist_log`. A session note is an orchestrator-driven integration record —
//! NOT a human stakeholder control — so it uses the same cell as
//! `issue.transition`, not the Human-only `Undo`/`Policy` cells.
//!
//! ## Payload
//!
//! Carries the same semantic fields as the D11 `JournalEntry` / `JournalKey`:
//! `note` (free text, scrubbed), `principal`, and the optional `workspace_id` /
//! `intent_id` binding. `seq` + `recorded_at` are NOT duplicated — they are the
//! `EventRecord`'s own fields.
//!
//! ## Serve parity
//!
//! No serve verb exists yet; this CLI verb FREEZES the `journal.note` kind. A
//! `write_journal_note` serve verb can mirror it later with zero interface change
//! (CLI/serve parity — no web-only verb), exactly as `issue.transition` did.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::campaign::CampaignError;
use crate::campaign::world::{World, persist_log};
use crate::porcelain::scrub_to_canonical;

/// The event kind this verb appends.
pub const JOURNAL_NOTE_KIND: &str = "journal.note";

/// Arguments for `hugit journal note`.
#[derive(clap::Args, Debug)]
pub struct NoteArgs {
    /// Path to the JSON event log (`[EventRecord, …]`). Read, then rewritten with
    /// the appended `journal.note` record.
    #[arg(long)]
    pub log: PathBuf,

    /// The note text (free text — scrubbed before persisting).
    #[arg(long)]
    pub note: String,

    /// The principal recording the note (default `orchestrator:hugit`).
    #[arg(long)]
    pub principal: Option<String>,

    /// Optional workspace id binding (D11 `JournalKey.workspace_id`).
    #[arg(long)]
    pub workspace: Option<String>,

    /// Optional intent id binding (D11 `JournalKey.intent_id`).
    #[arg(long)]
    pub intent: Option<String>,
}

/// Run `hugit journal note` — append a `journal.note` record, exit 0 on success
/// or exit 2 on a structured domain error.
pub fn run(args: NoteArgs) -> ExitCode {
    match do_run(args) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", err.to_json());
            err.exit_code()
        }
    }
}

fn do_run(args: NoteArgs) -> Result<String, CampaignError> {
    use hugit_refstore::{Endpoint, PrincipalClass};

    // An empty note is a no-op mistake — refuse it with a clear domain error
    // rather than persist a blank record.
    if args.note.trim().is_empty() {
        return Err(CampaignError::new(
            "empty_note",
            "a journal note must not be empty",
            "pass --note with the session note text",
        ));
    }

    // Scrub free text + identifiers at the read/write boundary. `note` is free
    // text (full engine); `principal`/`workspace`/`intent` are identifiers but a
    // smuggled secret must not land raw on the forever-log, so route them through
    // the same scrub. The `scrub_to_canonical` below is the structural backstop;
    // these pre-scrubs keep the echoed success JSON clean too.
    let principal = args
        .principal
        .as_deref()
        .map(crate::redaction::scrub)
        .unwrap_or_else(|| "orchestrator:hugit".to_string());
    let workspace = args.workspace.as_deref().map(crate::redaction::scrub);
    let intent = args.intent.as_deref().map(crate::redaction::scrub);

    // ── Lock BEFORE load (WC1) — bootstrap=false: a note requires an existing log
    // (a missing --log is `log_not_found`/exit-2, never a ghost record). `_lock`
    // is held across append→persist until scope end.
    let (_lock, world) = World::lock_and_load(&args.log, false)?;

    // ── Build + scrub payload (the WG/WH-SCRUB structural seam) ──────────────
    // Mirror the D11 JournalEntry/JournalKey semantic fields; omit absent
    // optional bindings rather than writing nulls. `seq`/`recorded_at` are the
    // EventRecord's own fields, not duplicated here.
    let mut payload_value = json!({ "note": args.note, "principal": principal });
    if let Some(ws) = &workspace {
        payload_value["workspace_id"] = json!(ws);
    }
    if let Some(it) = &intent {
        payload_value["intent_id"] = json!(it);
    }
    let payload = scrub_to_canonical(payload_value);

    // ── Append through the D14 guard (Orchestrator, Land) ─────────────────────
    let principal_chain = vec![principal.clone()];
    let mut log = world.log.clone();
    let record = log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            JOURNAL_NOTE_KIND,
            principal_chain,
            payload,
            0,
        )
        .map_err(|denied| {
            CampaignError::new(
                "authz_denied",
                format!(
                    "journal.note append denied by D14 guard: {}",
                    denied.reason.code()
                ),
                "a journal note must be recorded by an orchestrator principal (Orchestrator/Land)",
            )
        })?;

    // ── Atomic persist (WC1) ──────────────────────────────────────────────────
    persist_log(&args.log, &log)?;

    // ── Stable JSON success envelope (echo the SCRUBBED values) ───────────────
    let mut out = json!({ "kind": JOURNAL_NOTE_KIND, "seq": record.seq, "principal": principal });
    if let Some(ws) = &workspace {
        out["workspace_id"] = json!(ws);
    }
    if let Some(it) = &intent {
        out["intent_id"] = json!(it);
    }
    Ok(out.to_string())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-journal-note-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        use hugit_refstore::EventLog;
        let mut el = EventLog::new();
        el.append_for_test("repo.init", vec!["orchestrator:hugit".to_string()], "{}", 0);
        std::fs::write(&log, serde_json::to_string_pretty(el.records()).unwrap()).unwrap();
        log
    }

    fn records(path: &std::path::Path) -> Vec<serde_json::Value> {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn note_appends_journal_note_record() {
        let log = scratch("ok");
        let before = records(&log).len();

        let result = do_run(NoteArgs {
            log: log.clone(),
            note: "read 3 files, drafted the plan".to_string(),
            principal: None,
            workspace: None,
            intent: None,
        })
        .expect("note ok");

        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["kind"], JOURNAL_NOTE_KIND);
        assert_eq!(v["principal"], "orchestrator:hugit");
        assert!(v["seq"].is_u64());

        let after = records(&log);
        assert_eq!(after.len(), before + 1, "exactly one record appended");
        let rec = after.last().unwrap();
        assert_eq!(rec["kind"], JOURNAL_NOTE_KIND);
        let payload: serde_json::Value =
            serde_json::from_str(rec["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["note"], "read 3 files, drafted the plan");
    }

    #[test]
    fn optional_bindings_round_trip() {
        let log = scratch("bindings");
        let result = do_run(NoteArgs {
            log: log.clone(),
            note: "started session".to_string(),
            principal: Some("agent:worker-1".to_string()),
            workspace: Some("ws-7".to_string()),
            intent: Some("i-42".to_string()),
        })
        .expect("note ok");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["principal"], "agent:worker-1");
        assert_eq!(v["workspace_id"], "ws-7");
        assert_eq!(v["intent_id"], "i-42");

        let rec = records(&log).pop().unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(rec["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["workspace_id"], "ws-7");
        assert_eq!(payload["intent_id"], "i-42");
    }

    #[test]
    fn empty_note_is_rejected() {
        let log = scratch("empty");
        let before = records(&log).len();
        let err = do_run(NoteArgs {
            log: log.clone(),
            note: "   ".to_string(),
            principal: None,
            workspace: None,
            intent: None,
        })
        .expect_err("empty note must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "empty_note");
        assert_eq!(
            records(&log).len(),
            before,
            "nothing appended on empty note"
        );
    }

    #[test]
    fn missing_log_is_log_not_found() {
        let dir =
            std::env::temp_dir().join(format!("hugit-journal-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let absent = dir.join("no-such.json");
        let err = do_run(NoteArgs {
            log: absent,
            note: "note".to_string(),
            principal: None,
            workspace: None,
            intent: None,
        })
        .expect_err("missing log must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "log_not_found");
    }

    #[test]
    fn secret_in_note_is_redacted_before_appending() {
        let log = scratch("redact");
        let pat = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let result = do_run(NoteArgs {
            log: log.clone(),
            note: format!("token is {pat}"),
            principal: None,
            workspace: None,
            intent: None,
        })
        .expect("note still appends");
        assert!(!result.contains(pat), "PAT must not appear in success JSON");
        let raw = std::fs::read_to_string(&log).unwrap();
        assert!(!raw.contains(pat), "PAT must not appear on the log");
        assert!(
            raw.contains(hugit_ledger::redact::REDACTED),
            "the REDACTED sentinel must be present"
        );
    }
}
