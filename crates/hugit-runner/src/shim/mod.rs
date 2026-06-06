//! Actions-YAML compatibility shim v0 (WP-E4).
//!
//! A migration-lubricant shim that runs a **published supported subset** of
//! GitHub Actions workflow YAML on hugit's runners. Design invariants:
//!
//! 1. **Supported subset is a published contract** — every listed feature is
//!    proven-to-execute by a passing fixture, not merely documented
//!    (`docs/shim/supported-subset.md`).
//! 2. **Out-of-contract → explicit actionable report** — any construct outside
//!    the supported subset produces an [`OutOfContractReport`] naming the
//!    unsupported construct; zero silent skips.
//! 3. **Secrets via broker only** — raw secret material never enters the
//!    runner environment or logs. Missing/denied secrets fail CLOSED, naming
//!    the secret.
//! 4. **Execution equivalence** — a deterministic fixture workflow
//!    (determinism precondition: pinned toolchain/inputs, no wall-clock or net
//!    nondeterminism) produces equivalent observable outcomes (steps, env,
//!    artifacts, exit states) on real GitHub Actions and on the shim.
//!
//! **Not in scope here:** concurrency/throughput, expiry hard-kill, crash
//! recovery, fence ENOENT enforcement. Those are C2b and C5a respectively.

pub mod broker;
pub mod executor;
pub mod parser;
pub mod report;
pub mod subset;

pub use broker::{Broker, BrokerError, SecretResolution};
pub use executor::{EquivalenceOutcome, ExecutionResult, ShimExecutor, StepOutcome};
pub use parser::{ParsedWorkflow, Step, WorkflowParseError, parse_workflow};
pub use report::{OutOfContractReport, ShimDiagnostic, UnsupportedConstruct};
pub use subset::{SUPPORTED_SUBSET, SubsetFeature, is_supported};
