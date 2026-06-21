//! Live R2 compare-and-swap round-trip proof — `#[ignore]` (manual, never in CI).
//!
//! This is the AUTHORITATIVE check that the engine's conditional-PUT CAS actually
//! works against real Cloudflare R2: the docs say `If-Match` → 412 on mismatch, but
//! the only proof that our UNSIGNED `If-Match` header is honored (we leave it out of
//! the SigV4 `SignedHeaders`) is a live round-trip. It exercises the EXACT production
//! path — `R2Config::fetch` (capture the ETag) → `put_conditional` (the conditional
//! signed PUT) — and is fully NON-DESTRUCTIVE: the only successful write re-PUTs the
//! object's own bytes (a single-PUT ETag is the MD5 of the body, so identical bytes
//! keep the same ETag — the object is unchanged).
//!
//! Run it manually with a WRITE-scoped R2 credential sourced out-of-band, e.g.:
//!
//! ```sh
//! set -a; . ./your-r2-credentials.env; set +a
//! export HUGIT_SERVE_R2_BUCKET=example-bucket
//! export HUGIT_SERVE_R2_TENANT_ID=00000000-0000-4000-8000-000000000001
//! cargo test -p hugit-serve --test r2_cas_live -- --ignored --nocapture
//! ```
//!
//! Absent the R2 env it SKIPS (prints why), so an accidental run is a no-op.

use hugit_serve::state::R2Config;
use hugit_serve::writes::CasToken;

const REPO: &str = "hugit";

#[test]
#[ignore = "live R2 round-trip; needs a write-scoped R2 credential in env"]
fn live_r2_if_match_roundtrip() {
    let cfg = match R2Config::from_env() {
        Ok(c) => c,
        Err(why) => {
            eprintln!("SKIP live_r2_if_match_roundtrip — R2 env not set: {why}");
            return;
        }
    };

    // 1) Read the current head + its ETag (the CAS token).
    let (bytes, _label, token) = cfg
        .fetch(REPO)
        .expect("R2 GET must succeed")
        .expect("the object must exist (seed the snapshot first)");
    let etag = match &token {
        CasToken::Version(e) => e.clone(),
        other => panic!("expected a versioned ETag from R2, got {other:?}"),
    };
    eprintln!("live: head ETag = {etag}  ({} bytes)", bytes.len());

    // 2) A STALE If-Match must be rejected with 412 → our cas_conflict signal.
    let stale = CasToken::Version("\"00000000000000000000000000000000\"".to_string());
    let err = cfg
        .put_conditional(REPO, &bytes, &stale)
        .expect_err("a stale If-Match MUST be rejected (412), not accepted");
    assert!(
        err.is_cas_conflict(),
        "stale If-Match must map to cas_conflict, got: {} / {}",
        err.status,
        err.code
    );
    eprintln!("live: stale If-Match correctly rejected ({})", err.code);

    // 3) The CORRECT If-Match must succeed — re-PUT identical bytes (non-destructive;
    //    single-PUT ETag = MD5(body), so the object's ETag is unchanged).
    cfg.put_conditional(REPO, &bytes, &token)
        .expect("the correct If-Match MUST be accepted (the CAS round-trips)");
    eprintln!("live: correct If-Match accepted — CAS round-trip proven");

    // 4) Re-reading must yield the SAME ETag (identical content) — proof we did not
    //    mutate the object.
    let (_b2, _l2, token2) = cfg.fetch(REPO).unwrap().unwrap();
    assert_eq!(token, token2, "non-destructive: the ETag must be unchanged");
    eprintln!("live: ETag stable after the re-PUT — object unchanged ✅");
}
