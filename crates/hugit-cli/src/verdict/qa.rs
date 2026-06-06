//! Grounded human-review Q&A (WP-D7 ④).
//!
//! Human review is *interrogation*, not generated prose. An answer is a set of
//! citations to **real evidence objects** held in the [`EvidenceStore`]. When a
//! question has no grounding in the served evidence, the engine returns an
//! explicit [`Answer::Refused`] — it NEVER fabricates an answer.

use std::collections::BTreeMap;

/// A content-addressed store of real evidence objects keyed by ref.
#[derive(Debug, Clone, Default)]
pub struct EvidenceStore {
    objects: BTreeMap<String, EvidenceObject>,
}

/// One real evidence object retrievable by ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceObject {
    /// Content-addressed ref (the citation target).
    pub evidence_ref: String,
    /// The evidence body (e.g. a CheckResult stdout excerpt, a diff hunk).
    pub body: String,
    /// Keywords this evidence is indexed under for grounded retrieval.
    pub keywords: Vec<String>,
}

impl EvidenceStore {
    /// New empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a real evidence object.
    pub fn insert(&mut self, object: EvidenceObject) {
        self.objects.insert(object.evidence_ref.clone(), object);
    }

    /// Resolve a ref to its object, if present.
    pub fn get(&self, evidence_ref: &str) -> Option<&EvidenceObject> {
        self.objects.get(evidence_ref)
    }

    /// All evidence objects whose keyword set intersects the query terms.
    fn retrieve(&self, query: &str) -> Vec<&EvidenceObject> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| {
                t.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|t| !t.is_empty())
            .collect();
        self.objects
            .values()
            .filter(|obj| {
                obj.keywords
                    .iter()
                    .any(|k| terms.iter().any(|t| t == &k.to_lowercase()))
            })
            .collect()
    }
}

/// A grounded answer, or an explicit refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The answer is the cited evidence. `citations` are refs into the store
    /// (all guaranteed to resolve), `excerpts` carry the cited bodies.
    Cited {
        /// Content-addressed refs to the evidence objects cited.
        citations: Vec<String>,
        /// The cited evidence bodies, parallel to `citations`.
        excerpts: Vec<String>,
    },
    /// No grounding exists; the engine refuses rather than fabricate.
    Refused {
        /// Human-readable reason for the refusal.
        reason: String,
    },
}

impl Answer {
    /// Whether this answer is a refusal.
    pub fn is_refusal(&self) -> bool {
        matches!(self, Answer::Refused { .. })
    }

    /// The citation refs, if any.
    pub fn citations(&self) -> &[String] {
        match self {
            Answer::Cited { citations, .. } => citations,
            Answer::Refused { .. } => &[],
        }
    }
}

/// Q&A errors (reserved for malformed input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QaError {
    /// The question was empty.
    EmptyQuestion,
}

impl std::fmt::Display for QaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QaError::EmptyQuestion => write!(f, "question is empty"),
        }
    }
}

impl std::error::Error for QaError {}

/// Answer a human-review question STRICTLY by grounded retrieval over the
/// evidence store.
///
/// - If ≥1 evidence object grounds the question, return [`Answer::Cited`] with
///   every citation resolving to a real object.
/// - Otherwise return [`Answer::Refused`] — never a fabricated answer.
pub fn answer_question(store: &EvidenceStore, question: &str) -> Result<Answer, QaError> {
    if question.trim().is_empty() {
        return Err(QaError::EmptyQuestion);
    }

    let hits = store.retrieve(question);
    if hits.is_empty() {
        return Ok(Answer::Refused {
            reason: "no evidence object grounds this question".to_string(),
        });
    }

    let mut citations = Vec::with_capacity(hits.len());
    let mut excerpts = Vec::with_capacity(hits.len());
    for obj in hits {
        // Invariant: every citation resolves to a real object in the store.
        debug_assert!(store.get(&obj.evidence_ref).is_some());
        citations.push(obj.evidence_ref.clone());
        excerpts.push(obj.body.clone());
    }
    Ok(Answer::Cited {
        citations,
        excerpts,
    })
}
