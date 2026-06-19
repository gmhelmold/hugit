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
//!   - `porcelain`   — THE one error/exit law + shared JSON-on-stdout conventions
//!     for every verb (flow + legacy) (WP-PC0 scaffold; WP-WB0 one-law convergence)
//!   - `campaign/`   — `hugit campaign open/close/show` (WP-PC1; scaffold WP-PC0)
//!   - `intent/`     — `hugit intent new/show` (WP-PC2; scaffold WP-PC0)
//!   - `pr/`         — `hugit pr open/land/show` (WP-PC3; scaffold WP-PC0)
//!   - `checks/`     — `hugit checks show/key` (WP-WB2; honest stub WP-WB0)
//!   - `queue/`      — `hugit queue show` (WP-WB2; honest stub WP-WB0)

pub mod campaign;
pub mod checks;
pub mod diag;
pub mod export;
pub mod ident;
pub mod impact;
pub mod intent;
pub mod issue;
pub mod journal;
pub mod meta;
pub mod policy;
pub mod porcelain;
pub mod pr;
pub mod queue;
pub mod redaction;
pub mod tournament;
pub mod undo;
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
    // Flow porcelain (WP-PC wave) — dispatched as honest NOT-IMPLEMENTED stubs
    // at PC0; PC1/PC2/PC3 fill the bodies. The verb is LIVE the moment main.rs
    // routes it (the no-drift oracle asserts dispatched == registry), so these
    // belong here, not in HUGIT_RESERVED_VERBS.
    "campaign", // hugit campaign open/close/show — campaign lifecycle (PC1)
    "intent",   // hugit intent new/show         — intent ceremony (PC2)
    "pr",       // hugit pr open/land/show        — pull-request lifecycle (PC3)
    "meta",     // hugit meta set                 — repo.meta producer (owner_tenant seam).
    //            Named `meta`, not `repo`: git 2.54 added a `git repo` builtin and
    //            the WP-X5 namespace law forbids shadowing a git command, so the verb
    //            yields the name to git. The on-wire event kind stays `repo.meta`.
    // Wedge-visibility verbs (SOTA-fix Wave B) — dispatched as honest
    // NOT-IMPLEMENTED stubs at WB0 (clap skeletons); WB2 fills the projection.
    // LIVE the moment main.rs routes them (the no-drift oracle asserts
    // dispatched == registry), so they belong here, not in RESERVED.
    "checks", // hugit checks show/key          — memoized-CI visibility (WB2)
    "queue",  // hugit queue show               — landing-queue visibility (WB2)
    // Wedge EXECUTE verbs (PS-1 wedge wave) — graduated from RESERVED at W0.
    // `check` runs a memoized check for real and (with --store) records it;
    // `verdict` records an adversarial verdict. W0 lands the VERBS into the
    // registry + dispatch as honest NOT-IMPLEMENTED stubs (the EXECUTE bodies
    // land per W-CHECK / W-VERDICT); LIVE the moment main.rs routes them, so the
    // no-drift oracle requires them here, not in HUGIT_RESERVED_VERBS.
    "check",   // hugit check --def --log [--store] — memoized CI check (W-CHECK)
    "verdict", // hugit verdict …                   — adversarial verdict (W-VERDICT)
    // Issue lifecycle (roadmap W2) — graduated from RESERVED. `issue transition`
    // appends an `issue.transition` record (CLI parity with the serve verb,
    // ADR-0006). LIVE the moment main.rs routes it; the no-drift oracle requires
    // it here, not in HUGIT_RESERVED_VERBS.
    "issue", // hugit issue transition --log --n --to [--priority]
    // Stakeholder verbs (roadmap W3) — graduated from RESERVED with REAL wiring
    // (not stubs). `undo` appends a compensating event through the D14 Human-only
    // guard (hugit_refstore::undo); `policy test` runs the house gate set
    // (hugit_policy::Engine::house) — the SAME evaluator the forge landing path
    // uses. LIVE the moment main.rs routes them; the no-drift oracle requires
    // them here, not in HUGIT_RESERVED_VERBS.
    "undo",   // hugit undo --log --seq [--actor]   — event-sourced compensating undo
    "policy", // hugit policy test --context        — local≡forge gate preview
    // Stakeholder + session verbs (roadmap W3) — graduated from RESERVED, REAL.
    // `approve`/`reject` record a single-lens `verdict.recorded` via the SAME
    // path `hugit verdict` uses (parity with the serve POST /prs/{n}/verdict);
    // `journal note` appends a `journal.note` record onto the canonical log. LIVE
    // the moment main.rs routes them; the no-drift oracle requires them here.
    "approve", // hugit approve --intent --log       — record a single-lens approve
    "reject",  // hugit reject --intent --log        — record a single-lens reject
    "journal", // hugit journal note --log --note     — append a session note
    // Diagnosis verb (roadmap W) — graduated from RESERVED, REAL. `diag` drives
    // the hugit-diag bisect engine over a log-backed CheckOracle projected from
    // `check.recorded` events — read-only structured diagnosis. LIVE the moment
    // main.rs routes it; the no-drift oracle requires it here.
    "diag", // hugit diag --log --def-digest [--toolchain] — bisect a red check history
];

/// Planned hugit verb tokens that are RESERVED but NOT yet dispatched.
///
/// These verbs are on the product roadmap and are reserved so that they cannot
/// be accidentally taken by `git` (or another tool) before hugit claims them.
/// They are **not** part of the live binary surface: they do not appear in
/// `main.rs` dispatch, `hugit --help`, or the WP-X5 no-shadow oracle.
///
/// When a verb graduates to a live dispatch, move it from here to
/// [`HUGIT_VERBS`] and wire it in `main.rs`. (`campaign`, `intent`, and `pr`
/// graduated in the PC wave — dispatched as honest stubs at PC0.)
pub const HUGIT_RESERVED_VERBS: &[&str] = &[
    // Phase B — Orchestrator / Worker (planned)
    "land", // hugit land [--queue]        — union-testing landing queue
    // (`check` + `verdict` graduated to HUGIT_VERBS at W0 — wedge EXECUTE wave;
    // `diag` graduated at the diagnosis wave — log-backed bisect.)
    // Phase C — Workspace + context (planned)
    "ws",  // hugit ws spawn/attach/snap/gc — claim-fenced workspaces
    "ctx", // hugit ctx snap / resume      — short-horizon session resume
    // Phase D — The forge verbs (planned)
    "ledger", // hugit ledger [--live]    — default history view
    "review", // hugit review <intent>    — grounded-evidence answers
    "watch",  // hugit watch              — TUI forge monitoring
    // (`undo`, `policy`, `approve`, `reject`, `journal` graduated to HUGIT_VERBS
    // at W3 — stakeholder + session verbs, REAL-wired. `policy` ships `test` only;
    // `policy edit` stays deferred.)
    "dispatch", // hugit dispatch <intent>  — workspace + context packet
    "fleet",    // hugit fleet              — machine-readable fleet state
];

/// The live verb registry as a stable accessor for downstream consumers.
///
/// Returns [`HUGIT_VERBS`] verbatim. Provided as a function so downstream
/// consumers (e.g. WP-X5) can depend on a stable call shape even if the backing
/// representation changes.
pub fn hugit_verbs() -> &'static [&'static str] {
    HUGIT_VERBS
}
