//! The Wave-2 write verbs — each a PURE function `fn(&mut EventLog, …args, req,
//! principal_chain, at) -> Result<Accepted, EngineErr>` that the write-door
//! ([`crate::writes::with_write`]) wraps with idempotency + persistence. A verb
//! mutates the in-memory log (via `append_authorized`) and returns the success
//! body; it owns NO idempotency, HTTP, or I/O.

pub mod write_comment;
pub mod write_dispatch;
pub mod write_edit_propose;
pub mod write_erasure_decide;
pub mod write_issue_transition;
pub mod write_land;
pub mod write_policy;
pub mod write_undo;
pub mod write_verdict;
