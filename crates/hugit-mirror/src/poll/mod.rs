//! Webhook-loss poll fallback detector (WP-E1b, item ⑥).
//!
//! Divergence is normally signalled by a GitHub App webhook
//! ([`hugit_contracts::AppWebhooks`]). When webhook delivery is lost, a polling
//! loop is the fallback path: it periodically re-reads the mirror ref tips and
//! detects divergence on its own. The poll interval is chosen so that detection
//! still happens **within the stated SLA** even with zero webhooks.

/// Default detection SLA: a lost-webhook divergence must be caught within this
/// many milliseconds (item ⑥).
pub const DETECTION_SLA_MS: u64 = 5 * 60 * 1_000; // 5 minutes

/// Configuration of the poll fallback detector.
#[derive(Debug, Clone)]
pub struct PollConfig {
    /// Interval between polls (ms).
    pub interval_ms: u64,
    /// Detection SLA (ms) — the bound the poll cadence must satisfy.
    pub sla_ms: u64,
}

impl Default for PollConfig {
    fn default() -> Self {
        // Poll well inside the SLA so a single missed webhook is caught with
        // margin (at least two polls within the window).
        Self {
            interval_ms: DETECTION_SLA_MS / 5,
            sla_ms: DETECTION_SLA_MS,
        }
    }
}

impl PollConfig {
    /// `true` when the poll cadence guarantees detection within the SLA, i.e.
    /// at least one poll fires inside the SLA window.
    pub fn satisfies_sla(&self) -> bool {
        self.interval_ms > 0 && self.interval_ms <= self.sla_ms
    }

    /// Worst-case detection latency for a divergence that appears just after a
    /// poll: the next poll fires one interval later.
    pub fn worst_case_detection_ms(&self) -> u64 {
        self.interval_ms
    }
}

/// Outcome of a poll-driven divergence scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollDetection {
    /// Whether divergence was found by polling.
    pub diverged: bool,
    /// Simulated time (ms since outage start) the divergence was detected.
    pub detected_at_ms: u64,
    /// Whether detection landed within the SLA.
    pub within_sla: bool,
}

/// Simulate the poll fallback detecting a divergence that appeared at
/// `diverged_at_ms` after the last good webhook, under `cfg` (item ⑥).
///
/// Returns the first poll tick at or after the divergence, and whether it lands
/// within the SLA. Models the webhook-loss path: no webhook arrives, polling is
/// the only detector.
pub fn detect_via_poll(cfg: &PollConfig, diverged_at_ms: u64) -> PollDetection {
    let interval = cfg.interval_ms.max(1);
    // First poll tick at or after the divergence appeared.
    let next_tick = diverged_at_ms.div_ceil(interval) * interval;
    let detected = next_tick.max(diverged_at_ms);
    let latency = detected.saturating_sub(diverged_at_ms);
    PollDetection {
        diverged: true,
        detected_at_ms: detected,
        within_sla: latency <= cfg.sla_ms && cfg.satisfies_sla(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cadence_satisfies_sla() {
        let cfg = PollConfig::default();
        assert!(cfg.satisfies_sla());
        assert!(cfg.worst_case_detection_ms() <= cfg.sla_ms);
    }

    #[test]
    fn webhook_loss_detected_within_sla() {
        let cfg = PollConfig::default();
        let d = detect_via_poll(&cfg, 1);
        assert!(d.diverged);
        assert!(d.within_sla);
    }
}
