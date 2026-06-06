//! Shared fixtures + the D3 **write-path red-team** for the WP-D3b acceptance
//! suite (push concurrency / total order + external-change + flag/negatives).
//!
//! This module is included by `tests/acceptance_d3b.rs` via `#[path]`. It holds:
//!
//! - deterministic git-object-id fixtures (real-shaped 40-hex oids),
//! - a barrier-synchronized concurrent-push driver that forces **real overlap**
//!   (all writer threads are held at a barrier and released together, so the
//!   single-writer point is genuinely contended — not instant/serial), and
//! - the red-team assertions: stale-tip forgery, attribution spoofing, and
//!   intent-fabrication attempts, all of which the write path must defeat.

use std::sync::{Arc, Barrier};
use std::thread;

use hugit_proto::{PushOutcome, RefUpdate, SerializedWriter};

/// A deterministic, real-shaped 40-hex git object id from a short seed.
///
/// Not a real reachable object — the D3b write path treats targets as opaque
/// object ids (item ③: externals are opaque), so a well-formed oid suffices to
/// exercise total order / stale rejection without standing up a CAS.
pub fn oid(seed: &str) -> String {
    let mut s = String::new();
    for b in seed.bytes() {
        s.push_str(&format!("{:02x}", b));
    }
    while s.len() < 40 {
        s.push('0');
    }
    s.truncate(40);
    s
}

/// A compare-and-append create (`expected = None`) of `ref_name → target` by
/// `who`, recorded at `at`.
pub fn create(ref_name: &str, target: &str, who: &str, at: u64) -> RefUpdate {
    RefUpdate {
        ref_name: ref_name.to_string(),
        expected: None,
        target: target.to_string(),
        principal_chain: vec![who.to_string()],
        recorded_at: at,
    }
}

/// A compare-and-append move of `ref_name` from `from` to `to` by `who`.
pub fn advance(ref_name: &str, from: &str, to: &str, who: &str, at: u64) -> RefUpdate {
    RefUpdate {
        ref_name: ref_name.to_string(),
        expected: Some(from.to_string()),
        target: to.to_string(),
        principal_chain: vec![who.to_string()],
        recorded_at: at,
    }
}

/// Drive `n` pushes through one [`SerializedWriter`] from `n` real threads held
/// at a [`Barrier`] so they are released simultaneously — genuine overlap on the
/// single-writer point, not instant serial calls. Returns the per-thread outcomes
/// in thread-spawn order alongside the writer for post-hoc assertions.
pub fn drive_overlapping(
    writer: Arc<SerializedWriter>,
    updates: Vec<RefUpdate>,
) -> Vec<PushOutcome> {
    let n = updates.len();
    let barrier = Arc::new(Barrier::new(n));
    let mut handles = Vec::with_capacity(n);

    for update in updates {
        let w = Arc::clone(&writer);
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            // Hold every thread here until all are ready, then race the writer.
            b.wait();
            w.push(update)
        }));
    }

    handles
        .into_iter()
        .map(|h| h.join().expect("push thread panicked"))
        .collect()
}
