//! `hugit issue transition <n> <to> [--priority <p>]` — move an issue's state.
//!
//! Inline reimplementation of the `issue.transition` write logic for the CLI
//! (self-contained — no server-side dependency; the CLI is the product).
//!
//! Mirrors the canonical server-side transition verb exactly:
//! - the same 4-value `VALID_STATES` slice (`backlog|open|closed|dispatch`),
//! - priority scrubbed via the CLI redaction seam ([`crate::redaction::scrub`]),
//! - appended through `append_authorized(PrincipalClass::Orchestrator,
//!   Endpoint::Land, …)` so the D14 matrix gates the mutation and denials are
//!   audited (the matrix confirms Orchestrator/Land is the only `land` cell
//!   that is `Allow`),
//! - payload built as canonical JSON via
//!   [`crate::porcelain::scrub_to_canonical`] (the WG/WH-SCRUB structural seam
//!   that scrubs every user string BEFORE the bytes reach the hash chain),
//! - persisted atomically via `campaign::world::persist_log` (the one
//!   truncation-proof write path all porcelain verbs share).
//!
//! ## Hermetic file seam
//!
//! Operates on a local `--log <path>` JSON `[EventRecord, …]` array. Live R2
//! binding is the P2 disclosed seam.
//!
//! ## P2 existence gate
//!
//! As in the serve verb, an `issue.transition` may be recorded for issue `n`
//! without a pre-existing issue record (free-standing, Wave-2 semantics). A
//! P2 existence gate slots in with zero interface change.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::campaign::CampaignError;
use crate::campaign::world::{World, persist_log};
use crate::porcelain::scrub_to_canonical;

/// The event kind this verb appends — mirrors `ISSUE_TRANSITION_KIND` in the
/// serve verb.
pub const ISSUE_TRANSITION_KIND: &str = "issue.transition";

/// The closed set of valid target states (mirrors `VALID_STATES` in the serve
/// verb byte-for-byte).
const VALID_STATES: &[&str] = &["backlog", "open", "closed", "dispatch"];

/// Arguments for `hugit issue transition`.
#[derive(clap::Args, Debug)]
pub struct TransitionArgs {
    /// Path to the JSON event log (`[EventRecord, …]`). Read, then rewritten
    /// with the appended `issue.transition` record.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,

    /// The issue number to transition.
    #[arg(long)]
    pub n: u32,

    /// The target state: `backlog | open | closed | dispatch`.
    #[arg(long = "to")]
    pub to: String,

    /// Optional free-text priority label (scrubbed before persisting).
    #[arg(long)]
    pub priority: Option<String>,
}

/// Run `hugit issue transition` — append an `issue.transition` record, exit 0
/// on success or exit 2 on a structured domain error.
pub fn run(args: TransitionArgs) -> ExitCode {
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

fn do_run(args: TransitionArgs) -> Result<String, CampaignError> {
    use hugit_refstore::{Endpoint, PrincipalClass};

    // ── Validate the target state (mirrors the serve verb's 400 guard) ────────
    if !VALID_STATES.contains(&args.to.as_str()) {
        // Scrub the user-supplied `to` value before echoing it in the error
        // message so a smuggled secret never leaks into the error envelope.
        let safe_to = crate::redaction::scrub(&args.to);
        return Err(CampaignError::new(
            "invalid_state",
            format!(
                "invalid target state '{}': must be backlog|open|closed|dispatch",
                safe_to
            ),
            "pass --to backlog, --to open, --to closed, or --to dispatch",
        ));
    }

    // ── Scrub the optional priority (free text → full engine scrub) ───────────
    // Mirrors `req.priority.as_deref().map(scrub)` in the serve verb. The
    // `scrub_to_canonical` call below will also scrub via the structural seam,
    // but we pre-scrub here as defence-in-depth so the echoed success JSON
    // value is also clean.
    let priority: Option<String> = args.priority.as_deref().map(crate::redaction::scrub);

    // ── Lock BEFORE load (WC1 discipline) ────────────────────────────────────
    // `bootstrap = false`: a transition requires the log to already exist (an
    // issue cannot be transitioned before any events are on the log). A missing
    // `--log` is `log_not_found`/exit-2, never a ghost record on a fresh log.
    // `_lock` is held (its Drop releases) until the end of this fn scope — the
    // append→persist critical section. Underscore-prefixed so it is not read but
    // still dropped at scope end (NOT a bare `_`, which would drop immediately).
    let log_path = crate::log_resolve::resolve_log_checked(args.log.clone()).map_err(|e| {
        CampaignError::new(
            e.kind(),
            e.to_json(),
            "repair blocked migration before appending an issue transition",
        )
    })?;
    let (_lock, world) = World::lock_and_load(&log_path, false)?;

    // ── Build + scrub payload (WG/WH-SCRUB structural seam) ──────────────────
    // `issue_id` is a numeric u32 — not a string identifier, so no identifier
    // scrub is needed. `priority` is free text and pre-scrubbed above; it also
    // passes through scrub_to_canonical's recursive string scrub below.
    // `to` is a closed enum value validated above; not free text.
    let payload_value = match &priority {
        Some(p) => json!({"issue_id": args.n, "priority": p, "to": args.to}),
        None => json!({"issue_id": args.n, "to": args.to}),
    };
    // `scrub_to_canonical` applies the full structural scrub tree and returns
    // sorted-key canonical JSON — the same WG-SCRUB seam every porcelain verb
    // uses (the bytes the hash chain covers must be canonical + scrubbed).
    let payload = scrub_to_canonical(payload_value);

    // ── Append through the D14 guard (Orchestrator, Land) ─────────────────────
    // `issue.transition` is an orchestrator-driven landing verb (identical
    // authorization cell to the serve verb). The matrix confirms
    // (Orchestrator, Land) → Allow; any other class would be denied and the
    // denial audited as `authz.denied`.
    let principal_chain = vec!["orchestrator:hugit".to_string()];
    let mut log = world.log.clone();
    let record = log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            ISSUE_TRANSITION_KIND,
            principal_chain,
            payload,
            0,
        )
        .map_err(|denied| {
            CampaignError::new(
                "authz_denied",
                format!(
                    "issue.transition append denied by D14 guard: {}",
                    denied.reason.code()
                ),
                "issue transition must be driven by an orchestrator principal (Orchestrator/Land)",
            )
        })?;

    // ── Atomic persist (WC1 — truncation-proof) ───────────────────────────────
    // `_lock` (bound above) stays held across append→persist until scope end.
    persist_log(&log_path, &log)?;

    // ── Stable JSON success envelope ──────────────────────────────────────────
    // Use the SCRUBBED `priority` local (not raw `args.priority`) so a
    // secret-shaped priority never echoes back unredacted (read-boundary law).
    let out = match &priority {
        Some(p) => json!({
            "issue_id": args.n,
            "to": args.to,
            "priority": p,
            "seq": record.seq,
        }),
        None => json!({
            "issue_id": args.n,
            "to": args.to,
            "seq": record.seq,
        }),
    };
    Ok(out.to_string())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-issue-transition-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join(format!("{tag}.json"));
        // Bootstrap the log with one record so `bootstrap=false` does not
        // fail with `log_not_found`.  Use the canonical `[EventRecord, …]`
        // array shape the porcelain loader expects.
        use hugit_refstore::EventLog;
        let mut el = EventLog::new();
        el.append_for_test("repo.init", vec!["orchestrator:hugit".to_string()], "{}", 0);
        std::fs::write(&log, serde_json::to_string_pretty(el.records()).unwrap()).unwrap();
        log
    }

    fn records(path: &std::path::Path) -> Vec<serde_json::Value> {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    // ── empty tests (compile gate) ────────────────────────────────────────────

    /// The module compiles and the constant mirrors the serve verb.
    #[test]
    fn valid_states_constant_matches_serve_verb() {
        assert_eq!(VALID_STATES, &["backlog", "open", "closed", "dispatch"]);
    }

    /// The event kind constant matches the serve verb's wire kind.
    #[test]
    fn event_kind_matches_serve_verb() {
        assert_eq!(ISSUE_TRANSITION_KIND, "issue.transition");
    }

    // ── populated tests ───────────────────────────────────────────────────────

    /// A valid transition appends an `issue.transition` record with the correct
    /// payload fields and returns stable JSON carrying `seq`.
    #[test]
    fn valid_transition_appends_record_and_returns_seq() {
        let log = scratch("valid");
        let before = records(&log).len();

        let result = do_run(TransitionArgs {
            log: Some(log.clone()),
            n: 42,
            to: "open".to_string(),
            priority: None,
        })
        .expect("transition ok");

        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["issue_id"], 42);
        assert_eq!(v["to"], "open");
        assert!(v["seq"].is_u64());

        let after = records(&log);
        assert_eq!(after.len(), before + 1, "exactly one record appended");
        let rec = after.last().unwrap();
        assert_eq!(rec["kind"], ISSUE_TRANSITION_KIND);
        let payload: serde_json::Value =
            serde_json::from_str(rec["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["issue_id"], 42);
        assert_eq!(payload["to"], "open");
    }

    /// The optional `--priority` field round-trips through the payload and the
    /// success JSON, and the value is scrubbed through the redaction engine.
    #[test]
    fn priority_is_included_in_payload_and_success_json() {
        let log = scratch("priority");

        let result = do_run(TransitionArgs {
            log: Some(log.clone()),
            n: 7,
            to: "dispatch".to_string(),
            priority: Some("high".to_string()),
        })
        .expect("transition with priority ok");

        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["priority"], "high");

        let recs = records(&log);
        let rec = recs.last().unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(rec["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["priority"], "high");
        assert_eq!(payload["to"], "dispatch");
    }

    /// A PAT smuggled as `--priority` is redacted before it reaches the log.
    #[test]
    fn priority_secret_is_redacted_before_appending() {
        let log = scratch("redact-priority");
        let pat = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

        let result = do_run(TransitionArgs {
            log: Some(log.clone()),
            n: 1,
            to: "closed".to_string(),
            priority: Some(pat.to_string()),
        })
        .expect("redacted priority still appends ok");

        // The success JSON must carry the REDACTED sentinel, not the raw PAT.
        assert!(!result.contains(pat), "PAT must not appear in success JSON");

        let recs = records(&log);
        let rec = recs.last().unwrap();
        let raw = rec["payload"].as_str().unwrap();
        assert!(
            !raw.contains(pat),
            "PAT must not appear in the persisted payload"
        );
        assert!(
            raw.contains(hugit_ledger::redact::REDACTED),
            "the REDACTED sentinel must be present in the payload"
        );
    }

    /// An invalid target state is rejected with `invalid_state`/exit-2 and no
    /// record is appended.
    #[test]
    fn invalid_to_returns_invalid_state_error() {
        let log = scratch("invalid-to");
        let before = records(&log).len();

        let err = do_run(TransitionArgs {
            log: Some(log.clone()),
            n: 5,
            to: "in_review".to_string(),
            priority: None,
        })
        .expect_err("invalid state must fail");

        let json = err.to_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["error"]["kind"], "invalid_state");
        assert!(v["error"]["fix"].is_string());
        // No record appended on failure.
        assert_eq!(
            records(&log).len(),
            before,
            "no record appended on bad state"
        );
    }

    /// All four valid states are accepted.
    #[test]
    fn all_valid_states_are_accepted() {
        for state in VALID_STATES {
            let log = scratch(&format!("state-{state}"));
            do_run(TransitionArgs {
                log: Some(log),
                n: 1,
                to: state.to_string(),
                priority: None,
            })
            .unwrap_or_else(|e| panic!("state '{state}' must be accepted: {}", e.to_json()));
        }
    }

    /// A missing `--log` file is rejected with `log_not_found` (never a ghost
    /// record on a non-existent log — `bootstrap=false`).
    #[test]
    fn missing_log_is_log_not_found_not_a_ghost_record() {
        let dir = std::env::temp_dir().join(format!(
            "hugit-issue-transition-missing-{}",
            std::process::id()
        ));
        // Create the PARENT dir so we exercise "log FILE absent" (→ log_not_found),
        // not "parent dir absent" (which fails earlier at lock acquisition as `io`).
        std::fs::create_dir_all(&dir).unwrap();
        let absent = dir.join("no-such.json");

        let err = do_run(TransitionArgs {
            log: Some(absent),
            n: 1,
            to: "open".to_string(),
            priority: None,
        })
        .expect_err("missing log must fail");

        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "log_not_found");
    }

    /// A smuggled PAT in `--to` is scrubbed before appearing in the error
    /// message (the to-value is validated against VALID_STATES and fails, then
    /// echoed scrubbed in the `invalid_state` error).
    #[test]
    fn invalid_to_with_a_pat_does_not_leak_in_error_message() {
        let log = scratch("secret-in-to");
        let pat = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

        let err = do_run(TransitionArgs {
            log: Some(log),
            n: 1,
            to: pat.to_string(),
            priority: None,
        })
        .expect_err("invalid state");

        let json = err.to_json();
        assert!(!json.contains(pat), "PAT must not appear in the error JSON");
        assert!(
            json.contains(hugit_ledger::redact::REDACTED),
            "the REDACTED sentinel must be in the error message"
        );
    }
}
