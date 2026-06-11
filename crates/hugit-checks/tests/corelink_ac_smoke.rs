//! §6 LIVE smoke test for the CoreLink Action-Cache seam — the three probes,
//! run against the REAL endpoint through the REAL `UreqTransport`.
//!
//! GATED (run-not-skip / clean-skip): this test exercises the live CoreLink AC
//! ONLY when the runtime config is fully present —
//!   - `HUGIT_CORELINK_AC_URL` set + non-empty,
//!   - `HUGIT_CORELINK_TENANT` set + non-empty, and
//!   - a readable PAT (file `~/.hugit/secrets/corelink/pat`, or the
//!     `HUGIT_CORELINK_PAT_FILE` override, or the `HUGIT_CORELINK_PAT` env
//!     fallback).
//!
//! The gate IS the loader: `corelink_ac_from_env()` returning `Ok` means all
//! three are present; `Err(NotConfigured)` means absent → we SKIP with a printed
//! reason (so the gate cannot rot to a fake green before the tenant exists).
//!
//! The three probes (§6), each MUST hold against the live endpoint:
//!   (1) GET a random unused digest  → 404 MISS (decisively NOT 401 — the PAT
//!       authenticates, the object simply does not exist);
//!   (2) PUT a throwaway CheckResult then GET it → 200 HIT, byte-identical;
//!   (3) cross-tenant probe (a tenant slug the PAT is NOT scoped to) → 403.
//!
//! The PAT value is never printed. Cleanup of the throwaway object is best-effort
//! (CoreLink AC is content-addressed + idempotent; a unique random tree axis
//! makes the digest unique per run so it never collides with real data).

use hugit_checks::client::ac::{
    AcConfig, AcError, ActionCache, ENV_AC_URL, ENV_TENANT, HttpAcClient, UreqTransport,
    corelink_ac_from_env,
};
use hugit_contracts::CheckResult;

/// Build a `CheckResult` whose `memo_key` is the REAL three-axis key over its
/// own axes (so the client's content-address hit-guard accepts it), with a
/// RANDOM `tree_hash` so the digest is unique per run (never collides with real
/// memoized data; the GET-miss probe is genuinely "unused").
fn throwaway_result(rand_tree: &str) -> CheckResult {
    let tree_hash = rand_tree.to_string();
    let def_digest = "bb".repeat(32);
    let toolchain_digest = "cc".repeat(32);
    let memo_key = hugit_refstore::compute_memo_key(&tree_hash, &def_digest, &toolchain_digest);
    CheckResult {
        memo_key,
        tree_hash,
        def_digest,
        toolchain_digest,
        exit: 0,
        artifacts: vec![],
        stdout_ref: "blob:stdout".into(),
        stderr_ref: "blob:stderr".into(),
        duration_ms: 1,
        runner_ref: "runner:smoke".into(),
        produced_at: 1_717_000_000_000,
    }
}

/// A process-unique random 64-hex string for the tree axis (no extra deps:
/// derive from a SHA-256 over time + pid + a counter).
fn random_hex64() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!("{}-{}-{:p}", std::process::id(), nanos, &nanos as *const _);
    // hugit_refstore re-exports the canonical lowercase-hex SHA-256 helper via
    // compute_memo_key, but for a plain random axis we hash the seed directly.
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    hex::encode(h.finalize())
}

#[test]
fn corelink_ac_live_smoke() {
    // ── GATE: the loader is the gate. Ok ⇒ all three present ⇒ RUN; a
    //    NotConfigured ⇒ absent ⇒ clean SKIP with a printed reason. ──────────
    // NOTE: cargo test captures stdout by default; pass --nocapture to see this
    // skip message in the terminal. The eprintln! variant below reaches stderr
    // (always visible), but the canonical loud-skip protocol uses println! for
    // stdout parity with other env-gated suites in this workspace.
    let client = match corelink_ac_from_env() {
        Ok(c) => c,
        Err(AcError::NotConfigured(why)) => {
            println!(
                "SKIPPED corelink_ac_live_smoke: CoreLink AC config absent ({why}) — \
                 live lane not exercised. \
                 Set {ENV_AC_URL} + {ENV_TENANT} + the PAT (file ~/.hugit/secrets/corelink/pat \
                 or HUGIT_CORELINK_PAT) to run the live §6 probes."
            );
            return;
        }
        Err(other) => panic!("loader returned an unexpected error: {other:?}"),
    };

    // ── Probe (1): GET a random UNUSED digest → 404 MISS, NOT 401. ──────────
    // A miss maps to Ok(None); a 401 (bad PAT) would surface as Status(401).
    let unused = throwaway_result(&random_hex64());
    match client.lookup(&unused.memo_key) {
        Ok(None) => { /* 404 miss — correct: authenticated, object absent */ }
        Ok(Some(_)) => panic!("probe (1): a random unused digest must MISS, got a HIT"),
        Err(AcError::Status(401)) => {
            panic!("probe (1): got 401 — the PAT did not authenticate (must be a 404 MISS)")
        }
        Err(e) => panic!("probe (1): expected a 404 MISS (Ok(None)), got error {e}"),
    }

    // ── Probe (2): PUT a throwaway CheckResult, then GET it → 200 HIT, same
    //    bytes (content-verified by the client's own hit-guard). ─────────────
    let obj = throwaway_result(&random_hex64());
    client
        .store(&obj)
        .unwrap_or_else(|e| panic!("probe (2): store (PUT) must succeed (200/201), got {e}"));
    match client.lookup(&obj.memo_key) {
        Ok(Some(hit)) => assert_eq!(
            hit, obj,
            "probe (2): the HIT must be byte-identical to what we PUT"
        ),
        Ok(None) => panic!("probe (2): expected a HIT after store, got a MISS"),
        Err(e) => panic!("probe (2): expected a 200 HIT, got error {e}"),
    }

    // ── Probe (3): cross-tenant probe → 403. We build a client whose PATH
    //    tenant differs from the PAT's tenant (the edge Worker enforces
    //    path-tenant == PAT-tenant and answers 403). The SAME live config/PAT,
    //    only the tenant slug is wrong. ────────────────────────────────────
    let real_tenant = std::env::var(ENV_TENANT).expect("tenant present (gate passed)");
    let wrong_tenant = format!("{real_tenant}-not-mine-{}", &random_hex64()[..12]);
    let cross = cross_tenant_client(&wrong_tenant);
    match cross.lookup(&unused.memo_key) {
        Err(AcError::Status(403)) => { /* cross-tenant denied — correct */ }
        Err(AcError::Status(other)) => {
            panic!("probe (3): cross-tenant probe must be 403, got HTTP {other}")
        }
        Ok(_) => panic!("probe (3): cross-tenant probe must be DENIED (403), it was served"),
        Err(e) => panic!("probe (3): cross-tenant probe expected 403, got error {e}"),
    }
}

/// Build a live-transport client that reuses the real base URL + PAT but targets
/// a DIFFERENT tenant slug — the cross-tenant denial probe (§6.3). The PAT is
/// read through the same loader path so its value never appears here; we extract
/// it via the loader by reading the env/file the gate already proved present.
fn cross_tenant_client(wrong_tenant: &str) -> HttpAcClient<UreqTransport> {
    let base = std::env::var(ENV_AC_URL).expect("base URL present (gate passed)");
    let pat = read_pat_for_cross_tenant();
    let cfg = AcConfig::new(base.clone(), wrong_tenant.to_string(), pat)
        .expect("cross-tenant config (non-empty pieces)");
    HttpAcClient::with_transport(base, Some(cfg), UreqTransport)
}

/// Read the PAT for the cross-tenant probe using the SAME file-preferred,
/// env-fallback contract as the loader. The value is consumed straight into
/// `AcConfig` (which redacts it) — never printed. Only reached after the gate
/// proved a readable PAT exists.
fn read_pat_for_cross_tenant() -> String {
    // Mirror the loader's resolution: HUGIT_CORELINK_PAT_FILE override, else
    // ~/.hugit/secrets/corelink/pat, else HUGIT_CORELINK_PAT env.
    let path = std::env::var("HUGIT_CORELINK_PAT_FILE")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::Path::new(&h).join(".hugit/secrets/corelink/pat"))
        });
    if let Some(p) = path
        && let Ok(contents) = std::fs::read_to_string(&p)
    {
        let pat = contents.trim_end().to_string();
        if !pat.is_empty() {
            return pat;
        }
    }
    std::env::var("HUGIT_CORELINK_PAT").expect("PAT present (gate passed)")
}
