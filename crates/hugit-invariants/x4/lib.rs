//! hugit-invariants — Squad-X supply-chain invariant (WP-X4).
//!
//! This crate proves the supply chain is **pinned and verified end-to-end**
//! without owning any production path. It consumes the runner-spawn surface
//! ([`hugit_runner`]) and the frozen [`RunnerLease`](hugit_contracts::RunnerLease)
//! read-only; it modifies neither.
//!
//! # The three invariants (WP-X4 owned items)
//! 1. **Image content-pinned + integrity-verified at spawn.** A runner image
//!    reference must be pinned by *content digest* (`name@sha256:<64-hex>`),
//!    never a floating tag, and the digest must be integrity-verified against
//!    what the box actually resolves before the container is spawned.
//! 2. **App deps pinned + verified in CI.** The App/workspace dependencies are
//!    pinned (a committed `Cargo.lock`) and verified by the existing CI audit
//!    gate (`cargo audit`); a floating/unpinned dependency is rejected.
//! 3. **Tampered/unpinned image → fail CLOSED before any tenant work.** The
//!    verify-before-spawn ordering is load-bearing: if the image is unpinned or
//!    its digest does not match, the path fails CLOSED and **no tenant byte is
//!    processed** (no container spawned, no job run). A post-hoc detection is a
//!    contract FAIL.
//!
//! Everything here is verification logic over the *consumed* surfaces — there
//! is no production behavior to ship from this crate.

pub mod pin;
