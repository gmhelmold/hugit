//! `hugit-serve` — the `/v1` HTTP engine port (githugr window ⇄ hugit engine).
//!
//! The INTEGRATOR/adapter layer: it maps the domain-pure engine crates' state into
//! the frozen UI-shaped wire view-models ([`hugit_http_contracts`]) and (in the
//! scaffold step) serves them over a minimal synchronous HTTP server. The
//! VM-coupling — humanized ages, pt-BR summaries, `$`-formatted costs — lives ONLY
//! here; the engine core stays headless.
//!
//! ## The frozen handler interface (the scaffold ⇄ handlers seam)
//!
//! Each Wave-1 read is a PURE mapping function `build_<screen>(log, repo[, n]) ->
//! <Vm>` in [`handlers`]. It receives an ALREADY-verified [`EventLog`](hugit_refstore::EventLog)
//! (the HTTP scaffold owns load → `verify_chain` → 404/503 → auth → marshal) and
//! returns the frozen contract type. This decouples the handlers (pure, unit-
//! testable, no HTTP) from the server plumbing, so both build in parallel against
//! this frozen signature. Handlers serve REAL data where the engine has it and the
//! documented honest default (`""`/`null`/`0`/`[]`) elsewhere — NEVER a faked value
//! (the fail-honest contract). Each handler documents its per-field
//! REAL/PRESENTATION/STUB source inline.

pub mod auth;
pub mod authz;
pub mod blob_history_index;
pub mod budgeted_source;
pub mod cas;
pub mod clone_pack;
pub mod error;
pub mod fmt;
pub mod git;
pub mod handlers;
pub mod merge_hook;
pub mod metrics;
pub mod ratelimit;
pub mod receive_wire;
pub mod search_index;
pub mod server;
pub mod sigv4;
pub mod state;
pub mod tenant_registry;
pub mod token;
pub mod writes;
