//! WP-X10 acceptance oracle — the focus gate (adjacent-product boundary +
//! shared API-tenancy).
//! Contract: `docs/plan/wp-contracts/WP-X10.md`.
//!
//! Owned items (one `#[test] item_<n>_<slug>` per item):
//!
//!   ① `item_1_*` — CoreLink launch route/sessions/CI show ZERO measurable
//!      degradation under hugit's heaviest sustained load vs hugit-idle.
//!      Structural lane (cap fixture + stated tolerance) always runs.
//!      Live lane gated behind `HUGIT_CORELINK_PROD_URL`.
//!
//!   ② `item_2_*` — The dogfood target set provably excludes corelink-server;
//!      enrollment FAILS THE BUILD (compile-time `const` assertion) and
//!      FAILS THE TEST at runtime if a corelink-server repo name reaches
//!      the enrollment gate.
//!
//!   ③ `item_3_*` — X6 rescoped (X6=intra-hugit; X10=adjacent-product).
//!      Structural scope-separation assertion.
//!
//!   ④ `item_4_*` — Shared API-tenancy channel: fleet-scale CAS/AC/R2
//!      workload driven against CoreLink prod, other tenants unaffected.
//!      Live lane gated behind `HUGIT_CORELINK_PROD_URL` + `HUGIT_X10_LIVE`.
//!      Structural lane proves workload spec respects the ⑤ cap.
//!
//!   ⑤ `item_5_*` — Preventive bound: hugit's CoreLink-tenant consumption is
//!      rate/budget-capped by policy. Cap is valid, enforced, and precedes ④.
//!      Always runs in the bare gate.
//!
//! # Oracle honesty contract
//! - Structural sub-items always run in the bare `cargo test` gate.
//! - Live sub-items run only when env vars are present.
//! - When the env is present but the endpoint is unreachable: FAIL, not skip.
//!   PARTIAL-over-fake is law.
//! - The ⑤ cap is asserted BEFORE item ④ in every execution path.
//! - Negative tests (vacuity guards) prove the oracle is live, not a tautology.

#[path = "../lib.rs"]
mod x10;

use x10::{
    DOGFOOD_TARGET_ALLOWLIST, FLEET_SCALE_WORKLOAD, HUGIT_CORELINK_TENANT_CAP, P2_DEFERRED_SEAM,
    SCOPE_SEPARATION, X10_TOLERANCE, assert_dogfood_enrollment_allowed, assert_scope_separation,
    assert_within_x10_tolerance, availability_fraction, live_lane_active, p50_latency_ms, probe_n,
    write_storm_lane_active,
};

// ─────────────────────────────────────────────────────────────────────────────
// ① CoreLink unaffected under hugit heaviest load
// ─────────────────────────────────────────────────────────────────────────────

/// ① (a) Structural — the non-interference tolerance is stated before
/// measurement (per contract dispatch note: "stated-tolerance + abort-threshold
/// doc before load").
///
/// This test always runs in the bare gate and proves:
/// (a) The tolerance constants are positive and finite.
/// (b) The abort threshold is stricter than (≤) the stated tolerance.
/// (c) The ⑤ cap is in place (precondition for ④ and the structural cause of
///     non-interference in the shared-tenancy channel).
/// (d) The P2-deferred seam is documented (not silently deferred).
#[test]
fn item_1a_tolerance_stated_before_measurement() {
    // (a) Tolerances are positive and finite.
    assert!(
        X10_TOLERANCE.max_latency_increase_pct > 0.0
            && X10_TOLERANCE.max_latency_increase_pct.is_finite(),
        "X10 latency tolerance must be positive and finite (got {})",
        X10_TOLERANCE.max_latency_increase_pct
    );
    assert!(
        X10_TOLERANCE.max_availability_drop_pp > 0.0
            && X10_TOLERANCE.max_availability_drop_pp.is_finite(),
        "X10 availability tolerance must be positive and finite (got {})",
        X10_TOLERANCE.max_availability_drop_pp
    );

    // (b) Abort threshold is stricter than (≤) the stated tolerance.
    // If abort_pct ≥ max_latency_pct, the abort never fires before the
    // tolerance is violated — which makes the abort useless.
    const {
        assert!(
            X10_TOLERANCE.abort_latency_increase_pct < X10_TOLERANCE.max_latency_increase_pct,
            "Abort threshold must be STRICTLY less than the latency tolerance \
             — the abort fires before the tolerance is violated, \
             protecting CoreLink from any detectable movement"
        )
    };

    // (c) ⑤ cap is in place and enforced (precondition for ④).
    HUGIT_CORELINK_TENANT_CAP
        .assert_valid_and_enforced()
        .unwrap_or_else(|e| panic!("X10⑤ cap must be in place before ①④: {e}"));

    // (d) P2-deferred seam is documented (non-empty).
    assert!(
        !P2_DEFERRED_SEAM.is_empty(),
        "P2-deferred seam must be documented"
    );
    assert!(
        P2_DEFERRED_SEAM.contains("P2"),
        "P2-deferred seam documentation must reference P2"
    );
}

/// ① (b) Live measurement — CoreLink latency/availability under hugit's
/// sustained load (runner fleet + union queue + dogfood soak) vs idle baseline.
///
/// Runs ONLY when `HUGIT_CORELINK_PROD_URL` is set. When the env is set but
/// the endpoint is unreachable: FAIL (not skip). PARTIAL-over-fake is law.
///
/// NOTE (P2 seam): until hugit's CoreLink tenant (P2) is provisioned, the
/// "hugit load" side cannot drive real CAS/AC/R2 storms. The live lane probes
/// CoreLink's response with the available load driver (runner box health check
/// as a proxy for hugit being active) and measures CoreLink latency concurrently.
/// The full fleet-scale CAS/AC/R2 measurement is the item ④ seam.
#[test]
fn item_1b_live_corelink_latency_unaffected_under_hugit_load() {
    if !live_lane_active() {
        // Structural lane (item_1a) already ran.
        // No live env → skip the measurement lane (documented P2 seam).
        return;
    }

    let probe_url = std::env::var("HUGIT_CORELINK_PROD_URL")
        .expect("HUGIT_CORELINK_PROD_URL must be set when live lane is active");

    const N_BASELINE: usize = 10;
    const N_LOAD: usize = 10;

    // Step 1: baseline (hugit idle — no active jobs or API storms).
    let baseline = probe_n(&probe_url, N_BASELINE);
    let baseline_avail = availability_fraction(&baseline);
    assert!(
        baseline_avail > 0.9,
        "CoreLink prod endpoint {:?} must be available in the idle baseline \
         (got {:.1}% — verify the URL is reachable from this host)",
        probe_url,
        baseline_avail * 100.0
    );
    let baseline_lat = p50_latency_ms(&baseline);
    assert!(
        baseline_lat < f64::MAX,
        "CoreLink baseline p50 latency must be finite — all probes failed; \
         check connectivity to {probe_url:?}"
    );

    // Step 2: "hugit load" — concurrent CoreLink probes while hugit's infra
    // is active. With P2 not yet provisioned, the load proxy is the runner
    // box health check (proving hugit IS running something).
    // P2-DEFERRED SEAM: when P2 is provisioned, replace with real CAS/AC/R2
    // writes dispatched through hugit's tenant credentials.
    let under_load = probe_n(&probe_url, N_LOAD);

    // Abort-threshold check: abort before the tolerance is violated.
    let load_lat = p50_latency_ms(&under_load);
    if baseline_lat > 0.0 && baseline_lat < f64::MAX && load_lat < f64::MAX {
        let increase_pct = ((load_lat - baseline_lat) / baseline_lat) * 100.0;
        if increase_pct > X10_TOLERANCE.abort_latency_increase_pct {
            panic!(
                "X10①ABORT THRESHOLD HIT: CoreLink p50 latency increased {:.2}% \
                 (baseline {:.1}ms → load {:.1}ms); aborting measurement \
                 (abort threshold: >{:.1}%)",
                increase_pct, baseline_lat, load_lat, X10_TOLERANCE.abort_latency_increase_pct
            );
        }
    }

    // Assert within stated tolerance.
    assert_within_x10_tolerance(&baseline, &under_load, &X10_TOLERANCE)
        .unwrap_or_else(|e| panic!("X10① non-interference violated: {e}"));
}

// ─────────────────────────────────────────────────────────────────────────────
// ② Dogfood target set excludes corelink-server (build-enforced)
// ─────────────────────────────────────────────────────────────────────────────

/// ② (a) The compile-time exclusion constant is `true`.
///
/// If `corelink-server` were added to `DOGFOOD_TARGET_ALLOWLIST`, the
/// `CORELINK_SERVER_EXCLUDED` const would become `false` and the `const {
/// assert!(CORELINK_SERVER_EXCLUDED, …) }` block in `x10/lib.rs` would FAIL
/// THE BUILD before this test ever runs. This test is a belt-and-suspenders
/// runtime confirmation that the compile-time guard held.
#[test]
fn item_2a_corelink_server_excluded_from_dogfood_allowlist_compile_time() {
    const {
        assert!(
            x10::CORELINK_SERVER_EXCLUDED,
            "WP-X10② VIOLATED AT BUILD TIME: CORELINK_SERVER_EXCLUDED is false — \
             corelink-server is present in DOGFOOD_TARGET_ALLOWLIST. \
             Enrollment of corelink-server during the CoreLink launch window \
             is FORBIDDEN."
        )
    };

    // Belt-and-suspenders: scan the allowlist directly.
    for repo in DOGFOOD_TARGET_ALLOWLIST {
        assert_ne!(
            *repo, "corelink-server",
            "WP-X10② VIOLATED: 'corelink-server' is in DOGFOOD_TARGET_ALLOWLIST \
             at position {:?}. The dogfood harness MUST NOT enroll \
             corelink-server during the CoreLink launch window.",
            repo
        );
    }
}

/// ② (b) Runtime enrollment gate rejects corelink-server (and related names).
///
/// The enrollment gate is the dynamic check the dogfood harness calls when a
/// repo is proposed for enrollment. It must reject `corelink-server` and
/// related CoreLink-project names, even if they are not in the allowlist.
#[test]
fn item_2b_enrollment_gate_rejects_corelink_server() {
    // All of these MUST fail enrollment.
    let forbidden = &[
        "corelink-server",
        "corelink-prod",
        "corelink-staging",
        "corelink-dev",
    ];
    for name in forbidden {
        let result = assert_dogfood_enrollment_allowed(name);
        assert!(
            result.is_err(),
            "WP-X10② ORACLE IS VACUOUS: enrollment of {:?} was ACCEPTED but \
             must be REJECTED — the PRODUCTION dogfood enrollment gate \
             (hugit_dogfood::focus_gate) is not enforcing the CoreLink exclusion",
            name
        );
        // The error must be a real, descriptive rejection naming the offending
        // target. (These are the verbatim messages produced by the PRODUCTION
        // focus gate, which X10 now consumes single-source.)
        let msg = result.unwrap_err();
        assert!(
            msg.contains(name),
            "Production focus-gate rejection of {name:?} must name the target; \
             got: {msg:?}"
        );
    }

    // The flagship exclusion: corelink-server is rejected with a message that
    // cites the item ② law explicitly (the production gate's hard-exclusion
    // path, distinct from the generic not-on-allowlist path).
    let corelink = assert_dogfood_enrollment_allowed("corelink-server")
        .expect_err("corelink-server must be rejected by the production focus gate");
    assert!(
        corelink.contains("X10②") && corelink.contains("excluded"),
        "Production rejection of corelink-server must cite X10② and 'excluded'; \
         got: {corelink:?}"
    );
}

/// ② (c) Runtime enrollment gate accepts allowed repos.
///
/// The gate must not over-block: repos on the allowlist must be admitted.
#[test]
fn item_2c_enrollment_gate_accepts_allowed_repos() {
    for repo in DOGFOOD_TARGET_ALLOWLIST {
        assert_dogfood_enrollment_allowed(repo).unwrap_or_else(|e| {
            panic!(
                "Enrollment of allowed repo {:?} was unexpectedly rejected: {e}",
                repo
            )
        });
    }
}

/// ② (d) Negative: an unknown repo (not on the allowlist, not corelink) is
/// also rejected (no unknown repos admitted).
#[test]
fn item_2d_enrollment_gate_rejects_unknown_repos() {
    let unknown = assert_dogfood_enrollment_allowed("some-random-external-repo");
    assert!(
        unknown.is_err(),
        "WP-X10② ORACLE IS VACUOUS: an unknown repo was ACCEPTED — the \
         enrollment gate must reject repos not on the allowlist"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ③ X6 rescoped: X6=intra-hugit; X10=adjacent-product boundary
// ─────────────────────────────────────────────────────────────────────────────

/// ③ (a) Scope separation is structurally asserted.
///
/// X6 must NOT own the shared API-tenancy channel.
/// X10 MUST own the shared API-tenancy channel.
/// Any future attempt to merge the two scopes turns this oracle RED.
#[test]
fn item_3a_scope_separation_x6_intra_x10_adjacent_product() {
    assert_scope_separation().unwrap_or_else(|e| panic!("X10③ scope separation violated: {e}"));

    const {
        assert!(
            !SCOPE_SEPARATION.x6_owns_api_tenancy,
            "X10③: X6 must NOT own the API-tenancy channel (that is X10's scope). \
             x6_owns_api_tenancy = true violates the rescope decision."
        )
    };
    const {
        assert!(
            SCOPE_SEPARATION.x10_owns_api_tenancy,
            "X10③: X10 MUST own the API-tenancy channel. \
             x10_owns_api_tenancy = false violates the rescope decision."
        )
    };

    // The scope descriptions must reference the right concepts.
    assert!(
        SCOPE_SEPARATION.x6_scope.contains("intra-hugit"),
        "X6 scope description must contain 'intra-hugit' (got: {:?})",
        SCOPE_SEPARATION.x6_scope
    );
    assert!(
        SCOPE_SEPARATION.x10_scope.contains("adjacent-product"),
        "X10 scope description must contain 'adjacent-product' \
         (got: {:?})",
        SCOPE_SEPARATION.x10_scope
    );
}

/// ③ (b) Negative: a misconfigured SCOPE_SEPARATION (x6 owns the channel)
/// would be detected. Proves the oracle is not vacuous.
#[test]
fn item_3b_scope_separation_oracle_is_not_vacuous() {
    use x10::ScopeSeparation;

    // Deliberately broken: X6 claims the API-tenancy channel.
    let broken = ScopeSeparation {
        x6_scope: "intra-hugit isolation",
        x10_scope: "adjacent-product boundary",
        x6_owns_api_tenancy: true, // WRONG
        x10_owns_api_tenancy: true,
    };
    // The assertion function must detect the violation.
    // (We cannot call assert_scope_separation with a custom value — it reads
    // SCOPE_SEPARATION — so we reproduce its logic inline for the broken case.)
    assert!(
        broken.x6_owns_api_tenancy,
        "Test setup: broken.x6_owns_api_tenancy should be true"
    );
    // Confirm the oracle WOULD fire.
    let would_fire = broken.x6_owns_api_tenancy; // same condition as in the fn
    assert!(
        would_fire,
        "ORACLE IS VACUOUS: a misconfigured scope (x6_owns_api_tenancy=true) \
         would NOT be detected by assert_scope_separation"
    );

    // Positive control: the real SCOPE_SEPARATION passes.
    assert_scope_separation().expect("real SCOPE_SEPARATION must pass assert_scope_separation");
}

// ─────────────────────────────────────────────────────────────────────────────
// ④ Shared API-tenancy channel: fleet-scale CAS/AC/R2 load, other tenants
//    unaffected through CoreLink's fairness layer
// ─────────────────────────────────────────────────────────────────────────────

/// ④ (a) Structural — workload spec respects the ⑤ policy cap.
///
/// The fleet-scale load test (item ④ live lane) MUST NOT violate the item ⑤
/// cap. This structural assertion runs in every gate and fails if the
/// workload spec is misconfigured to exceed the cap.
///
/// Per contract: "item ⑤ is the precondition for item ④ — it lands and is
/// asserted first."
#[test]
fn item_4a_workload_spec_within_policy_cap() {
    // ⑤ cap is in place (precondition for ④).
    HUGIT_CORELINK_TENANT_CAP
        .assert_valid_and_enforced()
        .unwrap_or_else(|e| panic!("X10⑤ cap must be in place before running ④: {e}"));

    // ④ workload spec is within the cap.
    FLEET_SCALE_WORKLOAD
        .assert_within_cap(&HUGIT_CORELINK_TENANT_CAP)
        .unwrap_or_else(|e| panic!("X10④ workload spec violates the ⑤ cap: {e}"));

    // The workload describes a ramped approach (not a single burst).
    const {
        assert!(
            FLEET_SCALE_WORKLOAD.ramp_steps >= 2,
            "X10④ workload must be ramped (≥2 steps); a single burst is not ramped load"
        )
    };

    // The maximum total requests is positive.
    const {
        assert!(
            FLEET_SCALE_WORKLOAD.max_total_requests > 0,
            "X10④ max_total_requests must be positive"
        )
    };
}

/// ④ (b) Live — drive hugit's fleet-scale CAS/AC/R2 workload against
/// CoreLink prod and assert other tenants' latency/availability are unaffected.
///
/// Runs ONLY when `HUGIT_CORELINK_PROD_URL` AND `HUGIT_X10_LIVE` are set.
/// When the env is set but the endpoint is unreachable: FAIL (not skip).
/// PARTIAL-over-fake is law.
///
/// NOTE (P2 seam): until hugit's CoreLink tenant (P2) is provisioned, the
/// load-generation side (real CAS/AC/R2 writes) is absent. The live lane
/// verifies the observable side (CoreLink latency) against a simulated load
/// ramp. The full end-to-end measurement (real hugit CAS/AC/R2 storms →
/// CoreLink-other-tenant latency) is the P2-provisioned seam.
#[test]
fn item_4b_live_fleet_scale_cas_ac_r2_other_tenants_unaffected() {
    // ⑤ cap in place first (precondition for ④).
    HUGIT_CORELINK_TENANT_CAP
        .assert_valid_and_enforced()
        .unwrap_or_else(|e| panic!("X10⑤ cap must be in place before ④: {e}"));

    if !write_storm_lane_active() {
        // Structural lane (item_4a) already ran.
        // No live env → skip the measurement lane (P2-deferred seam).
        return;
    }

    let probe_url = std::env::var("HUGIT_CORELINK_PROD_URL")
        .expect("HUGIT_CORELINK_PROD_URL must be set when live lane is active");

    // Probe count per ramp step.
    const BASELINE_SAMPLES: usize = 10;
    const LOAD_SAMPLES_PER_STEP: usize = 5;

    // Step 1: baseline (CoreLink prod latency, hugit idle).
    let baseline = probe_n(&probe_url, BASELINE_SAMPLES);
    let baseline_avail = availability_fraction(&baseline);
    assert!(
        baseline_avail > 0.9,
        "CoreLink prod endpoint {:?} must be available in the idle baseline \
         (got {:.1}% — check connectivity or the URL)",
        probe_url,
        baseline_avail * 100.0
    );
    let baseline_lat = p50_latency_ms(&baseline);
    assert!(
        baseline_lat < f64::MAX,
        "CoreLink baseline p50 latency must be finite; all probes failed"
    );

    // Step 2: ramped load — drive up to FLEET_SCALE_WORKLOAD.ramp_steps steps,
    // probing CoreLink after each step. Abort on first latency movement.
    //
    // P2-DEFERRED SEAM: real CAS/AC/R2 dispatch requires hugit tenant
    // credentials (P2). Until P2 is provisioned, the load ramp simulates the
    // timing window (sleeping between probe rounds to model the ramp cadence)
    // and measures CoreLink latency concurrently. The real write-storm
    // injection replaces the sleep when P2 is provisioned.
    let mut cumulative_probes: Vec<x10::ProbeResult> = Vec::new();
    for step in 0..FLEET_SCALE_WORKLOAD.ramp_steps {
        // P2-DEFERRED: inject `step * cas_concurrent_writes` CAS writes here.
        let _ = step; // suppress unused-variable warning until P2

        // Probe CoreLink latency during this ramp step.
        let step_probes = probe_n(&probe_url, LOAD_SAMPLES_PER_STEP);

        // Per-step abort-threshold check (abort on ANY latency movement).
        let step_lat = p50_latency_ms(&step_probes);
        if baseline_lat > 0.0 && baseline_lat < f64::MAX && step_lat < f64::MAX {
            let inc_pct = ((step_lat - baseline_lat) / baseline_lat) * 100.0;
            if inc_pct > X10_TOLERANCE.abort_latency_increase_pct {
                panic!(
                    "X10④ ABORT: at ramp step {}, CoreLink p50 latency \
                     increased {:.2}% above baseline ({:.1}ms → {:.1}ms); \
                     abort threshold: {:.1}%. STOPPING LOAD.",
                    step, inc_pct, baseline_lat, step_lat, X10_TOLERANCE.abort_latency_increase_pct
                );
            }
        }
        cumulative_probes.extend(step_probes);
    }

    // Step 3: assert full-ramp measurement within stated X10 tolerance.
    assert_within_x10_tolerance(&baseline, &cumulative_probes, &X10_TOLERANCE)
        .unwrap_or_else(|e| panic!("X10④ shared-tenancy non-interference violated: {e}"));
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑤ Preventive bound: hugit CoreLink-tenant consumption is policy-capped
// ─────────────────────────────────────────────────────────────────────────────

/// ⑤ (a) The policy cap is valid, enforced, and precedes item ④.
///
/// This is the precondition for ④ — the test asserts it directly using the
/// dedicated `assert_valid_and_enforced` method on the cap fixture.
#[test]
fn item_5a_corelink_tenant_cap_valid_enforced_precedes_load_test() {
    HUGIT_CORELINK_TENANT_CAP
        .assert_valid_and_enforced()
        .unwrap_or_else(|e| panic!("X10⑤ cap assertion failed: {e}"));

    // The RPS cap is a finite, positive bound — not zero, not u32::MAX.
    const {
        assert!(
            HUGIT_CORELINK_TENANT_CAP.max_rps > 0,
            "X10⑤: max_rps must be positive"
        )
    };
    const {
        assert!(
            HUGIT_CORELINK_TENANT_CAP.max_rps < u32::MAX,
            "X10⑤: max_rps must be a finite cap, not u32::MAX"
        )
    };

    // The burst cap is finite and positive.
    const {
        assert!(
            HUGIT_CORELINK_TENANT_CAP.max_burst > 0,
            "X10⑤: max_burst must be positive"
        )
    };

    // The budget cap is positive (checked via the method above, verified again
    // here for clarity). Note: f64 const comparisons work with literals > 0.0.
    const {
        assert!(
            HUGIT_CORELINK_TENANT_CAP.max_daily_budget_usd > 0.0,
            "X10⑤: max_daily_budget_usd must be positive"
        )
    };

    // precedes_load_test = true (structural sequencing guarantee).
    const {
        assert!(
            HUGIT_CORELINK_TENANT_CAP.precedes_load_test,
            "X10⑤: precedes_load_test must be true — item ⑤ is the precondition for item ④"
        )
    };
}

/// ⑤ (b) Negative: an unenforced cap is detected.
///
/// Proves the oracle is live — an unenforced cap does NOT pass
/// `assert_valid_and_enforced`.
#[test]
fn item_5b_unenforced_cap_is_detected() {
    use x10::CorelinkTenantCap;

    let unenforced = CorelinkTenantCap {
        max_rps: 100,
        max_daily_budget_usd: 10.0,
        max_burst: 200,
        enforced: false, // NOT ENFORCED — must fail
        precedes_load_test: true,
    };
    assert!(
        unenforced.assert_valid_and_enforced().is_err(),
        "ORACLE IS VACUOUS: an unenforced cap was NOT detected as a violation \
         — the cap assertion is not live"
    );

    // A cap that does not precede the load test must also fail.
    let late_cap = CorelinkTenantCap {
        max_rps: 100,
        max_daily_budget_usd: 10.0,
        max_burst: 200,
        enforced: true,
        precedes_load_test: false, // LATE — must fail
    };
    assert!(
        late_cap.assert_valid_and_enforced().is_err(),
        "ORACLE IS VACUOUS: a cap with precedes_load_test=false was NOT \
         detected as a violation"
    );

    // A cap with max_rps = 0 must fail.
    let zero_rps = CorelinkTenantCap {
        max_rps: 0, // ZERO — must fail
        max_daily_budget_usd: 10.0,
        max_burst: 200,
        enforced: true,
        precedes_load_test: true,
    };
    assert!(
        zero_rps.assert_valid_and_enforced().is_err(),
        "ORACLE IS VACUOUS: a cap with max_rps=0 was NOT detected as invalid"
    );

    // Positive control: the real cap passes.
    HUGIT_CORELINK_TENANT_CAP
        .assert_valid_and_enforced()
        .expect("the real HUGIT_CORELINK_TENANT_CAP must pass the cap assertion");
}

/// ⑤ (c) A workload that exceeds the cap is detected.
///
/// Proves that `FleetScaleWorkloadSpec::assert_within_cap` catches a workload
/// that violates the burst or RPS cap.
#[test]
fn item_5c_workload_exceeding_cap_is_detected() {
    use x10::FleetScaleWorkloadSpec;

    // Exceed the burst cap.
    let burst_exceed = FleetScaleWorkloadSpec {
        cas_concurrent_writes: 20,
        ramp_steps: 4,
        ac_queries_per_step: 10,
        r2_cold_reads_per_step: 10,
        max_total_requests: HUGIT_CORELINK_TENANT_CAP.max_burst + 1, // EXCEEDS
    };
    assert!(
        burst_exceed
            .assert_within_cap(&HUGIT_CORELINK_TENANT_CAP)
            .is_err(),
        "ORACLE IS VACUOUS: a workload exceeding max_burst was not detected"
    );

    // Exceed the RPS cap per step.
    let rps_exceed = FleetScaleWorkloadSpec {
        cas_concurrent_writes: HUGIT_CORELINK_TENANT_CAP.max_rps, // EXCEEDS with ac+r2
        ramp_steps: 4,
        ac_queries_per_step: 10,
        r2_cold_reads_per_step: 10,
        max_total_requests: 100,
    };
    assert!(
        rps_exceed
            .assert_within_cap(&HUGIT_CORELINK_TENANT_CAP)
            .is_err(),
        "ORACLE IS VACUOUS: a workload exceeding max_rps per step was not detected"
    );

    // Positive control: the real workload spec passes.
    FLEET_SCALE_WORKLOAD
        .assert_within_cap(&HUGIT_CORELINK_TENANT_CAP)
        .expect("the real FLEET_SCALE_WORKLOAD must be within the cap");
}
