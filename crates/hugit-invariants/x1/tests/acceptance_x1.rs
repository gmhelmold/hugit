//! WP-X1 acceptance oracle — tenant-isolation RED-TEAM. Contract:
//! `docs/plan/wp-contracts/WP-X1.md`. The owned items ARE the attack scripts.
//!
//! Owned items (VERBATIM from the contract; one `#[test] item_<n>_<slug>` each):
//!   ① tenant B requesting a key whose private bytes came from tenant A →
//!      miss/deny, never served.
//!   ② forged/collision memo-key attempts → deny + alert, no poisoning.
//!   ③ public-deterministic artifact IS shared, with proof no private bytes
//!      rode along.
//!   ④ side-channels: private artifacts produce NO cross-tenant hits (no
//!      existence/timing signal); the existence signal inherent to
//!      PUBLIC-deterministic sharing is documented as by-design disclosure.
//!
//! This is FULLY HERMETIC and p2-independent: it builds a tenant-scoped memo
//! namespace + admission check against a real in-process CAS/AC surface
//! (`x1::isolation`) and drives the REAL canonical memo-key
//! (`hugit_refstore::compute_memo_key`) and the frozen `AttestationChain`. The
//! oracle goes RED if isolation is bypassed or a forged key poisons.
//!
//! The X1 surface lives under `x1/isolation.rs` and is re-included here via
//! `#[path]` so all WP-X1 sources stay inside the owned claim.

#[path = "../isolation.rs"]
mod isolation;

use std::time::Instant;

use hugit_contracts::AttestationChain;
use hugit_refstore::compute_memo_key;

use isolation::{
    AdmitError, AuditLog, Lookup, MemoStore, PLATFORM_PRINCIPAL, PLATFORM_RUNNER, SharedRegistry,
    TenantId, TenantNamespace, TenantSecret, anonymize, carries_none_of,
};

// ── shared fixtures ───────────────────────────────────────────────────────────

/// Tenant A's private HMAC root secret. Distinct from B's — the partition key.
const SECRET_A: &[u8] = b"tenant-A-private-root-secret-aaaaaaaaaaaa";
/// Tenant B's private HMAC root secret.
const SECRET_B: &[u8] = b"tenant-B-private-root-secret-bbbbbbbbbbbb";

/// The three canonical memo axes for the artifact under attack. Both tenants
/// ask for the SAME logical (tree,def,toolchain) — the cross-tenant collision
/// case the boundary must defeat.
const TREE_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const DEF_DIGEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const TOOLCHAIN: &str = "3333333333333333333333333333333333333333333333333333333333333333";

/// Tenant A's namespace (id + private secret).
fn tenant_a() -> TenantNamespace {
    TenantNamespace::new(
        TenantId::new("tenant-a"),
        TenantSecret::from_bytes(SECRET_A),
    )
}

/// Tenant B's namespace (id + private secret). B does NOT hold A's secret.
fn tenant_b() -> TenantNamespace {
    TenantNamespace::new(
        TenantId::new("tenant-b"),
        TenantSecret::from_bytes(SECRET_B),
    )
}

/// The canonical, tenant-INDEPENDENT base memo key for the artifact. Computed
/// via the single-source refstore fn — never re-transcribed.
fn base_key() -> String {
    compute_memo_key(TREE_HASH, DEF_DIGEST, TOOLCHAIN)
}

/// Tenant A's PRIVATE attestation for the artifact — carries A's real runner +
/// principal identity (the bytes that must NEVER cross to B on item ③).
fn tenant_a_private_attestation() -> AttestationChain {
    AttestationChain {
        tree: TREE_HASH.to_string(),
        def: DEF_DIGEST.to_string(),
        runner: "runner:tenant-a-box-42-secret".to_string(),
        model: "claude-opus-4-8".to_string(),
        principal: vec![
            "agent:tenant-a-fleet-lead".to_string(),
            "user:alice@tenant-a.example".to_string(),
        ],
        sig: "tenant-a-private-signature-blob".to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ① tenant B requesting a key whose private bytes came from tenant A →
//    miss/deny, never served.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn item_1_cross_tenant_private_lookup_denied_never_served() {
    let mut store = MemoStore::new();
    let mut audit = AuditLog::new();
    let (a, b) = (tenant_a(), tenant_b());
    let base = base_key();

    // Tenant A admits PRIVATE bytes under A's namespace.
    let private_bytes = b"tenant-A-PRIVATE-check-result-bytes";
    let a_physical = a.namespaced_key(&base).to_string();
    store
        .admit(&a, &base, &a_physical, private_bytes, &mut audit)
        .expect("tenant A admits its own private entry");

    // ATTACK: tenant B requests the SAME logical (tree,def,toolchain). B derives
    // its OWN namespaced key (different secret) — it physically cannot name A's
    // slot, so the lookup MISSES. The private bytes are NEVER served to B.
    let b_result = store.lookup(&b, &base, &mut audit);
    assert_eq!(
        b_result,
        Lookup::Miss,
        "cross-tenant private lookup MUST miss/deny — never serve A's bytes to B"
    );
    assert!(
        b_result.is_miss() && !b_result.is_hit(),
        "the verdict is a Miss, never a Hit, for a cross-tenant private key"
    );
    if let Lookup::Hit(bytes) = &b_result {
        assert_ne!(
            bytes.as_slice(),
            private_bytes,
            "ISOLATION BREACH: tenant A's private bytes were served to tenant B"
        );
    }

    // The two tenants' namespaced keys for the same base key are DIFFERENT —
    // the HMAC partition is real, not cosmetic.
    assert_ne!(
        a.namespaced_key(&base),
        b.namespaced_key(&base),
        "distinct tenant secrets MUST derive distinct namespaced keys"
    );

    // Positive control: tenant A DOES resolve its own entry (the boundary is not
    // vacuously denying everyone).
    assert_eq!(
        store.lookup(&a, &base, &mut audit),
        Lookup::Hit(private_bytes.to_vec()),
        "tenant A must still resolve its OWN private entry"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ② forged/collision memo-key attempts → deny + alert, no poisoning.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn item_2_forged_and_collision_keys_denied_alerted_no_poisoning() {
    let mut store = MemoStore::new();
    let mut audit = AuditLog::new();
    let (a, b) = (tenant_a(), tenant_b());
    let base = base_key();
    assert!(store.is_empty(), "a fresh store holds no entries");

    // Tenant A admits its legitimate private entry.
    let honest_bytes = b"tenant-A-honest-result";
    let a_physical = a.namespaced_key(&base);
    store
        .admit(&a, &base, &a_physical, honest_bytes, &mut audit)
        .expect("honest admit succeeds");
    let alerts_before = audit.alerts().count();
    let len_before = store.len();

    // ── ATTACK 1: FORGED key. Tenant B presents A's physical key (somehow
    //    guessed/observed) but admits under its OWN namespace claiming `base`.
    //    The store re-derives from B's secret; the claimed key does NOT match,
    //    so it is a FORGERY → deny + alert. ───────────────────────────────────
    let forged_attempt = store.admit(&b, &base, &a_physical, b"poison-bytes", &mut audit);
    assert_eq!(
        forged_attempt,
        Err(AdmitError::ForgedKey),
        "a forged namespaced key MUST be denied fail-closed"
    );

    // ── ATTACK 2: COLLISION/poison. Tenant A tries to OVERWRITE its own slot
    //    with DIFFERENT bytes (a poisoning write to a memoized, content-addressed
    //    slot). Refused → deny + alert; existing bytes preserved. ──────────────
    let collision_attempt = store.admit(&a, &base, &a_physical, b"DIFFERENT-poison", &mut audit);
    assert_eq!(
        collision_attempt,
        Err(AdmitError::CollisionWouldPoison),
        "a collision write with different bytes MUST be denied (no poisoning)"
    );

    // ── NO POISONING: read the slot back — the bytes are UNCHANGED, and the
    //    store grew by zero entries across both attacks. ──────────────────────
    assert_eq!(
        store.raw_get(&a_physical),
        Some(honest_bytes.as_slice()),
        "POISONING: the existing entry's bytes were mutated by a refused write"
    );
    assert_eq!(
        store.len(),
        len_before,
        "POISONING: a refused write added an entry to the store"
    );

    // ── ALERTS: both deny paths emitted an ALERT audit event. ────────────────
    let alerts_after = audit.alerts().count();
    assert_eq!(
        alerts_after - alerts_before,
        2,
        "each forged/collision deny MUST emit exactly one ALERT audit event"
    );
    for ev in audit.alerts() {
        assert!(!ev.admitted, "an alert event must be a refusal");
        assert!(
            !ev.reason.is_empty(),
            "an alert event must carry a machine-readable reason"
        );
    }

    // Positive control: an IDEMPOTENT re-admit (identical bytes) is NOT a
    // poison and succeeds — the guard is not vacuously rejecting every re-write.
    store
        .admit(&a, &base, &a_physical, honest_bytes, &mut audit)
        .expect("idempotent re-admit of identical bytes must succeed");
}

// ─────────────────────────────────────────────────────────────────────────────
// ③ public-deterministic artifact IS shared, with proof no private bytes
//    rode along.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn item_3_public_deterministic_shared_no_private_bytes() {
    let mut shared = SharedRegistry::new();
    let base = base_key();

    // Tenant A produces a public-deterministic artifact (e.g. a hermetic
    // toolchain layer). Its PRIVATE attestation carries A's runner + principal.
    let producer = tenant_a_private_attestation();
    let artifact_bytes = b"public-deterministic-toolchain-layer";
    shared.publish(&base, artifact_bytes, &producer);

    // ── SHARING IS REAL: ANY tenant resolves the SAME global slot identically.
    let (a_bytes, a_attest) = shared
        .get(&base)
        .expect("tenant A resolves the shared artifact");
    let (b_bytes, b_attest) = shared
        .get(&base)
        .expect("tenant B resolves the SAME shared artifact");
    assert_eq!(a_bytes, artifact_bytes, "shared bytes served to tenant A");
    assert_eq!(
        a_bytes, b_bytes,
        "a public-deterministic artifact IS shared cross-tenant (same bytes)"
    );
    assert_eq!(
        a_attest, b_attest,
        "the shared attestation is identical for every tenant (no per-tenant fork)"
    );

    // ── PROOF NO PRIVATE BYTES RODE ALONG: the served attestation carries NONE
    //    of A's private principal/runner/sig bytes. ───────────────────────────
    let a_private_secrets = [
        producer.runner.as_str(),
        producer.principal[0].as_str(),
        producer.principal[1].as_str(),
        producer.sig.as_str(),
    ];
    assert!(
        carries_none_of(b_attest, &a_private_secrets),
        "LEAK: the shared attestation carried tenant A's private principal/runner/sig bytes"
    );

    // The shared attestation IS the platform-anonymized form …
    assert_eq!(
        b_attest.runner, PLATFORM_RUNNER,
        "runner is the platform sentinel"
    );
    assert_eq!(
        b_attest.principal,
        vec![PLATFORM_PRINCIPAL.to_string()],
        "principal is the platform sentinel, not tenant A's chain"
    );
    assert!(
        b_attest.sig.is_empty(),
        "the producer's private signature is not carried into the shared slot"
    );

    // … while the PUBLIC-deterministic links (which carry no tenant identity)
    // ARE preserved, so the artifact stays reproducible.
    assert_eq!(b_attest.tree, producer.tree, "public tree link preserved");
    assert_eq!(b_attest.def, producer.def, "public def link preserved");
    assert_eq!(
        b_attest.model, producer.model,
        "public model link preserved"
    );

    // `anonymize` is the load-bearing transform; prove it drops, never copies,
    // the private links even when called directly.
    let anon = anonymize(&producer);
    assert!(
        carries_none_of(&anon, &a_private_secrets),
        "anonymize MUST drop every private runner/principal/sig byte"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ④ side-channels: private artifacts produce NO cross-tenant hits (no
//    existence/timing signal); the PUBLIC-deterministic existence signal is
//    by-design disclosure.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn item_4_no_private_artifact_side_channel() {
    let mut store = MemoStore::new();
    let mut audit = AuditLog::new();
    let (a, b) = (tenant_a(), tenant_b());
    let base = base_key();

    // Tenant A admits a private entry.
    let a_physical = a.namespaced_key(&base);
    store
        .admit(&a, &base, &a_physical, b"tenant-A-private", &mut audit)
        .expect("A admits a private entry");

    // ── NO EXISTENCE SIGNAL: from tenant B's vantage, a key that EXISTS for A
    //    (private) and a key that NEVER existed both return the IDENTICAL
    //    response shape: `Miss`. There is no "exists-but-forbidden" verdict. ───
    let absent_base = compute_memo_key(
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        DEF_DIGEST,
        TOOLCHAIN,
    );
    let b_private_miss = store.lookup(&b, &base, &mut audit); // exists for A
    let b_absent_miss = store.lookup(&b, &absent_base, &mut audit); // never existed
    assert_eq!(
        b_private_miss, b_absent_miss,
        "EXISTENCE SIGNAL: a cross-tenant private miss differs from a genuine absent"
    );
    assert_eq!(
        b_private_miss,
        Lookup::Miss,
        "both are an indistinguishable Miss"
    );

    // The audit event shape is also identical (same action, same admitted flag,
    // no alert) — the trail betrays nothing either.
    let evs = audit.events();
    let last_two = &evs[evs.len() - 2..];
    assert_eq!(last_two[0].action, last_two[1].action, "same audit action");
    assert_eq!(
        last_two[0].admitted, last_two[1].admitted,
        "same admitted flag (both false)"
    );
    assert!(
        !last_two[0].alert && !last_two[1].alert,
        "a benign cross-tenant miss is NOT an alert (no existence signal in the trail)"
    );

    // ── NO TIMING SIGNAL (latency CLASS): both miss paths do the SAME work —
    //    one HMAC derivation + one map probe. Measure many iterations of each
    //    and assert the means are within the same order of magnitude (a coarse
    //    latency-class check; the structural guarantee is the identical code
    //    path above, this is a sanity bound). ──────────────────────────────────
    const ITERS: u32 = 20_000;
    let mut sink = AuditLog::new();
    let t_private = {
        let start = Instant::now();
        for _ in 0..ITERS {
            std::hint::black_box(store.lookup(&b, &base, &mut sink));
        }
        start.elapsed().as_nanos().max(1)
    };
    let t_absent = {
        let start = Instant::now();
        for _ in 0..ITERS {
            std::hint::black_box(store.lookup(&b, &absent_base, &mut sink));
        }
        start.elapsed().as_nanos().max(1)
    };
    let ratio = (t_private as f64).max(t_absent as f64) / (t_private as f64).min(t_absent as f64);
    assert!(
        ratio < 5.0,
        "TIMING SIGNAL: private-miss vs absent latency class diverged (ratio {ratio:.2}); \
         both must be the same work (one HMAC + one probe)"
    );

    // ── BY-DESIGN DISCLOSURE: a PUBLIC-deterministic artifact IS resolvable by
    //    any tenant, which IS an existence signal — documented as by-design, not
    //    a leak (contract item ④). Prove the public path is the ONLY one that
    //    discloses existence cross-tenant. ────────────────────────────────────
    let mut shared = SharedRegistry::new();
    shared.publish(&base, b"public-bytes", &tenant_a_private_attestation());
    assert!(
        shared.get(&base).is_some(),
        "the public-deterministic artifact IS resolvable cross-tenant (by-design disclosure)"
    );
    // … and the PRIVATE store still does not betray existence for the same base.
    assert_eq!(
        store.lookup(&b, &base, &mut sink),
        Lookup::Miss,
        "the PRIVATE store still emits no existence signal — only the public registry discloses"
    );
}
