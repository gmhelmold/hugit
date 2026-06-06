//! WP-X3 acceptance oracle — context privacy.
//! Contract: `docs/plan/wp-contracts/WP-X3.md`.
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ① `item_1_tenant_scope_cross_fetch_denied` — cross-tenant fetch of a
//!      context/journal object is denied and an audit event is emitted.
//!   ② `item_2_redaction_at_capture_and_export` — a secret seeded into a
//!      captured context is absent in the at-rest captured object (capture-time)
//!      AND absent in the exported artifact validated against ExportSchema
//!      (export-time). Absence is verified by byte-scan, not flag state.
//!   ③ `item_3_retention_deletion_purge` — after retention/deletion is
//!      triggered, the seeded datum is absent from the store (actually purged,
//!      not tombstoned-with-bytes).
//!   ④ `item_4_training_exclusion_control_and_audit` — a documented control
//!      marks context/journals never-training-data; the test executes the
//!      control and asserts an audit trail entry for any access classified
//!      non-training.
//!
//! RED on current tree: `ExportSchema`, the context/journal object surfaces
//! (D11/E5), and the training-exclusion control surface do not yet exist as
//! importable types.

use hugit_contracts::{ExportSchema, TenantId};
use hugit_context::{
    capture::{CaptureRequest, ContextStore},
    export::ExportRequest,
    journal::JournalStore,
    retention::RetentionPolicy,
    training::TrainingExclusionControl,
};

// ── fixtures ─────────────────────────────────────────────────────────────────

fn tenant_a() -> TenantId {
    TenantId::from_hmac_prefix("tenant-a-hmac-prefix-fixture")
}

fn tenant_b() -> TenantId {
    TenantId::from_hmac_prefix("tenant-b-hmac-prefix-fixture")
}

const SECRET_CANARY: &str = "PRIVATE-SECRET-CANARY-XK3F9Z";

// ── item_1_tenant_scope_cross_fetch_denied ────────────────────────────────────

/// ① Tenant B cannot fetch tenant A's context or journal object.
/// Cross-tenant fetch must be denied and an audit event emitted.
#[test]
fn item_1_tenant_scope_cross_fetch_denied() {
    let store = ContextStore::new_in_memory();

    // Seed a context object under tenant A.
    let ctx_id = store
        .capture(CaptureRequest {
            tenant: tenant_a(),
            payload: b"tenant-a-private-context".to_vec(),
        })
        .expect("capture must succeed for tenant A");

    // Tenant B attempts to fetch tenant A's context object.
    let fetch_result = store.fetch_for_tenant(ctx_id, tenant_b());
    assert!(
        fetch_result.is_err(),
        "cross-tenant fetch must be denied: tenant B must not access tenant A's context"
    );

    // An audit event must have been emitted for the denied cross-tenant access.
    let audit = store.last_audit_event();
    assert!(
        audit.is_some(),
        "a denied cross-tenant fetch must emit an audit event"
    );
    let audit = audit.unwrap();
    assert!(
        audit.is_access_denied(),
        "audit event must be classified as access-denied"
    );

    // Same for journals.
    let journal_store = JournalStore::new_in_memory();
    let jid = journal_store
        .write(tenant_a(), b"tenant-a-journal-entry".to_vec())
        .expect("journal write must succeed for tenant A");

    let journal_fetch = journal_store.fetch_for_tenant(jid, tenant_b());
    assert!(
        journal_fetch.is_err(),
        "cross-tenant journal fetch must be denied"
    );
    assert!(
        journal_store.last_audit_event().map(|e| e.is_access_denied()).unwrap_or(false),
        "denied journal fetch must emit audit event"
    );
}

// ── item_2_redaction_at_capture_and_export ────────────────────────────────────

/// ② A secret seeded into a captured context is absent in the stored object
/// (capture-time redaction) AND absent in the exported artifact validated
/// against ExportSchema (export-time redaction). Absence is verified by
/// byte-scanning the stored/exported bytes — not by checking a flag.
#[test]
fn item_2_redaction_at_capture_and_export() {
    let store = ContextStore::new_in_memory();

    // Seed the canary secret into the captured context.
    let ctx_id = store
        .capture(CaptureRequest {
            tenant: tenant_a(),
            payload: format!("context including {SECRET_CANARY}").into_bytes(),
        })
        .expect("capture must succeed");

    // ── capture-time: canary must be absent in the at-rest stored bytes ────
    let stored_bytes = store
        .raw_stored_bytes(ctx_id)
        .expect("must be able to read stored bytes for redaction assertion");
    assert!(
        !stored_bytes
            .windows(SECRET_CANARY.len())
            .any(|w| w == SECRET_CANARY.as_bytes()),
        "canary secret must not appear in at-rest captured bytes (capture-time redaction)"
    );

    // ── export-time: canary must be absent in the exported artifact ────────
    let export_request = ExportRequest {
        tenant: tenant_a(),
        context_id: ctx_id,
        schema: ExportSchema::current(),
    };
    let exported = store
        .export(export_request)
        .expect("export must succeed");

    // Validate the exported artifact against ExportSchema.
    ExportSchema::current()
        .validate(&exported)
        .expect("exported artifact must validate against ExportSchema");

    // Byte-scan the exported artifact for the canary.
    let exported_bytes = exported.as_bytes();
    assert!(
        !exported_bytes
            .windows(SECRET_CANARY.len())
            .any(|w| w == SECRET_CANARY.as_bytes()),
        "canary secret must not appear in exported artifact bytes (export-time redaction)"
    );
}

// ── item_3_retention_deletion_purge ──────────────────────────────────────────

/// ③ After retention/deletion is triggered, the seeded datum is absent from
/// the context store — actually purged, not tombstoned-with-bytes.
#[test]
fn item_3_retention_deletion_purge() {
    let store = ContextStore::new_in_memory();

    // Seed the canary into the store.
    let ctx_id = store
        .capture(CaptureRequest {
            tenant: tenant_a(),
            payload: format!("context including {SECRET_CANARY}").into_bytes(),
        })
        .expect("capture must succeed");

    // Verify the context exists before deletion.
    assert!(
        store.exists(ctx_id, tenant_a()),
        "context must exist before retention/deletion"
    );

    // Trigger retention/deletion purge.
    RetentionPolicy::immediate_delete()
        .apply(&store, ctx_id, tenant_a())
        .expect("retention/deletion must execute without error");

    // The datum must be fully absent — fetch must return not-found.
    let fetch_after = store.fetch_for_tenant(ctx_id, tenant_a());
    assert!(
        fetch_after.is_err(),
        "context must not be fetchable after retention/deletion purge"
    );

    // Byte-scan the entire store for the canary — must be absent (not tombstoned).
    let all_stored = store.dump_all_raw_bytes();
    assert!(
        !all_stored
            .windows(SECRET_CANARY.len())
            .any(|w| w == SECRET_CANARY.as_bytes()),
        "canary must not appear anywhere in the store after purge (must be purged, not tombstoned)"
    );
}

// ── item_4_training_exclusion_control_and_audit ───────────────────────────────

/// ④ A documented training-exclusion control marks context/journals as
/// never-training-data. The test executes the control and asserts an audit
/// trail entry for any access classified non-training. No model is invoked.
#[test]
fn item_4_training_exclusion_control_and_audit() {
    // The documented exclusion control doc must be committed.
    let doc_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/x3/docs/training-exclusion-control.md"
    );
    assert!(
        std::path::Path::new(doc_path).exists(),
        "training-exclusion control doc must be committed at x3/docs/training-exclusion-control.md"
    );

    let store = ContextStore::new_in_memory();

    // Capture a context object.
    let ctx_id = store
        .capture(CaptureRequest {
            tenant: tenant_a(),
            payload: b"sensitive context payload".to_vec(),
        })
        .expect("capture must succeed");

    // Execute the training-exclusion control.
    let control = TrainingExclusionControl::new();
    control
        .mark_excluded(ctx_id, tenant_a())
        .expect("training-exclusion mark must succeed");

    // Any subsequent access to this object classified as non-training must
    // emit an audit trail entry.
    let _accessed = store
        .access_for_non_training_purpose(ctx_id, tenant_a(), "audit-test-accessor")
        .expect("access must succeed (the control is about audit, not blocking)");

    let audit_entries = control.audit_trail_for(ctx_id);
    assert!(
        !audit_entries.is_empty(),
        "audit trail must have at least one entry for non-training access on an excluded context"
    );
    assert!(
        audit_entries.iter().any(|e| e.is_non_training_access()),
        "at least one audit entry must be classified as non-training access"
    );
}
