//! The git wire-protocol **write path**: receive-pack → CAS + event log, under
//! concurrency and at its boundaries.
//!
//! WP-D3a owns the receive-pack→CAS+log **core** (item ①); WP-D3b owns the
//! concurrency / total-order + external-change + flag/negative halves (②③④⑤).
//!
//! - [`receive`] — receive-pack ingest: bound → unpack (system git pack engine)
//!   → verify → store → anchor → record (WP-D3a, item ①).
//! - [`store`] — object → CoreLink CAS persistence and ref-update → append-only
//!   D1 event (a raw-push event, never a synthesised intent) (WP-D3a).
//! - [`order`] — concurrent pushes serialized through the D1 single-writer point
//!   get a strict **total order**, with **compare-and-append stale rejection**
//!   (WP-D3b, item ②).
//! - [`external`] — a raw `git push` is recorded as an **opaque external-change
//!   event with attribution**, never an Intent (WP-D3b, item ③ + structural ⑤).
//! - [`flag`] — the write path is **off unless self-hosted-alpha** (WP-D3b, ④).
//!
//! The whole path rests on D1's append-only log ([`hugit_refstore`]): refs are a
//! derived view, externals stay external, and no raw push ever fabricates an
//! intent (the D3 leg of the no-fake-intents adjudication).

pub mod external;
pub mod flag;
pub mod order;
pub mod receive;
pub mod store;
