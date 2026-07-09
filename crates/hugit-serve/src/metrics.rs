//! Accept-loop observability + load-shed decision (WP W-SHED-METRICS).
//!
//! The deployed engine is a SINGLE-THREADED synchronous `tiny_http` accept loop
//! ([`crate::server::serve_on`]): it processes requests SERIALLY, one at a time.
//! This module adds two things without changing that model:
//!
//! 1. [`ShedGate`] — a fail-safe graceful load-shed. Its ONLY shed signal is
//!    CONTINUOUS accept-loop saturation (the loop has been busy back-to-back,
//!    never idle, for longer than `shed_after_ms`). It is fail-safe by
//!    construction: any idle gap (the queue drained — the engine is keeping up)
//!    RESETS the run, so normal/bursty-but-serviceable load NEVER trips it (a
//!    burst the loop clears in under the window is served in full). It sheds only
//!    a request arriving into a loop that has demonstrably been unable to keep up
//!    for seconds — a request the engine could NOT serve promptly. The decision is
//!    pure saturating arithmetic (cannot panic), and the caller additionally wraps
//!    it so a hypothetical panic degrades to "serve" (never shed, never crash).
//!
//! 2. [`Metrics`] — cheap aggregate counters (a monotonic request-id source,
//!    per-ROUTE-CLASS request counts, a rolling latency window for p50/p99, the
//!    shed total, and the 0/1 in-flight gauge). NO TENANT DATA: route labels are a
//!    fixed closed vocabulary of route CLASSES (`readyz`, `repo_read`, …), never a
//!    concrete path — a repo slug, a principal, a PR id, or a query never enters a
//!    metric. So `GET /metrics` can be served unauthenticated with no leak.
//!
//! Interior mutability (atomics + one `Mutex` for the latency window) lets the loop
//! share `&Metrics` freely and mutate cheaply. On the single-threaded loop every
//! access is uncontended; `Relaxed`/an uncontended lock add no meaningful latency.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The fail-safe load-shed gate (Part A). See the module docs for the safety
/// argument. `shed_after_ms == 0` DISABLES shedding entirely (a hard kill-switch).
///
/// State machine (all times monotonic ms since the loop started):
///   - `busy_since` = the start of the current CONTINUOUS-busy run, or `None` when
///     the loop is idle / has just drained its queue.
///   - On each request we learn `waited_ms` = how long the accept loop BLOCKED
///     waiting for this request. A block longer than `idle_reset_ms` means the
///     queue was empty (the engine caught up) → the run resets to `None`.
///   - `sat_run` = `now - busy_since` = how long we have been continuously busy.
///     We shed iff `sat_run > shed_after_ms`.
#[derive(Debug)]
pub struct ShedGate {
    busy_since_ms: Option<u64>,
    idle_reset_ms: u64,
    shed_after_ms: u64,
}

impl ShedGate {
    /// Build from the process env (called once at loop start):
    ///   - `HUGIT_SHED_SATURATION_MS` — continuous-saturation ms before shedding
    ///     (default `5000`; `0` = shedding OFF).
    ///   - `HUGIT_SHED_IDLE_RESET_MS` — a wait longer than this counts as "idle,
    ///     queue drained" and resets the run (default `5`).
    #[must_use]
    pub fn from_env() -> Self {
        let shed_after_ms = env_u64("HUGIT_SHED_SATURATION_MS", 5_000);
        let idle_reset_ms = env_u64("HUGIT_SHED_IDLE_RESET_MS", 5);
        Self {
            busy_since_ms: None,
            idle_reset_ms,
            shed_after_ms,
        }
    }

    /// Construct explicitly (tests).
    #[must_use]
    pub fn new(idle_reset_ms: u64, shed_after_ms: u64) -> Self {
        Self {
            busy_since_ms: None,
            idle_reset_ms,
            shed_after_ms,
        }
    }

    /// Whether shedding is enabled at all.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.shed_after_ms > 0
    }

    /// Advance the saturation state for one request and decide whether to shed it.
    ///
    /// - `waited_ms`: how long the loop blocked waiting for THIS request (the idle
    ///   indicator — a long block means the queue was empty, so the engine is
    ///   keeping up).
    /// - `now_ms`: monotonic ms since the loop started.
    ///
    /// Returns `true` iff the loop has been CONTINUOUSLY busy for longer than
    /// `shed_after_ms` — i.e. the engine has demonstrably been unable to drain its
    /// queue, so this arrival cannot be served promptly and is shed fast. Pure
    /// saturating arithmetic — CANNOT panic. Fail-safe: when disabled or when the
    /// loop has had any recent idle gap it returns `false` (serve).
    #[must_use]
    pub fn should_shed(&mut self, waited_ms: u64, now_ms: u64) -> bool {
        if self.shed_after_ms == 0 {
            return false; // disabled
        }
        // An idle gap (the queue drained) ends the current saturation run.
        if waited_ms > self.idle_reset_ms {
            self.busy_since_ms = None;
        }
        // How long have we been continuously busy?
        let sat_run = match self.busy_since_ms {
            Some(start) => now_ms.saturating_sub(start),
            None => 0,
        };
        if sat_run > self.shed_after_ms {
            // Still saturated — keep the run open so subsequent arrivals also shed
            // until an idle gap proves the engine caught up.
            return true;
        }
        // Serve. Open a run if this is the first request after an idle gap so the
        // saturation clock starts ticking; a request that opens a run has
        // `sat_run == 0` and is therefore NEVER the one that is shed.
        if self.busy_since_ms.is_none() {
            self.busy_since_ms = Some(now_ms);
        }
        false
    }
}

/// A route CLASS — the ONLY label a metric carries. A fixed closed vocabulary:
/// no concrete path, repo slug, principal, id, or query ever becomes a label, so
/// `/metrics` exposes zero tenant data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RouteClass {
    Readyz,
    Login,
    Metrics,
    RepoRead,
    MeRead,
    Git,
    Sse,
    Write,
    Token,
    Other,
}

impl RouteClass {
    /// The stable label (a metric key).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RouteClass::Readyz => "readyz",
            RouteClass::Login => "login",
            RouteClass::Metrics => "metrics",
            RouteClass::RepoRead => "repo_read",
            RouteClass::MeRead => "me_read",
            RouteClass::Git => "git",
            RouteClass::Sse => "sse",
            RouteClass::Write => "write",
            RouteClass::Token => "token",
            RouteClass::Other => "other",
        }
    }

    /// Whether this class is EXEMPT from load-shedding. Liveness (`/readyz`) and
    /// observability (`/metrics`) must stay answerable precisely DURING an overload
    /// (an operator needs them most then), so they are never shed.
    #[must_use]
    pub fn shed_exempt(self) -> bool {
        matches!(self, RouteClass::Readyz | RouteClass::Metrics)
    }

    /// Whether this class is EXEMPT from the per-principal rate limit (G10). Same
    /// carve-out as the load-shed: liveness (`/readyz`) and observability
    /// (`/metrics`) must stay answerable during a flood, so they are never throttled.
    #[must_use]
    pub fn rl_exempt(self) -> bool {
        self.shed_exempt()
    }

    /// The stable render order (so `/metrics` output is deterministic).
    const ALL: [RouteClass; 10] = [
        RouteClass::Readyz,
        RouteClass::Login,
        RouteClass::Metrics,
        RouteClass::RepoRead,
        RouteClass::MeRead,
        RouteClass::Git,
        RouteClass::Sse,
        RouteClass::Write,
        RouteClass::Token,
        RouteClass::Other,
    ];

    fn index(self) -> usize {
        match self {
            RouteClass::Readyz => 0,
            RouteClass::Login => 1,
            RouteClass::Metrics => 2,
            RouteClass::RepoRead => 3,
            RouteClass::MeRead => 4,
            RouteClass::Git => 5,
            RouteClass::Sse => 6,
            RouteClass::Write => 7,
            RouteClass::Token => 8,
            RouteClass::Other => 9,
        }
    }
}

/// The rolling latency window — the last `cap` per-request durations (ms), a fixed
/// ring buffer (bounded memory). p50/p99 are computed on read from a sorted copy.
#[derive(Debug)]
struct LatWindow {
    buf: Vec<u32>,
    cap: usize,
    next: usize,
}

impl LatWindow {
    fn new(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
            cap,
            next: 0,
        }
    }

    fn push(&mut self, ms: u32) {
        if self.buf.len() < self.cap {
            self.buf.push(ms);
        } else {
            self.buf[self.next] = ms;
            self.next = (self.next + 1) % self.cap;
        }
    }

    /// Nearest-rank percentile over the current window (`p` in `0..=100`). `0` for
    /// an empty window.
    fn percentile(&self, p: u32) -> u32 {
        if self.buf.is_empty() {
            return 0;
        }
        let mut sorted = self.buf.clone();
        sorted.sort_unstable();
        let n = sorted.len();
        // nearest-rank: idx = ceil(p/100 * n) - 1, clamped to [0, n-1].
        let rank = ((p as usize * n).div_ceil(100)).max(1);
        let idx = rank.min(n) - 1;
        sorted[idx]
    }
}

/// Cheap aggregate accept-loop counters (Part B). Interior-mutable so the loop
/// shares `&Metrics`. All values are aggregate integers — no tenant data.
#[derive(Debug)]
pub struct Metrics {
    /// Monotonic request counter — also the request-id source.
    req_counter: AtomicU64,
    /// Total requests shed fast (503) by the load-shed gate.
    shed_total: AtomicU64,
    /// 0/1 in-flight gauge (this loop processes one request at a time).
    in_flight: AtomicU64,
    /// Per-route-class request counts (indexed by [`RouteClass::index`]).
    per_route: [AtomicU64; 10],
    /// The rolling latency window (p50/p99).
    lat: Mutex<LatWindow>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            req_counter: AtomicU64::new(0),
            shed_total: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            per_route: Default::default(),
            lat: Mutex::new(LatWindow::new(1024)),
        }
    }

    /// Allocate the next monotonic request-id (starts at 1).
    pub fn next_request_id(&self) -> u64 {
        self.req_counter.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Set the 0/1 in-flight gauge.
    pub fn set_in_flight(&self, v: u64) {
        self.in_flight.store(v, Ordering::Relaxed);
    }

    /// Count one shed (503) request.
    pub fn record_shed(&self) {
        self.shed_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a served request: bump its route-class counter and push its latency
    /// sample. A poisoned latency lock is degraded past (a metrics lock must never
    /// take down the accept loop) — the counter still increments.
    pub fn record(&self, class: RouteClass, dur_ms: u64) {
        self.per_route[class.index()].fetch_add(1, Ordering::Relaxed);
        if let Ok(mut w) = self.lat.lock() {
            w.push(dur_ms.min(u32::MAX as u64) as u32);
        }
    }

    /// Render the aggregate counters as compact JSON (the `/metrics` body).
    ///
    /// Format (documented, stable): a single JSON object —
    /// ```json
    /// {"in_flight":0,"requests_total":42,"shed_total":0,
    ///  "latency_ms":{"p50":3,"p99":91,"window":42},
    ///  "per_route":{"readyz":10,"repo_read":30,...},
    ///  "cache_occupancy":null}
    /// ```
    /// `cache_occupancy` is `null`: the decoded-object cache lives behind
    /// `git.rs`/`state.rs`, which this WP does not own and which expose no
    /// occupancy accessor — so it is honestly reported as unavailable rather than
    /// faked (a tracked additive follow-up when those crates surface a getter).
    #[must_use]
    pub fn render_json(&self) -> String {
        let in_flight = self.in_flight.load(Ordering::Relaxed);
        let requests_total = self.req_counter.load(Ordering::Relaxed);
        let shed_total = self.shed_total.load(Ordering::Relaxed);
        let (p50, p99, window) = match self.lat.lock() {
            Ok(w) => (w.percentile(50), w.percentile(99), w.buf.len()),
            Err(_) => (0, 0, 0),
        };
        let mut routes = String::new();
        for (i, class) in RouteClass::ALL.iter().enumerate() {
            if i > 0 {
                routes.push(',');
            }
            let n = self.per_route[class.index()].load(Ordering::Relaxed);
            routes.push_str(&format!(r#""{}":{}"#, class.label(), n));
        }
        format!(
            r#"{{"in_flight":{in_flight},"requests_total":{requests_total},"shed_total":{shed_total},"latency_ms":{{"p50":{p50},"p99":{p99},"window":{window}}},"per_route":{{{routes}}},"cache_occupancy":null}}"#
        )
    }
}

/// Parse a `u64` env var; missing/unparseable → `default`.
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod shed_gate_tests {
    use super::*;

    /// NORMAL load NEVER false-503s: every request arrives after an idle gap
    /// (the queue was empty), which resets the saturation run → served.
    #[test]
    fn normal_load_never_sheds() {
        let mut g = ShedGate::new(5, 1_000);
        let mut now = 0u64;
        for _ in 0..1_000 {
            // A generous idle wait before each request (queue empty) + cheap work.
            let waited = 50; // > idle_reset_ms=5 → resets the run
            assert!(
                !g.should_shed(waited, now),
                "an idle-preceded request must never be shed"
            );
            now += 2; // 2 ms of work
        }
    }

    /// A bursty-but-SERVICEABLE burst (many back-to-back requests the loop clears
    /// well within the window) is served in FULL — no false 503.
    #[test]
    fn serviceable_burst_is_served_in_full() {
        let mut g = ShedGate::new(5, 5_000); // shed only after 5 s continuous busy
        // 500 back-to-back requests (waited ~0), each 1 ms → 500 ms total « 5 s.
        for now in 0..500u64 {
            assert!(
                !g.should_shed(0, now),
                "a burst the loop clears within the window must be served"
            );
        }
    }

    /// SUSTAINED overload sheds: once the loop has been continuously busy past the
    /// window, further arrivals are shed with a 503 — until an idle gap proves the
    /// engine caught up, which resumes serving.
    #[test]
    fn sustained_overload_sheds_then_recovers() {
        let mut g = ShedGate::new(5, 1_000); // 1 s window
        let mut now = 0u64;
        // Open the run + stay continuously busy (waited ~0) for > 1 s.
        assert!(!g.should_shed(0, now)); // opens the run at t=0, served
        now = 1_500; // 1.5 s of continuous back-to-back work elapsed
        assert!(
            g.should_shed(0, now),
            "past the saturation window a back-to-back arrival is shed"
        );
        // A subsequent still-saturated arrival also sheds.
        now = 1_600;
        assert!(g.should_shed(0, now), "still saturated → still shedding");
        // The engine catches up: a long idle wait resets the run → serve again.
        now = 5_000;
        assert!(
            !g.should_shed(500, now),
            "an idle gap (queue drained) must resume serving"
        );
    }

    /// `shed_after_ms == 0` is a hard kill-switch: NEVER sheds, whatever the state.
    #[test]
    fn disabled_never_sheds() {
        let mut g = ShedGate::new(5, 0);
        assert!(!g.enabled());
        // Even an absurd continuous-busy run does not shed when disabled.
        assert!(!g.should_shed(0, 0));
        assert!(!g.should_shed(0, 10_000_000));
    }

    /// The decision is pure saturating arithmetic — a `now_ms` that went backwards
    /// (a clock quirk) cannot panic; it just yields a non-negative `sat_run`.
    #[test]
    fn backwards_clock_does_not_panic() {
        let mut g = ShedGate::new(5, 1_000);
        assert!(!g.should_shed(0, 10_000)); // opens run at 10_000
        // now_ms < busy_since → saturating_sub → 0 → not shed, no panic.
        assert!(!g.should_shed(0, 5_000));
    }
}

#[cfg(test)]
mod metrics_tests {
    use super::*;

    /// Request-ids are monotonic starting at 1.
    #[test]
    fn request_ids_monotonic_from_one() {
        let m = Metrics::new();
        assert_eq!(m.next_request_id(), 1);
        assert_eq!(m.next_request_id(), 2);
        assert_eq!(m.next_request_id(), 3);
    }

    /// Per-route counts, shed total, and the latency window all render; no route
    /// label is ever a concrete path (closed vocabulary) → no tenant-data leak.
    #[test]
    fn render_json_has_aggregate_fields_only() {
        let m = Metrics::new();
        m.set_in_flight(1);
        m.record(RouteClass::RepoRead, 3);
        m.record(RouteClass::RepoRead, 91);
        m.record(RouteClass::Readyz, 1);
        m.record_shed();
        let j = m.render_json();
        assert!(j.contains(r#""in_flight":1"#));
        assert!(j.contains(r#""shed_total":1"#));
        assert!(j.contains(r#""repo_read":2"#));
        assert!(j.contains(r#""readyz":1"#));
        assert!(j.contains(r#""latency_ms":{"p50":"#));
        assert!(j.contains(r#""cache_occupancy":null"#));
        // No path/slug/id could appear — only fixed class labels + integers.
        for label in [
            "readyz",
            "login",
            "metrics",
            "repo_read",
            "me_read",
            "git",
            "sse",
            "write",
            "token",
            "other",
        ] {
            assert!(j.contains(label), "route label {label} present");
        }
    }

    /// Nearest-rank percentiles over a known window.
    #[test]
    fn percentiles_nearest_rank() {
        let mut w = LatWindow::new(100);
        for v in 1..=100u32 {
            w.push(v);
        }
        assert_eq!(w.percentile(50), 50);
        assert_eq!(w.percentile(99), 99);
        assert_eq!(w.percentile(100), 100);
        assert_eq!(LatWindow::new(4).percentile(50), 0); // empty → 0
    }

    /// The ring buffer stays bounded at `cap` (no unbounded growth).
    #[test]
    fn latency_window_is_bounded() {
        let mut w = LatWindow::new(8);
        for v in 0..1_000u32 {
            w.push(v);
        }
        assert_eq!(w.buf.len(), 8, "window never exceeds its cap");
    }
}
