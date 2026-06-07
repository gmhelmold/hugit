//! WP-X12 production surface — erasure × provenance × mirror.
//!
//! This module models the **intersection** of three already-built surfaces —
//! the erasure cascade (X7), the attestation/provenance chain (X2/refstore),
//! and the GitHub mirror (E1) tied into the export/exit proof (E5). It owns the
//! *tombstone + post-erasure verifiability* surface and the *mirror-obligation
//! disclosure* surface; it does NOT re-prove the cascade, the mirror, or the
//! export — it proves they **compose** without silently breaking provenance.
//!
//! # The composition law (WP-X12 owned items)
//!
//! ① **After an erasure request the attestation chain remains INDEPENDENTLY
//!    verifiable with the erased object as a tamper-evident TOMBSTONE — never
//!    silently re-linked.** Erasure operates on the *object store*, never on the
//!    append-only provenance chain: the [`EventRecord`] that references an
//!    object keeps referencing the SAME content hash, which now resolves to a
//!    [`Tombstone`] bearing that exact hash. The hash-chain
//!    ([`verify_chain`](hugit_refstore::verify_chain)) therefore still verifies
//!    byte-for-byte after erasure. A *silent re-link* — re-pointing the record
//!    at some substitute object so the link "still resolves" — mutates the
//!    record's payload and is caught fail-closed by `verify_chain`. The
//!    tombstone is *tamper-evident*: it is a deliberate, self-describing erasure
//!    marker carrying the original object hash, not a broken/dangling link.
//!
//! ② **The mirror-side erasure obligation is DISCHARGED or explicitly surfaced
//!    as RESIDUAL RISK — and that disclosure is part of the export/exit proof.**
//!    Data already replicated to the GitHub mirror creates an erasure obligation
//!    that physical control over GitHub cannot fully guarantee. The obligation
//!    is modelled as a [`MirrorObligation`] that resolves to EITHER
//!    [`MirrorObligationOutcome::Discharged`] (mirror-side erasure executed +
//!    verified) OR [`MirrorObligationOutcome::ResidualRisk`] (an honest, stated
//!    residual-risk disclosure). The export/exit artifact
//!    ([`ExitProof`]) carries that disclosure as a first-class element validated
//!    against the frozen [`ExportSchema`](hugit_contracts::ExportSchema); an
//!    export that omits the disclosure when an obligation exists is REJECTED.
//!    The honest disclosure IS the deliverable when full discharge is not
//!    provable.
//!
//! Everything here is composition logic over the *consumed* surfaces (X7/E1/E5
//! as-built, the canonical attestation/hash from `hugit-refstore`) — there is no
//! new production behavior to ship beyond the tombstone + disclosure surface.

use hugit_contracts::event_record::EventRecord;
use hugit_contracts::export_schema::ExportSchema;
use hugit_refstore::{TamperError, canonical_json, compute_this_hash, verify_chain};
use serde::{Deserialize, Serialize};

// ── Item ① — tombstone + post-erasure provenance verifiability ───────────────

/// The kind string used for the provenance event that *links to a content
/// object*. The payload of such a record names the object by its content hash;
/// erasure later resolves that hash to a [`Tombstone`].
pub const OBJECT_LINK_KIND: &str = "object.link";

/// A tamper-evident erasure marker — the resolution target of a provenance link
/// whose object has been erased (the X7 cascade leg, consumed as-built).
///
/// A tombstone is NOT a broken/dangling link: it is a deliberate, self-describing
/// record of a lawful erasure that carries the **original object's content hash**
/// so any independent verifier can confirm the surviving provenance link still
/// points at the SAME object (now erased) — not at a silent substitute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    /// The content hash of the object that was erased. The surviving provenance
    /// link references this exact hash; resolving it yields this tombstone.
    pub erased_object_hash: String,

    /// Why the object was erased (e.g. a right-to-erasure request id). An
    /// observability/audit annotation — the integrity of the marker comes from
    /// `erased_object_hash` matching the surviving link, not from this string.
    pub reason: String,

    /// Marker tag making a tombstone trivially distinguishable from a live
    /// object on the wire (`"tombstone"`).
    pub marker: String,
}

/// The fixed tombstone marker tag.
pub const TOMBSTONE_MARKER: &str = "tombstone";

impl Tombstone {
    /// Mint a tamper-evident tombstone for `erased_object_hash`.
    pub fn new(erased_object_hash: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            erased_object_hash: erased_object_hash.into(),
            reason: reason.into(),
            marker: TOMBSTONE_MARKER.to_string(),
        }
    }

    /// Whether this value is a well-formed tombstone (carries the marker tag).
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
    /// Nothing resolves — a genuinely orphaned/broken link (a CONTRACT FAIL for
    /// an erased object: erasure must leave a tombstone, never a void).
    Missing,
}

/// A minimal content-addressed object store with an erasure (→ tombstone)
/// operation, modelling the X7 cascade's object leg as-built.
///
/// Erasure NEVER touches the provenance chain — it operates here, on the object
/// store, swapping the live object for a tombstone keyed by the SAME content
/// hash. This is the property item ① turns on: the surviving provenance link is
/// not re-pointed, so the hash-chain still verifies.
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

    /// Erase the object at `content_hash`, replacing it with a tamper-evident
    /// tombstone keyed by the SAME hash. The provenance chain is never touched.
    ///
    /// Returns the minted [`Tombstone`]. Idempotent: erasing an already-erased
    /// (or never-present) hash still installs/keeps a tombstone, so an erased
    /// link never resolves to [`Resolution::Missing`].
    pub fn erase(&mut self, content_hash: &str, reason: impl Into<String>) -> Tombstone {
        self.live.remove(content_hash);
        let ts = Tombstone::new(content_hash.to_string(), reason);
        self.tombstones.insert(content_hash.to_string(), ts.clone());
        ts
    }

    /// Resolve a content hash to its current state in the store.
    pub fn resolve(&self, content_hash: &str) -> Resolution {
        if let Some(bytes) = self.live.get(content_hash) {
            Resolution::Live(bytes.clone())
        } else if let Some(ts) = self.tombstones.get(content_hash) {
            Resolution::Tombstone(ts.clone())
        } else {
            Resolution::Missing
        }
    }
}

/// The content hash a record uses as its provenance link target.
///
/// By convention an [`OBJECT_LINK_KIND`] record's payload is canonical JSON of
/// the shape `{"object":"<content-hash>"}`. This reads that link target back so
/// it can be resolved against an [`ObjectStore`]. Returns `None` for records
/// that are not object links or whose payload is malformed.
pub fn link_target(record: &EventRecord) -> Option<String> {
    if record.kind != OBJECT_LINK_KIND {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&record.payload).ok()?;
    v.get("object")?.as_str().map(|s| s.to_string())
}

/// Build the canonical payload for an object-link provenance record.
///
/// Goes through [`canonical_json`] so the chained bytes are deterministic — the
/// same single-source canonicaliser every emitter uses.
pub fn object_link_payload(content_hash: &str) -> String {
    let raw = serde_json::json!({ "object": content_hash }).to_string();
    canonical_json(&raw).expect("object-link payload is valid JSON")
}

/// The result of independently verifying a provenance chain *after* an erasure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostErasureVerification {
    /// `Ok` iff the hash-chain still verifies byte-for-byte (the chain is
    /// independently verifiable — item ① clause 1).
    pub chain: Result<(), TamperError>,
    /// What the surviving object link resolves to (a [`Resolution::Tombstone`]
    /// for an erased object — item ① clause 2: tamper-evident, never `Missing`).
    pub link_resolution: Resolution,
}

impl PostErasureVerification {
    /// True iff item ① holds: the chain still verifies AND the erased object's
    /// link resolves to a tamper-evident tombstone bearing the SAME content hash
    /// the record links to (never silently re-linked, never an orphan).
    pub fn is_independently_verifiable_over_tombstone(&self, linked_hash: &str) -> bool {
        if self.chain.is_err() {
            return false;
        }
        match &self.link_resolution {
            Resolution::Tombstone(ts) => ts.is_tombstone() && ts.erased_object_hash == linked_hash,
            _ => false,
        }
    }
}

/// Independently verify a provenance chain after an erasure, against an
/// [`ObjectStore`] in which the linked object has been erased.
///
/// This is item ①'s oracle surface: it re-runs the canonical
/// [`verify_chain`](hugit_refstore::verify_chain) over the (immutable) records
/// AND resolves the surviving object link. A correct erasure leaves the chain
/// intact and the link resolving to a tombstone; a *silent re-link* (a record
/// whose payload was swapped to point at a substitute) is caught because the
/// re-write changes `this_hash` and `verify_chain` fails closed.
pub fn verify_after_erasure(
    records: &[EventRecord],
    store: &ObjectStore,
    linked_hash: &str,
) -> PostErasureVerification {
    PostErasureVerification {
        chain: verify_chain(records),
        link_resolution: store.resolve(linked_hash),
    }
}

/// Detect a **silent re-link**: a record that claims to be the original
/// object-link (same `seq`/`kind`) but whose stored `this_hash` no longer
/// matches the canonical recomputation over its current payload — i.e. the link
/// was re-pointed at a substitute object after the fact.
///
/// This is the adversarial helper the oracle uses to PROVE the chain catches a
/// silent re-link rather than just trusting `verify_chain`. Returns the new
/// (substitute) link target if a re-link is detected on `record`.
pub fn detect_silent_relink(record: &EventRecord) -> Option<String> {
    let recomputed = compute_this_hash(
        &record.prev_hash,
        &record.kind,
        &record.principal_chain,
        &record.payload,
        record.seq,
    );
    if recomputed != record.this_hash {
        // The payload was altered after sealing — read what it now points at.
        return link_target(record);
    }
    None
}

// ── Item ② — mirror-side erasure obligation × export/exit proof ──────────────

/// How a mirror-side erasure obligation was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum MirrorObligationOutcome {
    /// Mirror-side erasure was executed AND verified (the GitHub copy is gone,
    /// confirmed). No residual risk remains for this object.
    Discharged {
        /// The mirror-side verification evidence (e.g. a confirmed 404 / deleted
        /// ref proof). Free-form audit string.
        verification: String,
    },
    /// Mirror-side erasure could NOT be fully guaranteed — physical control over
    /// GitHub does not extend to forks/caches/backups. The honest disclosure of
    /// this residual risk is the deliverable.
    ResidualRisk {
        /// Human-readable statement of the residual risk (what may persist and
        /// why it cannot be guaranteed erased).
        disclosure: String,
    },
}

/// An erasure obligation against data already replicated to the GitHub mirror
/// (the E1 leg, consumed as-built).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorObligation {
    /// The content hash of the object whose mirror copy must be erased.
    pub object_hash: String,
    /// Where the data was replicated (the mirror target, e.g. an org/repo).
    pub mirror_target: String,
    /// How the obligation resolved.
    pub outcome: MirrorObligationOutcome,
}

impl MirrorObligation {
    /// True iff this obligation was fully discharged (mirror-side erasure
    /// executed + verified). When false, a residual-risk disclosure is required
    /// in the exit proof.
    pub fn is_discharged(&self) -> bool {
        matches!(self.outcome, MirrorObligationOutcome::Discharged { .. })
    }

    /// The residual-risk disclosure text, if this obligation was NOT discharged.
    pub fn residual_disclosure(&self) -> Option<&str> {
        match &self.outcome {
            MirrorObligationOutcome::ResidualRisk { disclosure } => Some(disclosure),
            MirrorObligationOutcome::Discharged { .. } => None,
        }
    }
}

/// The object-class name, present in every X12 export, under which the
/// mirror-obligation disclosure block travels in the export/exit proof. Its
/// presence in [`ExportSchema::object_classes`] is the structural hook that ties
/// the disclosure into the E5 export proof.
pub const MIRROR_OBLIGATION_CLASS: &str = "mirror_erasure_obligation";

/// The export/exit proof artifact (the E5 leg, consumed as-built) extended with
/// the X12 mirror-obligation disclosure.
///
/// The `schema` is the frozen [`ExportSchema`] envelope; the `obligations` carry
/// the per-object discharge-or-residual-risk records. Item ②'s law: every
/// obligation is EITHER discharged OR carries a residual-risk disclosure, AND
/// when any obligation is present the schema MUST advertise
/// [`MIRROR_OBLIGATION_CLASS`] in its `object_classes` (the disclosure is a
/// stated element of the export proof, validated against the schema).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExitProof {
    /// The frozen export envelope (E5/`ExportSchema`).
    pub schema: ExportSchema,
    /// Per-object mirror-erasure obligations folded into the exit proof.
    pub obligations: Vec<MirrorObligation>,
}

/// Why an exit proof fails the X12 ② composition law.
///
/// Note: there is no "neither discharged nor disclosed" variant — the
/// [`MirrorObligationOutcome`] enum makes that case *structurally
/// unrepresentable* (an obligation is always one or the other). The remaining
/// failure modes are an EMPTY residual disclosure (an omission masquerading as a
/// disclosure) and the disclosure not being a stated element of the export
/// schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitProofError {
    /// A residual-risk obligation exists but its disclosure text is empty — an
    /// omission masquerading as a disclosure.
    EmptyResidualDisclosure { object_hash: String },
    /// Obligations are present but the export schema does not advertise the
    /// mirror-obligation object class — the disclosure is not a stated element of
    /// the export/exit proof.
    DisclosureNotInExportSchema,
}

impl std::fmt::Display for ExitProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitProofError::EmptyResidualDisclosure { object_hash } => write!(
                f,
                "exit-proof: residual-risk obligation for {object_hash} has an empty disclosure"
            ),
            ExitProofError::DisclosureNotInExportSchema => write!(
                f,
                "exit-proof: obligations present but '{MIRROR_OBLIGATION_CLASS}' missing from ExportSchema.object_classes"
            ),
        }
    }
}

impl std::error::Error for ExitProofError {}

impl ExitProof {
    /// Validate the X12 ② composition law over this exit proof.
    ///
    /// `Ok(())` iff: every obligation is discharged OR carries a non-empty
    /// residual-risk disclosure, AND — when any obligation is present — the
    /// export schema advertises [`MIRROR_OBLIGATION_CLASS`] (the disclosure is a
    /// stated element of the export/exit proof, validated against the frozen
    /// `ExportSchema`). Fails closed otherwise.
    pub fn validate(&self) -> Result<(), ExitProofError> {
        for ob in &self.obligations {
            match &ob.outcome {
                MirrorObligationOutcome::Discharged { .. } => {}
                MirrorObligationOutcome::ResidualRisk { disclosure } => {
                    if disclosure.trim().is_empty() {
                        return Err(ExitProofError::EmptyResidualDisclosure {
                            object_hash: ob.object_hash.clone(),
                        });
                    }
                }
            }
        }

        if !self.obligations.is_empty()
            && !self
                .schema
                .object_classes
                .iter()
                .any(|c| c == MIRROR_OBLIGATION_CLASS)
        {
            return Err(ExitProofError::DisclosureNotInExportSchema);
        }

        Ok(())
    }

    /// True iff the exit proof validates under the X12 ② law.
    pub fn discloses_or_discharges_all(&self) -> bool {
        self.validate().is_ok()
    }
}
