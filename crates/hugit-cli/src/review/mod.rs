//! `hugit review` — grounded-evidence Q&A over the log (D7 half), graduated from
//! RESERVED with REAL wiring.
//!
//! `hugit review --log <path> --question <text> [--intent <id>]` answers a
//! human-review question STRICTLY by grounded retrieval over evidence projected
//! from the canonical log, reusing the engine's OWN D7 Q&A
//! ([`crate::verdict::qa::answer_question`] + [`EvidenceStore`]). It returns
//! either an [`Answer::Cited`] (citations into real evidence + their excerpts) or
//! an explicit [`Answer::Refused`] — it NEVER fabricates an answer.
//!
//! # Evidence sources (and the honest-thin caveat)
//! Evidence is projected from the records that survive on the forever-log:
//! - `check.recorded` → one evidence object per memoized check
//!   (`name`/`exit`/`ok`/`cache_hit`/`duration`/`memo_key`).
//! - `verdict.recorded` → one per recorded adversarial verdict
//!   (`intent`/aggregate/`claims_checked` lenses).
//!
//! The forever-log SCRUBS rich bodies on append (stdout, diff hunks), so the
//! grounded evidence is limited to this surviving STRUCTURED metadata. A question
//! that needs a full stdout/diff body legitimately REFUSES until the served-
//! evidence (CAS) path is live — honest, never a fabricated answer.
//!
//! # Output (stable JSON, WB0 one-error/one-exit)
//! ```json
//! {"answer":"cited","citations":["check:<ref>", …],"excerpts":["…", …]}
//! {"answer":"refused","reason":"no evidence object grounds this question"}
//! ```
//! An empty `--question` is the canonical `{"error":{kind:"empty_question",…}}`
//! envelope, exit 2.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::checks::{CHECK_RECORDED_KIND, load_event_log};
use crate::porcelain::PorcelainError;
use crate::redaction::scrub;
use crate::verdict::VERDICT_RECORDED_KIND;
use crate::verdict::qa::{Answer, EvidenceObject, EvidenceStore, QaError, answer_question};

/// `hugit review` — grounded-evidence Q&A over the log.
#[derive(clap::Args, Debug)]
pub struct ReviewArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`).
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
    /// The human-review question to answer by grounded retrieval.
    #[arg(long)]
    pub question: String,
    /// Optionally scope verdict evidence to a single intent (matched scrubbed,
    /// the way the recorder stored it). Check evidence is always included.
    #[arg(long)]
    pub intent: Option<String>,
}

/// Dispatch `hugit review` under the WB0 one-exit-code law.
pub fn run(args: ReviewArgs) -> ExitCode {
    match project(&args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", e.to_json());
            e.exit_code()
        }
    }
}

/// Build the evidence store from the log and answer the question.
fn project(args: &ReviewArgs) -> Result<Value, PorcelainError> {
    let log_path = crate::log_resolve::resolve_log(args.log.clone())?;
    let log = load_event_log(&log_path)?;
    let want_intent = args.intent.as_deref().map(scrub);

    let mut store = EvidenceStore::new();
    for r in log.records() {
        let Ok(payload) = serde_json::from_str::<Value>(&r.payload) else {
            continue; // a malformed record never fabricates evidence
        };
        match r.kind.as_str() {
            CHECK_RECORDED_KIND => {
                if let Some(obj) = check_evidence(&payload) {
                    store.insert(obj);
                }
            }
            VERDICT_RECORDED_KIND => {
                if let Some(obj) = verdict_evidence(&payload, want_intent.as_deref()) {
                    store.insert(obj);
                }
            }
            _ => {}
        }
    }

    match answer_question(&store, &args.question) {
        Ok(Answer::Cited {
            citations,
            excerpts,
        }) => Ok(json!({
            "answer": "cited",
            "citations": citations,
            "excerpts": excerpts,
        })),
        Ok(Answer::Refused { reason }) => Ok(json!({
            "answer": "refused",
            "reason": reason,
        })),
        Err(QaError::EmptyQuestion) => Err(PorcelainError::new(
            "empty_question",
            "a review question must be non-empty",
            "pass --question \"<your question>\"",
        )),
    }
}

/// Project a `check.recorded` payload into an evidence object. Fields are already
/// scrubbed-on-append; re-scrubbed here as defense-in-depth at the read boundary.
fn check_evidence(p: &Value) -> Option<EvidenceObject> {
    let name = scrub(p.get("name").and_then(Value::as_str).unwrap_or(""));
    let memo_key = p.get("memo_key").and_then(Value::as_str).unwrap_or("");
    let exit = p.get("exit").and_then(Value::as_i64);
    let cache_hit = p.get("cache_hit").and_then(Value::as_bool);
    let duration_ms = p.get("duration_ms").and_then(Value::as_u64);
    if name.is_empty() && memo_key.is_empty() {
        return None; // nothing to cite
    }
    let ok = exit.map(|e| e == 0);
    let status = match ok {
        Some(true) => "passed (green)",
        Some(false) => "failed (red)",
        None => "unknown exit",
    };
    let ref_key = if memo_key.is_empty() {
        scrub(&name)
    } else {
        scrub(&memo_key.chars().take(16).collect::<String>())
    };
    let body = format!(
        "check `{name}` {status}{}{}.",
        exit.map(|e| format!(" (exit {e})")).unwrap_or_default(),
        match (cache_hit, duration_ms) {
            (Some(true), _) => " — served from cache".to_string(),
            (Some(false), Some(ms)) => format!(" — executed in {ms}ms"),
            _ => String::new(),
        }
    );
    // Keywords: the check name tokens + structural status terms for retrieval.
    let mut keywords = tokenize(&name);
    keywords.extend(["check", "gate", "ci"].map(String::from));
    match ok {
        Some(true) => keywords.extend(["pass", "passed", "green", "ok"].map(String::from)),
        Some(false) => keywords.extend(["fail", "failed", "red", "broken"].map(String::from)),
        None => {}
    }
    if cache_hit == Some(true) {
        keywords.extend(["cache", "cached", "hit"].map(String::from));
    }
    Some(EvidenceObject {
        evidence_ref: format!("check:{ref_key}"),
        body,
        keywords,
    })
}

/// Project a `verdict.recorded` payload into an evidence object, optionally scoped
/// to `want_intent` (already scrubbed). Fields are scrubbed-on-append.
fn verdict_evidence(p: &Value, want_intent: Option<&str>) -> Option<EvidenceObject> {
    let intent = scrub(p.get("intent").and_then(Value::as_str).unwrap_or(""));
    if intent.is_empty() {
        return None;
    }
    if let Some(want) = want_intent
        && intent != want
    {
        return None; // scoped out by --intent
    }
    let aggregate = p
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let lenses: Vec<String> = p
        .get("claims_checked")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(scrub)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let lenses_str = if lenses.is_empty() {
        String::new()
    } else {
        format!(" — lenses: {}", lenses.join(", "))
    };
    let body = format!("verdict for intent `{intent}`: {aggregate}{lenses_str}.");
    let mut keywords = tokenize(&intent);
    keywords.extend(tokenize(aggregate));
    for l in &lenses {
        keywords.extend(tokenize(l));
    }
    keywords.extend(["verdict", "review", "decision", "panel"].map(String::from));
    Some(EvidenceObject {
        evidence_ref: format!("verdict:{intent}"),
        body,
        keywords,
    })
}

/// Lowercased alphanumeric tokens of `s` (the same shape the qa retriever uses).
fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}
