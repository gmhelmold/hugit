//! `GET /v1/repos/{repo}/checks` → `ChecksVm` and its checks-specific nested
//! types. Shared atoms (`CheckRowVm`, `HunkVm`) come from [`crate::common`].
//! Transcribed BYTE-FOR-FIELD; derives copied verbatim (`ChecksKpisVm`/`ChecksVm`
//! carry an `f64` hit-rate → `PartialEq`-only, never `Eq`).

use serde::{Deserialize, Serialize};

use crate::common::{CheckRowVm, HunkVm};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecksKpisVm {
    /// Measured hit-rate percentage (0.0–100.0), displayed AS-IS.
    pub hit_rate_pct: f64,
    /// Honest shape label: FULL / PARTIAL / NONE / NO DATA.
    pub shape: String,
    pub hits: usize,
    pub executed: usize,
    /// Execution time saved by cache hits (ms), measured not promised.
    pub saved_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BisectVm {
    pub culprit: String,
    pub probes: u32,
    pub max_probes: u32,
    pub steps: Vec<String>,
}

/// The answer-first hero (checks v2): green or red headline + one-line answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecksHeroVm {
    pub green: bool,
    pub headline: String,
    pub answer: String,
    pub sub: String,
    #[serde(default)]
    pub cost: String,
    #[serde(default)]
    pub duration: String,
    #[serde(default)]
    pub hit_count: String,
    #[serde(default)]
    pub cached_label: String,
}

/// The culprit card (checks v2 red state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecksCulpritVm {
    pub check_name: String,
    pub failed_note: String,
    pub bisect_badge: String,
    pub log_lines: Vec<String>,
    pub suspect_intent: String,
    pub suspect_charter: String,
    pub suspect_meta: String,
    pub culprit_file: String,
    pub culprit_hunk: HunkVm,
    pub bisect: BisectVm,
}

/// One cache-hit pill (checks v2 cpills strip).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecksPillVm {
    pub key: String,
    pub cached: bool,
    pub command: String,
    pub saved: String,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub from_pr: String,
    #[serde(default)]
    pub ago: String,
    #[serde(default)]
    pub runner: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecksVm {
    pub repo: String,
    pub kpis: ChecksKpisVm,
    pub hero: ChecksHeroVm,
    pub culprit: Option<ChecksCulpritVm>,
    #[serde(default)]
    pub hero_red: Option<ChecksHeroVm>,
    pub cpills: Vec<ChecksPillVm>,
    pub checks: Vec<CheckRowVm>,
    pub bisect: Option<BisectVm>,
    pub memo_note: String,
    #[serde(default)]
    pub crumb_pr: u64,
    #[serde(default)]
    pub crumb_commit: String,
    #[serde(default)]
    pub crumb_campaign: String,
    #[serde(default)]
    pub executed_total_cost: String,
    #[serde(default)]
    pub quarantine_count: usize,
    #[serde(default)]
    pub cache_hit_rate_pct: u32,
    #[serde(default)]
    pub cache_saved_usd: String,
    #[serde(default)]
    pub cache_saved_runner_h: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical Appendix-A `checks` JSON round-trips losslessly through
    /// `ChecksVm` (the honest-KPI shape; `culprit`/`bisect`/`hero_red` null).
    #[test]
    fn checks_vm_round_trips_canonical_json() {
        let canonical = r##"{
          "repo": "hugit",
          "kpis": { "hit_rate_pct": 78.5, "shape": "FULL", "hits": 11, "executed": 3, "saved_ms": 540000 },
          "hero": { "green": true, "headline": "verde", "answer": "provado por $0.04 · 22s — 11 de 14 checks nem rodaram", "sub": "union-test batch #128 + #129", "cost": "$0.04", "duration": "22s", "hit_count": "11 de 14", "cached_label": "a cache é a prova" },
          "culprit": null, "hero_red": null,
          "cpills": [{ "key": "fmt", "cached": true, "command": "cargo fmt --check --all", "saved": "$0.28", "hash": "9c2f41a8", "from_pr": "#124", "ago": "2d atrás", "runner": "r-07" }],
          "checks": [{ "name": "test -p hugit-queue", "ok": true, "duration_ms": 8500, "cache_hit": true, "log": "running 9 tests... ok", "memo_key": "v1:queue:abc123", "reason": "o diff não tocou crates/queue", "cost": "" }],
          "bisect": null, "memo_note": "AC hit-rate: última hora 88%",
          "crumb_pr": 128, "crumb_commit": "a31f9c", "crumb_campaign": "auth-hardening",
          "executed_total_cost": "$0.04", "quarantine_count": 0,
          "cache_hit_rate_pct": 78, "cache_saved_usd": "$312", "cache_saved_runner_h": "9.4h"
        }"##;
        let vm: ChecksVm =
            serde_json::from_str(canonical).expect("checks JSON parses into ChecksVm");
        assert_eq!(vm.repo, "hugit");
        assert_eq!(vm.kpis.hit_rate_pct, 78.5);
        assert_eq!(vm.kpis.shape, "FULL");
        assert!(vm.culprit.is_none());
        assert!(vm.checks[0].cache_hit);
        assert_eq!(vm.cpills[0].runner, "r-07");
        let reparsed: ChecksVm =
            serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(vm, reparsed, "ChecksVm round-trip is lossless");
    }
}
