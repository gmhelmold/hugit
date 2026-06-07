//! The **self-hosted-alpha flag gate** for the write path (WP-D3b item ④).
//!
//! The push / write path is **OFF by default**. It is enabled only under the
//! `self-hosted-alpha` flag (warp §D3: "push path v0 … **self-hosted flag
//! only**"). With the flag off the write path is *absent*: it never accepts a
//! push, it refuses fail-closed.
//!
//! The gate is intentionally tiny and side-effect-free: it answers exactly one
//! question — "is the write path enabled for this configuration?" — and the
//! write entrypoints route every mutation through it. There is no second way in.

/// The deployment flag set that governs whether the write path is reachable.
///
/// Off by default ([`FlagGate::default`] ⇒ write path refused). The only enabler
/// is the `self-hosted-alpha` flag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlagGate {
    /// Whether the `self-hosted-alpha` flag is set for this deployment.
    self_hosted_alpha: bool,
}

/// The write path is gated off and refused the requested mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePathDisabled;

impl std::fmt::Display for WritePathDisabled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "write path disabled: enabled only under the self-hosted-alpha flag"
        )
    }
}

impl std::error::Error for WritePathDisabled {}

impl FlagGate {
    /// A gate with every flag off — the production default (write path refused).
    pub fn new() -> Self {
        Self::default()
    }

    /// A gate with the `self-hosted-alpha` flag enabled (write path reachable).
    pub fn self_hosted_alpha() -> Self {
        Self {
            self_hosted_alpha: true,
        }
    }

    /// Whether the write path is enabled for this configuration.
    ///
    /// True **iff** the `self-hosted-alpha` flag is set. This is the single
    /// predicate the write entrypoints consult.
    pub fn write_path_enabled(&self) -> bool {
        self.self_hosted_alpha
    }

    /// Fail-closed admission check: `Ok(())` only when the write path is enabled,
    /// otherwise [`WritePathDisabled`]. Every write entrypoint calls this first.
    pub fn admit_write(&self) -> Result<(), WritePathDisabled> {
        if self.write_path_enabled() {
            Ok(())
        } else {
            Err(WritePathDisabled)
        }
    }
}
