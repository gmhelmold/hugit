//! `hugit undo --log <path> --seq <n> [--actor <a>]` — undo an operation as a
//! **compensating event** (never a rewrite, never a deletion).
//!
//! Thin porcelain over the REAL event-sourced undo backing
//! [`hugit_refstore::undo`]: it replays the log prefix `[0, seq)` to recover the
//! ref state before the target operation, then appends the compensating event
//! that restores it. Both the original event and its compensator stay in the
//! chain — history is preserved, force-push data loss is unexpressible by
//! construction.
//!
//! ## Human-only (D14)
//!
//! `undo` is one of the four mutating forge verbs the D14 matrix gates, declared
//! **Human-only**. The backing routes the authorization through the same
//! [`AuditedGuard`](hugit_refstore::authz) under `Endpoint::Undo`: a
//! non-human (orchestrator/agent/model) or unrecognized/empty principal is
//! **denied fail-closed** — the compensator is NOT appended, an `authz.denied`
//! audit record is written instead, and we surface a structured `authz_denied`
//! error (exit 2). The default `--actor` is `user:cli` (the human at the
//! terminal); pass `--actor` to attribute the undo explicitly.
//!
//! ## Hermetic file seam
//!
//! Operates on a local `--log <path>` JSON `[EventRecord, …]` array — the same
//! canonical format every porcelain verb reads/writes. The chain is re-verified
//! before computing the compensator (fail-closed on tamper). Live R2 binding is
//! the P2 disclosed seam.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::campaign::CampaignError;
use crate::campaign::world::{World, persist_log};

/// Arguments for `hugit undo`.
#[derive(clap::Args, Debug)]
pub struct UndoArgs {
    /// Path to the JSON event log (`[EventRecord, …]`). Read, then rewritten
    /// with the appended compensating record.
    #[arg(long)]
    pub log: PathBuf,

    /// The sequence number (`seq`) of the operation to undo.
    #[arg(long)]
    pub seq: u64,

    /// The human actor issuing the undo (must be a `user:`/`human:` principal —
    /// `undo` is Human-only). Defaults to `user:cli`.
    #[arg(long)]
    pub actor: Option<String>,
}

/// Run `hugit undo` — append a compensating record, exit 0 on success or exit 2
/// on a structured domain error (out-of-range seq, nothing-to-compensate,
/// tampered chain, or a D14 denial).
pub fn run(args: UndoArgs) -> ExitCode {
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

fn do_run(args: UndoArgs) -> Result<String, CampaignError> {
    use hugit_refstore::UndoError;

    // The actor is an identifier, not free text, but a smuggled secret-shaped
    // actor must not land raw on the principal chain — scrub it structurally
    // (the engine leaves a normal `user:name` untouched and only redacts
    // secret-shaped tokens). Default to the human-at-the-terminal principal.
    let actor = args
        .actor
        .as_deref()
        .map(crate::redaction::scrub)
        .unwrap_or_else(|| "user:cli".to_string());

    // ── Lock BEFORE load (WC1 discipline) ────────────────────────────────────
    // `bootstrap = false`: an undo requires the log to already exist (you cannot
    // undo on a non-existent log). A missing `--log` is `log_not_found`/exit-2,
    // never a ghost record. `_lock` is held across the compute→append→persist
    // critical section until scope end (Drop releases).
    let (_lock, world) = World::lock_and_load(&args.log, false)?;

    // ── Undo through the REAL backing (D14 Human-only guard) ──────────────────
    // `hugit_refstore::undo` re-verifies the chain (fail-closed), computes the
    // compensator for the ref the target touched, routes the authorization
    // through the AuditedGuard under Endpoint::Undo, and on Allow appends the
    // compensating event. `recorded_at = 0` mirrors the CLI convention every
    // other porcelain verb uses (deterministic; the live clock is the P2 seam).
    let mut log = world.log.clone();
    match hugit_refstore::undo(&mut log, args.seq, vec![actor.clone()], 0) {
        Ok(comp) => {
            // ── Atomic persist (WC1 — truncation-proof) ──────────────────────
            // `_lock` (bound above) stays held across compute→append→persist
            // until scope end.
            persist_log(&args.log, &log)?;
            // The compensator is the last record on the now-extended chain.
            let appended_seq = log.records().last().map(|r| r.seq).ok_or_else(|| {
                CampaignError::new("internal", "undo appended no record", "report this bug")
            })?;
            let out = json!({
                "undone_seq": args.seq,
                "compensation_kind": comp.kind,
                "ref": comp.ref_name,
                "seq": appended_seq,
            });
            Ok(out.to_string())
        }
        // On a D14 DENIAL the guard already appended an `authz.denied` audit
        // record into `log` (the denial is never silent) — persist it so the
        // refusal is durable + attributable, THEN surface the structured error.
        // Every other UndoError variant errors BEFORE any append (the log is
        // unmutated), so we only persist on the Denied path.
        Err(e @ UndoError::Denied(_)) => {
            persist_log(&args.log, &log)?;
            Err(map_undo_error(&e, args.seq))
        }
        Err(e) => Err(map_undo_error(&e, args.seq)),
    }
}

/// Map the backing [`UndoError`] onto the one CLI error/exit law.
///
/// Every variant is a structured user/domain error (exit 2). The `target` seq is
/// a numeric arg (not free text), so it is safe to echo; no scrub needed.
fn map_undo_error(e: &hugit_refstore::UndoError, seq: u64) -> CampaignError {
    use hugit_refstore::UndoError;
    match e {
        UndoError::OutOfRange { len, .. } => CampaignError::new(
            "out_of_range",
            format!("undo target seq {seq} is out of range (log has {len} records)"),
            "pass --seq within [0, log length); inspect the log to find the op to undo",
        ),
        UndoError::NothingToCompensate { kind, .. } => CampaignError::new(
            "nothing_to_compensate",
            format!("seq {seq} ({kind}) touched no ref — there is nothing to undo"),
            "undo targets a ref-mutating op (ref.update / ref.delete / intent.landed)",
        ),
        UndoError::BadTargetPayload { kind, .. } => CampaignError::new(
            "bad_payload",
            format!("the {kind} record at seq {seq} has a malformed payload"),
            "the target record's payload is corrupt; it cannot be interpreted to compute a compensator",
        ),
        UndoError::Tamper(te) => CampaignError::new(
            "chain_broken",
            format!("undo refused — the log failed integrity verification: {te}"),
            "the --log file's hash chain is tampered or corrupt; undo fails closed",
        ),
        UndoError::Replay(re) => CampaignError::new(
            "replay_failed",
            format!("undo could not replay the prefix before seq {seq}: {re}"),
            "the log prefix is internally inconsistent; it cannot be replayed to recover prior ref state",
        ),
        UndoError::Denied(reason) => CampaignError::new(
            "authz_denied",
            format!(
                "undo denied by the D14 guard: {} (undo is Human-only)",
                reason.code()
            ),
            "undo must be issued by a human (`--actor user:<name>` or `human:<name>`); \
             an orchestrator/agent/model principal is refused fail-closed",
        ),
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::EventLog;

    /// Canonical `ref.update` payload (byte-identical to what D1a replay reads).
    fn ref_update(name: &str, target: &str) -> String {
        json!({ "ref": name, "target": target }).to_string()
    }

    /// A scratch log with two `ref.update`s on the same ref, so undoing seq 1 has
    /// a real compensator (restore the ref to its seq-0 value). Built via the
    /// engine's real append path so the hash chain is valid.
    fn scratch_two_updates(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-undo-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        let mut el = EventLog::new();
        let r = "refs/heads/main";
        el.append_for_test(
            "ref.update",
            vec!["user:alice".into()],
            ref_update(r, &"a".repeat(40)),
            1,
        );
        el.append_for_test(
            "ref.update",
            vec!["user:alice".into()],
            ref_update(r, &"b".repeat(40)),
            2,
        );
        std::fs::write(&log, serde_json::to_string_pretty(el.records()).unwrap()).unwrap();
        log
    }

    fn records(path: &std::path::Path) -> Vec<serde_json::Value> {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    /// A human undo of a ref.update appends a compensating ref.update and returns
    /// stable JSON carrying the new `seq` + the undone seq.
    #[test]
    fn human_undo_appends_compensator_and_returns_seq() {
        let log = scratch_two_updates("ok");
        let before = records(&log).len();

        let result = do_run(UndoArgs {
            log: log.clone(),
            seq: 1,
            actor: Some("user:alice".to_string()),
        })
        .expect("a human may undo");

        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["undone_seq"], 1);
        assert_eq!(v["compensation_kind"], "ref.update");
        assert!(v["seq"].is_u64());

        let after = records(&log);
        assert_eq!(
            after.len(),
            before + 1,
            "exactly one compensating record appended"
        );
        assert_eq!(after.last().unwrap()["kind"], "ref.update");
    }

    /// The default actor (`user:cli`) is a human principal and is accepted.
    #[test]
    fn default_actor_is_human_and_accepted() {
        let log = scratch_two_updates("default-actor");
        do_run(UndoArgs {
            log,
            seq: 1,
            actor: None,
        })
        .expect("default user:cli is human");
    }

    /// A non-human actor is denied by the D14 guard → `authz_denied`/exit-2, and
    /// only the audit record (not a compensator) is appended.
    #[test]
    fn non_human_actor_is_authz_denied() {
        let log = scratch_two_updates("denied");
        let before_updates = records(&log)
            .iter()
            .filter(|r| r["kind"] == "ref.update")
            .count();

        let err = do_run(UndoArgs {
            log: log.clone(),
            seq: 1,
            actor: Some("agent:runner-03".to_string()),
        })
        .expect_err("a non-human undo must be denied");

        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "authz_denied");

        // The denial is persisted as an audit record; no compensating ref.update.
        let after = records(&log);
        assert_eq!(
            after.iter().filter(|r| r["kind"] == "ref.update").count(),
            before_updates,
            "no compensating ref.update on a denied undo"
        );
        assert_eq!(after.last().unwrap()["kind"], "authz.denied");
    }

    /// An out-of-range `--seq` is `out_of_range`/exit-2 and appends nothing.
    #[test]
    fn out_of_range_seq_is_rejected() {
        let log = scratch_two_updates("oob");
        let before = records(&log).len();

        let err = do_run(UndoArgs {
            log: log.clone(),
            seq: 99,
            actor: Some("user:alice".to_string()),
        })
        .expect_err("out-of-range seq must fail");

        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "out_of_range");
        assert_eq!(
            records(&log).len(),
            before,
            "nothing appended on out-of-range"
        );
    }

    /// A missing `--log` file is `log_not_found`/exit-2 (never a ghost record).
    #[test]
    fn missing_log_is_log_not_found() {
        let dir = std::env::temp_dir().join(format!("hugit-undo-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let absent = dir.join("no-such.json");

        let err = do_run(UndoArgs {
            log: absent,
            seq: 0,
            actor: Some("user:alice".to_string()),
        })
        .expect_err("missing log must fail");

        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "log_not_found");
    }

    /// A secret-shaped `--actor` is scrubbed before it reaches the principal
    /// chain (defense-in-depth at the write boundary). The scrubbed actor is not
    /// a `user:`/`human:` principal, so the guard denies it — but the raw secret
    /// must never appear on the log.
    #[test]
    fn secret_shaped_actor_is_scrubbed_before_the_chain() {
        let log = scratch_two_updates("scrub-actor");
        let pat = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

        // Denied (not a human principal after scrub), but the assertion is that
        // the raw PAT never lands on the log.
        let _ = do_run(UndoArgs {
            log: log.clone(),
            seq: 1,
            actor: Some(pat.to_string()),
        });

        let raw = std::fs::read_to_string(&log).unwrap();
        assert!(
            !raw.contains(pat),
            "the raw PAT must never appear on the log"
        );
    }
}
