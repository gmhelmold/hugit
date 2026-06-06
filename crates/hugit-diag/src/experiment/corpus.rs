//! The promotion corpus: pre-registered, SEALED before evaluation, with
//! post-hoc-removal detection.
//!
//! Contract items handled here:
//!   - ④ anti-gaming: the corpus is pre-registered and SEALED before any
//!     evaluation; sample selection is auditable (the seal records the exact,
//!     ordered member set + a content digest).
//!   - ⑤ post-hoc removal detected → invalidates the verdict:
//!     [`SealedCorpus::verify`] re-derives the seal digest from the presented
//!     members; any removal/addition/reorder breaks the digest, fails CLOSED.
//!   - ⑦ degradation honesty: [`Corpus::seal`] partitions degraded datapoints
//!     out of the *evaluated* sample and records them explicitly as `excluded`
//!     — degraded waves never silently bias the evaluated corpus.

use sha2::{Digest, Sha256};

use crate::experiment::datapoint::{Datapoint, RegenOutcome};

/// Errors from corpus sealing / verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusError {
    /// The presented members do not match the seal digest — a post-hoc
    /// mutation (removal/addition/reorder) was detected (⑤). Fail-closed.
    SealBroken,
}

/// A mutable, pre-evaluation corpus buffer. Datapoints are added as waves
/// contribute; nothing is evaluated until [`Corpus::seal`] is called.
#[derive(Debug, Default, Clone)]
pub struct Corpus {
    members: Vec<Datapoint>,
}

impl Corpus {
    /// An empty corpus.
    pub fn new() -> Self {
        Corpus {
            members: Vec::new(),
        }
    }

    /// Build a corpus from an already-ingested datapoint buffer.
    pub fn from_buffer(members: Vec<Datapoint>) -> Self {
        Corpus { members }
    }

    /// Add one ingested datapoint to the corpus buffer.
    pub fn add(&mut self, dp: Datapoint) {
        self.members.push(dp);
    }

    /// Number of buffered datapoints (pre-seal).
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Pre-register and SEAL the corpus before evaluation (④).
    ///
    /// Sealing snapshots the exact ordered member set and computes a content
    /// digest over it (the auditable sample-selection record). Degraded
    /// datapoints (⑦) are partitioned OUT of the evaluated sample and recorded
    /// separately as `excluded` — they are never silently mixed into the
    /// evaluated corpus, and never silently dropped from the record.
    pub fn seal(self) -> SealedCorpus {
        let (evaluated, excluded): (Vec<Datapoint>, Vec<Datapoint>) =
            self.members.into_iter().partition(|d| !d.degraded);

        let digest = seal_digest(&evaluated);
        SealedCorpus {
            evaluated,
            excluded,
            seal_digest: digest,
        }
    }
}

/// A sealed corpus: the pre-registered, content-pinned sample that an
/// evaluation runs against. Once sealed it is immutable; the only operations
/// are verification (⑤) and read-only sample inspection.
#[derive(Debug, Clone, PartialEq)]
pub struct SealedCorpus {
    /// The evaluated sample (degraded datapoints excluded — ⑦).
    evaluated: Vec<Datapoint>,
    /// Datapoints excluded from evaluation because they ran in a degraded
    /// window (⑦) — recorded explicitly, never silently discarded.
    excluded: Vec<Datapoint>,
    /// Content digest over the ordered evaluated sample (the seal — ④).
    seal_digest: String,
}

impl SealedCorpus {
    /// The seal digest fixed at seal time (the auditable sample-selection
    /// record).
    pub fn seal_digest(&self) -> &str {
        &self.seal_digest
    }

    /// The evaluated sample (read-only).
    pub fn evaluated(&self) -> &[Datapoint] {
        &self.evaluated
    }

    /// The explicitly-excluded (degraded-window) datapoints (⑦).
    pub fn excluded(&self) -> &[Datapoint] {
        &self.excluded
    }

    /// Size of the evaluated sample — the `n` reported by the dashboard.
    pub fn n(&self) -> usize {
        self.evaluated.len()
    }

    /// Verify the sealed sample has not been mutated post-hoc (⑤).
    ///
    /// Re-derives the seal digest from the *current* evaluated members and
    /// compares it to the digest fixed at seal time. A post-hoc removal,
    /// addition, or reorder changes the digest → [`CorpusError::SealBroken`],
    /// fail-closed. Anything that consumes the verdict MUST call this and treat
    /// a broken seal as an invalidated verdict.
    pub fn verify(&self) -> Result<(), CorpusError> {
        if seal_digest(&self.evaluated) == self.seal_digest {
            Ok(())
        } else {
            Err(CorpusError::SealBroken)
        }
    }

    /// Produce a copy with one evaluated member removed post-hoc — a tampering
    /// FIXTURE used to prove ⑤ (the copy's [`verify`](SealedCorpus::verify)
    /// must fail). The seal digest is intentionally NOT recomputed, exactly as
    /// an attacker silently dropping a member would leave it.
    pub fn with_member_removed(&self, index: usize) -> SealedCorpus {
        let mut tampered = self.clone();
        if index < tampered.evaluated.len() {
            tampered.evaluated.remove(index);
        }
        tampered
    }
}

/// Compute the content digest over an ordered datapoint sample.
///
/// SHA-256 over each member's canonical, length-prefixed encoding. Any change
/// to the set or its order changes the digest — that is what makes a post-hoc
/// removal detectable (⑤) and the sample selection auditable (④).
fn seal_digest(members: &[Datapoint]) -> String {
    let mut hasher = Sha256::new();
    hasher.update((members.len() as u32).to_be_bytes());
    for m in members {
        for field in &[m.wave_id.as_str(), m.source.label()] {
            let bytes = field.as_bytes();
            hasher.update((bytes.len() as u32).to_be_bytes());
            hasher.update(bytes);
        }
        hasher.update([
            m.claims_disjoint as u8,
            matches!(m.regen, RegenOutcome::Agree) as u8,
            m.degraded as u8,
        ]);
    }
    hex::encode(hasher.finalize())
}
