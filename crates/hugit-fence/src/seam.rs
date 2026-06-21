//! The wire seam to the runner product (WP-R4, runner-transfer campaign).
//!
//! The execution core (lease lifecycle · isolation · teardown · fence
//! materialization/enforcement) TRANSFERRED to `corelink-runners`
//! on 2026-06-10. The broker — which stays repo-side, since it
//! alone holds secrets the forge owns — no longer links the runner crate; it
//! drives **this minimal seam** instead:
//!
//! - [`BoxExec`] — "run a command where the job container lives and capture
//!   its output". Transcribed signature-for-signature from the transferred
//!   runner's `lease::BoxExec` (hugit-runner @ the WP-R4 transfer commit;
//!   now `corelink-runner`), narrowed to the one method the broker calls.
//! - [`CmdOutput`] — the captured result, shape-identical to the runner's.
//! - [`RunningContainer`] — an opaque handle naming the per-job container
//!   the broker delivers results into.
//!
//! **The live implementation is the runner product across the wire**
//! (disclosed seam): `corelink-runner`'s `SshBox` / `DockerEngine` are the
//! production transports, reached over the frozen wire contract
//! (`conformance/` vectors, byte-identical in both repos). hugit carries no
//! production transport; the hermetic test/red-team impls (`SpyBox` in the
//! broker unit tests, the C5b suite's fakes) drive this seam in the bare
//! gate, and the box-gated acceptance lane carries its own disclosed test
//! transport. Equivalence is held by the wire contract, not by a shared
//! crate — no git dependency in either direction (campaign iron rule).

use anyhow::Result;

/// A seam over "run a command on the runner box and capture its output".
///
/// Narrowed to exactly what the broker needs: a single `run`. (The
/// transferred runner's trait also carries `run_with_stdin` for untrusted
/// payload streaming; the broker never streams untrusted bytes — its only
/// remote writes are shell-quoted public outputs — so the seam omits it.)
pub trait BoxExec {
    /// Run `argv` where the job container lives, returning the captured
    /// output. A `None` exit code means the command was killed by a signal.
    ///
    /// # Errors
    /// Fails if the transport cannot reach the box / spawn the process.
    fn run(&self, argv: &[&str]) -> Result<CmdOutput>;
}

/// Captured result of a command run on the box. Shape-identical to the
/// transferred runner's `CmdOutput` (wire-equivalent by transcription).
#[derive(Debug, Clone)]
pub struct CmdOutput {
    /// Exit status; `None` if the process was killed by a signal.
    pub code: Option<i32>,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl CmdOutput {
    /// `true` iff the command exited 0.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// An opaque handle to a running per-job container, named by the runner
/// product that spawned it. The broker only ever uses the name (to address
/// `docker exec`); container provenance — that this name is the one the
/// lease provisioned — is the runner product's responsibility (the
/// documented lease ↔ container trust boundary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningContainer {
    /// The container name (engine-level identifier).
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_output_ok_iff_exit_zero() {
        let ok = CmdOutput {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(ok.ok());
        let fail = CmdOutput {
            code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(!fail.ok());
        let killed = CmdOutput {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(!killed.ok(), "signal-killed is never ok");
    }
}
