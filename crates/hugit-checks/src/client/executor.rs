//! The local, memoized check executor (`hugit check --local`).
//!
//! This is the client half of the same pure check function the runner uses
//! (whitepaper §6.2). B2a owns the LOCAL/CLIENT path; B2b owns the runner-side
//! byte-identity proof. Here the flow is:
//!
//! ```text
//! key = derive_memo_key(def, tree, toolchain)
//! ac.lookup(key) → HIT  ⇒ return memoized CheckResult, ZERO local executions
//!                → MISS ⇒ execute the check ONCE, store the result, return it
//! ```
//!
//! The "zero local execution on hit" guarantee is the wedge. It is enforced
//! structurally: on a hit the executor returns BEFORE the [`CheckRunner`] is
//! ever invoked, and the runner's own invocation counter proves it in the
//! acceptance suite (a false hit that still executes would be caught).

use hugit_contracts::CheckDef;

use super::ac::{AcError, ActionCache};
use super::memo_key::{self, FileContent};

/// The outcome of a memoized check run, with the provenance the wedge promises.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckOutcome {
    /// The check result (memoized or freshly executed).
    pub result: hugit_contracts::CheckResult,
    /// True iff the result came from the Action Cache (no local execution).
    pub from_cache: bool,
    /// Number of times the underlying check was actually executed locally for
    /// THIS call: `0` on a cache hit, `1` on a miss. Load-bearing for item ①.
    pub local_executions: u32,
}

/// Errors from the memoized executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// The Action Cache layer failed.
    Ac(AcError),
    /// The underlying check execution failed to even run (distinct from a check
    /// that ran and reported a non-zero exit — that is a successful run with a
    /// non-zero `CheckResult::exit`).
    Run(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Ac(e) => write!(f, "{e}"),
            ExecError::Run(e) => write!(f, "check execution failed: {e}"),
        }
    }
}

impl std::error::Error for ExecError {}

impl From<AcError> for ExecError {
    fn from(e: AcError) -> Self {
        ExecError::Ac(e)
    }
}

/// Executes a check locally on a MISS. The implementor produces a fully-formed
/// [`CheckResult`] for the given key/def/toolchain. In production this spawns the
/// same isolated runner the forge uses (byte-identity is B2b's concern); the
/// trait keeps the executor logic testable without a real process.
pub trait CheckRunner {
    /// Run the check ONCE and return its result. `memo_key` is precomputed and
    /// MUST be stamped onto the returned [`CheckResult::memo_key`] so the stored
    /// record is self-keyed. `tree_root`/`def_digest`/`toolchain_digest` are the
    /// three axes (also stamped onto the result).
    fn run(
        &self,
        def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<hugit_contracts::CheckResult, ExecError>;
}

/// Run a check with memoization: derive the three-axis key, look it up, and only
/// execute (then store) on a MISS.
///
/// Returns a [`CheckOutcome`] carrying `local_executions` (0 on hit, 1 on miss)
/// so callers — and the acceptance suite — can prove the zero-execution wedge.
pub fn run_memoized<'a, A, R, I>(
    ac: &A,
    runner: &R,
    def: &CheckDef,
    files: I,
    toolchain_digest: &str,
) -> Result<CheckOutcome, ExecError>
where
    A: ActionCache,
    R: CheckRunner,
    I: IntoIterator<Item = (&'a str, &'a FileContent)> + Clone,
{
    // Derive the three axes once (the same files iterator scopes the tree).
    let tree_root = memo_key::scoped_tree_root(&def.glob_set, files.clone());
    let def_digest = memo_key::compute_def_digest(def);
    let key = hugit_refstore::compute_memo_key(&tree_root, &def_digest, toolchain_digest);

    // HIT: return the memoized result with ZERO local executions. We return
    // BEFORE the runner is ever touched — the wedge is structural, not advisory.
    if let Some(result) = ac.lookup(&key)? {
        return Ok(CheckOutcome {
            result,
            from_cache: true,
            local_executions: 0,
        });
    }

    // MISS: execute exactly once, then store under the same key.
    let result = runner.run(def, &key, &tree_root, &def_digest, toolchain_digest)?;
    ac.store(&result)?;

    Ok(CheckOutcome {
        result,
        from_cache: false,
        local_executions: 1,
    })
}
