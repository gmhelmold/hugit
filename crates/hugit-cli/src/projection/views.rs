//! Source-verified projection read model shared by human-facing views.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::{
    DerivedFact, ProjectionApplied, ProjectionOutcome, ProjectionStatus, SourceFact, SourceIdentity,
};

#[derive(Debug, Clone, Serialize)]
pub struct ProjectionView {
    /// `complete` means every required result matches canonical sources.
    /// `partial` means a retry or unattempted required result remains. Capture
    /// health is reported separately by `hugit health`.
    pub state: &'static str,
    pub applied: Vec<ProjectionApplied>,
    pub retry: Option<super::ProjectionRetry>,
    pub pending: Vec<PendingProjection>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct PendingProjection {
    pub source: SourceIdentity,
    pub projector: String,
}

pub fn load(log_path: &Path) -> Result<ProjectionView, String> {
    let root = log_path.parent().ok_or("projection log has no parent")?;
    let status_path = root.join(crate::runtime_store::STATUS);
    let log = crate::checks::load_event_log(log_path)
        .map_err(|error| format!("load log: {}", error.to_json()))?;
    let projection = materialize(&status_path, log.records())?;
    let pending = validate(&projection, log.records())?;
    Ok(ProjectionView {
        state: if projection.retry.is_some() || !pending.is_empty() {
            "partial"
        } else {
            "complete"
        },
        applied: projection.completed,
        retry: projection.retry,
        pending,
    })
}

/// Derive and atomically persist status from canonical facts. Views never need
/// a test-only or hook-written status file to report projection truth.
fn materialize(
    status_path: &Path,
    records: &[hugit_contracts::event_record::EventRecord],
) -> Result<ProjectionStatus, String> {
    let _lock = crate::pr::filelock::FileLock::acquire_unprepared(status_path)
        .map_err(|error| format!("lock projection status {}: {error}", status_path.display()))?;
    let mut stored = if status_path.exists() {
        serde_json::from_slice::<Value>(&std::fs::read(status_path).map_err(|error| {
            format!("read projection status {}: {error}", status_path.display())
        })?)
        .map_err(|_| format!("projection status {} is unreadable", status_path.display()))?
    } else {
        Value::Object(Default::default())
    };
    let object = stored.as_object_mut().ok_or_else(|| {
        format!(
            "projection status {} is not an object",
            status_path.display()
        )
    })?;
    let previous = object
        .get("projection")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| format!("projection status {} is invalid", status_path.display()))?;
    let projection = super::declare(records, previous, usize::MAX, super::default_projectors());
    object.insert(
        "projection".into(),
        serde_json::to_value(&projection)
            .map_err(|error| format!("serialize projection status: {error}"))?,
    );
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|error| format!("serialize projection status: {error}"))?;
    crate::pr::filelock::atomic_write_unprepared(status_path, &bytes)
        .map_err(|error| format!("write projection status {}: {error}", status_path.display()))?;
    Ok(projection)
}

fn validate(
    status: &ProjectionStatus,
    records: &[hugit_contracts::event_record::EventRecord],
) -> Result<Vec<PendingProjection>, String> {
    let source_facts: Vec<_> = records
        .iter()
        .filter_map(|record| SourceFact::from_record(record).ok())
        .collect();
    let sources: BTreeMap<_, _> = source_facts
        .iter()
        .map(|source| (source.identity.clone(), source.clone()))
        .collect();
    let required: Vec<_> = source_facts
        .iter()
        .flat_map(|source| {
            super::default_projectors()
                .iter()
                .map(move |projector| (source.identity.clone(), projector.name.to_owned()))
        })
        .collect();
    let mut incomplete = BTreeSet::new();
    for applied in &status.completed {
        let source = sources.get(&applied.source).ok_or_else(|| {
            format!(
                "projection {} references missing source receipt {}",
                applied.projector, applied.source.receipt_id
            )
        })?;
        if !outcome_matches_source(&applied.projector, &applied.outcome, source) {
            return Err(format!(
                "projection {} does not match source receipt {}",
                applied.projector, applied.source.receipt_id
            ));
        }
    }
    for pair in &required {
        if !status
            .completed
            .iter()
            .any(|applied| applied.source == pair.0 && applied.projector == pair.1)
        {
            incomplete.insert(pair.clone());
        }
    }
    let retry_pair = status
        .retry
        .as_ref()
        .map(|retry| {
            let pair = (retry.source.clone(), retry.projector.clone());
            if !sources.contains_key(&retry.source)
                || !super::default_projectors()
                    .iter()
                    .any(|projector| projector.name == retry.projector)
                || !incomplete.contains(&pair)
                || required.iter().find(|pair| incomplete.contains(*pair)) != Some(&pair)
            {
                Err("projection retry does not bind first incomplete required projector")
            } else {
                Ok(pair)
            }
        })
        .transpose()?;
    let cursor = status.cursor.unwrap_or(0);
    incomplete
        .into_iter()
        .filter(|pair| Some(pair) != retry_pair.as_ref())
        .map(|(source, projector)| {
            if source.event_seq <= cursor {
                Err("projection has missing attempted required projector".into())
            } else {
                Ok(PendingProjection { source, projector })
            }
        })
        .collect()
}

fn outcome_matches_source(
    projector: &str,
    outcome: &ProjectionOutcome,
    source: &SourceFact<'_>,
) -> bool {
    match (projector, outcome) {
        (
            "provenance",
            ProjectionOutcome::Applied {
                fact:
                    DerivedFact::Provenance {
                        receipt_id,
                        event_hash,
                        ref_name,
                        target_oid,
                    },
            },
        ) => {
            receipt_id == &source.identity.receipt_id
                && event_hash == &source.identity.event_hash
                && ref_name == &source.facts.ref_name
                && target_oid == &source.facts.target_oid
        }
        (
            "freshness",
            ProjectionOutcome::Applied {
                fact: DerivedFact::Freshness { observed_at_ms },
            },
        ) => *observed_at_ms == source.facts.recorded_at_ms,
        (
            "context",
            ProjectionOutcome::Applied {
                fact:
                    DerivedFact::ContextTimeline {
                        branch,
                        checkout_truth,
                        update_count,
                    },
            },
        ) => {
            branch == &source.facts.branch
                && checkout_truth == &source.facts.checkout_truth
                && *update_count == source.facts.update_count
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::{declare, default_projectors};
    use hugit_contracts::event_record::EventRecord;

    fn record() -> EventRecord {
        EventRecord {
            seq: 1,
            prev_hash: "0".repeat(64),
            this_hash: "a".repeat(64),
            kind: "ref.update".into(),
            principal_chain: vec!["orchestrator:hugit-hook".into()],
            payload: r#"{"receipt_id":"r1"}"#.into(),
            recorded_at: 1,
        }
    }

    #[test]
    fn mutation_or_missing_retry_fails_closed() {
        let record = record();
        let status = declare(std::slice::from_ref(&record), None, 1, default_projectors());
        assert!(validate(&status, std::slice::from_ref(&record)).is_ok());
        assert!(validate(&status, &[]).is_err());
        let mut partial = status;
        partial
            .completed
            .retain(|applied| applied.projector != "context");
        assert!(validate(&partial, std::slice::from_ref(&record)).is_err());
    }
}
