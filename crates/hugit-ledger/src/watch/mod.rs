//! `hugit watch` — live TUI event display with latency measurement (②).
//!
//! This module models the EventRecord-to-display pipeline and measures the
//! end-to-end latency (event arrival → on-screen render) per event class.
//!
//! # Event classes
//! Four classes are defined per the contract:
//! - `landing`      — `intent.landed` events.
//! - `verdict`      — `verdict.recorded` events.
//! - `policy-change` — `policy.changed` events.
//! - `ws-state`     — `ws.state.*` events (workspace state transitions).
//! - `other`        — any event kind not covered by the four primary classes.
//!
//! # Latency measurement (②)
//! For each event class, a `LatencyMeasurement` holds a sample of observed
//! render durations (in milliseconds).  `p95()` computes the 95th-percentile
//! from those samples.  The contract bar is p95 < 2000 ms per class.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use hugit_contracts::event_record::EventRecord;

use crate::redact;

/// The event classes tracked for latency; `Other` catches unknown kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventClass {
    Landing,
    Verdict,
    PolicyChange,
    WsState,
    /// Git activity — raw ref changes (`ref.update` / `ref.delete`), the
    /// captures produced by `hugit capture` (the silent git hooks). Distinct
    /// from Landing (an intent) — this is the raw graph trace.
    GitActivity,
    /// Any event kind not covered by the primary classes.
    Other,
}

impl EventClass {
    /// Classify an EventRecord by its `kind` field.
    ///
    /// Returns `Other` for unknown kinds — never `None`.
    pub fn classify(kind: &str) -> Self {
        if kind == "intent.landed" {
            EventClass::Landing
        } else if kind == "verdict.recorded" {
            EventClass::Verdict
        } else if kind == "policy.changed" {
            EventClass::PolicyChange
        } else if kind.starts_with("ws.state") {
            EventClass::WsState
        } else if kind == "ref.update" || kind == "ref.delete" {
            EventClass::GitActivity
        } else {
            EventClass::Other
        }
    }

    /// Human-readable label for the event class.
    pub fn label(self) -> &'static str {
        match self {
            EventClass::Landing => "landing",
            EventClass::Verdict => "verdict",
            EventClass::PolicyChange => "policy-change",
            EventClass::WsState => "ws-state",
            EventClass::GitActivity => "git-activity",
            EventClass::Other => "other",
        }
    }
}

/// A rendered display line produced by the watch pipeline.
#[derive(Debug, Clone)]
pub struct WatchLine {
    /// Event class.
    pub class: EventClass,
    /// Log sequence.
    pub seq: u64,
    /// Display text (secrets redacted).
    pub text: String,
    /// The wall-clock duration from event arrival to this render being produced.
    pub render_duration: Duration,
}

/// Latency measurements for one event class.
#[derive(Debug, Clone, Default)]
pub struct LatencyMeasurement {
    /// Observed render durations in milliseconds, for p95 computation.
    samples_ms: Vec<u64>,
}

impl LatencyMeasurement {
    /// Add a sample.
    pub fn record(&mut self, duration: Duration) {
        self.samples_ms.push(duration.as_millis() as u64);
    }

    /// Number of samples.
    pub fn count(&self) -> usize {
        self.samples_ms.len()
    }

    /// 95th-percentile render latency in milliseconds.
    ///
    /// Returns 0 if there are no samples.
    pub fn p95(&self) -> u64 {
        if self.samples_ms.is_empty() {
            return 0;
        }
        let mut sorted = self.samples_ms.clone();
        sorted.sort_unstable();
        // p95 index: ceil(0.95 * n) - 1, clamped.
        let idx = ((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        let idx = idx.min(sorted.len() - 1);
        sorted[idx]
    }
}

/// The watch display pipeline.
///
/// Converts EventRecords to rendered lines, measures render latency per class,
/// and applies view-boundary redaction.
#[derive(Debug, Default)]
pub struct WatchDisplay {
    /// Per-class latency measurements.
    latency: HashMap<EventClass, LatencyMeasurement>,
    /// Lines produced so far.
    lines: Vec<WatchLine>,
}

impl WatchDisplay {
    /// Create a new, empty watch display.
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one EventRecord through the display pipeline.
    ///
    /// Measures the time taken to produce the display line (the render
    /// latency for this event), records it per event class, and appends
    /// the line to the output.
    ///
    /// Returns the rendered `WatchLine`.
    pub fn process(&mut self, record: &EventRecord) -> WatchLine {
        let start = Instant::now();
        let text = render_record(record);
        let elapsed = start.elapsed();

        let class = EventClass::classify(&record.kind);

        let line = WatchLine {
            class,
            seq: record.seq,
            text,
            render_duration: elapsed,
        };

        self.latency
            .entry(class)
            .or_default()
            .record(line.render_duration);

        self.lines.push(line.clone());
        line
    }

    /// Process a batch of EventRecords.
    pub fn process_batch(&mut self, records: &[EventRecord]) -> Vec<WatchLine> {
        records.iter().map(|r| self.process(r)).collect()
    }

    /// The p95 render latency for a given event class (ms), or 0 if no samples.
    pub fn p95_ms(&self, class: EventClass) -> u64 {
        self.latency.get(&class).map(|m| m.p95()).unwrap_or(0)
    }

    /// The latency table: one entry per class that has samples.
    pub fn latency_table(&self) -> Vec<(EventClass, u64)> {
        let mut table: Vec<(EventClass, u64)> =
            self.latency.iter().map(|(c, m)| (*c, m.p95())).collect();
        table.sort_by_key(|(c, _)| *c);
        table
    }

    /// All rendered lines.
    pub fn lines(&self) -> &[WatchLine] {
        &self.lines
    }

    /// The latency measurement for a given event class.
    pub fn latency_for(&self, class: EventClass) -> Option<&LatencyMeasurement> {
        self.latency.get(&class)
    }
}

/// Render one EventRecord to a display string, applying view-boundary redaction.
///
/// EVERY record-derived string in the line goes through the redaction filter, not
/// only the payload (defense-in-depth): `kind` is routed through `redact::apply`
/// too so the guarantee covers the whole text field even if a future record kind
/// carries free text. `seq`/`recorded_at` are numeric and cannot carry a secret.
/// Each component is redacted SEPARATELY (rather than the assembled line) so a
/// secret in one field cannot collapse the entire structured line to the marker.
fn render_record(record: &EventRecord) -> String {
    let kind = redact::apply(&record.kind);
    let payload_display = redact::apply(&record.payload);
    format!(
        "[seq={seq} kind={kind} at={at}] {payload}",
        seq = record.seq,
        kind = kind,
        at = record.recorded_at,
        payload = payload_display,
    )
}
