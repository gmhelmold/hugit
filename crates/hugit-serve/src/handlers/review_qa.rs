//! `GET /v1/repos/{repo}/prs/{n}/review/qa?q=…` → [`ReviewQaVm`].
//!
//! Grounded human-review Q&A (WP-D7 ④) served over the canonical log. Reuses the
//! engine's OWN retriever ([`hugit_cli::verdict::qa`]) but BROADENS the evidence
//! corpus (review-legibility ⑤): the CLI `hugit review` grounds only on
//! `check.recorded` + `verdict.recorded` STRUCTURED metadata, so it refuses
//! code-/charter-level questions. This handler ALSO projects:
//!
//! - `journal.note` → one evidence object per session note (free-text body,
//!   scrubbed) — answers "what did the agent decide / why" without the CAS.
//! - `intent.envelope` → the intent CHARTER and ACCEPTANCE criteria (both already
//!   on the log) — answers "what was this change supposed to do".
//!
//! It NEVER fabricates: a question with no grounding returns the explicit
//! [`hugit_cli::verdict::qa::Answer::Refused`] reason. Every projected body is
//! SCRUBBED at this read boundary.

use hugit_cli::journal::note::JOURNAL_NOTE_KIND;
use hugit_cli::pr::INTENT_ENVELOPE_KIND;
use hugit_cli::verdict::VERDICT_RECORDED_KIND;
use hugit_cli::verdict::qa::{Answer, EvidenceObject, EvidenceStore, answer_question};
use hugit_contracts::context_envelope::{Altitude, ContextEnvelope};
use hugit_http_contracts::review::ReviewQaVm;
use hugit_refstore::EventLog;
use serde_json::Value;

use crate::fmt::scrub;

const CHECK_RECORDED_KIND: &str = "check.recorded";

/// Answer a single review question over the broadened evidence corpus. Always
/// returns a `ReviewQaVm` (the `answer` is the refusal reason when ungrounded —
/// never a fabricated answer). An empty question is itself an honest refusal.
#[must_use]
pub fn build_review_qa(log: &EventLog, question: &str) -> ReviewQaVm {
    let q = scrub(question);
    if q.trim().is_empty() {
        return refusal(&q, "uma pergunta de revisão não pode ser vazia");
    }
    let store = evidence_store(log);
    match answer_question(&store, &q) {
        Ok(Answer::Cited {
            citations,
            excerpts,
        }) => ReviewQaVm {
            // The retriever's excerpts are already projected from scrubbed bodies;
            // join them into the answer prose. Citations become the source chips.
            answer: excerpts.join(" "),
            question: q,
            sources: citations,
            answer_mono_terms: vec![],
            answer_lnk_terms: vec![],
            typed_sources: vec![],
        },
        Ok(Answer::Refused { reason }) => refusal(&q, &reason),
        // An empty question is the only QaError; the guard above already handled
        // it, but map defensively to an honest refusal rather than a panic.
        Err(_) => refusal(&q, "uma pergunta de revisão não pode ser vazia"),
    }
}

fn refusal(question: &str, reason: &str) -> ReviewQaVm {
    ReviewQaVm {
        question: question.to_string(),
        answer: scrub(reason),
        sources: vec![],
        answer_mono_terms: vec![],
        answer_lnk_terms: vec![],
        typed_sources: vec![],
    }
}

/// Build the BROADENED evidence store from the verified log: checks + verdicts
/// (the CLI corpus) PLUS journal notes + intent charter/acceptance.
fn evidence_store(log: &EventLog) -> EvidenceStore {
    let mut store = EvidenceStore::new();
    for r in log.records() {
        let Ok(p) = serde_json::from_str::<Value>(&r.payload) else {
            continue; // a malformed record never fabricates evidence
        };
        match r.kind.as_str() {
            CHECK_RECORDED_KIND => {
                if let Some(o) = check_evidence(&p) {
                    store.insert(o);
                }
            }
            VERDICT_RECORDED_KIND => {
                if let Some(o) = verdict_evidence(&p) {
                    store.insert(o);
                }
            }
            JOURNAL_NOTE_KIND => {
                if let Some(o) = journal_evidence(&p, r.seq) {
                    store.insert(o);
                }
            }
            INTENT_ENVELOPE_KIND => {
                for o in envelope_evidence(&r.payload) {
                    store.insert(o);
                }
            }
            _ => {}
        }
    }
    store
}

/// Lowercased alphanumeric tokens (the shape the qa retriever matches on).
fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A `check.recorded` → evidence object (name/status keywords).
fn check_evidence(p: &Value) -> Option<EvidenceObject> {
    let name = scrub(p.get("name").and_then(Value::as_str).unwrap_or(""));
    if name.is_empty() {
        return None;
    }
    let exit = p.get("exit").and_then(Value::as_i64);
    let status = match exit.map(|e| e == 0) {
        Some(true) => "passed (green)",
        Some(false) => "failed (red)",
        None => "unknown exit",
    };
    let mut keywords = tokenize(&name);
    keywords.extend(["check", "gate", "ci"].map(String::from));
    Some(EvidenceObject {
        evidence_ref: format!("check:{name}"),
        body: format!("check `{name}` {status}."),
        keywords,
    })
}

/// A `verdict.recorded` → evidence object (intent/outcome/lens keywords).
fn verdict_evidence(p: &Value) -> Option<EvidenceObject> {
    let intent = scrub(p.get("intent").and_then(Value::as_str).unwrap_or(""));
    if intent.is_empty() {
        return None;
    }
    let outcome = p
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let lenses: Vec<String> = p
        .get("claims_checked")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(scrub).collect())
        .unwrap_or_default();
    let mut keywords = tokenize(&intent);
    keywords.extend(tokenize(outcome));
    for l in &lenses {
        keywords.extend(tokenize(l));
    }
    keywords.extend(["verdict", "review", "decision", "panel"].map(String::from));
    Some(EvidenceObject {
        evidence_ref: format!("verdict:{intent}"),
        body: format!("verdict for intent `{intent}`: {outcome}."),
        keywords,
    })
}

/// A `journal.note` → evidence object. The note free-text IS the body (scrubbed);
/// the body words are the keywords so a question about the note's content grounds.
fn journal_evidence(p: &Value, seq: u64) -> Option<EvidenceObject> {
    let note = scrub(p.get("note").and_then(Value::as_str)?);
    if note.trim().is_empty() {
        return None;
    }
    let mut keywords = tokenize(&note);
    keywords.extend(["note", "journal", "session"].map(String::from));
    Some(EvidenceObject {
        evidence_ref: format!("journal:{seq}"),
        body: format!("session note: {note}"),
        keywords,
    })
}

/// An `intent.envelope` → up to two evidence objects: the CHARTER ("why") and the
/// ACCEPTANCE criteria. Both are real log text, scrubbed at the read boundary.
fn envelope_evidence(payload: &str) -> Vec<EvidenceObject> {
    let Ok(env) = serde_json::from_str::<ContextEnvelope>(payload) else {
        return vec![];
    };
    if env.altitude != Altitude::Intent {
        return vec![];
    }
    let id = scrub(&env.intent_id);
    let mut out = Vec::new();
    let charter = scrub(&env.charter);
    if !charter.trim().is_empty() {
        let mut keywords = tokenize(&charter);
        keywords.extend(tokenize(&id));
        keywords.extend(["charter", "intent", "why", "goal", "purpose"].map(String::from));
        out.push(EvidenceObject {
            evidence_ref: format!("charter:{id}"),
            body: format!("intent `{id}` charter: {charter}"),
            keywords,
        });
    }
    let acceptance: Vec<String> = env.acceptance.iter().map(|a| scrub(a)).collect();
    let acc_text: Vec<String> = acceptance.into_iter().filter(|a| !a.is_empty()).collect();
    if !acc_text.is_empty() {
        let joined = acc_text.join("; ");
        let mut keywords = tokenize(&joined);
        keywords.extend(tokenize(&id));
        keywords.extend(["acceptance", "criteria", "done", "requirement"].map(String::from));
        out.push(EvidenceObject {
            evidence_ref: format!("acceptance:{id}"),
            body: format!("intent `{id}` acceptance: {joined}"),
            keywords,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn append(log: &mut EventLog, kind: &str, payload: Value) {
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            kind,
            vec!["o".into()],
            payload.to_string(),
            0,
        )
        .expect("append");
    }

    #[test]
    fn empty_question_is_refused() {
        let qa = build_review_qa(&EventLog::new(), "  ");
        assert!(qa.sources.is_empty());
        assert!(qa.answer.contains("vazia"));
    }

    #[test]
    fn ungrounded_question_is_refused_not_fabricated() {
        let mut log = EventLog::new();
        append(
            &mut log,
            JOURNAL_NOTE_KIND,
            serde_json::json!({"note":"chose tokio"}),
        );
        let qa = build_review_qa(&log, "what is the meaning of life");
        assert!(
            qa.sources.is_empty(),
            "no grounding → refusal, no fabrication"
        );
    }

    #[test]
    fn journal_note_grounds_a_question() {
        let mut log = EventLog::new();
        append(
            &mut log,
            JOURNAL_NOTE_KIND,
            serde_json::json!({"note":"picked the rolling scheduler over a barrier"}),
        );
        let qa = build_review_qa(&log, "why was the scheduler chosen?");
        assert!(
            !qa.sources.is_empty(),
            "the journal note grounds the answer"
        );
        assert!(qa.answer.contains("scheduler"));
        assert!(qa.sources.iter().any(|s| s.starts_with("journal:")));
    }

    /// A minimal intent-altitude envelope carrying a charter + acceptance, built
    /// via the typed constructor (so `deny_unknown_fields` can never break the
    /// test) and serialized the way the engine captures it.
    fn intent_envelope(id: &str, charter: &str, acceptance: &[&str]) -> Value {
        use hugit_contracts::context_envelope::{
            Authorship, IntentMetrics, Snapshot, Spawn, TokenCounts, Trajectory,
        };
        let env = ContextEnvelope {
            schema_version: "1.1.0".to_string(),
            altitude: Altitude::Intent,
            intent_id: id.to_string(),
            commit: String::new(),
            tree_hash: String::new(),
            authorship: Authorship {
                model: "opus-4.8".to_string(),
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
            charter: charter.to_string(),
            campaign: None,
            constraints: vec![],
            acceptance: acceptance.iter().map(|s| s.to_string()).collect(),
            parent_intents: vec![],
            trajectory: Trajectory {
                raw_transcript_ref: None,
                task_transcript_ref: None,
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
        serde_json::to_value(&env).expect("envelope serializes")
    }

    #[test]
    fn charter_and_acceptance_ground_a_code_level_question() {
        let mut log = EventLog::new();
        let env = intent_envelope(
            "a31",
            "refactor the auth token expiry to use the current instant",
            &["exp is derived from now()", "tests cover full ttl refresh"],
        );
        append(&mut log, INTENT_ENVELOPE_KIND, env);
        let qa = build_review_qa(&log, "does the token expiry use the current instant?");
        assert!(
            !qa.sources.is_empty(),
            "charter/acceptance ground the answer"
        );
        assert!(
            qa.sources
                .iter()
                .any(|s| s.starts_with("charter:") || s.starts_with("acceptance:"))
        );
    }

    #[test]
    fn secret_in_a_note_is_scrubbed() {
        let mut log = EventLog::new();
        append(
            &mut log,
            JOURNAL_NOTE_KIND,
            serde_json::json!({"note": format!("the token is {PAT}")}),
        );
        let qa = build_review_qa(&log, "what is the token?");
        let j = serde_json::to_string(&qa).unwrap();
        assert!(!j.contains(PAT));
    }
}
