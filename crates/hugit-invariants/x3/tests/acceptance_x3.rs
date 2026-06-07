//! WP-X3 acceptance oracle — context privacy. Contract:
//! `docs/plan/wp-contracts/WP-X3.md`.
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ① `item_1_tenant_scope_cross_fetch_denied` — context/journals are
//!      tenant-scoped: tenant B fetching tenant A's journal is denied
//!      fail-closed by the D11 store, and the deny emits an audit event.
//!   ② `item_2_redaction_at_capture_and_export` — a seeded secret is redacted
//!      in the at-rest captured object (capture-time) AND in the exported
//!      artifact validated against `ExportSchema` (export-time). Every
//!      assertion verifies ABSENCE of the secret in the serialized bytes.
//!   ③ `item_3_retention_deletion_purge` — a seeded datum is purged on
//!      retention/deletion and a re-scan of the store bytes asserts it ABSENT
//!      (actually purged, not tombstoned-with-bytes).
//!   ④ `item_4_training_exclusion_control_and_audit` — a documented control
//!      marks context/journals never-training-data; executing it permits an
//!      operational access and refuses a training access WITH an audit-trail
//!      entry. No model is invoked.
//!
//! Every redaction/purge assertion is a grep-class ABSENCE scan over the
//! stored/exported bytes, never a mere flag check (contract Conventions).
//! This crate consumes the D11 journal + redaction surfaces
//! (`hugit_ledger::journal`, `hugit_ledger::redact`) and the frozen
//! `ExportSchema` (`hugit_contracts`) read-only; it modifies none of them.

// X3 owns only the `x3/` subtree of the shared `hugit-invariants` crate; the
// crate's single `[lib]` target is owned by WP-X4 (`x4/lib.rs`) and is not
// touched here. The X3 verification surface is therefore included directly into
// this integration test via `#[path]`, keeping every X3 source under `x3/`.
#[path = "../privacy.rs"]
mod privacy;

use hugit_ledger::journal::{Journal, JournalError, JournalKey, JournalStore};
use hugit_ledger::redact::SECRET_MARKER;
use privacy::{
    AccessClass, AuditTrail, ContextStore, KIND_CROSS_TENANT_DENIED, KIND_NON_TRAINING_ACCESS,
    RedactedExport, TrainingExclusionControl, cross_tenant_fetch, export_redacted,
    export_validates, redact_captured, secret_absent,
};

/// A planted secret used across the redaction proofs. It carries both the
/// canonical marker (`SECRET:`) AND a distinct secret VALUE; the oracle asserts
/// the *value* is absent, not merely the marker (a degenerate redactor that
/// drops only the marker would still leak the value).
const PLANTED_SECRET: &str = "SECRET:hugit-api-key-9f3a2b";

/// The bare secret VALUE (no marker). Redaction must remove THIS from the
/// exported bytes — asserting only the marker's absence is insufficient.
const SECRET_VALUE: &str = "hugit-api-key-9f3a2b";

/// A non-secret note planted in the SAME journal as the secret. Per-field
/// redaction must leave this field INTACT in the export; a degenerate whole-blob
/// redactor (replace the entire serialized journal with one `[REDACTED]` token)
/// destroys it and turns the oracle RED.
const SURVIVING_NOTE: &str = "non-secret-operational-note-keep-me";

/// A non-secret principal on a surviving entry — a second structural field that
/// must outlive redaction (proves redaction is per-field, not whole-blob).
const SURVIVING_PRINCIPAL: &str = "agent:keeper";

/// Build a one-tenant store holding tenant A's journal, plus the binding triple.
fn seed_tenant_a_journal() -> (JournalStore, &'static str, &'static str, &'static str) {
    let (tenant, ws, intent) = ("tenant-a", "ws-alpha", "intent-i1");
    let mut store = JournalStore::new();
    let mut journal = Journal::new(JournalKey::new(tenant, ws, intent));
    journal.append(1_749_081_600_000, "agent:worker", "private session note");
    store.put(journal);
    (store, tenant, ws, intent)
}

// ── ① context/journals tenant-scoped: cross-tenant fetch denied ──────────────
#[test]
fn item_1_tenant_scope_cross_fetch_denied() {
    let (store, tenant_a, ws, intent) = seed_tenant_a_journal();

    // Sanity: the owning tenant CAN read its own journal (the boundary is not
    // vacuously refusing everything).
    store
        .open(tenant_a, ws, intent)
        .expect("owning tenant must read its own journal");

    // Attack: tenant B fetches tenant A's journal. The D11 store is keyed by
    // tenant, so the cross-tenant lookup must be denied fail-closed.
    let mut audit = AuditTrail::new();
    let err = cross_tenant_fetch(&store, "tenant-b", ws, intent, &mut audit);

    // The deny is fail-closed: no journal bytes are returned. Either NotFound
    // (the cross-tenant key does not resolve) or an explicit TenantMismatch is
    // an acceptable closed denial — never an Ok with another tenant's bytes.
    assert!(
        matches!(
            err,
            JournalError::NotFound { .. } | JournalError::TenantMismatch { .. }
        ),
        "cross-tenant fetch must be a closed deny, got: {err:?}"
    );

    // The deny is NOT silent: an audit event was emitted attributing the
    // attacker tenant.
    assert!(
        audit.has_kind(KIND_CROSS_TENANT_DENIED),
        "cross-tenant deny must emit an audit event"
    );
    let ev = audit
        .events()
        .iter()
        .find(|e| e.kind == KIND_CROSS_TENANT_DENIED)
        .expect("deny audit event present");
    assert_eq!(ev.principal, "tenant-b", "audit attributes the attacker");
}

// ── ② redaction at capture AND export ────────────────────────────────────────
#[test]
fn item_2_redaction_at_capture_and_export() {
    // Capture-time: a secret-bearing field is redacted BEFORE it lands at rest.
    let at_rest = redact_captured(PLANTED_SECRET);
    assert!(
        secret_absent(&at_rest),
        "capture-time redaction must remove the secret from the at-rest bytes; got: {at_rest:?}"
    );
    assert_ne!(
        at_rest, PLANTED_SECRET,
        "captured field must not be stored verbatim"
    );

    // Build a captured journal that embeds the secret in ONE recorded note, and
    // ALSO carries non-secret structural fields (a binding key, a surviving
    // non-secret note, a surviving principal). The EXPORTED artifact must:
    //   (a) be redacted at export-time — the secret VALUE absent, not just the
    //       marker (a degenerate marker-only strip would leak the value), AND
    //   (b) preserve every NON-secret field — per-field redaction, not a
    //       whole-blob collapse to one `[REDACTED]` token, AND
    //   (c) validate against the frozen ExportSchema.
    let mut journal = Journal::new(JournalKey::new("tenant-a", "ws-alpha", "intent-i1"));
    // A non-secret entry that MUST survive redaction intact.
    journal.append(1_749_081_600_000, SURVIVING_PRINCIPAL, SURVIVING_NOTE);
    // The secret-bearing entry (only THIS field's value is redacted).
    journal.append(1_749_081_600_001, "agent:worker", PLANTED_SECRET);

    let export: RedactedExport = export_redacted(
        &journal,
        // content-addressed redaction-manifest ref (64-hex placeholder).
        "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90",
    );

    // (a) Export-time ABSENCE scan: the secret MARKER is gone …
    assert!(
        secret_absent(&export.payload),
        "export-time redaction must remove the secret marker from the exported \
         bytes; found `{SECRET_MARKER}` in: {payload:?}",
        payload = export.payload
    );
    // … AND the bare secret VALUE is gone (the load-bearing check: a redactor
    // that only drops the `SECRET:` marker would still leak the key value).
    assert!(
        !export.payload.contains(SECRET_VALUE),
        "export-time redaction must remove the secret VALUE ({SECRET_VALUE:?}) \
         from the exported bytes, not merely the marker; found it in: {payload:?}",
        payload = export.payload
    );

    // (b) Per-field survival: every NON-secret field is still present in the
    //     exported payload. A degenerate whole-blob `[REDACTED]` collapse fails
    //     here because it destroys these fields too.
    assert!(
        export.payload.contains(SURVIVING_NOTE),
        "non-secret note must SURVIVE export redaction (per-field, not \
         whole-blob); missing from: {payload:?}",
        payload = export.payload
    );
    assert!(
        export.payload.contains(SURVIVING_PRINCIPAL),
        "non-secret principal must SURVIVE export redaction; missing from: \
         {payload:?}",
        payload = export.payload
    );
    assert!(
        export.payload.contains("intent-i1"),
        "the binding intent_id (non-secret) must SURVIVE export redaction; \
         missing from: {payload:?}",
        payload = export.payload
    );
    assert!(
        export.payload.contains("tenant-a"),
        "the binding tenant_id (non-secret) must SURVIVE export redaction; \
         missing from: {payload:?}",
        payload = export.payload
    );

    // (c) The exported payload is still structured JSON the consumer can parse
    //     back into a Journal (redaction preserved the SHAPE, only scrubbed the
    //     secret-bearing field's value) — proves it is not a flattened token.
    let parsed: Journal = serde_json::from_str(&export.payload)
        .expect("redacted payload must remain valid Journal JSON");
    assert_eq!(
        parsed.entries.len(),
        2,
        "both entries survive (only the secret VALUE is scrubbed)"
    );
    assert_eq!(parsed.key.tenant_id, "tenant-a");
    assert_eq!(
        parsed.entries[0].note, SURVIVING_NOTE,
        "non-secret note byte-identical"
    );
    assert_ne!(
        parsed.entries[1].note, PLANTED_SECRET,
        "the secret-bearing note must be redacted in place"
    );
    assert!(
        !parsed.entries[1].note.contains(SECRET_VALUE),
        "the secret VALUE must not survive in the redacted note"
    );

    // The envelope is machine-validatable against the frozen ExportSchema.
    assert!(
        export_validates(&export),
        "exported envelope must validate against the frozen ExportSchema"
    );
    assert_eq!(export.envelope.version, "1.0.0");
    assert!(
        export
            .envelope
            .object_classes
            .iter()
            .any(|c| c == "journal"),
        "export must declare the journal object class"
    );
}

// ── ③ retention/deletion purge: verified absent ──────────────────────────────
#[test]
fn item_3_retention_deletion_purge() {
    let mut store = ContextStore::new();
    let datum = "tenant-a-context-payload-to-be-erased";
    store.seed("ctx-1", datum);
    store.seed("ctx-2", "an unrelated datum that must survive");

    // Pre-condition: the datum is present in the store's byte image.
    assert!(
        store.scan_bytes().contains(datum),
        "seeded datum must be present before purge"
    );

    // Trigger retention/deletion.
    let purged = store.purge("ctx-1");
    assert!(
        purged,
        "purge must report the datum was present and removed"
    );

    // Post-condition: a re-scan of the store bytes finds the datum ABSENT —
    // actually purged, not tombstoned-with-bytes.
    let after = store.scan_bytes();
    assert!(
        !after.contains(datum),
        "purged datum must be ABSENT from the store after retention/deletion; \
         still found in: {after:?}"
    );

    // The cascade is scoped: unrelated data survives (purge is not a wipe).
    assert!(
        after.contains("an unrelated datum that must survive"),
        "purge must remove only the targeted datum"
    );

    // Re-purge is a no-op (idempotent): nothing left to remove.
    assert!(
        !store.purge("ctx-1"),
        "re-purge of an already-purged datum is a no-op"
    );
}

// ── ④ training/eval exclusion: documented control + audit trail ──────────────
#[test]
fn item_4_training_exclusion_control_and_audit() {
    // The committed control doc is present and BINDING (names context, journal,
    // and never-training-data).
    assert!(
        TrainingExclusionControl::control_is_binding(),
        "the training-exclusion control doc must bind context/journals as never-training-data"
    );

    let control = TrainingExclusionControl;
    let mut audit = AuditTrail::new();

    // An operational access is permitted and emits NO non-training audit entry.
    let ok = control.classify_access("agent:resume", AccessClass::Operational, &mut audit);
    assert!(ok, "operational access must be permitted");
    assert!(
        !audit.has_kind(KIND_NON_TRAINING_ACCESS),
        "operational access must not be classified non-training"
    );

    // A training/eval access is REFUSED and produces the audit-trail entry.
    let denied = control.classify_access("pipeline:trainer", AccessClass::Training, &mut audit);
    assert!(!denied, "training/eval access must be refused");
    assert!(
        audit.has_kind(KIND_NON_TRAINING_ACCESS),
        "training/eval access must emit a non-training audit-trail entry"
    );
    let ev = audit
        .events()
        .iter()
        .find(|e| e.kind == KIND_NON_TRAINING_ACCESS)
        .expect("non-training audit event present");
    assert_eq!(
        ev.principal, "pipeline:trainer",
        "audit attributes the training principal"
    );
}
