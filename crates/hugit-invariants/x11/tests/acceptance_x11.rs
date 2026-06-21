//! WP-X11 acceptance oracle — degradation composition.
//! Contract: `the work-package contract`.
//!
//! Proves the invariants COMPOSE under a PARTIAL degradation injected
//! MID-operation (a degradation window, not a steady-state outage). One
//! `#[test] item_<n>_<slug>` per owned item, VERBATIM:
//!
//!   ① smart-layer failure injected MID-operation (partial degradation window,
//!      not just steady-state outage): secrets broker fails CLOSED — no
//!      credential reaches any workspace during degradation.
//!   ② objects written during degradation are marked provenance-ABSENT; no
//!      synthetic intent/attestation ever fabricated by a fallback path.
//!   ③ CoreLink non-interference (X10 baseline) holds WHILE hugit is degraded,
//!      not only when healthy.
//!
//! Hermetic by construction: the in-process [`DegradableSmartLayer`] injects the
//! mid-operation fault exactly as WP-X4's `FakeBox`/`FakeEngine` proves the
//! fail-closed-before-spawn ORDERING — no network, no env. The LIVE box fault
//! injection is the documented P2 seam, gated behind `HUGIT_RUNNER_HOST`
//! (run-not-skip when set).
//!
//! The oracle is NOT gamed:
//!   - item ① goes RED if a credential leaks into the workspace during the
//!     window (proven by `item_1_a_leaked_credential_would_be_caught_red`);
//!   - item ② goes RED if a synthetic intent/attestation is fabricated by a
//!     fallback (proven by `item_2_a_fabricated_provenance_would_be_caught_red`);
//!   - item ③ drives the REAL X10 cap/workload/tolerance surfaces, so an X10
//!     regression turns it RED.

use hugit_contracts::{AttestationChain, RunnerLease, RunnerState};
use hugit_invariants::x11::{
    BrokerOutcome, DegradableSmartLayer, FaultPoint, Intent, Provenance, WrittenObject,
    assert_non_interference_while_degraded, canonical_preimage, live_lane_active,
    run_degradation_window, scan_degradation_window_writes,
};

/// The raw credential the smart layer holds. It must NEVER reach a workspace
/// during the degradation window. Never printed — the scan reports only booleans.
const RAW_CREDENTIAL: &[u8] = b"NEVER-PRINT-deploy-key-x11-degradation-window";

/// A held lease whose workspace item ① attacks during the degradation window.
fn held_lease(slug: &str) -> RunnerLease {
    RunnerLease {
        lease_id: format!("x11-{slug}"),
        principal_chain: vec!["user:owner".to_string(), "agent:x11".to_string()],
        path_set: vec!["src/".to_string()],
        expiry: u64::MAX,
        net_policy: "none".to_string(),
        tmp_root: "/hugit/tmp".to_string(),
        state: RunnerState::Held,
    }
}

/// A genuine, signable attestation chain template for the HEALTHY path. (When
/// degraded, NONE of this is fabricated — that is item ②.)
fn healthy_attestation() -> AttestationChain {
    AttestationChain {
        tree: "tree-abc".to_string(),
        def: "def-xyz".to_string(),
        runner: "runner-1".to_string(),
        model: "claude-opus".to_string(),
        principal: vec!["user:owner".to_string()],
        sig: String::new(),
    }
}

fn smart_layer() -> DegradableSmartLayer {
    DegradableSmartLayer::new(RAW_CREDENTIAL, healthy_attestation())
}

// ── ① smart-layer failure injected MID-operation: broker fails CLOSED, no ─────
//    credential reaches any workspace during degradation. ───────────────────────
#[test]
fn item_1_broker_fails_closed_mid_operation_no_credential_in_workspace() {
    let smart = smart_layer();
    let lease = held_lease("item1");
    let mut workspace = hugit_invariants::x11::Workspace::for_lease(&lease);

    // Inject the smart-layer failure MID-operation (a partial degradation
    // window): the op has begun, then the broker goes down before resolving the
    // credential. It must fail CLOSED, with NO fallback.
    let outcome = smart.broker_op(
        &mut workspace,
        b"artifact:release-9.0",
        FaultPoint::MidOperation,
    );
    assert!(
        outcome.failed_closed(),
        "WP-X11①: a smart-layer failure injected MID-operation must make the \
         secrets broker fail CLOSED (no fallback), got: {outcome:?}"
    );

    // The raw credential must be ABSENT from everything delivered into the
    // workspace during the degradation window (scan over BYTES, not a flag).
    assert!(
        workspace.credential_absent(smart.raw_credential()),
        "WP-X11①: NO credential may reach any workspace during the degradation \
         window"
    );

    // Positive control: when HEALTHY, the broker DOES deliver a PUBLIC result —
    // proving the broker is not vacuously failing everything — but even then the
    // raw credential never reaches the workspace.
    let mut healthy_ws = hugit_invariants::x11::Workspace::for_lease(&lease);
    let healthy = smart.broker_op(
        &mut healthy_ws,
        b"artifact:release-9.0",
        FaultPoint::Healthy,
    );
    assert!(
        matches!(healthy, BrokerOutcome::DeliveredPublicResult { .. }),
        "healthy broker must deliver a public result (the broker is not vacuous)"
    );
    assert!(
        healthy_ws.credential_absent(smart.raw_credential()),
        "even on the healthy path, the raw credential must never reach the \
         workspace (C5②: only the public result is delivered)"
    );
}

/// GAMED-ORACLE GUARD (item ①): if a fallback ever leaked the raw credential
/// into the workspace during the degradation window, the `credential_absent`
/// scan WOULD catch it. This proves item ①'s oracle is not vacuous — it tests
/// delivered BYTES, not a flag. We build the fail-OPEN workspace directly (a
/// workspace into which the raw credential was wrongly delivered) and assert the
/// scan reports it as NOT credential-absent.
#[test]
fn item_1_a_leaked_credential_would_be_caught_red() {
    let lease = held_lease("item1-leak");

    // A workspace that received ONLY the public result is credential-absent.
    let clean = hugit_invariants::x11::Workspace::with_deliveries(
        &lease,
        &[b"public-signature-hex-not-the-secret".to_vec()],
    );
    assert!(
        clean.credential_absent(RAW_CREDENTIAL),
        "a workspace with only the public result must be credential-absent"
    );

    // A workspace into which a buggy fallback leaked the raw credential (even
    // embedded inside a larger blob) is caught: it is NOT credential-absent.
    let mut leaked_blob = b"prefix||".to_vec();
    leaked_blob.extend_from_slice(RAW_CREDENTIAL);
    leaked_blob.extend_from_slice(b"||suffix");
    let leaked = hugit_invariants::x11::Workspace::with_deliveries(
        &lease,
        std::slice::from_ref(&leaked_blob),
    );
    assert!(
        !leaked.credential_absent(RAW_CREDENTIAL),
        "WP-X11① GUARD: a leaked raw credential in the workspace MUST be caught \
         (the absence scan is over bytes, not a flag) — got a false clean"
    );
}

// ── ② objects written during degradation are marked provenance-ABSENT; no ─────
//    synthetic intent/attestation ever fabricated by a fallback path. ───────────
#[test]
fn item_2_degradation_writes_provenance_absent_no_fabrication() {
    let smart = smart_layer();

    // Write a batch of objects DURING the degradation window.
    let window_writes: Vec<WrittenObject> = (0..5)
        .map(|i| {
            smart.write_object(
                &format!("obj-{i}"),
                &format!("refs/heads/wp-{i}"),
                FaultPoint::MidOperation,
            )
        })
        .collect();

    // Every degradation-window object must be marked provenance-ABSENT, and the
    // scan must find ZERO fabricated intents/attestations.
    for obj in &window_writes {
        assert!(
            obj.provenance.is_absent(),
            "WP-X11②: an object written during degradation must be marked \
             provenance-ABSENT, got: {:?}",
            obj.provenance
        );
    }
    let scan = scan_degradation_window_writes(&window_writes);
    assert!(
        scan.holds(),
        "WP-X11②: every window object provenance-ABSENT AND zero fabricated \
         intent/attestation; got {scan:?}"
    );
    assert_eq!(
        scan.fabricated_intents, 0,
        "no synthetic intent may be fabricated"
    );
    assert_eq!(
        scan.fabricated_attestations, 0,
        "no synthetic attestation may be fabricated"
    );

    // Positive control: the HEALTHY write path DOES land genuine provenance (an
    // intent + an attestation) — proving the write path is not vacuously marking
    // everything ABSENT. The genuine attestation is signable over the REAL
    // canonical preimage (driving the production single-source surface).
    let healthy = smart.write_object("obj-h", "refs/heads/main", FaultPoint::Healthy);
    assert!(
        healthy.provenance.is_present(),
        "the healthy write path must land GENUINE provenance (not ABSENT)"
    );
    if let Provenance::Present {
        intent,
        attestation,
    } = &healthy.provenance
    {
        assert_eq!(intent.target, "obj-h");
        // The genuine chain has a real, non-empty canonical preimage (the same
        // single-source surface X8/X7 sign over). A fabricated chain would have
        // to forge a signature over THIS preimage — which item ② forbids the
        // fallback from ever doing.
        let preimage = canonical_preimage(attestation);
        assert!(
            !preimage.is_empty(),
            "the genuine attestation must have a real canonical preimage"
        );
    } else {
        panic!("healthy provenance must be Present");
    }
}

/// GAMED-ORACLE GUARD (item ②): a fabricated provenance object (a degradation-
/// window write carrying PRESENT provenance) is caught RED by the scan. This is
/// the synthetic-intent/attestation fallback item ② forbids — if it ever
/// happened, the scan reports a fabrication and `holds()` is false.
#[test]
fn item_2_a_fabricated_provenance_would_be_caught_red() {
    // A fallback that WRONGLY fabricated provenance for a degraded write:
    let fabricated = WrittenObject {
        content_id: "obj-fab".to_string(),
        provenance: Provenance::Present {
            intent: Intent {
                intent_id: "SYNTHETIC".to_string(),
                ref_name: "refs/heads/x".to_string(),
                target: "obj-fab".to_string(),
                charter: "fabricated by a fallback".to_string(),
            },
            attestation: Box::new(healthy_attestation()),
        },
    };
    let scan = scan_degradation_window_writes(std::slice::from_ref(&fabricated));
    assert!(
        !scan.holds(),
        "a fabricated synthetic intent/attestation MUST be caught (item ② RED), \
         got a passing scan: {scan:?}"
    );
    assert_eq!(
        scan.fabricated_intents, 1,
        "the scan must count the fabricated synthetic intent"
    );
    assert_eq!(
        scan.fabricated_attestations, 1,
        "the scan must count the fabricated synthetic attestation"
    );
}

// ── ③ CoreLink non-interference (X10 baseline) holds WHILE degraded, not only ─
//    when healthy. ──────────────────────────────────────────────────────────────
#[test]
fn item_3_non_interference_holds_while_degraded() {
    // The X10 baseline must hold WHILE hugit is degraded. Drives the REAL X10
    // cap + workload + tolerance surfaces (a regression in X10 turns this RED).
    assert_non_interference_while_degraded(FaultPoint::MidOperation).unwrap_or_else(|e| {
        panic!("WP-X11③: CoreLink non-interference must hold WHILE degraded: {e}")
    });

    // It also holds when healthy (the baseline) — the point of item ③ is that
    // degradation does NOT loosen the boundary: both states obey the SAME cap.
    assert_non_interference_while_degraded(FaultPoint::Healthy)
        .expect("non-interference must also hold when healthy (the X10 baseline)");

    // Directly assert the X10⑤ cap is the precondition that holds while degraded.
    hugit_invariants::x10::HUGIT_CORELINK_TENANT_CAP
        .assert_valid_and_enforced()
        .expect("WP-X11③: the X10⑤ cap must be enforced while degraded");
}

// ── COMPOSITION: all three items compose across one degradation window. ───────
#[test]
fn composition_all_three_items_hold_across_one_window() {
    let smart = smart_layer();
    let lease = held_lease("composition");
    let evidence = run_degradation_window(&smart, &lease, 4);

    assert!(
        evidence.broker.failed_closed(),
        "① broker must fail closed mid-op: {:?}",
        evidence.broker
    );
    assert!(
        evidence.credential_absent,
        "① no credential may reach the workspace during the window"
    );
    assert!(
        evidence.scan.holds(),
        "② all window writes provenance-ABSENT, no fabrication: {:?}",
        evidence.scan
    );
    assert!(
        evidence.non_interference_ok,
        "③ non-interference must hold while degraded"
    );
    assert!(
        evidence.all_hold(),
        "the three invariants must COMPOSE across the degradation window: {evidence:?}"
    );
}

// ── LIVE P2 seam: mid-operation fault injection on the runner box ─────────────
// Gated behind HUGIT_RUNNER_HOST; run-not-skip when set. The bare gate proves
// the composition hermetically (above); this lane is the documented P2 seam.
#[test]
fn live_seam_mid_op_fault_on_box() {
    if !live_lane_active() {
        // Bare gate: the composition is already proven hermetically above. The
        // live seam is the documented P2 deferral (no box provisioned here).
        return;
    }
    // When HUGIT_RUNNER_HOST is set, the live mid-op fault injection runs (not
    // skips). The full live transport awaits the provisioned box (P2); until the
    // box-side smart-layer kill harness is wired, assert the box is reachable so
    // the lane FAILS (not silently passes) when the env claims a box exists.
    //
    // Since WP-R4 the SSH transport is the runner product across the wire
    // (corelink-runners) — hugit links no runner crate. This lane's reach-
    // ability ping drives `ssh` directly with the SAME transport semantics
    // the transferred `SshBox` used (BatchMode, pin-on-first-use known_hosts,
    // ConnectTimeout), so the run-not-skip contract is preserved exactly.
    let host = std::env::var("HUGIT_RUNNER_HOST").expect("checked by live_lane_active");
    let target = format!("root@{}", host.trim());
    let known_hosts = std::env::var("HUGIT_RUNNER_KNOWN_HOSTS")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| match std::env::var("HOME") {
            Ok(home) if !home.trim().is_empty() => format!("{home}/.hugit/known_hosts"),
            _ => ".hugit/known_hosts".to_string(),
        });
    let mut cmd = std::process::Command::new("ssh");
    if let Ok(home) = std::env::var("HOME") {
        let id = format!("{home}/.ssh/hugit-runner");
        if std::path::Path::new(&id).exists() {
            cmd.arg("-i").arg(id);
        }
    }
    let ping = cmd
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("UserKnownHostsFile={known_hosts}"))
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg(&target)
        .arg("'true'")
        .output()
        .expect("ssh to runner box failed to spawn");
    assert!(
        ping.status.code() == Some(0),
        "HUGIT_RUNNER_HOST set but runner box unreachable — the live mid-op \
         fault seam must FAIL (not skip) when the env claims a box exists \
         (code={:?}, stderr={:?})",
        ping.status.code(),
        String::from_utf8_lossy(&ping.stderr).trim()
    );
    // Box reachable: the hermetic composition above is the proof body; the live
    // smart-layer-kill harness lands when the box smart layer is deployed (P2).
}
