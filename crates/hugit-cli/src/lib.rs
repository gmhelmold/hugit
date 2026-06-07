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

/// The canonical set of top-level `hugit` CLI verbs, in catalog order.
///
/// This is the SINGLE source of truth for hugit's verb surface. The real
/// `hugit` binary ([`main`](../bin/hugit)) dispatches exactly these tokens, and
/// the namespace-law invariant (hugit-invariants WP-X5 item ①) consumes THIS
/// constant — not a hand-copied list — so any verb added to the CLI is
/// automatically checked against `git help -a` for shadowing. Keeping the bin
/// and the invariant on one list makes "the oracle tests the real surface"
/// structurally true: the list cannot rot relative to the binary.
///
/// Sub-subcommands (`ws spawn`, `ctx snap`, …) are NOT verbs; only the
/// top-level token is namespace-law-relevant.
pub const HUGIT_VERBS: &[&str] = &[
    // Phase B — Orchestrator / Worker (the GitHub-App-riding commands)
    "land",    // hugit land [--queue]        — union-testing landing queue
    "verdict", // hugit verdict request …     — adversarial reviewer panels
    "check",   // hugit check [--local]       — memoized CI check
    "diag",    // hugit diag <failure>        — structured diagnosis
    // Phase C/D — Workspace + context
    "ws",     // hugit ws spawn/attach/snap/gc — claim-fenced workspaces
    "ctx",    // hugit ctx snap / resume      — short-horizon session resume
    "impact", // hugit impact <path|change>   — build-graph blast radius
    // Phase D — The forge verbs
    "ledger",     // hugit ledger [--live]    — default history view
    "review",     // hugit review <intent>    — grounded-evidence answers
    "approve",    // hugit approve            — policy-gated approval
    "reject",     // hugit reject             — policy-gated rejection
    "watch",      // hugit watch              — TUI forge monitoring
    "why",        // hugit why <line|symbol>  — provenance query
    "undo",       // hugit undo <op>          — event-sourced undo
    "policy",     // hugit policy edit / test — declarative gate management
    "campaign",   // hugit campaign / plan    — DAG + acceptance binding
    "dispatch",   // hugit dispatch <intent>  — workspace + context packet
    "fleet",      // hugit fleet              — machine-readable fleet state
    "tournament", // hugit tournament -n N    — exploration as a verb
    "intent",     // hugit intent seal        — the one ceremony verb
    "journal",    // hugit journal note       — session note
    "export",     // hugit export             — anti-lock-in dump + exit proof
];

/// The canonical verb registry as an owned, sorted-stable accessor.
///
/// Returns [`HUGIT_VERBS`] verbatim. Provided as a function so downstream
/// consumers (e.g. WP-X5) can depend on a stable call shape even if the backing
/// representation changes.
pub fn hugit_verbs() -> &'static [&'static str] {
    HUGIT_VERBS
}
