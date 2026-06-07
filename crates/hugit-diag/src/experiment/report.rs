//! The gate report — generated, never hand-written (③); an attested,
//! tamper-evident object (⑨).
//!
//! Contract items handled here:
//!   - ③ generated, never hand-written: the ONLY constructor,
//!     [`GateReport::generate`], derives the verdict from a sealed corpus +
//!     dashboard. There is no public way to set the verdict directly.
//!   - ⑨ attested tamper-evident object (X2-class): the report binds an
//!     [`AttestationChain`] and a `body_digest` that hashes the evaluated facts
//!     together with the attestation. [`GateReport::verify`] recomputes the
//!     digest; a forged/swapped PASS (verdict or facts changed without a
//!     genuine re-evaluation) fails verification → it cannot promote (the gate
//!     calls `verify` before honoring any report).

use hugit_contracts::AttestationChain;
use sha2::{Digest, Sha256};

use crate::experiment::corpus::SealedCorpus;
use crate::experiment::dashboard::Dashboard;

/// The minimum evaluated sample size below which the report is "insufficient".
/// Below this, the gate fails CLOSED (⑥) — an insufficient-n report can never
/// flip a feature on.
pub const MIN_SAMPLE_N: usize = 30;

/// The report's verdict (③). PASS is the ONLY value that can authorize a
/// promotion (⑥).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportVerdict {
    /// Genuine evaluation cleared the bar — promotion is permitted.
    Pass,
    /// Genuine evaluation did NOT clear the bar — promotion blocked.
    Fail,
    /// Not enough evaluated datapoints (`n < MIN_SAMPLE_N`) — promotion blocked,
    /// fail-closed.
    Insufficient,
}

/// Errors from report generation / verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    /// The report's `body_digest` does not match its contents + attestation —
    /// the report was forged or swapped (⑨). Fail-closed.
    Forged,
    /// The sealed corpus the report claims to cover has been mutated post-hoc —
    /// the verdict is invalidated (⑤). Fail-closed.
    CorpusTampered,
}

/// The generated, attested gate report (③⑨).
#[derive(Debug, Clone, PartialEq)]
pub struct GateReport {
    /// The derived verdict — never settable directly.
    verdict: ReportVerdict,
    /// Evaluated sample size at generation time.
    n: usize,
    /// Disjointness percentage at generation time.
    disjointness_pct: f64,
    /// Regen agree/disagree at generation time.
    regen_agree: usize,
    regen_disagree: usize,
    /// The seal digest of the corpus this report covers (binds report→corpus
    /// for the ⑤ post-hoc check).
    corpus_seal: String,
    /// The X2-class provenance attestation for this report (⑨).
    attestation: AttestationChain,
    /// Content digest binding verdict + facts + attestation (⑨).
    body_digest: String,
}

impl GateReport {
    /// Generate the report from a sealed corpus (③).
    ///
    /// The verdict is DERIVED:
    ///   - `n < MIN_SAMPLE_N` → [`ReportVerdict::Insufficient`] (fail-closed),
    ///   - otherwise the gate-clearing predicate decides PASS / FAIL.
    ///
    /// The predicate here is: ALL evaluated datapoints have disjoint claims AND
    /// regen agreed (the experiment's hypothesis held across the whole sample).
    /// The report is then sealed with `attestation` and a recomputable
    /// `body_digest` (⑨).
    pub fn generate(corpus: &SealedCorpus, attestation: AttestationChain) -> Self {
        let dash = Dashboard::from_sealed(corpus);
        let verdict = if dash.n < MIN_SAMPLE_N {
            ReportVerdict::Insufficient
        } else if dash.disjoint_count == dash.n && dash.regen_disagree == 0 {
            ReportVerdict::Pass
        } else {
            ReportVerdict::Fail
        };

        let mut report = GateReport {
            verdict,
            n: dash.n,
            disjointness_pct: dash.disjointness_pct(),
            regen_agree: dash.regen_agree,
            regen_disagree: dash.regen_disagree,
            corpus_seal: corpus.seal_digest().to_string(),
            attestation,
            body_digest: String::new(),
        };
        report.body_digest = report.compute_body_digest();
        report
    }

    /// The derived verdict (read-only).
    pub fn verdict(&self) -> ReportVerdict {
        self.verdict
    }

    /// Evaluated sample size.
    pub fn n(&self) -> usize {
        self.n
    }

    /// The provenance attestation (⑨).
    pub fn attestation(&self) -> &AttestationChain {
        &self.attestation
    }

    /// The seal digest of the corpus this report covers.
    pub fn corpus_seal(&self) -> &str {
        &self.corpus_seal
    }

    /// Verify the report is authentic and its corpus untampered (⑨ + ⑤).
    ///
    /// 1. Recompute `body_digest`; a mismatch means the verdict or facts were
    ///    altered without re-running `generate` → [`ReportError::Forged`].
    /// 2. Re-verify the bound sealed corpus has not been mutated post-hoc; a
    ///    broken seal → [`ReportError::CorpusTampered`].
    ///
    /// The gate calls this before honoring any report; a forged/swapped PASS
    /// therefore cannot enable promotion or billing through any path other than
    /// a genuine evaluation.
    pub fn verify(&self, corpus: &SealedCorpus) -> Result<(), ReportError> {
        if self.compute_body_digest() != self.body_digest {
            return Err(ReportError::Forged);
        }
        if self.corpus_seal != corpus.seal_digest() {
            return Err(ReportError::CorpusTampered);
        }
        if corpus.verify().is_err() {
            return Err(ReportError::CorpusTampered);
        }
        Ok(())
    }

    /// Produce a FORGED copy that swaps the verdict to PASS without a genuine
    /// re-evaluation — a fixture for ⑨. The `body_digest` is intentionally NOT
    /// recomputed, so [`verify`](GateReport::verify) must reject it.
    pub fn forged_pass(&self) -> GateReport {
        let mut forged = self.clone();
        forged.verdict = ReportVerdict::Pass;
        forged.n = MIN_SAMPLE_N.max(forged.n);
        // body_digest left stale on purpose — that is the forgery.
        forged
    }

    /// The content digest bound into the report (⑨): SHA-256 over the verdict,
    /// the evaluated facts, the covered corpus seal, and the attestation
    /// signature. Any change to any of these without re-`generate` breaks it.
    fn compute_body_digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update([self.verdict_tag()]);
        hasher.update((self.n as u64).to_be_bytes());
        hasher.update(self.disjointness_pct.to_be_bytes());
        hasher.update((self.regen_agree as u64).to_be_bytes());
        hasher.update((self.regen_disagree as u64).to_be_bytes());
        for field in &[self.corpus_seal.as_str(), self.attestation.sig.as_str()] {
            let bytes = field.as_bytes();
            hasher.update((bytes.len() as u32).to_be_bytes());
            hasher.update(bytes);
        }
        hex::encode(hasher.finalize())
    }

    fn verdict_tag(&self) -> u8 {
        match self.verdict {
            ReportVerdict::Pass => 1,
            ReportVerdict::Fail => 2,
            ReportVerdict::Insufficient => 3,
        }
    }
}
