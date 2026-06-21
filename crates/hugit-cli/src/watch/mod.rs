//! `hugit watch` — the forge event stream made visible (Phase D, graduated from
//! RESERVED with REAL wiring — not a stub).
//!
//! Projects the canonical event log into the classified, redacted display stream
//! the live TUI renders, reusing the engine's own [`hugit_ledger::WatchDisplay`]
//! pipeline (event → class → redacted line). The `--log` read is a DETERMINISTIC
//! REPLAY of the stream — the live tail (`p95 < 2s` SLA over the running engine's
//! SSE) is the serve surface; this is the same content, read statically.
//!
//! `hugit watch --log <path> [--class <c>]` emits stable JSON: the ordered lines
//! (`class`, `seq`, redacted `text`) + a `count`. `--class` filters to one of
//! `landing | verdict | policy-change | ws-state | other`. Render latency is
//! deliberately NOT surfaced here — it is a live-TUI SLA measurement, not stream
//! content, and would make a static replay non-reproducible.
//!
//! Every output is stable JSON on stdout under the WB0 one-error/one-exit law
//! ([`crate::porcelain`]): `log_not_found` / `parse_log` are the canonical
//! `{"error":{…}}` envelopes, exit `2`.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use hugit_ledger::WatchDisplay;

use crate::checks::load_event_log;
use crate::porcelain::PorcelainError;

/// `hugit watch` — replay the classified, redacted event stream (Phase D).
#[derive(clap::Args, Debug)]
pub struct WatchArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`) — the one
    /// `--log` seam every porcelain verb shares.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
    /// Optional class filter: `landing` | `verdict` | `policy-change` |
    /// `ws-state` | `other`. Unmatched ⇒ an empty (honest) line set.
    #[arg(long)]
    pub class: Option<String>,
}

/// Dispatch `hugit watch`, emitting stable JSON on stdout and returning the
/// process exit code under the WB0 one-exit-code law.
pub fn run(args: WatchArgs) -> ExitCode {
    match project(&args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", e.to_json());
            e.exit_code()
        }
    }
}

/// Project the classified, redacted event stream from the canonical log.
fn project(args: &WatchArgs) -> Result<Value, PorcelainError> {
    let log_path = crate::log_resolve::resolve_log(args.log.clone());
    let log = load_event_log(&log_path)?;
    let mut display = WatchDisplay::new();
    let lines = display.process_batch(log.records());

    let rows: Vec<Value> = lines
        .iter()
        .filter(|l| args.class.as_deref().is_none_or(|c| l.class.label() == c))
        .map(|l| json!({ "class": l.class.label(), "seq": l.seq, "text": l.text }))
        .collect();

    Ok(json!({ "count": rows.len(), "lines": rows }))
}
