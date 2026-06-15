//! `GET /v1/repos/{repo}/checks` → [`ChecksVm`]. FROZEN signature; body filled by
//! the fleet per master-plan §5 (REAL: local hit-rate KPIs + check rows via the
//! `hugit checks show` projection; STUB: bisect/culprit + FLEET KPIs = P2).

use hugit_cli::checks::CHECK_RECORDED_KIND;
use hugit_http_contracts::common::CheckRowVm;
use hugit_http_contracts::{ChecksHeroVm, ChecksKpisVm, ChecksPillVm, ChecksVm};
use hugit_refstore::EventLog;
use serde_json::Value;

use crate::fmt::{CHECKS_CAP, scrub, str_field};

// ---------------------------------------------------------------------------
// Internal row — mirrors hugit-cli's CheckRow without depending on it.
// ---------------------------------------------------------------------------

struct CheckRow {
    name: Option<String>,
    ok: Option<bool>,
    duration_ms: Option<u64>,
    cache_hit: Option<bool>,
    memo_key: Option<String>,
    pr_id: Option<String>,
}

impl CheckRow {
    /// Project a row from a `check.recorded` payload — honest reads, no defaults.
    fn from_payload(v: &Value) -> Self {
        let exit = v.get("exit").and_then(Value::as_i64);
        CheckRow {
            name: str_field(v, "name"),
            ok: exit.map(|e| e == 0),
            duration_ms: v.get("duration_ms").and_then(Value::as_u64),
            cache_hit: v.get("cache_hit").and_then(Value::as_bool),
            memo_key: str_field(v, "memo_key"),
            pr_id: str_field(v, "pr_id"),
        }
    }
}

// ---------------------------------------------------------------------------
// KPI aggregation (ported from hugit-cli aggregate_kpis — exact same logic).
// ---------------------------------------------------------------------------

struct Kpis {
    hits: usize,
    executed: usize,
    saved_ms: u64,
    /// Whether any row had a known `cache_hit` field.
    any_known: bool,
}

fn aggregate_kpis(rows: &[CheckRow]) -> Kpis {
    let mut hits: usize = 0;
    let mut executed: usize = 0;
    let mut saved_ms: u64 = 0;
    let mut any_known = false;

    for row in rows {
        match row.cache_hit {
            Some(true) => {
                any_known = true;
                hits += 1;
                if let Some(ms) = row.duration_ms {
                    saved_ms += ms;
                }
            }
            Some(false) => {
                any_known = true;
                executed += 1;
            }
            None => {}
        }
    }

    Kpis {
        hits,
        executed,
        saved_ms,
        any_known,
    }
}

// ---------------------------------------------------------------------------
// Public frozen entry point.
// ---------------------------------------------------------------------------

/// Build the checks view-model from a verified event log.
///
/// Log is ALREADY verified by the scaffold before this is called.
/// REAL: local hit-rate KPIs + per-check rows from `check.recorded` events.
/// STUB (P2): bisect, culprit, fleet KPIs (cache_hit_rate_pct / cache_saved_usd /
///            cache_saved_runner_h), hero_red.
pub fn build_checks(log: &EventLog, repo: &str) -> ChecksVm {
    // Collect check rows from the log, capped at CHECKS_CAP.
    let rows: Vec<CheckRow> = log
        .records()
        .iter()
        .filter(|r| r.kind == CHECK_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .map(|v| CheckRow::from_payload(&v))
        .take(CHECKS_CAP)
        .collect();

    let kpis = aggregate_kpis(&rows);

    // -- KpisVm -----------------------------------------------------------
    let denom = kpis.hits + kpis.executed;

    // hit_rate_pct: hits*10000/(hits+executed)/100.0; 0.0 if no rows.
    let hit_rate_pct: f64 = if kpis.any_known && denom > 0 {
        let bps = (kpis.hits * 10_000) / denom;
        (bps as f64) / 100.0
    } else {
        0.0
    };

    // shape: FULL / NONE / PARTIAL / NO DATA
    let shape = match (kpis.hits, kpis.executed) {
        (h, e) if !kpis.any_known || (h == 0 && e == 0) => "NO DATA",
        (h, 0) if h > 0 => "FULL",
        (0, e) if e > 0 => "NONE",
        _ => "PARTIAL",
    }
    .to_string();

    let kpis_vm = ChecksKpisVm {
        hit_rate_pct,
        shape,
        hits: kpis.hits,
        executed: kpis.executed,
        saved_ms: kpis.saved_ms,
    };

    // -- CheckRowVm list ---------------------------------------------------
    let checks: Vec<CheckRowVm> = rows
        .iter()
        .take(CHECKS_CAP)
        .map(|row| CheckRowVm {
            name: scrub(&row.name.clone().unwrap_or_default()), // REAL (scrubbed read-boundary)
            ok: row.ok.unwrap_or(false),                        // REAL (exit==0)
            duration_ms: row.duration_ms.unwrap_or(0),          // REAL
            cache_hit: row.cache_hit.unwrap_or(false),          // REAL
            memo_key: row.memo_key.clone().unwrap_or_default(), // REAL
            log: String::new(),                                 // STUB — not in payload
            reason: String::new(),                              // STUB — not in payload
            cost: String::new(),                                // STUB — not in payload
        })
        .collect();

    // -- Cache-hit pills (cpills) — one per HIT row, capped at CHECKS_CAP ---
    let cpills: Vec<ChecksPillVm> = rows
        .iter()
        .filter(|row| row.cache_hit == Some(true))
        .take(CHECKS_CAP)
        .map(|row| {
            // hash: first 8 chars of memo_key (REAL); empty if no key.
            let hash = row
                .memo_key
                .as_deref()
                .map(|k| k.chars().take(8).collect::<String>())
                .unwrap_or_default();
            ChecksPillVm {
                key: scrub(&row.name.clone().unwrap_or_default()), // REAL (scrubbed read-boundary)
                cached: true,                                      // REAL
                hash,                                              // REAL (prefix)
                from_pr: scrub(&row.pr_id.clone().unwrap_or_default()), // REAL or "" (payload id: scrubbed read-boundary)
                command: String::new(),                                 // STUB — not in log payload
                saved: String::new(),                                   // STUB — not in log payload
                ago: String::new(),                                     // STUB — not in log payload
                runner: String::new(),                                  // STUB — not in log payload
            }
        })
        .collect();

    // -- Hero (PRESENTATION from KPIs) ------------------------------------
    // P1 honesty fix: unknown exit (None) = NOT green (conservative, fail-safe).
    let all_ok = rows.iter().all(|r| r.ok.unwrap_or(false));
    let green = !rows.is_empty() && all_ok;

    let headline = if green {
        "verde".to_string()
    } else {
        "vermelho".to_string()
    };

    let (answer, sub, hit_count, cached_label) = if rows.is_empty() {
        (
            "sem dados de checks neste log".to_string(),
            "nenhum check.recorded encontrado".to_string(),
            "0 de 0".to_string(),
            "sem cache".to_string(),
        )
    } else {
        let total = kpis.hits + kpis.executed;
        let hit_count_str = format!("{} de {}", kpis.hits, total);
        let cached_label_str = if kpis.hits > 0 {
            "a cache é a prova".to_string()
        } else {
            "sem hits de cache".to_string()
        };
        let pct_str = format!("{:.1}%", hit_rate_pct);
        let answer_str = format!(
            "hit-rate local: {} — {} checks na cache",
            pct_str, kpis.hits
        );
        let sub_str = format!(
            "{} executados, {} ms poupados",
            kpis.executed, kpis.saved_ms
        );
        (answer_str, sub_str, hit_count_str, cached_label_str)
    };

    let hero = ChecksHeroVm {
        green,
        headline,
        answer,
        sub,
        cost: String::new(),     // STUB
        duration: String::new(), // STUB
        hit_count,               // PRESENTATION
        cached_label,            // PRESENTATION
    };

    // -- memo_note (honest disclosure) ------------------------------------
    let memo_note = if rows.is_empty() {
        "sem check.recorded neste log; KPIs são nulos, não zero".to_string()
    } else {
        format!("AC hit-rate local: {:.1}%", hit_rate_pct)
    };

    // -- Assemble the full VM ---------------------------------------------
    ChecksVm {
        repo: repo.to_string(),
        kpis: kpis_vm,
        hero,
        checks,
        cpills,
        culprit: None,  // STUB — P2 (needs live AC+queue)
        hero_red: None, // STUB — P2
        bisect: None,   // STUB — P2
        memo_note,
        // crumb_* — STUB (not in log)
        crumb_pr: 0,
        crumb_commit: String::new(),
        crumb_campaign: String::new(),
        // cost summary — STUB
        executed_total_cost: String::new(),
        quarantine_count: 0,
        // FLEET KPIs — P2 STUBs (honest zero/empty)
        cache_hit_rate_pct: 0,
        cache_saved_usd: String::new(),
        cache_saved_runner_h: String::new(),
    }
}
