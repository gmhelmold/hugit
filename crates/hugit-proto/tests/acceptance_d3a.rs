// WP-D3a acceptance oracle — receive-pack → CAS + log (push core).
// Owned item (verbatim from decomposition v2.0 — D3a):
//   ① push→clone round-trip identical
//
// Also includes red-team write-path fixtures required by D3a's source-of-truth
// bar (WP-D3a "Implementation notes — Red-team").
//
// Each #[test] is a STUB that panics. Implementation (WP-D3a) must make them
// pass by building the receive-pack ingest+store under
//   crates/hugit-proto/src/write/{receive,store}/

#[test]
fn item_1_push_clone_roundtrip_identical() {
    // D3a ① — push a commit pack to the hugit receive-pack endpoint, then
    // clone back via the D2a read path; assert every pushed object is present
    // in the clone with byte-identical SHAs, and the ref points to the pushed tip.
    // (Implementation: use a local fixture repo; drive receive-pack write path;
    // verify D1 event log contains the ref-update event; clone back and compare.)
    panic!("RED — item_1_push_clone_roundtrip_identical not implemented");
}

// ── red-team fixtures (source-of-truth bar, D3a scope) ───────────────────────

#[test]
fn redteam_malformed_pack_rejected() {
    // Feed a syntactically invalid packfile to receive-pack; assert the server
    // returns an error and does NOT write any objects to CAS or append any event
    // to the D1 log.
    panic!("RED — redteam_malformed_pack_rejected not implemented");
}

#[test]
fn redteam_oversized_pack_rejected() {
    // Feed a pack that exceeds the configured size ceiling; assert rejection
    // with no partial CAS writes and no D1 event appended.
    panic!("RED — redteam_oversized_pack_rejected not implemented");
}

#[test]
fn redteam_ref_update_tamper_rejected() {
    // Attempt to force-push to a ref without the force flag / with a
    // non-fast-forward update that violates policy; assert the ref update is
    // rejected and the D1 log is unchanged.
    panic!("RED — redteam_ref_update_tamper_rejected not implemented");
}
