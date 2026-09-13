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
//!   - `campaign/`   — `hugit campaign open/close/show/list/abandon` (WP-PC1)
//!   - `intent/`     — `hugit intent new/show/list` (WP-PC2)
//!   - `pr/`         — `hugit pr open/queue/land/show/list/abandon` (WP-PC3)
//!   - `checks/`     — `hugit check run/show/key` (WP-WB2 + W-CHECK)
//!   - `queue/`      — `hugit queue show` (WP-WB2)
//!   - `verdict/`    — `hugit verdict record/approve/reject` (WP-D7 + W-VERDICT)
//!   - `note/`       — `hugit note` (session note; was `journal note`)
//!   - `init/`       — `hugit init` (logic ready; pending an X5 namespace amendment)
//!   - `log_resolve` — the ONE shared default-`--log` resolver

pub mod campaign;
pub mod capture;
pub mod checks;
pub mod ctx;
pub mod diag;
pub mod dock;
pub mod export;
pub mod fleet;
pub mod health;
pub mod ident;
pub mod impact;
pub mod init;
pub mod intent;
pub mod issue;
pub mod journal;
pub mod land;
pub mod ledger;
pub mod log_resolve;
pub mod meta;
pub mod note;
pub mod policy;
pub mod porcelain;
pub mod pr;
pub mod projection;
pub mod queue;
pub mod redaction;
pub mod review;
pub mod runtime_store;
pub mod setup;
pub mod symbol;
pub mod tournament;
pub mod undo;
pub mod verdict;
pub mod watch;
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
    "setup",
    "capture",
    "attach",
    "detach",
    "health",
    "campaign", // hugit campaign open/close/show — campaign lifecycle (PC1)
    "intent",   // hugit intent new/show         — intent ceremony (PC2)
    "pr",       // hugit pr open/land/show        — pull-request lifecycle (PC3)
    "meta",     // hugit meta set                 — repo.meta producer (owner_tenant seam).
    //            Named `meta`, not `repo`: git 2.54 added a `git repo` builtin and
    //            the WP-X5 namespace law forbids shadowing a git command, so the verb
    //            yields the name to git. The on-wire event kind stays `repo.meta`.
    // Landing-queue visibility (SOTA-fix Wave B) — dispatched read verb.
    "queue", // hugit queue show               — landing-queue visibility (WB2)
    // Wedge verbs (PS-1 wedge wave). `check` is ONE verb with three subcommands
    // (git-proximate cleanup — the old split between a top-level `check` EXECUTE
    // and a plural `checks show|key` READ collapsed): `check run` executes a
    // memoized check (and with --store records it), `check show` projects the
    // hit-rate, `check key` predicts the memo key. `verdict` likewise gathers the
    // multi-lens panel (`verdict record`) AND the single-lens stakeholder
    // decisions (`verdict approve` / `verdict reject`) — the old top-level
    // `approve`/`reject` verbs moved UNDER it. LIVE the moment main.rs routes
    // them, so the no-drift oracle requires them here.
    "check",   // hugit check run|show|key          — memoized CI check (W-CHECK)
    "verdict", // hugit verdict record|approve|reject — adversarial verdict (W-VERDICT)
    // Issue lifecycle (roadmap W2) — graduated from RESERVED. `issue transition`
    // appends an `issue.transition` record (CLI parity with the serve verb —
    // no web-only verb). LIVE the moment main.rs routes it; the no-drift oracle requires
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
    // Session note (roadmap W3) — graduated from RESERVED, REAL. `note` appends a
    // `journal.note` record onto the canonical log (git-proximate cleanup: this
    // was `journal note`; a session note is a single frequent action, so it is a
    // top-level verb — the on-wire `journal.note` kind is unchanged). LIVE the
    // moment main.rs routes it; the no-drift oracle requires it here. (The
    // single-lens approve/reject decisions moved under `verdict` — see above.)
    "note", // hugit note --log --note            — append a session note
    // Diagnosis verb (roadmap W) — graduated from RESERVED, REAL. `diag` drives
    // the hugit-diag bisect engine over a log-backed CheckOracle projected from
    // `check.recorded` events — read-only structured diagnosis. LIVE the moment
    // main.rs routes it; the no-drift oracle requires it here.
    "diag", // hugit diag --log --def-digest [--toolchain] — bisect a red check history
    // Forge read surface (Phase D) — graduated from RESERVED with REAL wiring
    // (not a stub). `ledger` projects the canonical `--log` into the
    // asked→done→proven history via the SAME `hugit_ledger::Ledger` fold
    // `campaign show` / `queue show` read (one projection, surfaces agree by
    // construction). LIVE the moment main.rs routes it; the no-drift oracle
    // requires it here, not in HUGIT_RESERVED_VERBS.
    "ledger", // hugit ledger --log [--campaign]    — default forge history view
    // Forge read surface (Phase D) continued — graduated from RESERVED, REAL.
    // `fleet` projects the canonical `--log` into the versioned
    // `hugit_ledger::FleetState` schema (workspaces + agents); `watch` replays
    // the classified, redacted event stream via `hugit_ledger::WatchDisplay`
    // (the live SSE tail stays the serve surface). Both reuse the engine's own
    // projection — one source of truth. LIVE the moment main.rs routes them; the
    // no-drift oracle requires them here, not in HUGIT_RESERVED_VERBS.
    "fleet", // hugit fleet --log                   — machine-readable fleet state
    "watch", // hugit watch --log [--class]         — replay the forge event stream
    // Semantic index (W6) — graduated with REAL wiring (not a stub). `symbol`
    // outlines a LOCAL source file via the `hugit-symbols` tree-sitter crate (the
    // SAME producer the serve `/v1` blob `outline` uses — one source of truth).
    // Does NOT shadow a git command (X5-checked). LIVE the moment main.rs routes
    // it; the no-drift oracle requires it here, not in HUGIT_RESERVED_VERBS.
    "symbol", // hugit symbol --file <path>          — local symbol outline
    // Short-horizon session resume (D11) — graduated from RESERVED with REAL
    // wiring. `ctx resume` reconstructs a crashed/replaced session from the log's
    // `journal.note` records via `hugit_ledger::journal::ctx_resume_from_journal`,
    // refusing honestly beyond the horizon (never a silent stale reconstruction).
    // `ctx snap` (the P2 JournalStore writer) is deliberately NOT offered yet.
    // Does NOT shadow a git command (X5). LIVE the moment main.rs routes it.
    "ctx", // hugit ctx resume --log --workspace --intent — short-horizon resume
    // Grounded-evidence review Q&A (D7 half) — graduated from RESERVED with REAL
    // wiring. `review` answers a question STRICTLY by grounded retrieval over
    // evidence projected from the log's `check.recorded` / `verdict.recorded`
    // records, via `crate::verdict::qa::answer_question` — an explicit Refusal
    // when nothing grounds it, never a fabricated answer. Does NOT shadow a git
    // command (X5). LIVE the moment main.rs routes it.
    "review", // hugit review --log --question [--intent] — grounded-evidence Q&A
    // The wedge front door (Phase B) — graduated from RESERVED with REAL wiring.
    // `land queue` runs the REAL union-test + bisect + memoize engine over the
    // queued PRs: it folds them into a `hugit_queue::core::Batch`, runs
    // `evaluate_union` over a real `MemoCheck` oracle backed by
    // `hugit_checks::run_memoized` + a file-backed AC, lands the green set, and on
    // a red union bisects to the minimal failing pair (recording `queue.union_fail`
    // so `queue show`'s failing_pair lights up). Single-tenant local today; the
    // distributed runner fabric (F7) swaps in behind the same MemoCheck trait.
    // Does NOT shadow a git command (git has no `land`; X5-checked). LIVE the
    // moment main.rs routes it; the no-drift oracle requires it here.
    "land", // hugit land queue --log [--campaign] — batch land via the union engine
    // Worktree-dock (ADR-0005, WP-DOCK-1) — graduated with REAL wiring. `dock
    // coin` is called by the post-checkout hook at worktree/clone checkout time:
    // it coins the physical binding (gitdir+branch hash) that later carries cost
    // and verification. Silent (exit 0 always, never blocks git), idempotent
    // (marker present ⇒ no-op), R2-env-warning (never silent divergence). Does
    // NOT shadow a git command (X5-checked: git has no `dock`). LIVE the moment
    // main.rs routes it; the no-drift oracle requires it here.
    "dock", // hugit dock coin --top-level --gitdir --branch — hook-born dock
];

/// Non-v1 hugit verb tokens that are RESERVED but NOT dispatched.
///
/// These tokens remain reserved so they cannot be accidentally taken by `git`
/// (or another tool), but they are not CLI v1 commitments. They are **not**
/// part of the live binary surface: they do not appear in `main.rs` dispatch,
/// `hugit --help`, or the WP-X5 no-shadow oracle.
///
/// When a verb graduates to a live dispatch, move it from here to
/// [`HUGIT_VERBS`] and wire it in `main.rs`. (`campaign`, `intent`, and `pr`
/// graduated in the PC wave — dispatched as honest stubs at PC0.)
pub const HUGIT_RESERVED_VERBS: &[&str] = &[
    // External orchestration surface — not CLI v1.
    // (`land` graduated to HUGIT_VERBS — `land queue` runs the REAL union-test +
    // bisect + memoize engine over the queue, file-backed-AC + local-memoized at
    // single-tenant altitude; the distributed runner fabric swaps in behind the
    // same MemoCheck trait.)
    // (`check` + `verdict` graduated to HUGIT_VERBS at W0 — wedge EXECUTE wave;
    // `diag` graduated at the diagnosis wave — log-backed bisect.)
    // External workspace/context surface — not CLI v1.
    "ws", // hugit ws spawn/attach/snap/gc — claim-fenced workspaces
    // (`ctx` graduated to HUGIT_VERBS — `ctx resume` is REAL over the log's
    // journal.note records; `ctx snap` stays outside CLI v1.)
    // Reserved namespace area — no current CLI v1 commitment.
    // (`ledger` graduated to HUGIT_VERBS — Phase-D read surface, REAL-wired over
    // the canonical `--log` via `hugit_ledger::Ledger`; `fleet` + `watch`
    // graduated alongside it — `hugit_ledger::FleetState` / `WatchDisplay`;
    // `review` graduated — grounded-evidence Q&A over the log via `verdict::qa`.)
    // (`undo`, `policy`, `verdict approve`, `verdict reject`, `note` graduated
    // to HUGIT_VERBS at W3 — stakeholder + session verbs, REAL-wired. `policy`
    // ships both REAL `test` and Human-only append-only `edit`. `note` replaces
    // the old `journal note` sub-verb.)
    "dispatch", // hugit dispatch <intent>  — workspace + context packet
];

/// The live verb registry as a stable accessor for downstream consumers.
///
/// Returns [`HUGIT_VERBS`] verbatim. Provided as a function so downstream
/// consumers (e.g. WP-X5) can depend on a stable call shape even if the backing
/// representation changes.
pub fn hugit_verbs() -> &'static [&'static str] {
    HUGIT_VERBS
}
