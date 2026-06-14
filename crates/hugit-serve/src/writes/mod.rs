//! The write path — the shared foundation every `/v1` POST verb rides.
//!
//! ## The factorization (why this is the spine)
//!
//! A verb body is a PURE function `fn(&mut EventLog, …args, req) -> Result<Accepted,
//! EngineErr>` (see [`verbs`]). The cross-cutting law — idempotency, durable
//! persistence, the body-hash replay guard — lives HERE, in [`with_write`], so it
//! is implemented ONCE and the verbs cannot drift from it. This is the
//! close-the-product Wave-2 §1 forcing insight: the idempotency ledger is the one
//! component every verb shares, so it is built once and frozen.
//!
//! ## Idempotency (spec §3)
//!
//! `Idempotency-Key` is MANDATORY (absent ⇒ `400 IDEMPOTENCY_REQUIRED`). On a
//! SUCCESS, the door appends an `idem.recorded` event capturing `(principal, verb,
//! key, body_sha256, outcome)` and persists it atomically alongside the verb's own
//! record(s). A replay of the same key returns the stored outcome **byte-identical**
//! — so a lost-response retry of `land` never enqueues twice (one land, one
//! position). Same key + a DIFFERENT body ⇒ `409 IDEM_MISMATCH`, never re-executed.
//!
//! A verb REJECTION (4xx) is NOT recorded: the T1 verbs reject deterministically
//! from the log state (a 404/400 re-evaluates identically on replay), so
//! determinism gives byte-identical rejection replay for free. The non-deterministic
//! rejection-replay (policy/erasure STEP-UP) is a Tier-3 concern, gated on the §6
//! owner design checkpoint.
//!
//! ## Persistence ([`LogSink`])
//!
//! The read server is read-only by design; the write path's durable persistence is
//! abstracted behind [`LogSink`] so the WRITE credential / R2-write architecture is
//! isolated to one impl (the read path is untouched). A verb mutates an in-memory
//! [`EventLog`]; the door persists ONCE on success (atomic — both the verb record
//! and the `idem.recorded` land together, or neither does).

pub mod verbs;

use hugit_http_contracts::actions::Accepted;
use hugit_refstore::EventLog;
use sha2::{Digest, Sha256};

use crate::error::EngineErr;

/// The `idem.recorded` event kind — the idempotency ledger's record.
pub const IDEM_RECORDED_KIND: &str = "idem.recorded";

/// Max raw request-body size the write-door accepts (audit P2 — DoS / log-bloat
/// guard). Generous enough for a large file edit, small enough to refuse a
/// pathological multi-MB comment/ask. Over-limit ⇒ `400 INVALID_REQUEST`.
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Verbs that require fresh re-auth (STEP-UP, spec §3). The door refuses them
/// with `403 STEP_UP_REQUIRED` unless the route presents a fresh-auth proof.
/// (The real fresh-auth is the P2 Clerk seam; the door enforces the GATE today.)
pub const STEP_UP_VERBS: &[&str] = &["policy", "erasure"];

/// Durable persistence for a repo's event log. The read path stays read-only; the
/// WRITE credential / R2-write architecture lives ENTIRELY behind this trait.
pub trait LogSink: Send + Sync {
    /// Load + chain-verify `<repo>`'s event log (same verification as the read
    /// path). A missing log is an empty world ONLY if the verb can create it; for
    /// writes against an existing repo, absence is the verb's own `404`.
    fn load(&self, repo: &str) -> Result<EventLog, EngineErr>;

    /// Durably persist `log` as the new source of truth for `<repo>`. Fail-honest:
    /// a non-durable write is an `Err` (never a silent partial / fake success).
    ///
    /// **CAS OBLIGATION (hard contract — audit P2→P1).** `with_write` does
    /// load → mutate → persist with NO lock held across the gap. A correct impl
    /// MUST persist as a COMPARE-AND-SWAP against the head the matching [`load`]
    /// returned (R2 `If-Match`/conditional PUT, or a per-repo write lock): on a
    /// concurrent-write head mismatch it MUST reject (so the caller reloads +
    /// retries), NEVER last-writer-wins (which would silently drop the other
    /// request's records AND its idempotency-ledger entry). A non-CAS impl breaks
    /// cross-request atomicity — it is a contract violation, not an optimization.
    fn persist(&self, repo: &str, log: &EventLog) -> Result<(), EngineErr>;
}

/// Hex SHA-256 of the raw request body — the idempotency body fingerprint.
fn body_sha256(body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(body);
    hex::encode(h.finalize())
}

/// A prior idempotent outcome recovered from the ledger.
struct PriorOutcome {
    body_sha256: String,
    accepted: Accepted,
}

/// Scan the log for a prior `idem.recorded` matching `(principal, verb, key)`.
/// Returns the stored body-hash + the replayable `Accepted`.
fn idem_lookup(log: &EventLog, principal: &str, verb: &str, key: &str) -> Option<PriorOutcome> {
    log.records()
        .iter()
        .filter(|r| r.kind == IDEM_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<serde_json::Value>(&r.payload).ok())
        .find(|v| {
            v.get("principal").and_then(|x| x.as_str()) == Some(principal)
                && v.get("verb").and_then(|x| x.as_str()) == Some(verb)
                && v.get("key").and_then(|x| x.as_str()) == Some(key)
        })
        .and_then(|v| {
            let bh = v.get("body_sha256")?.as_str()?.to_string();
            let accepted: Accepted = serde_json::from_value(v.get("outcome")?.clone()).ok()?;
            Some(PriorOutcome {
                body_sha256: bh,
                accepted,
            })
        })
}

/// Append the `idem.recorded` ledger entry for a successful write. Canonical JSON
/// (sorted keys) so the chain pre-image is deterministic.
fn idem_record(
    log: &mut EventLog,
    principal: &str,
    verb: &str,
    key: &str,
    body_sha256: &str,
    accepted: &Accepted,
    at: u64,
) -> Result<(), EngineErr> {
    let outcome = serde_json::to_value(accepted)
        .map_err(|e| EngineErr::unavailable(format!("idem outcome serialize: {e}")))?;
    let value = serde_json::json!({
        "body_sha256": body_sha256,
        "key": key,
        "outcome": outcome,
        "principal": principal,
        "verb": verb,
    });
    let payload =
        hugit_refstore::canonical_json(&value.to_string()).unwrap_or_else(|| value.to_string());
    // The ledger record is system-emitted bookkeeping, not a user mutation; route
    // it through the same D14-guarded door under the acting principal.
    log.append_authorized(
        hugit_refstore::PrincipalClass::Orchestrator,
        hugit_refstore::Endpoint::Land,
        IDEM_RECORDED_KIND,
        vec![principal.to_string()],
        payload,
        at,
    )
    .map_err(|d| {
        EngineErr::unavailable(format!("idem ledger append denied: {}", d.reason.code()))
    })?;
    Ok(())
}

/// The write-door: the one chokepoint every POST verb rides.
///
/// - `verb` — the stable verb name keyed in the idempotency ledger (e.g. `"land"`).
/// - `idem_key` — the `Idempotency-Key` header (empty ⇒ `400`).
/// - `body` — the raw request body bytes (the replay fingerprint; size-capped).
/// - `step_up_presented` — did the route present fresh re-auth? Required for the
///   [`STEP_UP_VERBS`] (policy/erasure), else `403 STEP_UP_REQUIRED`.
/// - `run` — the pure verb body; it mutates the loaded log and returns `Accepted`.
///
/// On success the verb's record(s) + the `idem.recorded` entry persist atomically.
#[allow(clippy::too_many_arguments)]
pub fn with_write<F>(
    sink: &dyn LogSink,
    repo: &str,
    verb: &str,
    idem_key: &str,
    body: &[u8],
    step_up_presented: bool,
    principal_chain: Vec<String>,
    at: u64,
    run: F,
) -> Result<Accepted, EngineErr>
where
    F: FnOnce(&mut EventLog, Vec<String>, u64) -> Result<Accepted, EngineErr>,
{
    // STEP-UP gate (audit P1): a step-up verb without fresh re-auth is refused at
    // the door, BEFORE any load/effect — the verb itself is step-up-agnostic.
    if STEP_UP_VERBS.contains(&verb) && !step_up_presented {
        return Err(EngineErr::step_up_required());
    }
    // Body-size cap (audit P2): refuse a pathological body before scanning/hashing it.
    if body.len() > MAX_BODY_BYTES {
        return Err(EngineErr::invalid_request(
            "corpo da requisição excede o limite",
        ));
    }
    if idem_key.trim().is_empty() {
        return Err(EngineErr::idempotency_required());
    }
    let principal = principal_chain.last().cloned().unwrap_or_default();
    let body_hash = body_sha256(body);

    let mut log = sink.load(repo)?;

    // Replay guard: a seen key returns the stored outcome (or 409 on a body change)
    // BEFORE the verb runs — so a lost-response retry never re-executes the effect.
    if let Some(prior) = idem_lookup(&log, &principal, verb, idem_key) {
        if prior.body_sha256 != body_hash {
            return Err(EngineErr::idem_mismatch());
        }
        return Ok(prior.accepted);
    }

    // First time for this key: run the verb (mutates the in-memory log), then record
    // the idempotent outcome, then persist ONCE (atomic: both records or neither).
    let accepted = run(&mut log, principal_chain, at)?;
    idem_record(
        &mut log, &principal, verb, idem_key, &body_hash, &accepted, at,
    )?;
    sink.persist(repo, &log)?;
    Ok(accepted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// An in-memory sink over a single repo's log — the testable `LogSink`.
    struct MemSink {
        log: RefCell<EventLog>,
    }
    impl LogSink for MemSink {
        fn load(&self, _repo: &str) -> Result<EventLog, EngineErr> {
            Ok(self.log.borrow().clone())
        }
        fn persist(&self, _repo: &str, log: &EventLog) -> Result<(), EngineErr> {
            *self.log.borrow_mut() = log.clone();
            Ok(())
        }
    }
    // RefCell isn't Sync; for single-threaded tests we only need the trait shape.
    // SAFETY: tests are single-threaded; this unblocks `&dyn LogSink` use.
    unsafe impl Sync for MemSink {}

    fn sink_with_pr(pr_id: &str) -> MemSink {
        let mut log = EventLog::new();
        let payload = hugit_refstore::canonical_json(
            &serde_json::json!({
                "author_kind":"orchestrator","campaign":"c","intent_ids":["i"],
                "pr_id":pr_id,"principal":null,"run_id":"r"
            })
            .to_string(),
        )
        .unwrap();
        log.append_for_test("pr.opened", vec!["orchestrator:t".into()], payload, 1);
        MemSink {
            log: RefCell::new(log),
        }
    }

    /// A trivial verb that appends one record and returns a fixed Accepted.
    fn dummy_verb(
        log: &mut EventLog,
        principals: Vec<String>,
        at: u64,
    ) -> Result<Accepted, EngineErr> {
        let rec = log
            .append_authorized(
                hugit_refstore::PrincipalClass::Orchestrator,
                hugit_refstore::Endpoint::Land,
                "pr.queued",
                principals,
                r#"{"pr_id":"1"}"#,
                at,
            )
            .map_err(|d| EngineErr::unavailable(d.reason.code().to_string()))?;
        Ok(Accepted {
            seq: rec.seq,
            note: "ok".into(),
            extra: None,
            queue_pos: Some(1),
            pr_number: Some(1),
            branch: None,
            state: None,
            charter_preview: None,
        })
    }

    #[test]
    fn absent_idem_key_is_400() {
        let sink = sink_with_pr("1");
        let err = with_write(
            &sink,
            "r",
            "land",
            "",
            b"{}",
            true,
            vec!["o".into()],
            2,
            dummy_verb,
        )
        .expect_err("empty key must 400");
        assert_eq!(err.status, 400);
        assert_eq!(err.code, "IDEMPOTENCY_REQUIRED");
    }

    #[test]
    fn replay_same_key_same_body_is_byte_identical_no_double_effect() {
        let sink = sink_with_pr("1");
        let body = b"{\"mode\":\"union\"}";
        let first = with_write(
            &sink,
            "r",
            "land",
            "K1",
            body,
            true,
            vec!["o".into()],
            2,
            dummy_verb,
        )
        .expect("first ok");
        // A replay (lost-response retry) must return the SAME outcome and NOT append
        // a second pr.queued — the land one-position invariant.
        let replay = with_write(
            &sink,
            "r",
            "land",
            "K1",
            body,
            true,
            vec!["o".into()],
            3,
            dummy_verb,
        )
        .expect("replay ok");
        assert_eq!(first.seq, replay.seq, "replay must return the original seq");
        let queued = sink
            .log
            .borrow()
            .records()
            .iter()
            .filter(|r| r.kind == "pr.queued")
            .count();
        assert_eq!(queued, 1, "exactly ONE pr.queued despite the replay");
    }

    #[test]
    fn same_key_different_body_is_409() {
        let sink = sink_with_pr("1");
        with_write(
            &sink,
            "r",
            "land",
            "K1",
            b"{\"mode\":\"union\"}",
            true,
            vec!["o".into()],
            2,
            dummy_verb,
        )
        .expect("first ok");
        let err = with_write(
            &sink,
            "r",
            "land",
            "K1",
            b"{\"mode\":\"serial\"}",
            true,
            vec!["o".into()],
            3,
            dummy_verb,
        )
        .expect_err("different body must 409");
        assert_eq!(err.status, 409);
        assert_eq!(err.code, "IDEM_MISMATCH");
    }

    #[test]
    fn step_up_verb_without_fresh_auth_is_403() {
        let sink = sink_with_pr("1");
        // `policy` is a STEP_UP verb; step_up_presented=false ⇒ 403 before any effect.
        let err = with_write(
            &sink, "r", "policy", "K1", b"{}", false, vec!["o".into()], 2, dummy_verb,
        )
        .expect_err("step-up verb without fresh auth must 403");
        assert_eq!(err.status, 403);
        assert_eq!(err.code, "STEP_UP_REQUIRED");
        // A non-step-up verb is unaffected by step_up_presented=false.
        assert!(
            with_write(&sink, "r", "land", "K2", b"{}", false, vec!["o".into()], 3, dummy_verb)
                .is_ok()
        );
    }

    #[test]
    fn oversize_body_is_400() {
        let sink = sink_with_pr("1");
        let big = vec![b'x'; MAX_BODY_BYTES + 1];
        let err = with_write(
            &sink, "r", "land", "K1", &big, true, vec!["o".into()], 2, dummy_verb,
        )
        .expect_err("oversize body must 400");
        assert_eq!(err.status, 400);
        assert_eq!(err.code, "INVALID_REQUEST");
    }
}
