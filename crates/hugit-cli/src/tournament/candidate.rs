//! Candidate — one independent implementation produced by `hugit tournament -n N`.
//!
//! Each candidate is an independent implementation of the same intent. Independence
//! means: no shared mutable state between candidates; each is constructed in
//! isolation from its own dedicated input set.

use hugit_contracts::{EventRecord, IntentSidecar};
use sha2::{Digest, Sha256};

/// One candidate implementation of an intent, produced independently.
///
/// Candidates are produced by the tournament fan-out (`-n N`). Independence is
/// enforced by construction: a `Candidate` holds its own content-addressed
/// representation, and two candidates with the same content will share a
/// `candidate_ref` (CAS identity) but remain independently addressable objects.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Stable index: position in the N-candidate set (0-based, immutable).
    pub index: usize,
    /// The intent this candidate implements.
    pub intent_id: String,
    /// Content-addressed ref for this candidate's implementation blob.
    ///
    /// Computed as SHA-256 of `(intent_id || index || strategy)` so that two
    /// candidates with different strategies always carry distinct refs.
    pub candidate_ref: String,
    /// The strategy descriptor for this candidate (distinguishes implementations).
    pub strategy: String,
    /// Whether this candidate was selected as the winner by the judge panel.
    pub selected: bool,
    /// Evidence refs for this candidate (its artifacts / outputs).
    pub evidence_refs: Vec<String>,
}

impl Candidate {
    /// Construct a new, unselected candidate.
    ///
    /// `candidate_ref` is computed deterministically from `(intent_id, index,
    /// strategy)`, guaranteeing that every candidate in an N-set carries a
    /// distinct, content-addressed ref — even if two strategies produce
    /// byte-identical outputs.
    pub fn new(intent_id: impl Into<String>, index: usize, strategy: impl Into<String>) -> Self {
        let intent_id = intent_id.into();
        let strategy = strategy.into();
        let candidate_ref = Self::compute_ref(&intent_id, index, &strategy);
        Self {
            index,
            intent_id,
            candidate_ref,
            strategy,
            selected: false,
            evidence_refs: Vec::new(),
        }
    }

    /// Compute the content-addressed ref for a candidate.
    ///
    /// Formula: `SHA-256(intent_id || ":" || index_decimal || ":" || strategy)`.
    pub fn compute_ref(intent_id: &str, index: usize, strategy: &str) -> String {
        let mut h = Sha256::new();
        h.update(intent_id.as_bytes());
        h.update(b":");
        h.update(index.to_string().as_bytes());
        h.update(b":");
        h.update(strategy.as_bytes());
        hex::encode(h.finalize())
    }

    /// Attach evidence refs to this candidate.
    pub fn with_evidence(mut self, refs: Vec<String>) -> Self {
        self.evidence_refs = refs;
        self
    }

    /// Mark this candidate as the selected winner.
    pub fn mark_selected(mut self) -> Self {
        self.selected = true;
        self
    }
}

/// Produce a set of N independent candidates for the given intent.
///
/// Independence invariant: no two candidates share mutable state. Each is
/// constructed from a distinct strategy string that encodes both its position
/// and its implementation descriptor. Strategy strings are intentionally
/// distinct across the full set.
pub fn produce_candidates(intent: &IntentSidecar, strategies: &[&str]) -> Vec<Candidate> {
    strategies
        .iter()
        .enumerate()
        .map(|(i, &strat)| Candidate::new(intent.intent_id.clone(), i, strat))
        .collect()
}

/// Build an [`EventRecord`] recording the candidate fan-out.
///
/// The payload is a JSON object: `{ "intent_id": "...", "n": N,
/// "candidate_refs": [...] }`.
pub fn fan_out_event(candidates: &[Candidate], seq: u64, prev_hash: &str) -> EventRecord {
    let refs: Vec<String> = candidates.iter().map(|c| c.candidate_ref.clone()).collect();
    let payload = serde_json::json!({
        "intent_id": candidates.first().map(|c| c.intent_id.as_str()).unwrap_or(""),
        "n": candidates.len(),
        "candidate_refs": refs,
    })
    .to_string();

    let this_hash = hash_event(prev_hash, "tournament.fan_out", &[], &payload, seq);

    EventRecord {
        seq,
        prev_hash: prev_hash.to_string(),
        this_hash,
        kind: "tournament.fan_out".to_string(),
        principal_chain: vec!["tournament".to_string()],
        payload,
        recorded_at: 0,
    }
}

/// Build an [`EventRecord`] recording the winner selection.
pub fn selection_event(
    winner: &Candidate,
    loser_refs: &[String],
    seq: u64,
    prev_hash: &str,
) -> EventRecord {
    let payload = serde_json::json!({
        "winner_ref": winner.candidate_ref,
        "loser_refs": loser_refs,
        "criteria": "highest_score",
    })
    .to_string();

    let this_hash = hash_event(prev_hash, "tournament.selection", &[], &payload, seq);

    EventRecord {
        seq,
        prev_hash: prev_hash.to_string(),
        this_hash,
        kind: "tournament.selection".to_string(),
        principal_chain: vec!["tournament".to_string()],
        payload,
        recorded_at: 0,
    }
}

/// Compute the event hash per the frozen formula from EventRecord contract:
/// `H(prev_hash ‖ kind ‖ principal_chain ‖ payload ‖ seq)`.
fn hash_event(prev_hash: &str, kind: &str, principals: &[&str], payload: &str, seq: u64) -> String {
    let mut h = Sha256::new();
    len_prefix_field(&mut h, prev_hash.as_bytes());
    len_prefix_field(&mut h, kind.as_bytes());
    let chain = principals.join(",");
    len_prefix_field(&mut h, chain.as_bytes());
    len_prefix_field(&mut h, payload.as_bytes());
    h.update(seq.to_be_bytes());
    hex::encode(h.finalize())
}

fn len_prefix_field(h: &mut Sha256, data: &[u8]) {
    let len = data.len() as u32;
    h.update(len.to_be_bytes());
    h.update(data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_contracts::IntentSidecar;

    fn test_intent() -> IntentSidecar {
        IntentSidecar {
            intent_id: "intent-test-001".into(),
            charter: "test intent".into(),
            acceptance: vec![],
            context_ref: "blob://ctx".into(),
            authoritative: false,
        }
    }

    #[test]
    fn candidates_are_independent() {
        let intent = test_intent();
        let candidates = produce_candidates(&intent, &["strategy-a", "strategy-b", "strategy-c"]);
        assert_eq!(candidates.len(), 3);

        // All refs are distinct.
        let refs: std::collections::HashSet<_> =
            candidates.iter().map(|c| &c.candidate_ref).collect();
        assert_eq!(refs.len(), 3, "all candidate refs must be distinct");

        // No shared mutation: modify one does not affect others.
        let c0 = candidates[0].clone().mark_selected();
        assert!(c0.selected);
        assert!(!candidates[1].selected);
        assert!(!candidates[2].selected);
    }

    #[test]
    fn candidate_ref_is_deterministic() {
        let r1 = Candidate::compute_ref("intent-001", 0, "strat-a");
        let r2 = Candidate::compute_ref("intent-001", 0, "strat-a");
        assert_eq!(r1, r2);

        let r3 = Candidate::compute_ref("intent-001", 1, "strat-a");
        assert_ne!(r1, r3, "different index → different ref");

        let r4 = Candidate::compute_ref("intent-001", 0, "strat-b");
        assert_ne!(r1, r4, "different strategy → different ref");
    }
}
