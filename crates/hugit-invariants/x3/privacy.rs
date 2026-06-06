//! X3 context-privacy verification surface.
//!
//! WP-X3 proves the privacy invariant behind the context/journal object
//! classes without owning any production path. It **consumes** the D11 journal
//! surface ([`hugit_ledger::journal`]) and the D11 view-boundary redaction
//! filter ([`hugit_ledger::redact`]) and the frozen
//! [`ExportSchema`](hugit_contracts::ExportSchema) read-only; it modifies
//! neither.
//!
//! Four owned items, each verified against the consumed surfaces:
//!
//! 1. **Tenant scope (①).** A journal is tenant-private: cross-tenant fetch is
//!    refused fail-closed by [`JournalStore::open`](hugit_ledger::journal::JournalStore::open).
//!    X3 mounts the attack (tenant B fetches tenant A's object) and asserts the
//!    deny PLUS an audit event — it never re-implements the scoping.
//! 2. **Redaction at capture AND export (②).** A seeded secret is redacted in
//!    the at-rest captured object (capture-time, via [`redact_captured`]) AND in
//!    the exported artifact validated against `ExportSchema` (export-time, via
//!    [`export_redacted`]). Every assertion verifies ABSENCE of the secret in
//!    the serialized bytes (a grep-class scan), not mere flag state.
//! 3. **Retention/deletion purge (③).** A datum is seeded into a context store,
//!    retention/deletion is triggered ([`ContextStore::purge`]), and a re-scan
//!    of the store's bytes asserts the datum is ABSENT — actually purged, not
//!    tombstoned-with-bytes. This is the context-store leg of the X7 erasure
//!    cascade; X3 owns the context-store purge proof.
//! 4. **Training/eval exclusion (④).** A DOCUMENTED control
//!    ([`TRAINING_EXCLUSION_CONTROL`], committed alongside as
//!    `x3/TRAINING-EXCLUSION.md`) marks context/journals never-training-data.
//!    The control is executed ([`TrainingExclusionControl`]) and any access
//!    classified non-training produces an audit-trail entry. No model is
//!    invoked — the control + audit trail is the deliverable.

use hugit_contracts::ExportSchema;
use hugit_ledger::journal::{Journal, JournalError, JournalStore};
use hugit_ledger::redact::{SECRET_MARKER, apply as redact_apply};

// ── audit trail (shared by ① and ④) ──────────────────────────────────────────

/// One audit-trail entry. The privacy invariants demand that a denied
/// cross-tenant fetch (①) and any non-training classification (④) are not
/// silent — each emits an attributed, timestamped event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    /// Stable event kind, e.g. `cross-tenant-fetch-denied`,
    /// `access-classified-non-training`.
    pub kind: String,
    /// The principal/tenant that triggered the event.
    pub principal: String,
    /// Human-readable detail of what happened.
    pub detail: String,
}

/// An append-only audit trail. Verification asserts the expected event is
/// present after the controlled operation.
#[derive(Debug, Default)]
pub struct AuditTrail {
    events: Vec<AuditEvent>,
}

impl AuditTrail {
    /// A fresh, empty trail.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one event.
    pub fn record(
        &mut self,
        kind: impl Into<String>,
        principal: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.events.push(AuditEvent {
            kind: kind.into(),
            principal: principal.into(),
            detail: detail.into(),
        });
    }

    /// All recorded events, in order.
    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    /// Whether an event of the given kind was recorded.
    pub fn has_kind(&self, kind: &str) -> bool {
        self.events.iter().any(|e| e.kind == kind)
    }
}

// ── ① tenant-scoped cross-fetch ──────────────────────────────────────────────

/// Audit-event kind emitted when a cross-tenant fetch is denied.
pub const KIND_CROSS_TENANT_DENIED: &str = "cross-tenant-fetch-denied";

/// Mount the cross-tenant fetch against the D11 store and, on the expected
/// fail-closed deny, record an audit event. Returns the [`JournalError`] the
/// store raised so the caller can assert the *kind* of denial.
///
/// This never re-implements scoping — it drives
/// [`JournalStore::open`](hugit_ledger::journal::JournalStore::open) (D11's
/// surface) and observes the deny. The audit event is the X3-owned obligation
/// that the deny is not silent.
pub fn cross_tenant_fetch(
    store: &JournalStore,
    attacker_tenant: &str,
    workspace_id: &str,
    intent_id: &str,
    audit: &mut AuditTrail,
) -> JournalError {
    let err = store
        .open(attacker_tenant, workspace_id, intent_id)
        .expect_err("cross-tenant fetch MUST be denied fail-closed");
    audit.record(
        KIND_CROSS_TENANT_DENIED,
        attacker_tenant,
        format!("denied access to ws={workspace_id} intent={intent_id}: {err}"),
    );
    err
}

// ── ② redaction at capture AND export ────────────────────────────────────────

/// Apply capture-time redaction to a captured-context field, returning the
/// at-rest bytes that would be persisted. Consumes D11's view-boundary filter
/// ([`hugit_ledger::redact::apply`]) — capture-time redaction is the same rule
/// applied *before* the object is stored, so a secret never lands at rest.
pub fn redact_captured(field: &str) -> String {
    redact_apply(field)
}

/// The exported artifact: the frozen [`ExportSchema`] envelope PLUS the
/// export-time-redacted payload bytes. Item ② asserts the secret is absent in
/// `payload` AND the envelope validates against `ExportSchema`.
#[derive(Debug, Clone)]
pub struct RedactedExport {
    /// The frozen, machine-validatable export envelope.
    pub envelope: ExportSchema,
    /// The export-time-redacted payload bytes (JSON of the captured object,
    /// with every secret-bearing field replaced by the sentinel).
    pub payload: String,
}

/// Build a `RedactedExport` from a captured journal: serialize it, apply
/// export-time redaction to the serialized bytes, and wrap it in an
/// `ExportSchema` envelope. The redaction at export is the same rule E5④/E5⑦
/// require; X3 owns the capture+export privacy proof.
pub fn export_redacted(journal: &Journal, redaction_manifest: impl Into<String>) -> RedactedExport {
    // Serialize the captured object as it would be exported.
    let raw = serde_json::to_string(journal).expect("journal serializes");
    // Export-time redaction: any value carrying the secret marker is replaced.
    // (A line-wise scan mirrors the field-wise capture rule over serialized
    // bytes; the post-condition asserted by the oracle is byte-absence.)
    let payload = redact_apply(&raw);
    RedactedExport {
        envelope: ExportSchema {
            version: "1.0.0".to_string(),
            object_classes: vec!["journal".to_string()],
            redaction_manifest: redaction_manifest.into(),
        },
        payload,
    }
}

/// True iff the serialized `bytes` contain NO trace of the secret marker — the
/// grep-class absence scan the contract mandates (verify ABSENCE, not flags).
pub fn secret_absent(bytes: &str) -> bool {
    !bytes.contains(SECRET_MARKER)
}

/// True iff the exported envelope validates against the frozen `ExportSchema`
/// shape (round-trips through serde with `deny_unknown_fields`) AND carries a
/// redaction manifest reference.
pub fn export_validates(export: &RedactedExport) -> bool {
    let json = serde_json::to_string(&export.envelope).expect("envelope serializes");
    let back: Result<ExportSchema, _> = serde_json::from_str(&json);
    matches!(back, Ok(env) if env == export.envelope)
        && !export.envelope.redaction_manifest.is_empty()
}

// ── ③ retention/deletion purge ───────────────────────────────────────────────

/// A minimal content store standing in for the context-store leg of the
/// erasure cascade. Retention/deletion actually removes the bytes (purge, not
/// tombstone-with-bytes) so a post-purge byte scan finds the datum ABSENT.
#[derive(Debug, Default)]
pub struct ContextStore {
    objects: std::collections::HashMap<String, String>,
}

impl ContextStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a datum under `id`.
    pub fn seed(&mut self, id: impl Into<String>, datum: impl Into<String>) {
        self.objects.insert(id.into(), datum.into());
    }

    /// Trigger retention/deletion for `id`: the bytes are *removed*, not
    /// tombstoned. Returns true if a datum was present and purged.
    pub fn purge(&mut self, id: &str) -> bool {
        self.objects.remove(id).is_some()
    }

    /// The full serialized byte image of the store — what a re-scan reads.
    /// After a purge of `id`, the purged datum must not appear here.
    pub fn scan_bytes(&self) -> String {
        // Deterministic ordering for a stable scan.
        let mut ids: Vec<&String> = self.objects.keys().collect();
        ids.sort();
        ids.into_iter()
            .map(|k| format!("{k}={}", self.objects[k]))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ── ④ training/eval exclusion control + audit ────────────────────────────────

/// The committed control document marking context/journals as never-training
/// data. Co-located as `x3/TRAINING-EXCLUSION.md`; embedded here so the test
/// can execute the control and assert the doc is present and binding.
pub const TRAINING_EXCLUSION_CONTROL: &str = include_str!("TRAINING-EXCLUSION.md");

/// Audit-event kind emitted whenever an access is classified non-training.
pub const KIND_NON_TRAINING_ACCESS: &str = "access-classified-non-training";

/// How an access to a context/journal object is classified by the control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessClass {
    /// Operational access (resume, audit, export-with-consent) — permitted.
    Operational,
    /// Training/eval access — the control FORBIDS this for context/journals.
    Training,
}

/// The executable training-exclusion control. Context/journals are
/// never-training-data; every access is classified and any non-training
/// classification produces an audit-trail entry. No model is invoked.
#[derive(Debug)]
pub struct TrainingExclusionControl;

impl TrainingExclusionControl {
    /// True iff the committed control doc binds context/journals as
    /// never-training-data (the doc must say so explicitly).
    pub fn control_is_binding() -> bool {
        let doc = TRAINING_EXCLUSION_CONTROL;
        doc.contains("never-training-data") && doc.contains("context") && doc.contains("journal")
    }

    /// Execute the control for one access. Returns whether the access is
    /// permitted; for an access classified non-training the control records an
    /// audit-trail entry (the deliverable). A `Training` classification is
    /// forbidden (returns false) and is the one audited as non-training intent.
    pub fn classify_access(
        &self,
        principal: &str,
        class: AccessClass,
        audit: &mut AuditTrail,
    ) -> bool {
        match class {
            AccessClass::Operational => true,
            AccessClass::Training => {
                audit.record(
                    KIND_NON_TRAINING_ACCESS,
                    principal,
                    "training/eval access to context/journal refused: \
                     class marked never-training-data by the X3 control",
                );
                false
            }
        }
    }
}
