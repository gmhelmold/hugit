//! The git wire-protocol **write path**: receive-pack → CAS + event log, under
//! concurrency and at its boundaries.
//!
//! WP-D3b owns the concurrency / total-order + external-change + flag/negative
//! halves of D3 (items ②③④⑤); the receive-pack→CAS+log core (item ①) is WP-D3a's.
//!
//! - [`order`] — concurrent pushes serialized through the D1 single-writer point
//!   get a strict **total order**, with **compare-and-append stale rejection**
//!   (item ②).
//! - [`external`] — a raw `git push` is recorded as an **opaque external-change
//!   event with attribution** (who/when/which ref), never an Intent (item ③, and
//!   the structural half of item ⑤).
//! - [`flag`] — the write path is **off unless self-hosted-alpha** (item ④).
//!
//! The whole path rests on D1's append-only log ([`hugit_refstore`]): refs are a
//! derived view, externals stay external, and no raw push ever fabricates an
//! intent (the D3 leg of the no-fake-intents adjudication).

pub mod external;
pub mod flag;
pub mod order;
