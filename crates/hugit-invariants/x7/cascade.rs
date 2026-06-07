//! WP-X7 production surface — the right-to-erasure CASCADE.
//!
//! Where X12 proves erasure × provenance × mirror *compose* at the seam, X7
//! owns the broader **cross-store cascade**: a single data-subject erasure that
//! sweeps EVERY store and leaves no orphan, no silently broken seal. X7 composes
//! with X12's tombstone surface (the object/CAS leg) rather than re-transcribing
//! it; it adds the legs X12 does not own — the append-only ledger orphan scan,
//! the context-store purge (closing the X3③ PARTIAL), the experiment-corpus
//! seal-precedence resolution, and the post-erasure attestation re-seal-or-fail.
//!
//! # The five stores (item ①)
//!
//! A data subject's personal data is provably erased across:
//!
//! 1. **CAS / object store** — the live object is replaced by a tamper-evident
//!    [`Tombstone`] keyed by the SAME content hash (X12's surface, consumed).
//! 2. **provenance / ledger** — the append-only [`EventLog`] is NEVER rewritten;
//!    every surviving link resolves to a live target or a tombstone, never a
//!    void (item ②: no orphans).
//! 3. **context store** — bytes are genuinely PURGED (removed), not
//!    tombstoned-with-bytes; a post-purge byte scan finds the datum ABSENT. This
//!    closes the X3③ context-store-purge PARTIAL with the cross-store cascade it
//!    was waiting on.
//! 4. **GitHub mirror** — modelled as a [`MirrorObligation`] discharged-or-
//!    residual-risk-disclosed (the documented P2 seam, identical to X12②: live
//!    GitHub erasure is out of process for a hermetic test).
//! 5. **experiment corpus** — the sealed datapoint is removed despite the seal
//!    (item ④), and its absence is scannable.
//!
//! Each leg exposes an **absence scan** (not a flag) so the oracle asserts the
//! datum is gone by re-reading the store, never by trusting a boolean.
//!
//! # Items ②–④
//!
//! - **② no orphans:** [`scan_orphans`] walks every provenance link and reports
//!   any that resolves to [`Resolution::Missing`]. An erased object MUST leave a
//!   tombstone; a void is an orphan and a CONTRACT FAIL.
//! - **③ attestation re-seal OR fail-closed:** after erasure the
//!   [`AttestationChain`] is re-signed over the post-erasure manifest
//!   ([`reseal_attestation`]) and verifies via REAL ed25519 over the canonical
//!   `attestation_sig_preimage`; a chain left stale (payload changed, not
//!   re-sealed) fails CLOSED under [`verify_attestation`] — never a silently
//!   broken seal.
//! - **④ erasure × seal precedence:** [`SealedCorpus::erase_datapoint`] permits
//!   lawful erasure of a SEALED datapoint (the seal does not block it) and
//!   returns a [`GateInvalidation`] that re-pins the [`RegenGate`] fail-closed
//!   (`repass=false`, advisory) and AUDITS the invalidation — never silent.
//!
//! Everything here is cascade logic over the *consumed* surfaces (CAS/ledger/
//! context/mirror/corpus as-built, the canonical hash/preimage from
//! `hugit-refstore`, the frozen contracts types) — no new production behavior to
//! ship beyond the cascade + tombstone + invalidation surface.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use hugit_contracts::attestation_chain::AttestationChain;
use hugit_contracts::event_record::EventRecord;
use hugit_contracts::regen_gate::RegenGate;
use hugit_refstore::{
    TamperError, attestation_sig_preimage, canonical_json, compute_this_hash, verify_chain,
};
use serde::{Deserialize, Serialize};

// ── leg 1 + 2: CAS object store + tombstone (X12 surface, re-modelled here) ───
//
// X7 re-models the minimal object-store + tombstone here rather than depending
// on x12's module: the X7 claim is "only x7/** paths owned", and a #[path]
// include of x12/erasure.rs would couple the two test crates' source layout. The
// SHAPE is identical to X12's (same tamper-evident-tombstone law) — they compose
// at the property level, not the file level. The chain-verifiability proof below
// drives the REAL `hugit_refstore::verify_chain`, so the production hasher is the
// single source of truth either way.

/// The kind string for a provenance event that links to a content object. Its
/// payload names the object by content hash; erasure resolves that hash to a
/// [`Tombstone`].
pub const OBJECT_LINK_KIND: &str = "object.link";

/// The fixed tombstone marker tag.
pub const TOMBSTONE_MARKER: &str = "tombstone";

/// A tamper-evident erasure marker — the resolution target of a provenance link
/// whose object was lawfully erased. Carries the ORIGINAL object hash so any
/// verifier can confirm the surviving link still names the same (erased) object,
/// never a silent substitute. Not a broken/dangling link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    /// Content hash of the erased object; the surviving link references this.
    pub erased_object_hash: String,
    /// Why it was erased (e.g. a right-to-erasure request id) — audit annotation.
    pub reason: String,
    /// Marker tag (`"tombstone"`) distinguishing a tombstone from a live object.
    pub marker: String,
}

impl Tombstone {
    /// Mint a tamper-evident tombstone for `erased_object_hash`.
    pub fn new(erased_object_hash: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            erased_object_hash: erased_object_hash.into(),
            reason: reason.into(),
            marker: TOMBSTONE_MARKER.to_string(),
        }
    }

    /// Whether this is a well-formed tombstone (carries the marker tag).
    pub fn is_tombstone(&self) -> bool {
        self.marker == TOMBSTONE_MARKER
    }
}

/// What a provenance link resolves to in the object store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A live object (the bytes are present).
    Live(String),
    /// A tamper-evident tombstone (the object was lawfully erased).
    Tombstone(Tombstone),
    /// Nothing resolves — a genuinely orphaned/broken link. For an erased object
    /// this is a CONTRACT FAIL (erasure must leave a tombstone, never a void).
    Missing,
}

/// Content-addressed object store with an erasure (→ tombstone) operation — the
/// CAS leg of the cascade. Erasure NEVER touches the provenance chain: it swaps
/// the live object for a tombstone keyed by the SAME hash here, in the object
/// store, so the surviving link is not re-pointed and the hash-chain still
/// verifies.
#[derive(Debug, Clone, Default)]
pub struct ObjectStore {
    live: std::collections::BTreeMap<String, String>,
    tombstones: std::collections::BTreeMap<String, Tombstone>,
}

impl ObjectStore {
    /// A fresh, empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Put a live object under its content hash.
    pub fn put(&mut self, content_hash: impl Into<String>, bytes: impl Into<String>) {
        self.live.insert(content_hash.into(), bytes.into());
    }

    /// Erase the object at `content_hash`, installing a tamper-evident tombstone
    /// keyed by the SAME hash. Idempotent: an erased link never becomes
    /// [`Resolution::Missing`].
    pub fn erase(&mut self, content_hash: &str, reason: impl Into<String>) -> Tombstone {
        self.live.remove(content_hash);
        let ts = Tombstone::new(content_hash.to_string(), reason);
        self.tombstones.insert(content_hash.to_string(), ts.clone());
        ts
    }

    /// Resolve a content hash to its current state.
    pub fn resolve(&self, content_hash: &str) -> Resolution {
        if let Some(bytes) = self.live.get(content_hash) {
            Resolution::Live(bytes.clone())
        } else if let Some(ts) = self.tombstones.get(content_hash) {
            Resolution::Tombstone(ts.clone())
        } else {
            Resolution::Missing
        }
    }

    /// Whether the LIVE bytes of `content_hash` are present (the CAS absence scan
    /// for item ①: after erasure this is false — the live bytes are gone).
    pub fn live_bytes_present(&self, content_hash: &str) -> bool {
        self.live.contains_key(content_hash)
    }
}

/// Build the canonical payload for an object-link provenance record (shape
/// `{"object":"<hash>"}`), through the single-source [`canonical_json`].
pub fn object_link_payload(content_hash: &str) -> String {
    let raw = serde_json::json!({ "object": content_hash }).to_string();
    canonical_json(&raw).expect("object-link payload is valid JSON")
}

/// Read back the content hash an [`OBJECT_LINK_KIND`] record links to. `None`
/// for non-link records or malformed payloads.
pub fn link_target(record: &EventRecord) -> Option<String> {
    if record.kind != OBJECT_LINK_KIND {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&record.payload).ok()?;
    v.get("object")?.as_str().map(|s| s.to_string())
}

// ── item ② — orphan scan over the surviving provenance chain ──────────────────

/// An orphaned provenance link: a record that links to an object which resolves
/// to NOTHING (neither live nor a tombstone) after erasure — the dangling-ref a
/// correct cascade must never produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanRef {
    /// The seq of the record bearing the orphaned link.
    pub seq: u64,
    /// The content hash that fails to resolve.
    pub target: String,
}

/// Scan every provenance link in `records` against `store` and return any that
/// resolves to [`Resolution::Missing`] — the orphaned refs.
///
/// Item ②'s oracle surface: a CORRECT erasure leaves zero orphans (every link
/// resolves to a live object or a tamper-evident tombstone). It returns the
/// orphans rather than a bool so the oracle can assert WHICH ref dangled.
pub fn scan_orphans(records: &[EventRecord], store: &ObjectStore) -> Vec<OrphanRef> {
    let mut orphans = Vec::new();
    for r in records {
        if let Some(target) = link_target(r)
            && matches!(store.resolve(&target), Resolution::Missing)
        {
            orphans.push(OrphanRef { seq: r.seq, target });
        }
    }
    orphans
}

/// Detect a **silent re-link**: a record whose stored `this_hash` no longer
/// matches the canonical recomputation over its current payload — i.e. its link
/// was re-pointed at a substitute after sealing. Returns the substitute target.
///
/// The adversarial helper proving the cascade never hides an erasure behind a
/// re-pointed link rather than a tombstone.
pub fn detect_silent_relink(record: &EventRecord) -> Option<String> {
    let recomputed = compute_this_hash(
        &record.prev_hash,
        &record.kind,
        &record.principal_chain,
        &record.payload,
        record.seq,
    );
    if recomputed != record.this_hash {
        return link_target(record);
    }
    None
}

// ── leg 3 — context store (purge, not tombstone) — closes X3③ ─────────────────

/// A minimal context store whose erasure genuinely PURGES bytes (removes them),
/// not tombstones-them-with-bytes. The context/journal leg of the cascade.
///
/// This is the cross-store cascade leg the X3③ purge proof was waiting on: X3
/// proves the property in isolation against a PARTIAL stand-in (no production
/// context-store erasure API existed); X7 drives the SAME purge as part of the
/// five-store cascade. When a real context-store erasure API ships, both legs
/// re-point at it.
#[derive(Debug, Clone, Default)]
pub struct ContextStore {
    entries: std::collections::BTreeMap<String, String>,
}

impl ContextStore {
    /// A fresh, empty context store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a context/journal datum under `key`.
    pub fn put(&mut self, key: impl Into<String>, datum: impl Into<String>) {
        self.entries.insert(key.into(), datum.into());
    }

    /// Purge `key` — actually remove the bytes (not a tombstone-with-bytes).
    pub fn purge(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// Byte-level absence scan (item ①, context leg): true iff `needle` appears
    /// in NO stored datum. After purge of the subject's data this is true.
    pub fn datum_absent(&self, needle: &str) -> bool {
        !self.entries.values().any(|v| v.contains(needle))
    }
}

// ── leg 4 — GitHub mirror obligation (the documented P2 seam) ─────────────────

/// How a mirror-side erasure obligation resolved. Identical SHAPE to X12② — the
/// live GitHub erasure is out of process for a hermetic test, so the mirror leg
/// is modelled as discharge-or-residual-risk (the P2 seam).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum MirrorObligationOutcome {
    /// Mirror-side erasure executed AND verified (the GitHub copy is gone).
    Discharged {
        /// Mirror-side verification evidence (e.g. a confirmed deleted-ref / 404).
        verification: String,
    },
    /// Mirror-side erasure could not be fully guaranteed — the honest disclosure
    /// of residual risk is the deliverable.
    ResidualRisk {
        /// Statement of what may persist and why it cannot be guaranteed erased.
        disclosure: String,
    },
}

/// An erasure obligation against data already replicated to the GitHub mirror.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorObligation {
    /// Content hash whose mirror copy must be erased.
    pub object_hash: String,
    /// Where the data was replicated (the mirror target).
    pub mirror_target: String,
    /// How the obligation resolved.
    pub outcome: MirrorObligationOutcome,
}

impl MirrorObligation {
    /// True iff fully discharged (mirror-side erasure executed + verified).
    pub fn is_discharged(&self) -> bool {
        matches!(self.outcome, MirrorObligationOutcome::Discharged { .. })
    }

    /// The residual-risk disclosure text, if not discharged.
    pub fn residual_disclosure(&self) -> Option<&str> {
        match &self.outcome {
            MirrorObligationOutcome::ResidualRisk { disclosure } => Some(disclosure),
            MirrorObligationOutcome::Discharged { .. } => None,
        }
    }

    /// True iff the mirror obligation is HONESTLY resolved: discharged, OR a
    /// non-empty residual-risk disclosure (an empty disclosure is an omission
    /// masquerading as one — fails closed). The mirror leg's absence-or-honest
    /// -disclosure predicate for item ①.
    pub fn is_honestly_resolved(&self) -> bool {
        match &self.outcome {
            MirrorObligationOutcome::Discharged { .. } => true,
            MirrorObligationOutcome::ResidualRisk { disclosure } => !disclosure.trim().is_empty(),
        }
    }
}

// ── leg 5 + item ④ — sealed experiment corpus × erasure precedence ────────────

/// A sealed experiment-corpus datapoint (D8 seals the corpus before evaluation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusDatapoint {
    /// Stable id of the datapoint within the corpus.
    pub id: String,
    /// The datapoint payload (may carry personal data).
    pub data: String,
}

/// The audited record of a gate-verdict invalidation caused by a lawful erasure
/// of a SEALED corpus datapoint. The invalidation is NEVER silent: it is a
/// first-class, serialisable audit object naming the erased datapoint and the
/// re-pinned (fail-closed) gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateInvalidation {
    /// The id of the corpus datapoint whose erasure invalidated the verdict.
    pub erased_datapoint: String,
    /// The right-to-erasure request that authorised the erasure.
    pub reason: String,
    /// The gate re-pinned fail-closed: `repass=false`, advisory until
    /// re-evaluation. Claims-as-oracle stays advisory, regen promotion blocked.
    pub repinned_gate: RegenGate,
}

impl GateInvalidation {
    /// True iff the gate was genuinely re-pinned FAIL-CLOSED: not re-passed
    /// (regen promotion blocked) AND the independent verdict cleared (the prior
    /// PASS no longer counts) — the verdict is invalidated, not silently kept.
    pub fn is_failclosed(&self) -> bool {
        !self.repinned_gate.repass && self.repinned_gate.indep_verdict.is_empty()
    }
}

/// Outcome of erasing a sealed corpus datapoint — item ④'s three clauses.
#[derive(Debug, Clone, PartialEq)]
pub enum CorpusErasureError {
    /// The datapoint was not present in the corpus (nothing to erase).
    NotFound { id: String },
}

impl std::fmt::Display for CorpusErasureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CorpusErasureError::NotFound { id } => {
                write!(f, "corpus erasure: datapoint {id} not found")
            }
        }
    }
}

impl std::error::Error for CorpusErasureError {}

/// The sealed experiment corpus. The seal does NOT block a lawful erasure (item
/// ④a): erasure wins. Erasing a datapoint after the seal invalidates the gate
/// verdict fail-closed (item ④b) and records that invalidation (item ④c — not
/// silent).
#[derive(Debug, Clone)]
pub struct SealedCorpus {
    datapoints: std::collections::BTreeMap<String, CorpusDatapoint>,
    /// Whether the corpus has been sealed (D8④ pre-evaluation seal).
    sealed: bool,
}

impl SealedCorpus {
    /// Build an UNSEALED corpus from datapoints.
    pub fn new(datapoints: Vec<CorpusDatapoint>) -> Self {
        Self {
            datapoints: datapoints.into_iter().map(|d| (d.id.clone(), d)).collect(),
            sealed: false,
        }
    }

    /// Seal the corpus (D8④: sealed before evaluation; sample selection frozen).
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    /// Whether the corpus is sealed.
    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Whether `id` is still present in the corpus.
    pub fn contains(&self, id: &str) -> bool {
        self.datapoints.contains_key(id)
    }

    /// Byte-level absence scan (item ①, corpus leg): true iff `needle` appears in
    /// NO datapoint payload. After erasure of the subject's datapoint this holds.
    pub fn datum_absent(&self, needle: &str) -> bool {
        !self.datapoints.values().any(|d| d.data.contains(needle))
    }

    /// Erase a datapoint and re-pin the gate fail-closed.
    ///
    /// Item ④, all three clauses:
    /// - **(a)** erasure is PERMITTED despite the seal — the seal does not block
    ///   it; the datapoint is genuinely removed.
    /// - **(b)** the gate verdict INVALIDATES fail-closed — `current_gate` is
    ///   re-pinned to `repass=false`, `indep_verdict=""` (advisory/OFF/blocked
    ///   until re-evaluation), regardless of its prior PASS.
    /// - **(c)** the invalidation is RECORDED (the returned [`GateInvalidation`]
    ///   is an audited object) — never a silently broken seal.
    ///
    /// Returns the audit record. Erasing on an unsealed corpus is still lawful
    /// and still invalidates any standing verdict (the precedence is the same).
    pub fn erase_datapoint(
        &mut self,
        id: &str,
        reason: impl Into<String>,
        current_gate: &RegenGate,
    ) -> Result<GateInvalidation, CorpusErasureError> {
        if !self.datapoints.contains_key(id) {
            return Err(CorpusErasureError::NotFound { id: id.to_string() });
        }
        // (a) erasure wins over the seal — remove the datapoint.
        self.datapoints.remove(id);

        // (b)+(c) re-pin the gate fail-closed and record the invalidation.
        let repinned_gate = RegenGate {
            optin_scope: current_gate.optin_scope.clone(),
            repass: false,
            indep_verdict: String::new(),
        };
        Ok(GateInvalidation {
            erased_datapoint: id.to_string(),
            reason: reason.into(),
            repinned_gate,
        })
    }
}

// ── item ③ — attestation re-seal OR fail-closed after erasure ─────────────────

/// Sign `chain` with `signing_key` over the canonical
/// [`attestation_sig_preimage`], returning a chain whose `sig` is the
/// base64-encoded Ed25519 signature. The single-sourced preimage from
/// `hugit-refstore` is used — never a re-transcribed byte format.
pub fn sign_attestation(signing_key: &SigningKey, chain: &AttestationChain) -> AttestationChain {
    let msg = attestation_sig_preimage(
        &chain.tree,
        &chain.def,
        &chain.runner,
        &chain.model,
        &chain.principal,
    );
    let sig: Signature = signing_key.sign(&msg);
    AttestationChain {
        sig: B64.encode(sig.to_bytes()),
        ..chain.clone()
    }
}

/// Why a post-erasure attestation failed to verify — every variant is a
/// fail-closed rejection (item ③: never a silently broken seal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationError {
    /// The `sig` field is empty (an unsigned chain).
    Unsigned,
    /// The signature bytes are malformed (not a valid base64 Ed25519 signature).
    MalformedSignature,
    /// The signature does not verify against the public key over the chain's
    /// current links — e.g. a chain re-pointed at a tombstone manifest but NOT
    /// re-sealed (the seal would be silently broken; this catches it).
    SignatureMismatch,
}

impl std::fmt::Display for AttestationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttestationError::Unsigned => write!(f, "attestation: unsigned (empty sig)"),
            AttestationError::MalformedSignature => {
                write!(f, "attestation: malformed ed25519 signature")
            }
            AttestationError::SignatureMismatch => {
                write!(f, "attestation: signature does not verify (seal broken)")
            }
        }
    }
}

impl std::error::Error for AttestationError {}

/// Verify `chain`'s Ed25519 signature against `verifying_key` over the canonical
/// preimage. Fails CLOSED on an unsigned, malformed, or non-verifying chain.
///
/// Item ③'s verifier: after erasure a chain is EITHER re-sealed (re-signed over
/// the post-erasure manifest — this returns `Ok`) OR left stale (links changed,
/// sig not refreshed — this returns `Err`, fail-closed). There is no third
/// "silently broken but accepted" outcome.
pub fn verify_attestation(
    verifying_key: &VerifyingKey,
    chain: &AttestationChain,
) -> Result<(), AttestationError> {
    if chain.sig.is_empty() {
        return Err(AttestationError::Unsigned);
    }
    let raw = B64
        .decode(chain.sig.as_bytes())
        .map_err(|_| AttestationError::MalformedSignature)?;
    let arr: [u8; 64] = raw
        .try_into()
        .map_err(|_| AttestationError::MalformedSignature)?;
    let sig = Signature::from_bytes(&arr);
    let msg = attestation_sig_preimage(
        &chain.tree,
        &chain.def,
        &chain.runner,
        &chain.model,
        &chain.principal,
    );
    verifying_key
        .verify_strict(&msg, &sig)
        .map_err(|_| AttestationError::SignatureMismatch)
}

/// Re-seal an attestation after erasure: re-point its `tree` link at the
/// post-erasure manifest (here, the tombstone-bearing tree) and RE-SIGN it, so
/// the chain stays cryptographically intact over the new state.
///
/// Item ③'s "re-seals" branch. The alternative — re-pointing the link WITHOUT
/// re-signing — leaves a stale sig that [`verify_attestation`] rejects
/// (fail-closed). Both branches are honest; only a stale-and-accepted chain
/// would be a silently broken seal, which the verifier makes impossible.
pub fn reseal_attestation(
    signing_key: &SigningKey,
    chain: &AttestationChain,
    post_erasure_tree: impl Into<String>,
) -> AttestationChain {
    let relinked = AttestationChain {
        tree: post_erasure_tree.into(),
        ..chain.clone()
    };
    sign_attestation(signing_key, &relinked)
}

// ── the cascade composite (item ① — all five legs in one scan) ────────────────

/// The result of scanning all five stores for the data subject's data AFTER the
/// cascade. Each field is an ABSENCE scan (re-reading the store), not a flag.
#[derive(Debug, Clone)]
pub struct CascadeAbsenceScan {
    /// CAS: the live object bytes are gone (resolves to a tombstone, not live).
    pub cas_live_absent: bool,
    /// Provenance/ledger: zero orphaned refs survive (every link → live|tombstone).
    pub no_orphans: bool,
    /// Provenance chain still verifies byte-for-byte (never silently re-linked).
    pub chain_verifies: Result<(), TamperError>,
    /// Context store: the datum bytes are purged (absent on a byte scan).
    pub context_absent: bool,
    /// GitHub mirror: the obligation is honestly resolved (discharged or a
    /// non-empty residual-risk disclosure — the P2 seam).
    pub mirror_resolved: bool,
    /// Experiment corpus: the sealed datapoint's bytes are absent on a byte scan.
    pub corpus_absent: bool,
}

impl CascadeAbsenceScan {
    /// True iff item ① holds across ALL FIVE stores: CAS live bytes gone,
    /// provenance verifiable with zero orphans, context purged, mirror honestly
    /// resolved, corpus datapoint absent.
    pub fn fully_erased(&self) -> bool {
        self.cas_live_absent
            && self.no_orphans
            && self.chain_verifies.is_ok()
            && self.context_absent
            && self.mirror_resolved
            && self.corpus_absent
    }
}

/// Scan all five stores for the subject's data after the cascade. The provenance
/// chain is checked with the REAL `hugit_refstore::verify_chain`; orphans via
/// [`scan_orphans`]; CAS/context/corpus via their byte-absence scans; the mirror
/// via the obligation's honest-resolution predicate.
#[allow(clippy::too_many_arguments)]
pub fn scan_cascade_absence(
    records: &[EventRecord],
    cas: &ObjectStore,
    cas_object_hash: &str,
    context: &ContextStore,
    corpus: &SealedCorpus,
    mirror: &MirrorObligation,
    needle: &str,
) -> CascadeAbsenceScan {
    CascadeAbsenceScan {
        cas_live_absent: !cas.live_bytes_present(cas_object_hash),
        no_orphans: scan_orphans(records, cas).is_empty(),
        chain_verifies: verify_chain(records),
        context_absent: context.datum_absent(needle),
        mirror_resolved: mirror.is_honestly_resolved(),
        corpus_absent: corpus.datum_absent(needle),
    }
}
