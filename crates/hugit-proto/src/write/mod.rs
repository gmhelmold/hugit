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

/// JSON string escaping for the canonical write-path payloads — quotes,
/// backslash, and **all control characters** (`\n` `\r` `\t` and any byte below
/// `0x20` as `\uXXXX`). This is the single full-escaping encoder used by every
/// write-path payload builder ([`external`] and [`store`]); a lossy variant that
/// dropped control chars would let a crafted ref name corrupt the canonical JSON.
pub(crate) fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
