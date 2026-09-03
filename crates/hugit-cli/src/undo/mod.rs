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
//! ## Hook-captured `ref.update` (W7)
//!
//! The silent hooks (`hugit capture`, principal `orchestrator:hugit-hook`)
//! append `ref.update` captures of what the agent did to the git graph. A human
//! may undo a captured `ref.update` exactly like any other ref mutation — it is
//! a raw graph trace, NOT an intent (`intent.landed` maps to a landing undo;
//! a capture never has an intent to map). The compensator for a capture is the
//! same restore-the-prior-ref machinery (`compute_compensation`), but its
//! payload is **enriched**: it records the supersession explicitly —
//! `{"ref", "target", "capture_undone_seq": <undone seq>}` — so "captured
//! `ref.update` at seq=N is superseded/undone" is on-record, not implicit.
//! The D14 Human-only gate is untouched: the enriched append routes through
//! the same guard under [`Endpoint::Undo`] (denied + audited for a non-human).
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

use hugit_refstore::authz::{AuditedGuard, Decision, Endpoint, PrincipalClass};
use hugit_refstore::{Compensation, UndoError, compute_compensation};

use crate::campaign::CampaignError;
use crate::campaign::world::{World, persist_log};

/// The recorder identity hook-captured `ref.update` events carry
/// (`hugit capture` → [`crate::capture`]). A `ref.update` under this principal
/// is a **capture** (a silent record of what the agent did), not an explicit
/// push and never an intent.
const CAPTURED_PRINCIPAL: &str = "orchestrator:hugit-hook";

/// Arguments for `hugit undo`.
#[derive(clap::Args, Debug)]
pub struct UndoArgs {
    /// Path to the JSON event log (`[EventRecord, …]`). Read, then rewritten
    /// with the appended compensating record.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,

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

    // Resolve the default --log ($HUGIT_LOG → .hugit/log.json) once.
    let log_path = crate::log_resolve::resolve_log(args.log.clone());

    // ── Lock BEFORE load (WC1 discipline) ────────────────────────────────────
    // `bootstrap = false`: an undo requires the log to already exist (you cannot
    // undo on a non-existent log). A missing `--log` is `log_not_found`/exit-2,
    // never a ghost record. `_lock` is held across the compute→append→persist
    // critical section until scope end (Drop releases).
    let (_lock, world) = World::lock_and_load(&log_path, false)?;

    // ── Undo through the REAL backing (D14 Human-only guard) ──────────────────
    // `hugit_refstore::undo` re-verifies the chain (fail-closed), computes the
    // compensator for the ref the target touched, routes the authorization
    // through the AuditedGuard under Endpoint::Undo, and on Allow appends the
    // compensating event. `recorded_at = 0` mirrors the CLI convention every
    // other porcelain verb uses (deterministic; the live clock is the P2 seam).
    //
    // A **hook-captured** `ref.update` (W7) takes the explicit capture path:
    // the same machinery computes the restore-the-prior-ref compensator, but
    // the appended payload records the supersession (`capture_undone_seq`) and
    // still routes through the D14 guard under Endpoint::Undo — the Human-only
    // gate is untouched, denials are audited exactly as on the generic path.
    let mut log = world.log.clone();
    let result = if captured_ref_update_at(&log, args.seq) {
        undo_captured_ref_update(&mut log, args.seq, vec![actor.clone()], 0)
    } else {
        hugit_refstore::undo(&mut log, args.seq, vec![actor.clone()], 0)
    };
    match result {
        Ok(comp) => {
            // ── Atomic persist (WC1 — truncation-proof) ──────────────────────
            // `_lock` (bound above) stays held across compute→append→persist
            // until scope end.
            persist_log(&log_path, &log)?;
            // The compensator is the last record on the now-extended chain.
            let appended_seq = log.records().last().map(|r| r.seq).ok_or_else(|| {
                CampaignError::internal("undo appended no record", "report this bug")
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
            persist_log(&log_path, &log)?;
            Err(map_undo_error(&e, args.seq))
        }
        Err(e) => Err(map_undo_error(&e, args.seq)),
    }
}

/// Whether the record at `seq` is a **hook-captured** `ref.update` — a silent
/// graph capture (`hugit capture`, principal [`CAPTURED_PRINCIPAL`]), not an
/// explicit push and never an intent. Identity is by seq + kinds semantics:
/// the KIND is the frozen raw-push `ref.update`; the principal chain marks it
/// as captured. A captured event has no intent, so undoing it can never map to
/// a landing undo — the compensator is the plain prior-ref restore.
fn captured_ref_update_at(log: &hugit_refstore::EventLog, seq: u64) -> bool {
    let Some(record) = log.records().get(seq as usize) else {
        return false;
    };
    record.kind == "ref.update"
        && record
            .principal_chain
            .iter()
            .any(|p| p == CAPTURED_PRINCIPAL)
}

/// Undo a hook-captured `ref.update` (W7): append the compensating event that
/// records "captured `ref.update` at seq=N is superseded/undone".
///
/// Reuses the existing undo machinery — [`compute_compensation`] re-verifies
/// the chain and recovers the ref's prior value from the log prefix — then
/// enriches the compensator payload to name the supersession explicitly:
///
/// ```json
/// {"ref": <name>, "target": <prior oid>, "capture_undone_seq": <seq>}
/// ```
///
/// (`target` is absent when the ref did not exist before the capture, so the
/// compensator is a `ref.delete`.) The append routes through the same D14
/// guard under [`Endpoint::Undo`] the generic path uses — a non-human actor is
/// denied fail-closed and an `authz.denied` audit record is written (③). The
/// gate is unchanged: Human-only.
fn undo_captured_ref_update(
    log: &mut hugit_refstore::EventLog,
    target: u64,
    principal_chain: Vec<String>,
    recorded_at: u64,
) -> Result<Compensation, UndoError> {
    // The existing machinery: chain verify (fail-closed) + prior-ref recovery.
    let comp = compute_compensation(log, target)?;

    // Enrich the compensator payload with the supersession marker.
    let comp_payload: serde_json::Value =
        serde_json::from_str(&comp.payload).map_err(|_| UndoError::BadTargetPayload {
            target,
            kind: comp.kind.clone(),
        })?;
    let mut body = serde_json::Map::new();
    body.insert("ref".to_string(), json!(comp.ref_name));
    if let Some(prior) = comp_payload
        .get("target")
        .and_then(serde_json::Value::as_str)
    {
        body.insert("target".to_string(), json!(prior));
    }
    body.insert("capture_undone_seq".to_string(), json!(target));
    let payload = serde_json::Value::Object(body).to_string();

    // The SAME D14 gate the generic undo path uses: chain-classified
    // authorization under Endpoint::Undo, denials audited (③). Human-only.
    let mut guard = AuditedGuard::new(log);
    let (decision, _audit) = guard.authorize(&principal_chain, Endpoint::Undo, recorded_at);
    match decision {
        Decision::Allow => {
            // (Human, Undo) is pinned Allow by the frozen matrix, so this
            // guarded append cannot be denied after the allow decision above.
            log.append_authorized(
                PrincipalClass::Human,
                Endpoint::Undo,
                comp.kind.clone(),
                principal_chain,
                payload,
                recorded_at,
            )
            .map_err(|denied| UndoError::Denied(denied.reason))?;
            Ok(comp)
        }
        Decision::Deny(reason) => Err(UndoError::Denied(reason)),
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
            log: Some(log.clone()),
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
            log: Some(log),
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
            log: Some(log.clone()),
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

    /// A scratch log with two hook-captured `ref.update`s on the same ref, so a
    /// captured commit is followed by a later captured commit and undoing the
    /// second has a real compensator (restore the ref to the first capture's
    /// target). Built via the capture principal + the real append path so the
    /// hash chain is valid and the record reads exactly like a hook capture.
    fn scratch_captures(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-undo-captured-{}-{}-{:?}",
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
        let capture =
            |target: &str| json!({ "ref": r, "target": target, "branch": "main" }).to_string();
        el.append_for_test(
            "ref.update",
            vec![super::CAPTURED_PRINCIPAL.to_string()],
            capture(&"a".repeat(40)),
            1,
        );
        el.append_for_test(
            "ref.update",
            vec![super::CAPTURED_PRINCIPAL.to_string()],
            capture(&"b".repeat(40)),
            2,
        );
        std::fs::write(&log, serde_json::to_string_pretty(el.records()).unwrap()).unwrap();
        log
    }

    /// A human may undo a hook-captured `ref.update`: the compensator restores
    /// the ref to its prior target AND records the supersession — `{"ref",
    /// "target", "capture_undone_seq"}` — while the chain still verifies.
    #[test]
    fn human_undo_of_captured_ref_update_enriches_and_restores() {
        let log = scratch_captures("captured-ok");
        let mut el = hugit_refstore::EventLog::new();
        for r in records(&log) {
            let rec: hugit_contracts::event_record::EventRecord =
                serde_json::from_value(r).unwrap();
            el.push_record(rec).unwrap();
        }
        let before = el.len();
        let state_before = hugit_refstore::replay(&el).unwrap();
        let second_capture = "b".repeat(40);
        assert_eq!(
            state_before.get("refs/heads/main"),
            Some(second_capture.as_str())
        );

        let result = do_run(UndoArgs {
            log: Some(log.clone()),
            seq: 1,
            actor: Some("user:alice".to_string()),
        })
        .expect("a human may undo a captured ref.update");

        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["undone_seq"], 1);
        assert_eq!(v["compensation_kind"], "ref.update");
        assert_eq!(v["ref"], "refs/heads/main");

        let after = records(&log);
        assert_eq!(after.len(), before + 1, "exactly one compensator appended");
        let comp = after.last().unwrap();
        assert_eq!(comp["kind"], "ref.update");
        assert_eq!(comp["principal_chain"][0], "user:alice");
        let p: serde_json::Value = serde_json::from_str(comp["payload"].as_str().unwrap()).unwrap();
        assert_eq!(p["ref"], "refs/heads/main");
        assert_eq!(p["target"], "a".repeat(40)); // prior value restored
        assert_eq!(p["capture_undone_seq"], 1); // supersession on-record
        assert_eq!(
            p["branch"],
            serde_json::Value::Null,
            "enriched payload is the canonical restore + marker"
        );

        // Chain still valid; the ref is recorded as undone (back at the
        // seq-0 capture's target).
        let mut final_el = hugit_refstore::EventLog::new();
        for r in &after {
            let rec: hugit_contracts::event_record::EventRecord =
                serde_json::from_value(r.clone()).unwrap();
            final_el.push_record(rec).unwrap();
        }
        hugit_refstore::verify_chain(final_el.records()).expect("chain verifies after the undo");
        let final_state = hugit_refstore::replay(&final_el).unwrap();
        let first_capture = "a".repeat(40);
        assert_eq!(
            final_state.get("refs/heads/main"),
            Some(first_capture.as_str())
        );
    }

    /// A hook-captured `ref.update` at genesis (the ref did not exist before)
    /// compenses as a `ref.delete` — still marked with the superseded seq.
    #[test]
    fn captured_ref_update_at_genesis_compenses_as_ref_delete() {
        let dir = std::env::temp_dir().join(format!(
            "hugit-undo-captured-genesis-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log.json");
        let mut el = EventLog::new();
        el.append_for_test(
            "ref.update",
            vec![super::CAPTURED_PRINCIPAL.to_string()],
            json!({ "ref": "refs/heads/main", "target": "a".repeat(40), "branch": "main" })
                .to_string(),
            1,
        );
        std::fs::write(&log, serde_json::to_string_pretty(el.records()).unwrap()).unwrap();

        let result = do_run(UndoArgs {
            log: Some(log.clone()),
            seq: 0,
            actor: Some("user:alice".to_string()),
        })
        .expect("a human may undo a genesis capture");

        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["compensation_kind"], "ref.delete");
        let after = records(&log);
        let comp = after.last().unwrap();
        let p: serde_json::Value = serde_json::from_str(comp["payload"].as_str().unwrap()).unwrap();
        assert_eq!(p["ref"], "refs/heads/main");
        assert_eq!(p["capture_undone_seq"], 0);
        assert!(p.get("target").is_none(), "no target on a ref.delete");
    }

    /// A non-human principal attempting to undo a captured `ref.update` is
    /// denied by the unchanged D14 gate: `authz_denied`, an audit record lands,
    /// and NO compensator is appended.
    #[test]
    fn non_human_undo_of_captured_ref_update_is_authz_denied() {
        let log = scratch_captures("captured-denied");
        let before_updates = records(&log)
            .iter()
            .filter(|r| r["kind"] == "ref.update")
            .count();

        let err = do_run(UndoArgs {
            log: Some(log.clone()),
            seq: 1,
            actor: Some("agent:runner-03".to_string()),
        })
        .expect_err("a non-human undo of a capture must be denied");

        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "authz_denied");

        let after = records(&log);
        assert_eq!(
            after.iter().filter(|r| r["kind"] == "ref.update").count(),
            before_updates,
            "no compensating ref.update on a denied undo"
        );
        let last = after.last().unwrap();
        assert_eq!(last["kind"], "authz.denied");
        assert!(
            last["payload"]
                .as_str()
                .unwrap()
                .contains("\"endpoint\":\"undo\"")
        );
        // No capture_undone_seq marker slipped onto the log.
        assert!(!after.iter().any(|r| {
            r["payload"]
                .as_str()
                .unwrap_or("")
                .contains("capture_undone_seq")
        }));
    }

    /// An out-of-range `--seq` is `out_of_range`/exit-2 and appends nothing.
    #[test]
    fn out_of_range_seq_is_rejected() {
        let log = scratch_two_updates("oob");
        let before = records(&log).len();

        let err = do_run(UndoArgs {
            log: Some(log.clone()),
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
            log: Some(absent),
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
            log: Some(log.clone()),
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
