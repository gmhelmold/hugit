//! LIVE smoke for the cost-killer "A-path" — the OFF-BOX lease lifecycle
//! (acquire → §13.2 envelope ingest → close-with-attested-metrics), driven
//! through hugit's REAL [`LeaseClient`] over the REAL `ureq` transport against
//! the REAL fabric control plane.
//!
//! WHY THIS EXISTS (the gap it closes): the lease client was, for its whole
//! life, exercised ONLY through the fake in-memory transport + PAT-gated — so it
//! was NEVER run against a live fabric. That blind spot let THREE separate wire
//! drifts land green (the acquire-response wrapper #204, the close status body
//! #205, the acquire-request shape #214) — each caught only by a hand-run manual
//! smoke, never a standing test. This test IS that standing smoke: a real
//! request/response round-trip against the live fabric that FAILS LOUDLY the
//! instant hugit's DTOs drift from the frozen fabric wire.
//!
//! GATED (run-not-skip / clean-skip): the loader is the gate. `runner_from_env()`
//! returning `Ok` means `HUGIT_RUNNER_HOST` is set + the PAT is readable (file
//! `~/.hugit/secrets/runner/pat`, or the `HUGIT_RUNNER_PAT_FILE`/`HUGIT_RUNNER_PAT`
//! fallbacks) ⇒ RUN the live A-path. `Err(NotConfigured)` ⇒ absent ⇒ clean SKIP
//! with a printed reason, so the gate cannot rot to a fake green (CI leaves
//! `HUGIT_RUNNER_HOST` unset, so this skips there by construction).
//!
//! To run live (e.g. against the Cloudflare fabricd):
//!   HUGIT_RUNNER_HOST=https://corelink-fabricd.gmhelmold.workers.dev \
//!     cargo test -p hugit-checks --test runner_apath_live_smoke -- --nocapture
//!
//! HONESTY: `close` is called with `cost_usd_micros = None` (the honest-zero
//! floor) — this smoke has NO real provider-`/usage` figure to submit, and the
//! per-PR honesty law forbids a derived/misattributed stand-in. The PAT and the
//! scoped §13 ingest credential are NEVER printed (both are `<redacted>` in
//! Debug; only HTTP statuses and non-secret fields are surfaced).

use hugit_checks::runner::{
    AcquireLeaseRequest, CloseStatus, IngestEvent, IngestUsage, RunnerError, runner_from_env,
};

#[test]
fn runner_apath_live_smoke() {
    // ── GATE: Ok ⇒ host + PAT present ⇒ RUN; NotConfigured ⇒ clean SKIP. ─────
    let client = match runner_from_env() {
        Ok(c) => c,
        Err(RunnerError::NotConfigured(why)) => {
            println!(
                "SKIPPED runner_apath_live_smoke: runner config absent ({why}) — live A-path \
                 not exercised. Set HUGIT_RUNNER_HOST + the PAT (file ~/.hugit/secrets/runner/pat \
                 or HUGIT_RUNNER_PAT) to drive acquire → §13 ingest → close against the fabric."
            );
            return;
        }
        Err(other) => panic!("loader returned an unexpected error: {other:?}"),
    };

    // ── (1) ACQUIRE an off-box A-mode lease. ────────────────────────────────
    // For hugit's off-box path the image is recorded metadata (no box is ever
    // spawned — hugit submits §13 off-box, never execs); the fabric only
    // FORMAT-checks for `sha256:`. A short TTL bounds the lease.
    // Values are the fabric-ACCEPTED shape for a check/off-box acquire, confirmed
    // live by the runners TL (2026-07-07):
    //   - `net_policy: "none"` — an isolated class the C2a `requires_no_network`
    //     check admits (`none`/`isolated`/`deny-all`/`""`). A plain isolated lease
    //     that carries NO `toolchain_digest` is the OFF-BOX marker: the CF
    //     provisioner admits it NO-BOX (fast `200 Held`, hosts §13 + attestation,
    //     never spawns) — no extra field needed.
    //   - `image_digest: "<name>@sha256:<64hex>"` — the X4 pin check requires a
    //     content-pinned ref; a BARE `sha256:<hex>` is rejected.
    // TRAP (do not copy): `conformance/AcquireRequest.json` is a RUNNER example
    // with PLACEHOLDER values (`net_policy:"hermetic"`, bare `sha256:`) where
    // net_policy is ignored server-side — those strings are NOT fabric-accepted on
    // an off-box acquire.
    // This test performs EXACTLY ONE acquire and never loops. Off-box A-path
    // acquires are no-box (burst-safe since the 2026-07-07 fix), but the
    // single-acquire discipline stays: box-provisioning leases (runner/check-host)
    // could still saturate the single-flight fabric singleton.
    let req = AcquireLeaseRequest {
        image_digest: format!("alpine@sha256:{}", "d".repeat(64)),
        net_policy: "none".to_string(),
        tmp_root: "/tmp/hugit-apath-smoke".to_string(),
        expiry_ms: 300_000,
    };
    let acq = client
        .acquire(&req)
        .expect("live acquire must return 200/201 with the AcquireResponse wrapper");
    let lease_id = acq.lease.lease_id.clone();
    assert!(
        !lease_id.is_empty(),
        "acquire must grant a non-empty lease_id"
    );
    println!(
        "A-path acquire OK: lease_id={lease_id} state={:?} exec_endpoint_present={} \
         envelope_ingest_present={}",
        acq.lease.state,
        !acq.exec_endpoint.is_empty(),
        acq.envelope_ingest.is_some(),
    );

    // ── (2) §13.2 ENVELOPE INGEST (the cost-killer trajectory events). ──────
    // Present for non-runner/check leases when §13 is wired. If the fabric
    // surfaces the scoped ingest credential, POST one trajectory event to it
    // with the SCOPED cred (never the PAT).
    match acq.envelope_ingest.as_ref() {
        Some(ing) => {
            let events = vec![IngestEvent {
                kind: "model_turn".to_string(),
                // A tiny, non-secret smoke transcript, standard-base64.
                bytes_b64: base64_std("hugit A-path live smoke: model_turn"),
                tool: None,
                usage: Some(IngestUsage {
                    input: 10,
                    output: 5,
                    cache_read: 0,
                    cache_write: 0,
                }),
                busy_ms: 1,
            }];
            client
                .submit_envelope(&ing.ingest_path, &ing.credential, &events)
                .expect("live §13 envelope ingest must ack (200/201/202/204)");
            println!(
                "A-path §13 ingest OK: 1 model_turn event accepted at {}",
                ing.ingest_path
            );
        }
        None => {
            // Honest: a runner-shaped lease (or §13-off) surfaces no ingest cred.
            // Not a failure of the lifecycle — record it and continue to close.
            println!(
                "A-path §13 ingest SKIPPED: acquire surfaced no envelope_ingest \
                 (runner-shaped lease or §13 not wired for this lease type on the live host)"
            );
        }
    }

    // ── (3) CLOSE with the attested §13.1 metrics. ──────────────────────────
    // cost_usd_micros = None (honest-zero floor): this smoke has no real
    // provider-billed figure and MUST NOT fabricate one (per-PR honesty law).
    let close = client
        .close(&lease_id, CloseStatus::Succeeded, None)
        .expect("live close must return 200 + the atomic CloseResponse (metrics)");
    assert_eq!(
        close.lease_id, lease_id,
        "close must echo the closed lease_id"
    );
    println!(
        "A-path close OK: released={} capture_incomplete={} metrics={:?}",
        close.released, close.capture_incomplete, close.metrics,
    );

    println!(
        "LIVE A-path smoke PASSED end-to-end (acquire → §13 → close) against the fabric — \
         hugit's LeaseClient wire matches the live fabric DTOs."
    );
}

/// Minimal standard-alphabet base64 (RFC 4648 §4, padded) — no extra dep; the
/// `base64` crate isn't a dev-dependency here and this smoke only needs to
/// encode a short ASCII string.
fn base64_std(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for chunk in b.chunks(3) {
        let n = chunk.len();
        let b0 = chunk[0] as u32;
        let b1 = if n > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if n > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((triple >> 18) & 0x3f) as usize] as char);
        out.push(T[((triple >> 12) & 0x3f) as usize] as char);
        out.push(if n > 1 {
            T[((triple >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if n > 2 {
            T[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}
