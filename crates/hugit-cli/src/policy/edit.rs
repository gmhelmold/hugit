//! `hugit policy edit --log <path> --gate <id> (--enable|--disable) [--principal
//! <p>]` — toggle a house gate's enabled state and record it.
//!
//! Mutates the declarative gate set and appends a Human-only `policy.change`
//! event onto the canonical log. The current gate set is reconstructed by folding
//! prior `policy.change` events over the [`hugit_policy::house_gates`] baseline
//! (latest `new` wins); the named gate's `enabled` flag is toggled; the
//! before/after gate lists are recorded as the `{old, new}` payload — the same
//! wire shape [`hugit_policy::emit_policy_change`] produces, appended through the
//! D14 `(Human, Endpoint::Policy)` guard (the policy row is **Human-only**; an
//! orchestrator/agent/model principal is denied fail-closed).
//!
//! `policy` is a closed house set (`dco` / `changelog` / `secrets`); an unknown
//! `--gate` id is rejected (`unknown_gate`/exit-2) — the engine has no evaluator
//! for it and would fail-closed `Blocked`. A no-op edit (the gate is already in
//! the requested state) records nothing and returns `"changed": false`.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::campaign::CampaignError;
use crate::campaign::world::{World, persist_log};
use crate::porcelain::scrub_to_canonical;

use hugit_policy::{GateDescriptor, POLICY_CHANGE_KIND, house_gates};

/// Arguments for `hugit policy edit`.
#[derive(clap::Args, Debug)]
pub struct EditArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`).
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,

    /// The gate id to toggle (`dco` | `changelog` | `secrets`).
    #[arg(long)]
    pub gate: String,

    /// Enable the gate.
    #[arg(long, conflicts_with = "disable")]
    pub enable: bool,

    /// Disable the gate.
    #[arg(long, conflicts_with = "enable")]
    pub disable: bool,

    /// The human principal making the change (must be `user:`/`human:` — policy
    /// is Human-only). Defaults to `user:cli`.
    #[arg(long)]
    pub principal: Option<String>,
}

/// Run `hugit policy edit` — exit 0 on success (recorded or no-op), exit 2 on a
/// structured domain error.
pub fn run(args: EditArgs) -> ExitCode {
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

fn do_run(args: EditArgs) -> Result<String, CampaignError> {
    use hugit_refstore::{Endpoint, PrincipalClass};

    // ── Exactly one of --enable / --disable ──────────────────────────────────
    let target_enabled = match (args.enable, args.disable) {
        (true, false) => true,
        (false, true) => false,
        _ => {
            return Err(CampaignError::new(
                "invalid_argument",
                "pass exactly one of --enable or --disable",
                "e.g. `hugit policy edit --log L --gate dco --disable`",
            ));
        }
    };

    // The principal is an identifier; scrub defensively (a normal `user:name`
    // passes untouched). Default to the human-at-the-terminal principal.
    let principal = args
        .principal
        .as_deref()
        .map(crate::redaction::scrub)
        .unwrap_or_else(|| "user:cli".to_string());

    // ── Fail-closed principal gate BEFORE the D14 append ─────────────────────
    // policy is Human-only (authz matrix, Endpoint::Policy). The D14 guard below
    // is caller-asserted (`PrincipalClass::Human`); a non-human `--principal`
    // would otherwise be recorded as the author of a Human-only change, breaking
    // the audit trail's honesty. Classify the label here and refuse any actor
    // that is not a `user:`/`human:` identity, mirroring `undo`'s chain-classified
    // route. An unrecognized principal is refused too (never defaulted to Human).
    if PrincipalClass::classify(&principal) != Some(PrincipalClass::Human) {
        return Err(CampaignError::new(
            "authz_denied",
            "policy.edit denied: policy is Human-only and the given principal is not a human",
            "policy edits must be issued by a human (`--principal user:<name>` or \
             `human:<name>`); an orchestrator/agent/model principal is refused fail-closed",
        ));
    }

    // Resolve the default --log ($HUGIT_LOG → .hugit/log.json) once.
    let log_path = crate::log_resolve::resolve_log(args.log.clone());

    // ── Lock BEFORE load (WC1); bootstrap=false (a policy edit needs a log) ───
    let (_lock, world) = World::lock_and_load(&log_path, false)?;

    // ── Reconstruct the current gate set: fold policy.change over the baseline ─
    // Start from the house baseline; each policy.change's `new` (a serialised
    // gate list) replaces the current set, latest-wins (the log is append-only +
    // monotonic). An edit only ever toggles `enabled`, never adds/removes ids, so
    // the folded set always carries exactly the house gate ids.
    let mut current = house_gates();
    for rec in world
        .log
        .records()
        .iter()
        .filter(|r| r.kind == POLICY_CHANGE_KIND)
    {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&rec.payload)
            && let Some(new_str) = v.get("new").and_then(|n| n.as_str())
            && let Ok(gates) = serde_json::from_str::<Vec<GateDescriptor>>(new_str)
        {
            current = gates;
        }
    }

    // ── Locate the gate (closed house set — unknown id is rejected) ───────────
    let Some(idx) = current.iter().position(|g| g.id == args.gate) else {
        let safe_gate = crate::redaction::scrub(&args.gate);
        let known: Vec<&str> = current.iter().map(|g| g.id.as_str()).collect();
        return Err(CampaignError::new(
            "unknown_gate",
            format!("'{safe_gate}' is not a house gate"),
            "the house gate set is dco | changelog | secrets",
        )
        .with_context("known_gates", json!(known)));
    };

    // ── No-op short-circuit: already in the requested state → record nothing ──
    if current[idx].enabled == target_enabled {
        return Ok(json!({
            "gate": args.gate,
            "enabled": target_enabled,
            "changed": false,
        })
        .to_string());
    }

    // ── Build old/new gate lists, toggle the flag ─────────────────────────────
    let old = current.clone();
    current[idx].enabled = target_enabled;
    let new = current;
    let old_json = serde_json::to_string(&old).map_err(|e| {
        CampaignError::new(
            "internal",
            format!("serialise gates: {e}"),
            "report this bug",
        )
    })?;
    let new_json = serde_json::to_string(&new).map_err(|e| {
        CampaignError::new(
            "internal",
            format!("serialise gates: {e}"),
            "report this bug",
        )
    })?;

    // ── Append the policy.change through the D14 Human-only guard ─────────────
    // Same `{old, new}` wire shape as `hugit_policy::emit_policy_change`, same
    // `(Human, Endpoint::Policy)` cell — a non-human principal is denied
    // fail-closed (the guard writes an `authz.denied` audit record).
    let payload = scrub_to_canonical(json!({ "old": old_json, "new": new_json }));
    let mut log = world.log.clone();
    let record = log
        .append_authorized(
            PrincipalClass::Human,
            Endpoint::Policy,
            POLICY_CHANGE_KIND,
            vec![principal.clone()],
            payload,
            0,
        )
        .map_err(|denied| {
            CampaignError::new(
                "authz_denied",
                format!(
                    "policy.change denied by D14 guard: {} (policy is Human-only)",
                    denied.reason.code()
                ),
                "policy edits must be made by a human (`--principal user:<name>` or \
                 `human:<name>`); an orchestrator/agent/model principal is refused",
            )
        })?;

    // ── Atomic persist (WC1) ──────────────────────────────────────────────────
    persist_log(&log_path, &log)?;

    Ok(json!({
        "gate": args.gate,
        "enabled": target_enabled,
        "changed": true,
        "seq": record.seq,
    })
    .to_string())
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-policy-edit-{}-{}-{:?}",
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

    /// Disabling a gate records a `policy.change` whose `new` list has that gate
    /// disabled; a subsequent fold reflects it.
    #[test]
    fn disable_records_policy_change() {
        let log = scratch("disable");
        let before = records(&log).len();
        let out = do_run(EditArgs {
            log: Some(log.clone()),
            gate: "dco".to_string(),
            enable: false,
            disable: true,
            principal: Some("user:alice".to_string()),
        })
        .expect("disable ok");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["gate"], "dco");
        assert_eq!(v["enabled"], false);
        assert_eq!(v["changed"], true);

        let after = records(&log);
        assert_eq!(after.len(), before + 1, "one policy.change appended");
        let rec = after.last().unwrap();
        assert_eq!(rec["kind"], POLICY_CHANGE_KIND);
        let payload: serde_json::Value =
            serde_json::from_str(rec["payload"].as_str().unwrap()).unwrap();
        let new: Vec<GateDescriptor> =
            serde_json::from_str(payload["new"].as_str().unwrap()).unwrap();
        let dco = new.iter().find(|g| g.id == "dco").unwrap();
        assert!(!dco.enabled, "dco must be disabled in the recorded new set");
    }

    /// A second edit folds over the first: re-enabling dco after disabling it
    /// produces an enabled dco (latest-wins fold is correct).
    #[test]
    fn second_edit_folds_over_first() {
        let log = scratch("fold");
        do_run(EditArgs {
            log: Some(log.clone()),
            gate: "dco".to_string(),
            enable: false,
            disable: true,
            principal: None,
        })
        .expect("disable");
        // Now re-enable — the fold must see dco currently disabled and flip it.
        let out = do_run(EditArgs {
            log: Some(log.clone()),
            gate: "dco".to_string(),
            enable: true,
            disable: false,
            principal: None,
        })
        .expect("re-enable");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["enabled"], true);
        assert_eq!(v["changed"], true);
    }

    /// Editing a gate already in the requested state records nothing.
    #[test]
    fn noop_edit_records_nothing() {
        let log = scratch("noop");
        let before = records(&log).len();
        // dco is enabled by default; --enable is a no-op.
        let out = do_run(EditArgs {
            log: Some(log.clone()),
            gate: "dco".to_string(),
            enable: true,
            disable: false,
            principal: None,
        })
        .expect("noop ok");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["changed"], false);
        assert_eq!(records(&log).len(), before, "no record on a no-op edit");
    }

    /// An unknown gate id is rejected with `unknown_gate`/exit-2.
    #[test]
    fn unknown_gate_is_rejected() {
        let log = scratch("unknown");
        let err = do_run(EditArgs {
            log: Some(log.clone()),
            gate: "nonsense".to_string(),
            enable: false,
            disable: true,
            principal: None,
        })
        .expect_err("unknown gate must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "unknown_gate");
    }

    /// Neither --enable nor --disable → invalid_argument.
    #[test]
    fn missing_enable_disable_is_invalid() {
        let log = scratch("noflag");
        let err = do_run(EditArgs {
            log: Some(log),
            gate: "dco".to_string(),
            enable: false,
            disable: false,
            principal: None,
        })
        .expect_err("must require a direction");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "invalid_argument");
    }

    /// A missing `--log` is `log_not_found`/exit-2.
    #[test]
    fn missing_log_is_log_not_found() {
        let dir =
            std::env::temp_dir().join(format!("hugit-policy-edit-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = do_run(EditArgs {
            log: Some(dir.join("no-such.json")),
            gate: "dco".to_string(),
            enable: false,
            disable: true,
            principal: None,
        })
        .expect_err("missing log must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "log_not_found");
    }

    /// A non-human `--principal` is refused fail-closed BEFORE any append — the
    /// recorded author of a `policy.change` is always a `user:`/`human:`
    /// identity (policy is Human-only). Regression: `agent:<id>` used to pass
    /// the caller-asserted D14 guard and be recorded verbatim, breaking the
    /// audit trail's honesty.
    #[test]
    fn non_human_principal_is_authz_denied_before_append() {
        for actor in [
            "agent:runner",
            "orchestrator:opus",
            "model:claude",
            "alien:x",
        ] {
            let log = scratch(&format!("deny-{actor}"));
            let before = records(&log).len();
            let err = do_run(EditArgs {
                log: Some(log.clone()),
                gate: "dco".to_string(),
                enable: false,
                disable: true,
                principal: Some(actor.to_string()),
            })
            .expect_err("non-human principal must be refused");
            let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
            assert_eq!(v["error"]["kind"], "authz_denied", "actor {actor}");
            assert_eq!(
                records(&log).len(),
                before,
                "no record appended for refused actor {actor}"
            );
        }
    }

    /// An unrecognized principal must never be defaulted to Human.
    #[test]
    fn unrecognized_principal_is_never_defaulted_to_human() {
        let log = scratch("unnamed");
        let before = records(&log).len();
        let err = do_run(EditArgs {
            log: Some(log.clone()),
            gate: "dco".to_string(),
            enable: false,
            disable: true,
            principal: Some("nobody".to_string()),
        })
        .expect_err("unrecognized principal must be refused");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "authz_denied");
        assert_eq!(records(&log).len(), before, "no append on refusal");
    }
}
