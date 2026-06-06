// WP-D2a acceptance oracle — pack assembly + clone/fetch core.
// Owned items (verbatim from decomposition v2.0 — D2a):
//   ① clone byte-identical to mirror
//   ② delta-only fetch
//
// Each #[test] is a STUB that panics. Implementation (WP-D2a) must make them
// pass by building the real read-path logic under
//   crates/hugit-proto/src/read/{negotiate,pack,serve}/

#[test]
fn item_1_clone_byte_identical() {
    // D2a ① — serve a clone of a local hugit repo and verify the received
    // packfile produces byte-identical object SHAs to the source.
    // (Implementation: drive the smart-HTTP read path against a test fixture
    // backed by the D1 derived-view + CAS stub; compare sha1 of every object.)
    panic!("RED — item_1_clone_byte_identical not implemented");
}

#[test]
fn item_2_delta_only_fetch() {
    // D2a ② — after a clone, push one new commit to the source fixture, then
    // fetch; assert that only the new object(s) are transferred (want/have
    // negotiation excludes objects the client already has).
    // (Implementation: inspect the pack exchanged in the fetch response; assert
    // its object count equals the delta — not the full object set.)
    panic!("RED — item_2_delta_only_fetch not implemented");
}
