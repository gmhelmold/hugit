//! Receipt-linked derived facts and verified human-facing projection views.
//!
//! This module has no authority to create intents, checks, lands, or approvals.

use hugit_contracts::event_record::EventRecord;
use serde::{Deserialize, Serialize};

pub mod views;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize)]
pub struct SourceIdentity {
    pub receipt_id: String,
    pub event_seq: u64,
    pub event_hash: String,
}

#[derive(Debug, Clone)]
pub struct SourceFact<'a> {
    pub identity: SourceIdentity,
    pub record: &'a EventRecord,
    pub facts: ReceiptFacts,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReceiptFacts {
    pub recorded_at_ms: u64,
    pub ref_name: Option<String>,
    pub target_oid: Option<String>,
    pub branch: Option<String>,
    pub checkout_truth: Option<String>,
    pub update_count: u64,
}

impl<'a> SourceFact<'a> {
    pub fn from_record(record: &'a EventRecord) -> Result<Self, &'static str> {
        if record.kind != "ref.update" {
            return Err("source event kind is not receipt-derived");
        }
        if record.principal_chain.as_slice() != ["orchestrator:hugit-hook"] {
            return Err("source event principal is not hook capture");
        }
        let payload: serde_json::Value =
            serde_json::from_str(&record.payload).map_err(|_| "source event payload is invalid")?;
        let receipt_id = payload
            .get("receipt_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or("source event has no receipt identity")?;
        if record.this_hash.is_empty() {
            return Err("source event has no event identity");
        }
        let string = |key| {
            payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        Ok(Self {
            identity: SourceIdentity {
                receipt_id: receipt_id.to_owned(),
                event_seq: record.seq,
                event_hash: record.this_hash.clone(),
            },
            record,
            facts: ReceiptFacts {
                recorded_at_ms: record.recorded_at,
                ref_name: string("ref"),
                target_oid: string("target")
                    .or_else(|| string("to"))
                    .or_else(|| string("new")),
                branch: string("branch"),
                checkout_truth: payload
                    .get("checkout")
                    .and_then(|checkout| checkout.get("truth"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                update_count: payload
                    .get("updates")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, |updates| updates.len() as u64),
            },
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedFact {
    Provenance {
        receipt_id: String,
        event_hash: String,
        ref_name: Option<String>,
        target_oid: Option<String>,
    },
    Freshness {
        observed_at_ms: u64,
    },
    ContextTimeline {
        branch: Option<String>,
        checkout_truth: Option<String>,
        update_count: u64,
    },
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProjectionOutcome {
    Applied { fact: DerivedFact },
    Skipped,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionFailureCode {
    ProjectorPanic,
    SourcePayloadInvalid,
    Retryable,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProjectionFailure {
    Failed { code: ProjectionFailureCode },
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProjectionRetry {
    pub source: SourceIdentity,
    pub projector: String,
    pub failure: ProjectionFailure,
}

#[derive(Default, Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProjectionStatus {
    pub version: u8,
    #[serde(default)]
    pub cursor: Option<u64>,
    #[serde(default)]
    pub completed: Vec<ProjectionApplied>,
    #[serde(default)]
    pub applied: Vec<ProjectionApplied>,
    #[serde(default)]
    pub summary: ProjectionSummary,
    pub retry: Option<ProjectionRetry>,
}

#[derive(Default, Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProjectionSummary {
    pub completed: u64,
    pub evicted: u64,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProjectionApplied {
    pub source: SourceIdentity,
    pub projector: String,
    pub outcome: ProjectionOutcome,
}

#[derive(Clone, Copy)]
pub struct Projector {
    pub name: &'static str,
    pub declare: fn(&SourceFact<'_>) -> Result<ProjectionOutcome, ProjectionFailure>,
}

const DEFAULT_PROJECTORS: &[Projector] = &[
    Projector {
        name: "provenance",
        declare: provenance,
    },
    Projector {
        name: "freshness",
        declare: freshness,
    },
    Projector {
        name: "context",
        declare: context,
    },
];

pub fn default_projectors() -> &'static [Projector] {
    DEFAULT_PROJECTORS
}

pub fn declare(
    records: &[EventRecord],
    previous: Option<ProjectionStatus>,
    limit: usize,
    projectors: &[Projector],
) -> ProjectionStatus {
    let mut status = previous.unwrap_or_else(|| ProjectionStatus {
        version: 1,
        ..ProjectionStatus::default()
    });
    let mut processed = 0;
    for record in records {
        if status.cursor.is_some_and(|cursor| record.seq <= cursor) || processed == limit {
            continue;
        }
        processed += 1;
        let Ok(source) = SourceFact::from_record(record) else {
            status.cursor = Some(record.seq);
            continue;
        };
        for projector in projectors {
            if status.completed.iter().any(|applied| {
                applied.source == source.identity && applied.projector == projector.name
            }) {
                continue;
            }
            let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (projector.declare)(&source)
            })) {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(failure)) => {
                    status.retry = Some(ProjectionRetry {
                        source: source.identity.clone(),
                        projector: projector.name.to_owned(),
                        failure,
                    });
                    return status;
                }
                Err(_) => {
                    status.retry = Some(ProjectionRetry {
                        source: source.identity.clone(),
                        projector: projector.name.to_owned(),
                        failure: ProjectionFailure::Failed {
                            code: ProjectionFailureCode::ProjectorPanic,
                        },
                    });
                    return status;
                }
            };
            let applied = ProjectionApplied {
                source: source.identity.clone(),
                projector: projector.name.to_owned(),
                outcome,
            };
            status.completed.push(applied.clone());
            status.applied.push(applied);
            status.summary.completed += 1;
        }
        status.cursor = Some(record.seq);
    }
    status
}

fn provenance(source: &SourceFact<'_>) -> Result<ProjectionOutcome, ProjectionFailure> {
    Ok(ProjectionOutcome::Applied {
        fact: DerivedFact::Provenance {
            receipt_id: source.identity.receipt_id.clone(),
            event_hash: source.identity.event_hash.clone(),
            ref_name: source.facts.ref_name.clone(),
            target_oid: source.facts.target_oid.clone(),
        },
    })
}

fn freshness(source: &SourceFact<'_>) -> Result<ProjectionOutcome, ProjectionFailure> {
    Ok(ProjectionOutcome::Applied {
        fact: DerivedFact::Freshness {
            observed_at_ms: source.facts.recorded_at_ms,
        },
    })
}

fn context(source: &SourceFact<'_>) -> Result<ProjectionOutcome, ProjectionFailure> {
    Ok(ProjectionOutcome::Applied {
        fact: DerivedFact::ContextTimeline {
            branch: source.facts.branch.clone(),
            checkout_truth: source.facts.checkout_truth.clone(),
            update_count: source.facts.update_count,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> EventRecord {
        EventRecord {
            seq: 1,
            prev_hash: "0".repeat(64),
            this_hash: "a".repeat(64),
            kind: "ref.update".into(),
            principal_chain: vec!["orchestrator:hugit-hook".into()],
            payload: r#"{"receipt_id":"r1","branch":"main","updates":[{}]}"#.into(),
            recorded_at: 1,
        }
    }

    #[test]
    fn facts_stay_receipt_linked_and_idempotent() {
        let record = record();
        let first = declare(std::slice::from_ref(&record), None, 1, default_projectors());
        let second = declare(
            std::slice::from_ref(&record),
            Some(first.clone()),
            1,
            default_projectors(),
        );
        assert_eq!(first.completed.len(), 3);
        assert_eq!(second, first);
        assert!(first.completed.iter().all(|applied| {
            applied.source.receipt_id == "r1" && applied.source.event_hash == "a".repeat(64)
        }));
    }
}
