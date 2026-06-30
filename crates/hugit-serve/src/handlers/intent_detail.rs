//! `GET /v1/repos/{repo}/intents/{id}` → [`IntentDetailVm`] (None = 404).
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: id/repo/charter/
//! title/status/commit_hash/verdicts/pr_number from intent + ledger + PR.
//! REAL (envelope-gated): authorship / metrics / snapshot / trajectory / CAS
//! refs / acceptance when an `intent.envelope` for `id` exists, else honest
//! defaults.
//!
//! REAL (review-legibility wave):
//! - `verdicts.reviewer` = the verdict's `model` + a derived `summary` +
//!   `adversarial: true` (panel-sourced), read from the raw `verdict.recorded`
//!   VerdictObject (the redacted ledger view drops `model`).
//! - `diff` = a real numstat from the intent's commit vs. its first parent, via
//!   the git source (`HUGIT_SERVE_GIT_DIR`); honest-empty with no git source / no
//!   parent.
//! - `task_transcript`/`full_transcript` = the CAS blob fetched by ref when the
//!   ref resolves to a git object in the wired source; honest-`None` otherwise.
//!
//! HONEST-DEFAULT (no seam): `*_mono_terms`, `journal`, `context_json`. Every
//! free-text field passes `crate::fmt::scrub`.

use std::sync::Arc;

use crate::fmt::{scrub, scrub_all, sha_prefix};
use hugit_cli::pr::{INTENT_ENVELOPE_KIND, PR_OPENED_KIND};
use hugit_cli::verdict::VERDICT_RECORDED_KIND;
use hugit_contracts::VerdictObject;
use hugit_contracts::context_envelope::{Altitude, ContextEnvelope};
use hugit_http_contracts::common::{CampaignChipVm, DiffVm, VerdictVm, decision_of};
use hugit_http_contracts::intent_detail::{
    AuthorshipVm, EnvelopeAltitudeVm, IntentDetailVm, MetricsVm, SnapshotVm,
};
use hugit_ledger::Ledger;
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;
use serde_json::Value;

use super::diff::{diff_vm, empty_diff};

/// The git object source threaded in for the diff + transcript-blob seams. `None`
/// = no `HUGIT_SERVE_GIT_DIR` (honest-empty diff, honest-`None` transcripts).
type GitSrc<'a> = Option<&'a Arc<dyn hugit_proto::ObjectSource + Send + Sync>>;

/// Build the intent-detail view-model for intent `id`. `None` → 404 (no leak).
///
/// `src` is the deploy-gated git object source (`HUGIT_SERVE_GIT_DIR`); when
/// present it powers the REAL `diff` (intent commit vs. parent) and the
/// transcript-blob fetch. `None` → those fields are honest defaults.
pub fn build_intent_detail(
    log: &EventLog,
    repo: &str,
    id: &str,
    src: GitSrc<'_>,
) -> Option<IntentDetailVm> {
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
    // REAL (review-legibility ②): prefer the raw `verdict.recorded`
    // VerdictObject records for this intent — they carry the `model` (→ reviewer)
    // the redacted ledger VerdictView drops. Every such record IS an adversarial-
    // panel verdict (the forge's only verdict producer), so `adversarial` is the
    // invariant `true`, matching the review-panel projection in `review.rs`.
    let raw_verdicts = raw_verdict_objects(log, id);
    let verdicts: Vec<VerdictVm> = if !raw_verdicts.is_empty() {
        raw_verdicts.iter().map(verdict_vm_from_object).collect()
    } else {
        // Honest fallback: the redacted ledger VerdictView (no model seam → empty
        // reviewer), but `adversarial: true` is now correct (panel-sourced).
        ledger_entry
            .and_then(|e| e.verdict.as_ref())
            .map(|v| {
                vec![VerdictVm {
                    verdict: scrub(&v.outcome),
                    reviewer: String::new(), // honest — VerdictView drops `model`
                    summary: format!("{} · {}", scrub(&v.lens), scrub(&v.outcome)),
                    adversarial: true, // panel-sourced (was wrongly hardcoded false)
                    lens: scrub(&v.lens),
                    evidence_mono_terms: v.claims_checked.iter().map(|c| scrub(c)).collect(),
                    // REAL — structured, from the canonical (unscrubbed) outcome.
                    decision: decision_of(&v.outcome),
                }]
            })
            .unwrap_or_default()
    };

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
                // payload free-text from the envelope — scrubbed at the read boundary
                // (a real agent-type / uuid survives; a secret-shaped value is redacted).
                agent_type: scrub(&e.authorship.agent_type),
                run_id: scrub(&e.authorship.spawn.run_id),
                parent_run_id: scrub(
                    e.authorship
                        .spawn
                        .parent_run_id
                        .as_deref()
                        .unwrap_or_default(),
                ),
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
                    cas_ref: scrub(cas), // payload ref — scrubbed (a real algo:hex survives)
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
                    cas_ref: scrub(cas), // payload ref — scrubbed (a real algo:hex survives)
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
        // payload refs scrubbed at the read boundary (a real `algo:hex` ref survives).
        Some(e) => (
            scrub(e.snapshot.prompt_ref.as_deref().unwrap_or_default()),
            String::new(),
            String::new(),
            scrub(
                e.trajectory
                    .task_transcript_ref
                    .as_deref()
                    .unwrap_or_default(),
            ),
        ),
        None => (String::new(), String::new(), String::new(), String::new()),
    };

    // REAL (review-legibility ①): the intent's diff — its commit's tree vs. its
    // first parent's tree, walked over the git source. Honest-empty when there is
    // no git source, the commit is absent, or it has no parent (a root commit).
    let diff = intent_diff(src, &intent.target);

    // REAL (review-legibility ③): fetch the transcript blobs by their `cas:` refs
    // when the ref resolves to an object in the wired git source; honest-`None`
    // when the ref is absent or unresolvable (the unwired / live-CAS-less path).
    let task_transcript = env
        .as_ref()
        .and_then(|e| e.trajectory.task_transcript_ref.as_deref())
        .and_then(|r| fetch_transcript(src, r));
    let full_transcript = env
        .as_ref()
        .and_then(|e| e.trajectory.raw_transcript_ref.as_deref())
        .and_then(|r| fetch_transcript(src, r));

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
        task_transcript, // REAL when the ref resolves in the git source; else None
        full_transcript, // REAL when the ref resolves in the git source; else None
        journal: None,   // STUB — journal CAS blob not piped this wave
        context_json: String::new(), // STUB — pretty context.json not piped
        trajectory,
        context_cas,
        compact_context_ref,
        bundle_ref,
        compact_transcript_ref,
        campaign, // REAL — owning PR's campaign chip (scrubbed); None when uncampaigned
        diff,     // REAL (git source) — intent commit vs. parent; else honest-empty
        authorship,
        metrics,
        snapshot,
        verdicts,
        commit_hash,
    })
}

/// The raw `verdict.recorded` VerdictObjects for `intent` (chain order). These
/// carry `model` — which the redacted ledger VerdictView drops.
fn raw_verdict_objects(log: &EventLog, intent: &str) -> Vec<VerdictObject> {
    log.records()
        .iter()
        .filter(|r| r.kind == VERDICT_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
        .filter(|vo| vo.intent == intent)
        .collect()
}

/// Project a raw VerdictObject to a [`VerdictVm`] with `reviewer = model`, a
/// derived `summary`, and `adversarial: true` (every recorded verdict is a panel
/// verdict). Mirrors `review.rs::verdict_pair` so the two surfaces agree.
fn verdict_vm_from_object(vo: &VerdictObject) -> VerdictVm {
    use hugit_contracts::verdict_object::Verdict;
    let outcome = match vo.verdict {
        Verdict::Approve => "APPROVE",
        Verdict::FixFirst => "FIX-FIRST",
        Verdict::Reject => "REJECT",
    };
    let lens = scrub(&vo.lens);
    let evidence_mono_terms: Vec<String> = vo.claims_checked.iter().map(|c| scrub(c)).collect();
    let summary = if evidence_mono_terms.is_empty() {
        format!("{lens} · {outcome}")
    } else {
        format!(
            "{lens} · {outcome} · {} claim(s) checked",
            evidence_mono_terms.len()
        )
    };
    VerdictVm {
        verdict: outcome.to_string(),
        reviewer: scrub(&vo.model), // REAL — the model that produced the verdict
        summary,
        adversarial: true, // panel-sourced invariant (matches review.rs)
        lens,
        evidence_mono_terms,
        decision: decision_of(outcome), // REAL — structured, same outcome source
    }
}

/// The diff for an intent: its commit's tree vs. its first parent's tree. Empty
/// (honest) when there is no git source, the commit oid is malformed/absent, or
/// the commit has no parent (a root commit — nothing to diff against).
fn intent_diff(src: GitSrc<'_>, target_hex: &str) -> DiffVm {
    let Some(src) = src else {
        return empty_diff();
    };
    // The intent's `target` is the commit oid (hex). A malformed/empty oid → empty.
    let Ok(commit) = gix_hash::ObjectId::from_hex(target_hex.as_bytes()) else {
        return empty_diff();
    };
    // Resolve the commit's tree and its first parent's tree. Any failure (absent
    // object, not a commit, no parent) → honest-empty.
    let new_tree = match hugit_proto::commit_root_tree(src.as_ref(), &commit) {
        Ok(Some(t)) => t,
        _ => return empty_diff(),
    };
    let Some(parent_tree) = first_parent_tree(src, &commit) else {
        return empty_diff();
    };
    diff_vm(Some(src), Some(&parent_tree), Some(&new_tree))
}

/// The root-tree oid of a commit's first parent. `None` when the commit is
/// absent, not a commit, has no parent (root commit), or the parent's tree is
/// unresolvable.
fn first_parent_tree(
    src: &Arc<dyn hugit_proto::ObjectSource + Send + Sync>,
    commit: &gix_hash::ObjectId,
) -> Option<gix_hash::ObjectId> {
    let object = src.get(commit).ok()??;
    if object.kind != hugit_proto::ObjectKind::Commit {
        return None;
    }
    let parent = gix_object::CommitRefIter::from_bytes(&object.data)
        .parent_ids()
        .next()?;
    hugit_proto::commit_root_tree(src.as_ref(), &parent)
        .ok()
        .flatten()
}

/// Fetch a transcript blob by its `cas:`/oid ref from the git source and return
/// its scrubbed UTF-8 (lossy) text. `None` when there is no git source, the ref
/// does not parse as a git oid, or the object is absent / not a blob.
///
/// Production transcript refs are content-addressed; in the wired `git-dir` /
/// hermetic path a transcript stored as a git blob under its oid resolves here.
/// When the live CoreLink CAS-by-blake3 path is the source, a `cas:`-prefixed ref
/// that is not a git oid honestly yields `None` rather than a fabricated body.
fn fetch_transcript(src: GitSrc<'_>, cas_ref: &str) -> Option<String> {
    let src = src?;
    // Accept a bare oid OR a `cas:<oid>` / `<algo>:<oid>` ref — take the segment
    // after the last ':' and parse it as a git oid. A non-oid ref → None.
    let oid_hex = cas_ref.rsplit(':').next().unwrap_or(cas_ref);
    let oid = gix_hash::ObjectId::from_hex(oid_hex.as_bytes()).ok()?;
    let object = src.get(&oid).ok()??;
    if object.kind != hugit_proto::ObjectKind::Blob {
        return None;
    }
    // SCRUB at the read boundary: a transcript can carry secrets, redacted on read.
    Some(scrub(&String::from_utf8_lossy(&object.data)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind, ObjectSource};
    use hugit_refstore::intent::INTENT_LANDED_KIND;

    const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn push(log: &mut EventLog, kind: &str, payload: Value, seq: u64) {
        log.append_for_test(kind, vec!["t".to_string()], payload.to_string(), seq);
    }
    fn land_intent(log: &mut EventLog, id: &str, target: &str, seq: u64) {
        push(
            log,
            INTENT_LANDED_KIND,
            serde_json::json!({"intent_id":id,"ref":"refs/heads/x","target":target,"charter":"do it"}),
            seq,
        );
    }
    fn verdict(log: &mut EventLog, intent: &str, model: &str, seq: u64) {
        push(
            log,
            VERDICT_RECORDED_KIND,
            serde_json::json!({"intent":intent,"tree_hash":"","lens":"correctness","model":model,"prompt_digest":"0".repeat(64),"verdict":"approve","claims_checked":["ok"],"evidence_refs":[]}),
            seq,
        );
    }

    fn blob(src: &mut CasObjectSource, body: &str) -> gix_hash::ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(
        src: &mut CasObjectSource,
        mut entries: Vec<(&str, &str, gix_hash::ObjectId)>,
    ) -> gix_hash::ObjectId {
        entries.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &entries {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }
    fn commit(
        src: &mut CasObjectSource,
        tree_oid: gix_hash::ObjectId,
        parent: Option<gix_hash::ObjectId>,
    ) -> gix_hash::ObjectId {
        let p = parent.map(|p| format!("parent {p}\n")).unwrap_or_default();
        let body =
            format!("tree {tree_oid}\n{p}author a <a@a> 0 +0000\ncommitter a <a@a> 0 +0000\n\nm\n");
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    #[test]
    fn absent_intent_is_none() {
        assert!(build_intent_detail(&EventLog::new(), "hugit", "nope", None).is_none());
    }

    #[test]
    fn reviewer_is_the_verdict_model() {
        let mut log = EventLog::new();
        land_intent(&mut log, "a31", &"0".repeat(40), 1);
        verdict(&mut log, "a31", "opus-4.8", 2);
        let vm = build_intent_detail(&log, "hugit", "a31", None).expect("present");
        assert_eq!(vm.verdicts.len(), 1);
        assert_eq!(vm.verdicts[0].reviewer, "opus-4.8");
        assert!(
            vm.verdicts[0].adversarial,
            "panel-sourced, not hardcoded false"
        );
    }

    #[test]
    fn no_git_source_is_honest_empty_diff() {
        let mut log = EventLog::new();
        land_intent(&mut log, "a31", &"0".repeat(40), 1);
        let vm = build_intent_detail(&log, "hugit", "a31", None).expect("present");
        assert!(
            vm.diff.files.is_empty(),
            "no git source → empty diff, not error"
        );
        assert!(vm.task_transcript.is_none());
    }

    #[test]
    fn populated_git_source_projects_real_diff() {
        let mut src = CasObjectSource::new();
        let old = blob(&mut src, "x\n");
        let new = blob(&mut src, "x\ny\nz\n");
        let pt = tree(&mut src, vec![("100644", "f", old)]);
        let ct = tree(&mut src, vec![("100644", "f", new)]);
        let pc = commit(&mut src, pt, None);
        let cc = commit(&mut src, ct, Some(pc));
        let target = cc.to_string();

        let mut log = EventLog::new();
        land_intent(&mut log, "a31", &target, 1);
        let arc: Arc<dyn ObjectSource + Send + Sync> = Arc::new(src);
        let vm = build_intent_detail(&log, "hugit", "a31", Some(&arc)).expect("present");
        assert_eq!(vm.diff.files.len(), 1);
        assert_eq!(vm.diff.files[0].path, "f");
        assert_eq!((vm.diff.files[0].added, vm.diff.files[0].removed), (2, 0));
    }

    #[test]
    fn transcript_blob_fetched_by_ref_and_scrubbed() {
        // Build an intent-altitude envelope whose task_transcript_ref points at a
        // git blob held in the source — fetch_transcript resolves + scrubs it.
        use hugit_contracts::context_envelope::{
            Authorship, IntentMetrics, Snapshot, Spawn, TokenCounts, Trajectory,
        };
        let mut src = CasObjectSource::new();
        // A clean transcript body resolves + passes through scrub verbatim.
        let transcript = blob(&mut src, "agent said: the work is done");
        let oid_hex = transcript.to_string();
        // A secret-bearing transcript is redacted on read (whole-value scrub).
        let secret_blob = blob(&mut src, &format!("the key is {PAT}"));
        let secret_hex = secret_blob.to_string();

        let env = ContextEnvelope {
            schema_version: "1.1.0".to_string(),
            altitude: Altitude::Intent,
            intent_id: "a31".to_string(),
            commit: String::new(),
            tree_hash: String::new(),
            authorship: Authorship {
                model: "opus".to_string(),
                model_digest: "sha256:0".to_string(),
                agent_type: "main".to_string(),
                spawn: Spawn {
                    run_id: "r".to_string(),
                    parent_run_id: None,
                    born_at: 0,
                    died_at: 0,
                },
                operator: "g".to_string(),
            },
            charter: "c".to_string(),
            campaign: None,
            constraints: vec![],
            acceptance: vec![],
            parent_intents: vec![],
            trajectory: Trajectory {
                // a `cas:<oid>` secret-bearing ref → redacted on read.
                raw_transcript_ref: Some(format!("cas:{secret_hex}")),
                // a `cas:<oid>` ref the source resolves as a git blob.
                task_transcript_ref: Some(format!("cas:{oid_hex}")),
                summary: None,
                journal_ref: None,
                redaction_policy: "default".to_string(),
            },
            snapshot: Snapshot {
                files_read: vec![],
                prompt_ref: None,
                env_manifest: String::new(),
            },
            metrics: IntentMetrics {
                tokens: TokenCounts {
                    input: 0,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    total: 0,
                },
                wall_ms: 0,
                active_ms: 0,
                tool_calls: 0,
                tool_breakdown: vec![],
                model_turns: 0,
                cost_usd_micros: 0,
            },
            verdicts_ref: None,
        };

        let mut log = EventLog::new();
        land_intent(&mut log, "a31", &"0".repeat(40), 1);
        push(
            &mut log,
            INTENT_ENVELOPE_KIND,
            serde_json::to_value(&env).unwrap(),
            2,
        );
        let arc: Arc<dyn ObjectSource + Send + Sync> = Arc::new(src);
        let vm = build_intent_detail(&log, "hugit", "a31", Some(&arc)).expect("present");
        // The clean task transcript is fetched + passed through verbatim.
        let t = vm.task_transcript.expect("transcript fetched");
        assert!(t.contains("the work is done"));
        // The secret-bearing full transcript is fetched but redacted on read.
        let full = vm.full_transcript.expect("full transcript fetched");
        assert!(!full.contains(PAT), "secret scrubbed on read");
    }
}
