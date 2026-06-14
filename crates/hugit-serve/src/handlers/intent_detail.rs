//! `GET /v1/repos/{repo}/intents/{id}` → [`IntentDetailVm`] (None = 404).
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: id/repo/charter/
//! title/status/commit_hash/verdicts/pr_number from intent + ledger + PR.
//! REAL (envelope-gated): authorship / metrics / snapshot / trajectory / CAS
//! refs / acceptance when an `intent.envelope` for `id` exists, else honest
//! defaults. STUB (no seam): diff, *_mono_terms, transcripts, journal,
//! context_json. Every free-text field passes `crate::fmt::scrub`.

use crate::fmt::{scrub, scrub_all, sha_prefix};
use hugit_cli::pr::{INTENT_ENVELOPE_KIND, PR_OPENED_KIND};
use hugit_contracts::context_envelope::{Altitude, ContextEnvelope};
use hugit_http_contracts::common::{CampaignChipVm, DiffVm, VerdictVm};
use hugit_http_contracts::intent_detail::{
    AuthorshipVm, EnvelopeAltitudeVm, IntentDetailVm, MetricsVm, SnapshotVm,
};
use hugit_ledger::Ledger;
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;
use serde_json::Value;

/// Build the intent-detail view-model for intent `id`. `None` → 404 (no leak).
pub fn build_intent_detail(log: &EventLog, repo: &str, id: &str) -> Option<IntentDetailVm> {
    // REAL: the intent must exist (else 404).
    let intent_log = intents_from_log(log).ok()?;
    let intent = intent_log.by_id(id)?;

    let charter = scrub(&intent.charter);
    let title = scrub(intent.charter.lines().next().unwrap_or(""));
    let commit_hash = sha_prefix(&intent.target, 6); // structural — not scrubbed

    // REAL: status + verdicts from the ledger entry.
    let ledger = Ledger::from_records(log.records());
    let ledger_entry = ledger
        .entries()
        .iter()
        .find(|e| e.intent_id == id || e.seq == intent.seq);
    let status = match ledger_entry {
        Some(e) if e.rejected => "REJECTED".to_string(),
        Some(e) if e.proven => "PROVEN".to_string(),
        _ => "LANDED".to_string(),
    };
    let verdicts: Vec<VerdictVm> = ledger_entry
        .and_then(|e| e.verdict.as_ref())
        .map(|v| {
            vec![VerdictVm {
                verdict: scrub(&v.outcome),
                reviewer: String::new(), // STUB — no reviewer seam at this altitude
                summary: String::new(),  // STUB — no prose summary seam
                adversarial: false,      // STUB — panel flag not in VerdictView
                lens: scrub(&v.lens),
                evidence_mono_terms: v.claims_checked.iter().map(|c| scrub(c)).collect(),
            }]
        })
        .unwrap_or_default();

    // REAL: pr_number — first pr.opened whose intent_ids contains id.
    // REAL: the pr.opened whose intent_ids contains id — yields BOTH pr_number
    // and the campaign chip (one parse, two real fields).
    let opened_pr: Option<Value> = log
        .records()
        .iter()
        .filter(|r| r.kind == PR_OPENED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .find(|v| {
            v.get("intent_ids")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().any(|x| x.as_str() == Some(id)))
                .unwrap_or(false)
        });
    let pr_number: Option<u64> = opened_pr
        .as_ref()
        .and_then(|v| v.get("pr_id").and_then(Value::as_str))
        .and_then(|s| s.parse::<u64>().ok());
    // REAL: the campaign chip from the owning PR (scrubbed id; None when uncampaigned).
    let campaign: Option<CampaignChipVm> = opened_pr
        .as_ref()
        .and_then(|v| v.get("campaign").and_then(Value::as_str))
        .filter(|c| !c.is_empty())
        .map(|c| {
            let s = scrub(c);
            CampaignChipVm {
                id: s.clone(),
                label: s.clone(),
                color_class: String::new(),
                display_label: s,
            }
        });

    // REAL (envelope-gated): the intent-altitude envelope for `id`.
    let env: Option<ContextEnvelope> = log
        .records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .rfind(|e| e.altitude == Altitude::Intent && e.intent_id == id);

    let summary = env
        .as_ref()
        .and_then(|e| e.trajectory.summary.as_deref())
        .map(scrub)
        .unwrap_or_default();
    let acceptance = env
        .as_ref()
        .map(|e| scrub_all(&e.acceptance))
        .unwrap_or_default();

    let authorship = {
        let principal_chain = scrub_all(&intent.principal_chain); // ALWAYS from intent
        match &env {
            Some(e) => AuthorshipVm {
                model: scrub(&e.authorship.model),
                principal_chain,
                operator: scrub(&e.authorship.operator),
                agent_type: e.authorship.agent_type.clone(),
                run_id: e.authorship.spawn.run_id.clone(),
                parent_run_id: e.authorship.spawn.parent_run_id.clone().unwrap_or_default(),
                wall_time_human: humanize_wall_ms(
                    e.authorship
                        .spawn
                        .died_at
                        .saturating_sub(e.authorship.spawn.born_at),
                ),
            },
            None => AuthorshipVm {
                model: String::new(),
                principal_chain,
                operator: String::new(),
                agent_type: String::new(),
                run_id: String::new(),
                parent_run_id: String::new(),
                wall_time_human: String::new(),
            },
        }
    };

    let metrics = match &env {
        Some(e) => MetricsVm {
            tokens: e.metrics.tokens.total,
            wall_ms: e.metrics.wall_ms,
            tool_calls: e.metrics.tool_calls,
            cost_usd_micros: e.metrics.cost_usd_micros,
            active_ms: e.metrics.active_ms,
            model_turns: e.metrics.model_turns,
        },
        None => MetricsVm {
            tokens: 0,
            wall_ms: 0,
            tool_calls: 0,
            cost_usd_micros: 0,
            active_ms: 0,
            model_turns: 0,
        },
    };

    let snapshot = match &env {
        Some(e) => SnapshotVm {
            tree: e.tree_hash.clone(), // structural — not scrubbed
            // env_manifest is free-form prose → scrub at the read boundary (P0).
            toolchain: scrub(&e.snapshot.env_manifest),
            workspace: repo.to_string(),
            files_read: e
                .snapshot
                .files_read
                .iter()
                .map(|f| scrub(&f.path))
                .collect(),
            prompt_policy: e
                .snapshot
                .prompt_ref
                .as_ref()
                .map(|_| "redactado".to_string())
                .unwrap_or_default(),
            ambiente: scrub(&e.snapshot.env_manifest),
        },
        None => SnapshotVm {
            tree: String::new(),
            toolchain: String::new(),
            workspace: String::new(),
            files_read: vec![],
            prompt_policy: String::new(),
            ambiente: String::new(),
        },
    };

    let trajectory: Vec<EnvelopeAltitudeVm> = match &env {
        Some(e) => {
            let mut alts = Vec::new();
            if let Some(cas) = &e.trajectory.task_transcript_ref {
                alts.push(EnvelopeAltitudeVm {
                    label: "resumo compactado".to_string(),
                    desc: "transcript compactado da sessão".to_string(),
                    cas_ref: cas.clone(), // structural CAS ref — not scrubbed
                    body: vec![],
                    priv_note: None,
                    body_tool_terms: vec![],
                    body_res_terms: vec![],
                });
            }
            if let Some(cas) = &e.trajectory.raw_transcript_ref {
                alts.push(EnvelopeAltitudeVm {
                    label: "transcript completo".to_string(),
                    desc: "transcript bruto born → die".to_string(),
                    cas_ref: cas.clone(),
                    body: vec![],
                    priv_note: Some("tenant-private · redactado na escrita".to_string()),
                    body_tool_terms: vec![],
                    body_res_terms: vec![],
                });
            }
            alts
        }
        None => vec![],
    };

    let (context_cas, compact_context_ref, bundle_ref, compact_transcript_ref) = match &env {
        Some(e) => (
            e.snapshot.prompt_ref.clone().unwrap_or_default(),
            String::new(),
            String::new(),
            e.trajectory.task_transcript_ref.clone().unwrap_or_default(),
        ),
        None => (String::new(), String::new(), String::new(), String::new()),
    };

    Some(IntentDetailVm {
        repo: repo.to_string(),
        id: id.to_string(),
        title,
        status,
        pr_number,
        summary,
        charter,
        summary_mono_terms: vec![], // STUB — no term-extraction seam
        charter_mono_terms: vec![], // STUB
        acceptance,
        task_transcript: None, // STUB — CAS blob not fetched (renders "não capturado")
        full_transcript: None, // STUB
        journal: None,         // STUB
        context_json: String::new(), // STUB — pretty context.json not piped
        trajectory,
        context_cas,
        compact_context_ref,
        bundle_ref,
        compact_transcript_ref,
        campaign, // REAL — owning PR's campaign chip (scrubbed); None when uncampaigned
        diff: DiffVm {
            files: vec![],
            hunks: vec![],
        }, // STUB — no diffstat seam
        authorship,
        metrics,
        snapshot,
        verdicts,
        commit_hash,
    })
}

/// Humanize a born→die wall-clock ms span ("14m37s" / "2h05m" / "47s").
fn humanize_wall_ms(ms: u64) -> String {
    let s = ms / 1_000;
    let h = s / 3_600;
    let m = (s % 3_600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{sec:02}s")
    } else {
        format!("{sec}s")
    }
}
