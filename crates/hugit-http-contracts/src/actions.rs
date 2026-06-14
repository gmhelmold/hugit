//! Write-path wire shapes (backend-API-v1 §3). Transcribed byte-for-field from
//! `../githugr/crates/githugr-vm/src/actions.rs`.
//!
//! [`Accepted`] is the success body every mutating verb returns. The engine also
//! emits a top-level `"accepted": true` alongside these fields (spec §3 common
//! body); the `githugr-live` client reads the typed fields and treats the 2xx
//! status as the truth, so the extra key is tolerated either way.
//!
//! The error body is `{ "code", "reason" }` (handled by `hugit-serve`'s
//! `EngineErr`); the client's internal `Denied` enum is NOT a wire type the engine
//! emits, so it is intentionally not transcribed here.

use serde::{Deserialize, Serialize};

/// The success body returned by a mutating verb. Field presence is verb-specific
/// (spec §3): `queue_pos` only on `land`; `pr_number`/`branch` on
/// `dispatch`/`edit_propose`; `state` on the erasure verbs; `charter_preview` on
/// `dispatch`. No f64 → derives `Eq`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    /// The event-stream sequence the execution reconciles by.
    pub seq: u64,
    /// pt-BR toast line, e.g. "entrou na fila de união".
    pub note: String,
    /// Optional display detail, e.g. "fila #3" | "PR #143 (rascunho)".
    pub extra: Option<String>,
    /// Position in the landing queue, when the verb enqueues the PR (`land`).
    pub queue_pos: Option<u32>,
    /// The PR number opened or targeted by this op (`dispatch`, `edit_propose`).
    pub pr_number: Option<u64>,
    /// The branch created for this op (`edit_propose`).
    pub branch: Option<String>,
    /// The erasure decision state — `"pending"|"approved"|"denied"|"executed"`.
    pub state: Option<String>,
    /// A one-line pt-BR preview of the charter derived from the dispatch `ask`.
    pub charter_preview: Option<String>,
}
