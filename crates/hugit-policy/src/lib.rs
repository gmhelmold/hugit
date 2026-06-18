//! hugit-policy — declarative gate engine v0 (WP-D6).
//!
//! # Design
//!
//! Gates are **pure functions** over an [`EvalContext`]. The *same* evaluator
//! code path is invoked locally (`hugit policy test`) and at the forge
//! enforcement callsite — one codepath, byte-identical verdicts (①).
//!
//! The engine is **fail-CLOSED**: if the engine is unavailable / its gate set
//! cannot be loaded, [`Engine::eval_closed`] returns [`GateOutcome::Blocked`]
//! and the landing path must refuse (②).
//!
//! Every policy change emits a hash-chained [`EventRecord`] (③).

use hugit_contracts::EventRecord;
use hugit_refstore::authz::{DenyReason, Endpoint, PrincipalClass};
use hugit_refstore::{EventLog, canonical_json};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── public re-exports ───────────────────────────────────────────────────────

pub mod gates;
pub use gates::{changelog, dco, secrets};

// ─── core types ──────────────────────────────────────────────────────────────

/// The input context fed to every gate evaluator.
///
/// A gate is a **pure function** `fn(&EvalContext) -> GateOutcome`; it must
/// not perform I/O.  `EvalContext` carries everything the gate can inspect.
#[derive(Debug, Clone)]
pub struct EvalContext {
    /// Commit messages in the range being evaluated (e.g. BASE..HEAD).
    pub commit_messages: Vec<String>,
    /// Parent counts for each commit, parallel to `commit_messages`.
    ///
    /// A value ≥ 2 means the commit is a real merge commit (equivalent to the
    /// `--no-merges` git filter).  When the slice is shorter than
    /// `commit_messages` the missing entries are treated as 1 (regular commit).
    pub commit_parent_counts: Vec<usize>,
    /// File paths changed in the range.
    pub changed_files: Vec<String>,
    /// Full text content of changed files, keyed by path.
    pub file_contents: HashMap<String, String>,
    /// Arbitrary string metadata (e.g. author, intent_id).
    pub metadata: HashMap<String, String>,
}

impl EvalContext {
    /// Construct a new, empty context.
    pub fn new() -> Self {
        Self {
            commit_messages: Vec::new(),
            commit_parent_counts: Vec::new(),
            changed_files: Vec::new(),
            file_contents: HashMap::new(),
            metadata: HashMap::new(),
        }
    }
}

impl Default for EvalContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of a single gate evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateOutcome {
    /// The gate passed — change may proceed (for this gate).
    Pass,
    /// The gate failed — change must not land.
    Fail { reason: String },
    /// The engine or gate is unavailable; the change is blocked fail-closed.
    Blocked { reason: String },
}

impl GateOutcome {
    /// Returns `true` if the outcome permits landing.
    pub fn is_pass(&self) -> bool {
        matches!(self, GateOutcome::Pass)
    }

    /// Returns `true` if the outcome blocks landing (fail or blocked).
    pub fn blocks_landing(&self) -> bool {
        !self.is_pass()
    }
}

// ─── gate descriptor ─────────────────────────────────────────────────────────

/// A declarative gate descriptor.  The engine uses `id` to look up the
/// registered evaluator function and calls it with the shared `EvalContext`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateDescriptor {
    /// Unique, stable gate identifier (e.g. `"dco"`, `"changelog"`, `"secrets"`).
    pub id: String,
    /// Human-readable description of what this gate enforces.
    pub description: String,
    /// Whether this gate is currently enabled.
    pub enabled: bool,
}

// ─── engine ──────────────────────────────────────────────────────────────────

/// The canonical house gate set (the baseline `Engine::house` enables).
///
/// Exposed so consumers can reconstruct the gate state without reaching into the
/// private `Engine::gates` field — e.g. `hugit policy edit` folds `policy.change`
/// events over THIS baseline to compute the current gate set. `Engine::house`
/// builds its descriptors from this exact list, so the local≡forge gate set and
/// the edit baseline cannot drift.
pub fn house_gates() -> Vec<GateDescriptor> {
    vec![
        GateDescriptor {
            id: "dco".into(),
            description: "Every non-merge commit must carry a Signed-off-by: trailer (DCO).".into(),
            enabled: true,
        },
        GateDescriptor {
            id: "changelog".into(),
            description:
                "feat/fix commits require a non-empty ## [Unreleased] section in CHANGELOG.md."
                    .into(),
            enabled: true,
        },
        GateDescriptor {
            id: "secrets".into(),
            description:
                "No commit may introduce obvious secret patterns (API keys, tokens, passwords)."
                    .into(),
            enabled: true,
        },
    ]
}

/// Type alias for a gate evaluator function.
pub type GateFn = fn(&EvalContext) -> GateOutcome;

/// The declarative policy engine.
///
/// Evaluates a set of [`GateDescriptor`]s against an [`EvalContext`] by
/// dispatching to registered [`GateFn`]s.  The same `Engine` instance (or an
/// identically configured one) is used both locally and at the forge —
/// local≡forge is a structural property, not a runtime check.
pub struct Engine {
    gates: Vec<GateDescriptor>,
    registry: HashMap<String, GateFn>,
}

impl Engine {
    /// Build an engine from a gate list and a function registry.
    pub fn new(gates: Vec<GateDescriptor>, registry: HashMap<String, GateFn>) -> Self {
        Self { gates, registry }
    }

    /// Build the canonical house engine containing the 3 ported gates.
    ///
    /// This is the single factory used by both `hugit policy test` (local) and
    /// the forge enforcement adapter — one codepath, identical registry.
    pub fn house() -> Self {
        let mut registry: HashMap<String, GateFn> = HashMap::new();
        registry.insert("dco".into(), dco::eval);
        registry.insert("changelog".into(), changelog::eval);
        registry.insert("secrets".into(), secrets::eval);

        Self::new(house_gates(), registry)
    }

    /// Evaluate all enabled gates against `ctx`.
    ///
    /// Returns one outcome per enabled gate, in declaration order.
    /// If a gate's evaluator is not registered, returns
    /// [`GateOutcome::Blocked`] (fail-closed).
    pub fn eval(&self, ctx: &EvalContext) -> Vec<(String, GateOutcome)> {
        self.gates
            .iter()
            .filter(|g| g.enabled)
            .map(|g| {
                let outcome = match self.registry.get(&g.id) {
                    Some(f) => f(ctx),
                    None => GateOutcome::Blocked {
                        reason: format!(
                            "gate '{}' has no registered evaluator (fail-closed)",
                            g.id
                        ),
                    },
                };
                (g.id.clone(), outcome)
            })
            .collect()
    }

    /// Fail-closed engine-down variant.
    ///
    /// Simulates engine unavailability: always returns `Blocked` for every
    /// gate.  The landing path must treat this identically to a hard gate
    /// failure — the kill-test (②) verifies this invariant.
    pub fn eval_closed(gate_ids: &[&str]) -> Vec<(String, GateOutcome)> {
        gate_ids
            .iter()
            .map(|id| {
                (
                    (*id).to_string(),
                    GateOutcome::Blocked {
                        reason: "policy engine unavailable (fail-closed)".into(),
                    },
                )
            })
            .collect()
    }

    /// Returns `true` if **all** outcomes permit landing.
    pub fn all_pass(outcomes: &[(String, GateOutcome)]) -> bool {
        outcomes.iter().all(|(_, o)| o.is_pass())
    }

    /// Returns `true` if **any** outcome blocks landing.
    pub fn any_blocks(outcomes: &[(String, GateOutcome)]) -> bool {
        outcomes.iter().any(|(_, o)| o.blocks_landing())
    }
}

// ─── audit event emitter ─────────────────────────────────────────────────────

/// The event kind every policy change appends.
pub const POLICY_CHANGE_KIND: &str = "policy.change";

/// Why an [`emit_policy_change`] did not append a `policy.change` event.
///
/// `policy` is one of the four mutating forge verbs the D14 matrix gates
/// ([`hugit_refstore::authz`]); the matrix declares it **Human-only** (a human
/// stakeholder control). A non-human asserted class — or an attempt to seed the
/// emitter with a malformed prior chain — is refused, fail-closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyEmitError {
    /// The D14 guard denied the policy change: `policy` is Human-only and the
    /// asserted [`PrincipalClass`] was not [`PrincipalClass::Human`]. The
    /// `policy.change` event was **not** appended; an `authz.denied` audit
    /// record (③) was written to the log instead, so the denial is attributable.
    /// Carries the [`DenyReason`] for the caller to map to its own error.
    Denied(DenyReason),
    /// The caller-provided prior records do not form a gap-free, monotonic hash
    /// chain, so the guarded append point cannot extend it (fail-closed). The
    /// log is left untouched.
    BadPriorChain,
}

impl std::fmt::Display for PolicyEmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyEmitError::Denied(reason) => {
                write!(
                    f,
                    "policy change denied: {} (policy is Human-only)",
                    reason.code()
                )
            }
            PolicyEmitError::BadPriorChain => {
                write!(f, "prior policy log is not a gap-free monotonic chain")
            }
        }
    }
}

impl std::error::Error for PolicyEmitError {}

/// Emits an [`EventRecord`] describing a policy change — **through the D14
/// authorization guard** (`policy` is a Human-only forge verb).
///
/// `policy` is one of the four mutating verbs the D14 matrix gates
/// ([`hugit_refstore::authz`]); the matrix declares it **Human-only** (a human
/// stakeholder control — the `policy` row). This emitter therefore routes the
/// append through [`EventLog::append_authorized`] under [`Endpoint::Policy`]
/// with the **caller-asserted** `class`, rather than hand-rolling the
/// hash-chained record (the bypass the adversarial round caught — D14 was
/// gated nowhere on this path). The class→principal binding is the disclosed
/// authentication seam (P2 / ADR-0002); the matrix decision is real and
/// enforced here:
///
/// - `class == `[`PrincipalClass::Human`] → the `policy.change` event is
///   appended and returned.
/// - any non-human `class` → **denied** fail-closed: no `policy.change` is
///   appended, an `authz.denied` audit record (③) is written by the guard, and
///   [`PolicyEmitError::Denied`] carries the [`DenyReason`].
///
/// The event is appended to the caller-provided log (a canonical
/// `Vec<EventRecord>` chain); attribution is carried in `principal` (who made
/// the change). `old_gates_json` / `new_gates_json` are the serialised gate
/// lists before and after the edit, so the delta is fully reconstructable.
///
/// The hash-chain math is the refstore's frozen formula (via
/// [`EventLog::append_authorized`] → [`EventLog::append`]) — byte-identical to
/// what every other guarded verb produces, so a verifier re-canonicalises and
/// re-chains it exactly.
pub fn emit_policy_change(
    log: &mut Vec<EventRecord>,
    class: PrincipalClass,
    principal: &str,
    old_gates_json: &str,
    new_gates_json: &str,
) -> Result<EventRecord, PolicyEmitError> {
    let payload_raw = serde_json::json!({
        "old": old_gates_json,
        "new": new_gates_json,
    })
    .to_string();
    // Payload MUST be canonical JSON before chaining — verifiers re-canonicalise
    // and compare bytes; a non-canonical payload would produce a different hash.
    let payload = canonical_json(&payload_raw).unwrap_or(payload_raw);

    // Rehydrate the caller's prior records into an EventLog so the append goes
    // through the one guarded mutating primitive. A prior chain that is not
    // gap-free/monotonic is refused fail-closed (the guarded append point owns
    // the seq invariant; we never silently re-seq).
    let mut event_log = EventLog::new();
    for record in log.iter() {
        event_log
            .push_record(record.clone())
            .map_err(|_| PolicyEmitError::BadPriorChain)?;
    }

    // Route through the D14 guard under Endpoint::Policy with the caller-asserted
    // class. On Allow the real event is appended; on Deny the guard writes an
    // `authz.denied` audit record (③) into `event_log` instead — which we mirror
    // back so the denial is persisted and attributable, then surface the error.
    match event_log.append_authorized(
        class,
        Endpoint::Policy,
        POLICY_CHANGE_KIND,
        vec![principal.to_string()],
        payload,
        0, // deterministic for tests; callers may overwrite recorded_at
    ) {
        Ok(event) => {
            log.push(event.clone());
            Ok(event)
        }
        Err(denied) => {
            // Persist the guard's audit record (the denial is never silent) and
            // return the structured error.
            log.push((*denied.audit).clone());
            Err(PolicyEmitError::Denied(denied.reason))
        }
    }
}

/// Landing-path enforcement adapter.
///
/// Wraps the engine call for the forge landing path.  If `engine` is `None`
/// (engine down / unavailable), returns blocked outcomes for all known gate
/// ids — fail-CLOSED, never open.
pub fn landing_gate_check(
    engine: Option<&Engine>,
    ctx: &EvalContext,
    known_gate_ids: &[&str],
) -> Vec<(String, GateOutcome)> {
    match engine {
        Some(e) => e.eval(ctx),
        None => Engine::eval_closed(known_gate_ids),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `None` arm in `Engine::eval` (unregistered gate id) returns
    /// `GateOutcome::Blocked` — the fail-closed path is covered.
    #[test]
    fn unregistered_gate_id_is_blocked_fail_closed() {
        let gates = vec![GateDescriptor {
            id: "no_such_gate".into(),
            description: "a gate whose evaluator is deliberately not registered".into(),
            enabled: true,
        }];
        // Empty registry: no GateFn for "no_such_gate".
        let registry = std::collections::HashMap::new();
        let engine = Engine::new(gates, registry);

        let ctx = EvalContext::new();
        let outcomes = engine.eval(&ctx);

        assert_eq!(outcomes.len(), 1, "one enabled gate → one outcome");
        let (id, outcome) = &outcomes[0];
        assert_eq!(id, "no_such_gate");
        assert!(
            matches!(outcome, GateOutcome::Blocked { .. }),
            "unregistered gate must produce Blocked (fail-closed), got {:?}",
            outcome
        );
    }
}
