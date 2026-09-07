//! The export envelope and its machine validation against the frozen
//! [`ExportSchema`] (E5③/⑥).
//!
//! The on-disk JSON dump is an [`ExportEnvelope`]. Its `schema` field is the
//! frozen [`hugit_contracts::ExportSchema`] — the versioned, machine-validatable
//! contract surface — and the rest of the envelope carries the first-class
//! objects object-for-object. Validation is a *machine check* against the frozen
//! schema's invariants (③); it is never a doc-presence heuristic.

use hugit_contracts::{AttestationChain, EventRecord, ExportSchema, IntentSidecar, VerdictObject};
use serde::{Deserialize, Serialize};

/// Export format version this dumper emits (semver). Carried in the envelope's
/// frozen [`ExportSchema::version`] field.
pub const EXPORT_FORMAT_VERSION: &str = "1.0.0";

/// The canonical, exhaustive list of first-class object classes hugit reproduces
/// **object-for-object** on export (E5⑥). The envelope's
/// [`ExportSchema::object_classes`] is asserted equal to this set at validation.
///
/// Every class here has a corresponding field on [`ExportEnvelope`]. Any class
/// hugit does NOT export must instead be enumerated in [`OUT_OF_SCOPE_CLASSES`]
/// — no silent omission (E5⑥).
pub const FIRST_CLASS_OBJECT_CLASSES: &[&str] = &[
    "refs",
    "intents",
    "events",
    "ledger",
    "verdicts",
    "journals",
    "policy",
    "provenance_links",
];

/// Classes deliberately **not** exported, enumerated in the schema so the
/// omission is explicit and auditable (E5⑥ "any out-of-scope class explicitly
/// enumerated"). Empty today — hugit exports every first-class class — but the
/// envelope still records it so an empty set is a positive assertion, not an
/// accident.
pub const OUT_OF_SCOPE_CLASSES: &[&str] = &[];

/// A single ledger entry (cost/billing record) reproduced object-for-object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Stable id of the ledger entry.
    pub entry_id: String,
    /// Intent or event this charge attaches to.
    pub subject: String,
    /// Flat, content-memoized amount (never usage-billing whiplash).
    pub amount_cents: u64,
}

/// A session journal object, bound to a workspace/intent (cf. D11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Stable id of the journal.
    pub journal_id: String,
    /// The intent this journal is bound to.
    pub intent_id: String,
    /// Free-form journal body (subject to redaction at export).
    pub body: String,
}

/// A policy object (a declarative gate set snapshot).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyObject {
    /// Stable id of the policy.
    pub policy_id: String,
    /// Canonical serialized gate set.
    pub gates: String,
}

/// A provenance link tying a derived/landed object back to its originating
/// event (cf. X14 deep-link integrity). The exit-cut invariant (E5⑨) is: every
/// link's `event_seq` references an event that is present in the cut.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceLink {
    /// The object (ref/intent/verdict/…) this link is about.
    pub object_id: String,
    /// The log sequence number of the originating event.
    pub event_seq: u64,
}

/// The full on-disk export envelope. Serialized to the JSON sidecar of the dump;
/// the git artifact is written alongside (E5①).
///
/// Every first-class class in [`FIRST_CLASS_OBJECT_CLASSES`] has a field here and
/// is reproduced object-for-object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportEnvelope {
    /// The frozen, versioned, machine-validatable schema descriptor.
    pub schema: ExportSchema,

    /// Refs: name → target object id (the D1 [`RefState`] projection).
    ///
    /// [`RefState`]: hugit_refstore::RefState
    pub refs: Vec<RefEntry>,
    /// Native landed intents (the D4 intent altitude).
    pub intents: Vec<IntentSidecar>,
    /// The canonical full append-only, hash-chained event log (D1),
    /// object-for-object. Restore verifies this chain and rejects altered events.
    pub events: Vec<EventRecord>,
    /// Ledger entries.
    pub ledger: Vec<LedgerEntry>,
    /// Adversarial verdict objects.
    pub verdicts: Vec<VerdictObject>,
    /// Session journals.
    pub journals: Vec<JournalEntry>,
    /// Policy snapshots.
    pub policy: Vec<PolicyObject>,
    /// Provenance links (deep-link integrity).
    pub provenance_links: Vec<ProvenanceLink>,
    /// Attestation chains (CI provenance), part of the provenance surface.
    pub attestations: Vec<AttestationChain>,
}

/// One ref binding in the export (name → target object id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefEntry {
    /// The ref name (e.g. `refs/heads/main`).
    pub name: String,
    /// The git object id the ref points at.
    pub target: String,
}

/// Why a machine-check validation of an [`ExportEnvelope`] failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// The envelope declared no schema version.
    MissingVersion,
    /// The redaction-manifest ref was empty (a manifest is mandatory — even a
    /// no-op redaction manifests its empty removal set, E5⑦).
    MissingRedactionManifest,
    /// The declared `object_classes` set did not equal the canonical first-class
    /// set — a class was silently added or dropped (E5⑥).
    ObjectClassMismatch {
        /// What the canonical set is.
        expected: Vec<String>,
        /// What the envelope declared.
        got: Vec<String>,
    },
    /// A first-class class is declared in the schema but carries no objects AND
    /// is not enumerated out-of-scope — i.e. a silent omission (E5⑥). This is
    /// only an error when the class is genuinely absent from the envelope shape;
    /// an empty-but-present field is legal.
    UndeclaredOmission {
        /// The class that was neither populated nor enumerated out-of-scope.
        class: String,
    },
    /// A provenance link references an event sequence not present in the cut —
    /// a dangling link (E5⑨).
    DanglingProvenanceLink {
        /// The object whose link dangles.
        object_id: String,
        /// The missing event sequence.
        event_seq: u64,
    },
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::MissingVersion => write!(f, "export schema: missing version"),
            SchemaError::MissingRedactionManifest => {
                write!(f, "export schema: missing redaction manifest ref")
            }
            SchemaError::ObjectClassMismatch { expected, got } => write!(
                f,
                "export schema: object_classes mismatch (expected {expected:?}, got {got:?})"
            ),
            SchemaError::UndeclaredOmission { class } => write!(
                f,
                "export schema: class '{class}' is empty and not enumerated out-of-scope (silent omission)"
            ),
            SchemaError::DanglingProvenanceLink {
                object_id,
                event_seq,
            } => write!(
                f,
                "export schema: provenance link for '{object_id}' references absent event seq {event_seq}"
            ),
        }
    }
}

impl std::error::Error for SchemaError {}

impl ExportEnvelope {
    /// Build the frozen [`ExportSchema`] descriptor for an export, declaring the
    /// canonical first-class set plus the explicit out-of-scope enumeration.
    pub fn build_schema(redaction_manifest: String) -> ExportSchema {
        let mut object_classes: Vec<String> = FIRST_CLASS_OBJECT_CLASSES
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Out-of-scope classes are enumerated with an explicit marker so they
        // are never confused with exported classes.
        for oos in OUT_OF_SCOPE_CLASSES {
            object_classes.push(format!("!out-of-scope:{oos}"));
        }
        ExportSchema {
            version: EXPORT_FORMAT_VERSION.to_string(),
            object_classes,
            redaction_manifest,
        }
    }

    /// Machine-validate this envelope against the frozen [`ExportSchema`]
    /// invariants (E5③/⑥/⑨). Returns `Ok(())` iff:
    ///
    /// 1. a non-empty version is declared,
    /// 2. a redaction manifest ref is present (even a no-op manifests its empty
    ///    removal set),
    /// 3. the declared first-class `object_classes` equal the canonical set
    ///    (no silent add/drop), and out-of-scope classes are enumerated,
    /// 4. every populated/known class is accounted for (no silent omission),
    /// 5. no provenance link dangles past the cut.
    pub fn validate(&self) -> Result<(), SchemaError> {
        // (1) version present.
        if self.schema.version.trim().is_empty() {
            return Err(SchemaError::MissingVersion);
        }

        // (2) redaction manifest present.
        if self.schema.redaction_manifest.trim().is_empty() {
            return Err(SchemaError::MissingRedactionManifest);
        }

        // (3) first-class object_classes == canonical set (ignoring the
        // out-of-scope markers, which we check separately).
        let declared_first_class: Vec<String> = self
            .schema
            .object_classes
            .iter()
            .filter(|c| !c.starts_with("!out-of-scope:"))
            .cloned()
            .collect();
        let canonical: Vec<String> = FIRST_CLASS_OBJECT_CLASSES
            .iter()
            .map(|s| s.to_string())
            .collect();
        if declared_first_class != canonical {
            return Err(SchemaError::ObjectClassMismatch {
                expected: canonical,
                got: declared_first_class,
            });
        }

        // (4) the out-of-scope enumeration must exactly match the compiled set —
        // a class that is genuinely not exported has to be named.
        let declared_oos: std::collections::BTreeSet<String> = self
            .schema
            .object_classes
            .iter()
            .filter_map(|c| c.strip_prefix("!out-of-scope:").map(str::to_string))
            .collect();
        let expected_oos: std::collections::BTreeSet<String> =
            OUT_OF_SCOPE_CLASSES.iter().map(|s| s.to_string()).collect();
        if declared_oos != expected_oos {
            // Any class that is in neither the populated first-class set nor the
            // out-of-scope set is a silent omission — report the first such class.
            if let Some(missing) = expected_oos.symmetric_difference(&declared_oos).next() {
                return Err(SchemaError::UndeclaredOmission {
                    class: missing.clone(),
                });
            }
        }

        // (5) no dangling provenance link (E5⑨ cut self-consistency).
        let present_seqs: std::collections::BTreeSet<u64> =
            self.events.iter().map(|e| e.seq).collect();
        for link in &self.provenance_links {
            if !present_seqs.contains(&link.event_seq) {
                return Err(SchemaError::DanglingProvenanceLink {
                    object_id: link.object_id.clone(),
                    event_seq: link.event_seq,
                });
            }
        }

        Ok(())
    }
}
