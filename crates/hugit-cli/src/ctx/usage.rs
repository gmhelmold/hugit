//! `hugit ctx usage --intent <id> --model <model_id> --input N --output N
//! --cache-read N --cache-write N [--model-digest H] [--recorded-at <ms>]
//! [--log <path>]` (or `--pr <id>` as the target alias) — record an authoring
//! run's REAL token usage onto the canonical event log at authoring finish
//! (WP-COST-2).
//!
//! # Why this verb exists — the cost-killer's authoring source
//!
//! The cost-killer prices a landed intent as `tokens × exact rate` (Option A)
//! and submits the figure on close. Nothing captured the authoring agent's REAL
//! token spend before this verb: dogfood is honest-zero, `--dispatch` submits
//! zeros, and the `--tokens`/`--cost-usd-micros` land flags are hand-typed, not
//! measured. This verb is the CAPTURE seam: the authoring harness reads the LLM
//! provider's `/usage` figures at the end of a run and records them here,
//! VERBATIM. If the harness never calls it, no `ctx.usage` record exists and
//! land stays honest-zero — a real figure is never fabricated.
//!
//! # hugit records, it does NOT price
//!
//! This verb computes NOTHING but the trivially-consistent `total`
//! (`input + output + cache_read + cache_write`, the [`TokenCounts`] identity).
//! It performs NO network call and NO pricing — pricing (`tokens × rate`) is
//! WP-COST-1 / land's job. The provider figures land as-submitted.
//!
//! # Append-only ACCUMULATE (the lead's decision)
//!
//! Multiple `ctx.usage` records for one target are BOTH kept (append-only); land
//! SUMS them. This verb never dedups, overwrites, or merges — a second call for
//! the same intent appends a second record.
//!
//! # Fail-closed + structural secret-scrub
//!
//! Token counts are `u64` (clap rejects negatives / non-numeric with exit 2);
//! the `total` is checked for overflow (fail-closed). The target id, model id,
//! and optional model digest are ADDRESSES, not free text — they are routed
//! through [`crate::porcelain::structural_secret_scrub`] (a prefixed / JWT / PEM
//! / conn-string / high-entropy-blob secret REDACTS; a real slug / digest
//! address SURVIVES) BEFORE the bytes reach the forever hash-chained log, so a
//! secret-shaped model or id can never land verbatim.
//!
//! # Serve parity + the `ctx.usage` kind
//!
//! No serve verb exists yet; this CLI verb FREEZES the `ctx.usage` kind as a
//! CLI/refstore-local constant ([`CTX_USAGE_KIND`]) — it does NOT live in
//! `hugit-contracts` (a concurrent WP owns that crate). A future serve mirror
//! can reuse the constant with zero interface change.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::campaign::CampaignError;
use crate::campaign::world::{World, persist_log};
use crate::porcelain::structural_secret_scrub;

/// The event kind this verb appends. Declared CLI/refstore-locally (NOT in
/// `hugit-contracts`) — a concurrent WP owns that crate.
pub const CTX_USAGE_KIND: &str = "ctx.usage";

/// The provider-usage source marker recorded on every `ctx.usage` payload — the
/// figures came from the LLM provider's `/usage` field, recorded verbatim.
const USAGE_SOURCE: &str = "provider_usage";

/// The orchestrator principal that records an authoring-usage capture (the
/// authoring harness runs AS the orchestrator).
const USAGE_PRINCIPAL: &str = "orchestrator:hugit";

/// Arguments for `hugit ctx usage`.
#[derive(clap::Args, Debug)]
pub struct UsageArgs {
    /// Path to the JSON event log. Defaults to $HUGIT_LOG, else .hugit/log.json.
    /// Read, then rewritten with the appended `ctx.usage` record.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,

    /// The intent id this usage is recorded against (the record target). Exactly
    /// one of `--intent` / `--pr` must be given.
    #[arg(long)]
    pub intent: Option<String>,

    /// Alias target: record the usage against a PR id instead of an intent id.
    /// Exactly one of `--intent` / `--pr` must be given.
    #[arg(long)]
    pub pr: Option<String>,

    /// The model id the usage was measured on (e.g. `claude-opus-4-8`).
    #[arg(long)]
    pub model: String,

    /// Optional content-address digest pinning the exact model build.
    #[arg(long)]
    pub model_digest: Option<String>,

    /// Input tokens (non-cached), read from the provider's `/usage`.
    #[arg(long)]
    pub input: u64,

    /// Output tokens, read from the provider's `/usage`.
    #[arg(long)]
    pub output: u64,

    /// Tokens read from the prompt cache.
    #[arg(long)]
    pub cache_read: u64,

    /// Tokens written to the prompt cache.
    #[arg(long)]
    pub cache_write: u64,

    /// Optional authoring-finish timestamp (unix ms) stamped on the payload.
    /// Defaults to `0` — the canonical forever-log is clock-untrusted (time is
    /// not part of the content address), so the default keeps the record
    /// deterministic; a caller MAY pass the provider's real finish time.
    #[arg(long)]
    pub recorded_at: Option<u64>,
}

/// Run `hugit ctx usage` — append a `ctx.usage` record, exit 0 on success or
/// exit 2 on a structured domain error (the WB0 one-exit law).
pub fn run(args: UsageArgs) -> ExitCode {
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

fn do_run(args: UsageArgs) -> Result<String, CampaignError> {
    use hugit_refstore::{Endpoint, PrincipalClass};

    // ── Exactly one target (intent XOR pr) — a missing/ambiguous target is a
    // porcelain error (exit 2), never a partial record. ────────────────────────
    let (raw_target, target_kind) = match (args.intent.as_deref(), args.pr.as_deref()) {
        (Some(i), None) => (i, "intent"),
        (None, Some(p)) => (p, "pr"),
        (None, None) => {
            return Err(CampaignError::new(
                "missing_target",
                "a ctx.usage capture must name exactly one target",
                "pass --intent <id> (or --pr <id> as the alias target)",
            ));
        }
        (Some(_), Some(_)) => {
            return Err(CampaignError::new(
                "ambiguous_target",
                "--intent and --pr are mutually exclusive (exactly one target)",
                "pass only one of --intent / --pr",
            ));
        }
    };

    // ── Structural secret-scrub the id/model/digest at the write boundary
    // (identifier treatment: a prefixed/JWT/PEM/conn-string/high-entropy-blob
    // secret REDACTS, a real slug/digest address SURVIVES). A secret-shaped
    // model or id can never land verbatim on the forever-log. ───────────────────
    let target_id = structural_secret_scrub(raw_target);
    let model = structural_secret_scrub(&args.model);
    let model_digest = args.model_digest.as_deref().map(structural_secret_scrub);

    // ── Compute the total token count — fail-closed on overflow (never a wrong
    // or panicking figure). Consistent with the TokenCounts identity. ───────────
    let total = args
        .input
        .checked_add(args.output)
        .and_then(|s| s.checked_add(args.cache_read))
        .and_then(|s| s.checked_add(args.cache_write))
        .ok_or_else(|| {
            CampaignError::new(
                "token_overflow",
                "the token counts sum beyond u64::MAX",
                "check the --input/--output/--cache-read/--cache-write figures from the provider",
            )
        })?;

    let recorded_at = args.recorded_at.unwrap_or(0);

    // Resolve the default --log ($HUGIT_LOG → .hugit/log.json) once.
    let log_path = crate::log_resolve::resolve_log(args.log.clone());

    // ── Lock BEFORE load (WC1) — bootstrap=false: a capture requires an existing
    // log (a missing --log is `log_not_found`/exit-2, never a ghost record).
    // `_lock` is held across append→persist until scope end. ────────────────────
    let (_lock, world) = World::lock_and_load(&log_path, false)?;

    // ── Build the payload. The id/model/digest values are ALREADY structurally
    // scrubbed above; `source`/`recorded_at`/`tokens` carry no user free text.
    // The bytes are canonicalised directly (sorted keys, no whitespace — the
    // hash-chain byte shape). ───────────────────────────────────────────────────
    let mut payload_value = json!({
        "target_id": target_id,
        "target_kind": target_kind,
        "source": USAGE_SOURCE,
        "model": model,
        "recorded_at": recorded_at,
        "tokens": {
            "input": args.input,
            "output": args.output,
            "cache_read": args.cache_read,
            "cache_write": args.cache_write,
            "total": total,
        },
    });
    if let Some(d) = &model_digest {
        payload_value["model_digest"] = json!(d);
    }
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());

    // ── Append through the D14 guard (Orchestrator, Land) — the authoring
    // harness is an orchestrator-driven integration record, the same cell as
    // `journal.note`. ───────────────────────────────────────────────────────────
    let principal_chain = vec![USAGE_PRINCIPAL.to_string()];
    let mut log = world.log.clone();
    let record = log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            CTX_USAGE_KIND,
            principal_chain,
            payload,
            recorded_at,
        )
        .map_err(|denied| {
            CampaignError::new(
                "authz_denied",
                format!(
                    "ctx.usage append denied by D14 guard: {}",
                    denied.reason.code()
                ),
                "a usage capture must be recorded by an orchestrator principal (Orchestrator/Land)",
            )
        })?;

    // ── Atomic persist (WC1) ────────────────────────────────────────────────────
    persist_log(&log_path, &log)?;

    // ── Stable JSON success envelope (echo the SCRUBBED values) ─────────────────
    let mut out = json!({
        "kind": CTX_USAGE_KIND,
        "seq": record.seq,
        "target_id": target_id,
        "target_kind": target_kind,
        "model": model,
        "tokens": {
            "input": args.input,
            "output": args.output,
            "cache_read": args.cache_read,
            "cache_write": args.cache_write,
            "total": total,
        },
    });
    if let Some(d) = &model_digest {
        out["model_digest"] = json!(d);
    }
    Ok(out.to_string())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-ctx-usage-{}-{}-{:?}",
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

    fn args(log: &std::path::Path) -> UsageArgs {
        UsageArgs {
            log: Some(log.to_path_buf()),
            intent: Some("X".to_string()),
            pr: None,
            model: "claude-opus-4-8".to_string(),
            model_digest: None,
            input: 1000,
            output: 500,
            cache_read: 200,
            cache_write: 0,
            recorded_at: None,
        }
    }

    #[test]
    fn usage_appends_a_well_formed_ctx_usage_record() {
        let log = scratch("ok");
        let before = records(&log).len();

        let result = do_run(args(&log)).expect("usage ok");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["kind"], CTX_USAGE_KIND);
        assert_eq!(v["target_id"], "X");
        assert_eq!(v["model"], "claude-opus-4-8");
        assert_eq!(v["tokens"]["total"], 1700, "total = 1000+500+200+0");
        assert!(v["seq"].is_u64());

        let after = records(&log);
        assert_eq!(after.len(), before + 1, "exactly one record appended");
        let rec = after.last().unwrap();
        assert_eq!(rec["kind"], CTX_USAGE_KIND);
        let payload: serde_json::Value =
            serde_json::from_str(rec["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["target_id"], "X");
        assert_eq!(payload["target_kind"], "intent");
        assert_eq!(payload["source"], "provider_usage");
        assert_eq!(payload["model"], "claude-opus-4-8");
        assert_eq!(payload["tokens"]["input"], 1000);
        assert_eq!(payload["tokens"]["output"], 500);
        assert_eq!(payload["tokens"]["cache_read"], 200);
        assert_eq!(payload["tokens"]["cache_write"], 0);
        assert_eq!(payload["tokens"]["total"], 1700);
        assert!(payload.get("recorded_at").is_some());
    }

    #[test]
    fn two_calls_for_one_intent_append_two_records() {
        let log = scratch("accumulate");
        let before = records(&log).len();
        do_run(args(&log)).expect("first ok");
        do_run(args(&log)).expect("second ok");
        let after = records(&log);
        assert_eq!(
            after.len(),
            before + 2,
            "append-only accumulate: two ctx.usage records, land sums them"
        );
        for rec in after.iter().filter(|r| r["kind"] == CTX_USAGE_KIND) {
            let payload: serde_json::Value =
                serde_json::from_str(rec["payload"].as_str().unwrap()).unwrap();
            assert_eq!(payload["tokens"]["total"], 1700);
        }
    }

    #[test]
    fn pr_alias_targets_the_record() {
        let log = scratch("pralias");
        let mut a = args(&log);
        a.intent = None;
        a.pr = Some("pr-42".to_string());
        let result = do_run(a).expect("pr alias ok");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["target_id"], "pr-42");
        assert_eq!(v["target_kind"], "pr");
    }

    #[test]
    fn missing_target_is_a_porcelain_error_with_no_record() {
        let log = scratch("notarget");
        let before = records(&log).len();
        let mut a = args(&log);
        a.intent = None;
        a.pr = None;
        let err = do_run(a).expect_err("missing target must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "missing_target");
        assert!(v["error"]["fix"].is_string(), "actionable fix");
        assert_eq!(records(&log).len(), before, "nothing appended");
    }

    #[test]
    fn both_targets_is_ambiguous_with_no_record() {
        let log = scratch("both");
        let before = records(&log).len();
        let mut a = args(&log);
        a.pr = Some("pr-1".to_string()); // intent already Some("X")
        let err = do_run(a).expect_err("ambiguous target must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "ambiguous_target");
        assert_eq!(records(&log).len(), before, "nothing appended");
    }

    #[test]
    fn token_sum_overflow_fails_closed() {
        let log = scratch("overflow");
        let before = records(&log).len();
        let mut a = args(&log);
        a.input = u64::MAX;
        a.output = 1;
        let err = do_run(a).expect_err("overflow must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "token_overflow");
        assert_eq!(records(&log).len(), before, "nothing appended on overflow");
    }

    #[test]
    fn missing_log_is_log_not_found() {
        let dir =
            std::env::temp_dir().join(format!("hugit-ctx-usage-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let absent = dir.join("no-such.json");
        let mut a = args(&absent);
        a.log = Some(absent.clone());
        let err = do_run(a).expect_err("missing log must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "log_not_found");
    }

    #[test]
    fn secret_shaped_model_is_scrubbed_before_appending() {
        let log = scratch("secretmodel");
        let pat = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let mut a = args(&log);
        a.model = pat.to_string();
        let result = do_run(a).expect("still appends");
        assert!(!result.contains(pat), "PAT must not appear in success JSON");
        let raw = std::fs::read_to_string(&log).unwrap();
        assert!(!raw.contains(pat), "PAT must not appear on the log");
        assert!(
            raw.contains(hugit_ledger::redact::REDACTED),
            "the REDACTED sentinel must be present"
        );
    }

    #[test]
    fn secret_shaped_target_id_is_scrubbed() {
        let log = scratch("secretid");
        let pat = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let mut a = args(&log);
        a.intent = Some(pat.to_string());
        let result = do_run(a).expect("still appends");
        assert!(!result.contains(pat), "PAT must not appear in success JSON");
        let raw = std::fs::read_to_string(&log).unwrap();
        assert!(!raw.contains(pat), "PAT must not appear on the log");
    }

    #[test]
    fn real_digest_survives_the_scrub() {
        let log = scratch("digest");
        let digest = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let mut a = args(&log);
        a.model_digest = Some(digest.to_string());
        let result = do_run(a).expect("ok");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            v["model_digest"], digest,
            "a real content-address digest survives verbatim"
        );
    }
}
