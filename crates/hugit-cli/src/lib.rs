//! hugit-cli — command implementations.
//!
//! Module layout (body lands per owning WP):
//!   - `verdict/`    — adversarial verdict panels (WP-D7)
//!   - `why/`        — `hugit why` provenance resolver (WP-D10)
//!   - `impact/`     — `hugit impact` blast-radius query + ground-truth export (WP-D10)
//!   - `attention/`  — attention queue: composite ranking, fast-approve gating,
//!     honest degradation (WP-D9). Lives at the crate root per its WP-D9
//!     Claims, wired in via `#[path]`.
//!   - `tournament/` — `hugit tournament -n N` fan-out: N candidates, judge
//!     panel, budget-bounded (WP-D13)
//!   - `export/`     — `hugit export` + the exit proof (WP-E5)

pub mod export;
pub mod impact;
pub mod tournament;
pub mod verdict;
pub mod why;

#[path = "../attention/mod.rs"]
pub mod attention;

// ---------------------------------------------------------------------------
// The canonical command registry (WP-R-cli defect #1)
// ---------------------------------------------------------------------------

/// The LIVE dispatched top-level `hugit` CLI verbs — the actual binary surface.
///
/// **This constant contains ONLY verbs that `main.rs` actually wires and
/// dispatches.** It is the single source of truth for the *live* CLI surface:
///
/// - `main.rs` MUST dispatch every verb listed here (no phantom entries).
/// - The namespace-law invariant (hugit-invariants WP-X5 item ①) consumes THIS
///   constant to check that no live hugit verb shadows a `git` verb.  Testing
///   phantom (unwired) verbs here produces false positives in X5 and a false
///   sense of coverage.
/// - The cli oracle (`acceptance_rcli` item ⑥) asserts EQUALITY between this
///   list and the binary's real `Subcommand` enum, so the registry cannot drift
///   from the binary in either direction.
///
/// Sub-subcommands (`ws spawn`, `ctx snap`, …) are NOT verbs; only the
/// top-level token is namespace-law-relevant.
///
/// For planned-but-not-yet-dispatched verbs see [`HUGIT_RESERVED_VERBS`].
pub const HUGIT_VERBS: &[&str] = &[
    "why",        // hugit why <line|symbol>  — provenance query
    "impact",     // hugit impact <path|change>   — build-graph blast radius
    "tournament", // hugit tournament -n N    — exploration as a verb
    "export",     // hugit export             — anti-lock-in dump + exit proof
];

/// Planned hugit verb tokens that are RESERVED but NOT yet dispatched.
///
/// These verbs are on the product roadmap and are reserved so that they cannot
/// be accidentally taken by `git` (or another tool) before hugit claims them.
/// They are **not** part of the live binary surface: they do not appear in
/// `main.rs` dispatch, `hugit --help`, or the WP-X5 no-shadow oracle.
///
/// When a verb graduates to a live dispatch, move it from here to
/// [`HUGIT_VERBS`] and wire it in `main.rs`.
pub const HUGIT_RESERVED_VERBS: &[&str] = &[
    // Phase B — Orchestrator / Worker (planned)
    "land",    // hugit land [--queue]        — union-testing landing queue
    "verdict", // hugit verdict request …     — adversarial reviewer panels
    "check",   // hugit check [--local]       — memoized CI check
    "diag",    // hugit diag <failure>        — structured diagnosis
    // Phase C — Workspace + context (planned)
    "ws",  // hugit ws spawn/attach/snap/gc — claim-fenced workspaces
    "ctx", // hugit ctx snap / resume      — short-horizon session resume
    // Phase D — The forge verbs (planned)
    "ledger",   // hugit ledger [--live]    — default history view
    "review",   // hugit review <intent>    — grounded-evidence answers
    "approve",  // hugit approve            — policy-gated approval
    "reject",   // hugit reject             — policy-gated rejection
    "watch",    // hugit watch              — TUI forge monitoring
    "undo",     // hugit undo <op>          — event-sourced undo
    "policy",   // hugit policy edit / test — declarative gate management
    "campaign", // hugit campaign / plan    — DAG + acceptance binding
    "dispatch", // hugit dispatch <intent>  — workspace + context packet
    "fleet",    // hugit fleet              — machine-readable fleet state
    "intent",   // hugit intent seal        — the one ceremony verb
    "journal",  // hugit journal note       — session note
];

/// The live verb registry as a stable accessor for downstream consumers.
///
/// Returns [`HUGIT_VERBS`] verbatim. Provided as a function so downstream
/// consumers (e.g. WP-X5) can depend on a stable call shape even if the backing
/// representation changes.
pub fn hugit_verbs() -> &'static [&'static str] {
    HUGIT_VERBS
}
