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
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

        let gates = vec![
            GateDescriptor {
                id: "dco".into(),
                description: "Every non-merge commit must carry a Signed-off-by: trailer (DCO)."
                    .into(),
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
        ];

        Self::new(gates, registry)
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

/// Emits an [`EventRecord`] describing a policy change.
///
/// Every call to [`emit_policy_change`] produces one audited event (③).
/// The event is appended to the caller-provided log; attribution is carried
/// in `principal` (who made the change).
///
/// `old_gate_json` / `new_gate_json` are the serialised gate lists before
/// and after the edit, so the delta is fully reconstructable from the log.
pub fn emit_policy_change(
    log: &mut Vec<EventRecord>,
    principal: &str,
    old_gates_json: &str,
    new_gates_json: &str,
) -> EventRecord {
    let seq = log.len() as u64;
    let prev_hash = log
        .last()
        .map(|e| e.this_hash.clone())
        .unwrap_or_else(|| "0".repeat(64));

    let payload = serde_json::json!({
        "old": old_gates_json,
        "new": new_gates_json,
    })
    .to_string();

    let this_hash = compute_event_hash(&prev_hash, "policy.change", principal, &payload, seq);

    let event = EventRecord {
        seq,
        prev_hash,
        this_hash: this_hash.clone(),
        kind: "policy.change".into(),
        principal_chain: vec![principal.to_string()],
        payload,
        recorded_at: 0, // deterministic for tests; callers may overwrite
    };

    log.push(event.clone());
    event
}

/// Compute the event hash per the frozen formula:
/// `H(prev_hash ‖ kind ‖ principal_chain ‖ payload ‖ seq)`
/// where each field is length-prefixed (4-byte big-endian u32), seq is 8-byte big-endian u64.
fn compute_event_hash(
    prev_hash: &str,
    kind: &str,
    principal: &str,
    payload: &str,
    seq: u64,
) -> String {
    let mut hasher = Sha256::new();

    // Each string field: 4-byte BE u32 length prefix + UTF-8 bytes
    for field in &[prev_hash, kind, principal, payload] {
        let bytes = field.as_bytes();
        let len = bytes.len() as u32;
        hasher.update(len.to_be_bytes());
        hasher.update(bytes);
    }
    // seq: 8-byte BE u64
    hasher.update(seq.to_be_bytes());

    hex::encode(hasher.finalize())
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
