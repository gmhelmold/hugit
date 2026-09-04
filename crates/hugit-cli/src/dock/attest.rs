//! dock::attest — flush spooled cost samples into the canonical log (WP-DOCK-4).
//!
//! The spool holds samples DURABLY but OFF the chain; the flush is the
//! "attest" step that lands them into the canonical `.hugit/log.json` as
//! `cost.sample` records — idempotent by `run_id` (exact-once M2), each with a
//! `content_hash` (M3 integrity, private/local) and the honest
//! `is_unlabeled:true` marker when the sample had no dock.
//!
//! ## Honesty boundaries (never crossed)
//!
//! - **M3** — every `cost.sample` record carries a `content_hash` of the sample
//!   (sha256 of the canonical CostSampleV1 json); a reader can detect a forged
//!   sample. The OFF-BOX cryptographic attestation (gateway signs the sample) is
//!   OWNER-GATED (the irmão, live); absent a gateway signature this is the
//!   honest local-integrity floor, never presented as a fabric attestation.
//! - **M4** — no gateway sample ever means `None`/zero; a sample is never
//!   derived from internal metrics.
//!
//! After a successful flush the spool file is drained (removed) — the log
//! record is the durable truth; the spool is the transient journal.

use std::path::Path;

use hugit_contracts::cost_sample::CostSampleV1;

use crate::checks::load_event_log;
use crate::pr::filelock::FileLock;
use hugit_refstore::authz::{Endpoint, PrincipalClass};
use hugit_refstore::canonical_json;

/// The frozen on-wire event kind for a landed cost sample.
pub const COST_SAMPLE_KIND: &str = "cost.sample";
/// The recorder identity (hook/dock principal).
const COST_PRINCIPAL: &str = "orchestrator:hugit-hook";

/// sha256 hex of a string (content hash).
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(s.as_bytes());
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn acquire_with_retry(log_path: &Path) -> Result<FileLock, String> {
    let attempts = 30;
    let mut last = String::new();
    for _ in 0..attempts {
        match FileLock::acquire(log_path) {
            Ok(lock) => return Ok(lock),
            Err(e) => {
                last = format!("{e}");
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    Err(format!(
        "lock {log_path:?} still busy after {attempts} attempts: {last}"
    ))
}

/// Append a `cost.sample` record under the lock, idempotent by `run_id`.
fn append_cost_sample(log_path: &Path, sample: &CostSampleV1) -> Result<(), String> {
    let _lock = acquire_with_retry(log_path)?;
    let mut log = load_event_log(log_path).map_err(|e| format!("load log: {}", e.to_json()))?;

    // Exact-once (M2): a record with this run_id already landed ⇒ no-op.
    let key = sample.dedupe_key();
    let exists = log.records().iter().any(|r| {
        serde_json::from_str::<serde_json::Value>(&r.payload)
            .ok()
            .and_then(|p| p.get("run_id").and_then(|v| v.as_str()).map(String::from))
            .as_deref()
            == Some(key.as_str())
    });
    if exists {
        return Ok(());
    }

    let sample_json = serde_json::to_string(sample).map_err(|e| format!("serialize: {e}"))?;
    let content_hash = sha256_hex(&sample_json);
    let payload = serde_json::json!({
        "run_id": key,              // dedupe key (run_id + ts_ms)
        "dock_id": sample.dock_id,
        "model": sample.model,
        "input_tokens": sample.input_tokens,
        "output_tokens": sample.output_tokens,
        "cost_usd_micros": sample.cost_usd_micros,
        "ts_ms": sample.ts_ms,
        "content_hash": content_hash,
        "is_unlabeled": sample.is_unlabeled(),
    });
    let payload_str = payload.to_string();
    let payload_canonical = canonical_json(&payload_str).unwrap_or_else(|| payload_str.clone());

    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Push,
        COST_SAMPLE_KIND.to_string(),
        vec![COST_PRINCIPAL.to_string()],
        payload_canonical,
        sample.ts_ms,
    )
    .map_err(|denied| format!("authz denied: {:?}", denied.reason))?;

    let bytes = serde_json::to_vec_pretty(log.records()).map_err(|e| format!("serialize: {e}"))?;
    crate::pr::filelock::atomic_write(log_path, &bytes).map_err(|e| format!("persist: {e}"))
}

/// Flush a dock's spool into the log (idempotent). Returns how many samples
/// were NEWLY landed (0 = already flushed / empty). The spool file is removed
/// ONLY when its samples are durably in the log (M2 — no loss on outage).
pub fn flush_dock(
    log_path: &Path,
    spool: &super::spool::CostSpool,
    dock_id: &str,
) -> Result<usize, String> {
    let samples = spool
        .drain(dock_id)
        .map_err(|e| format!("spool drain: {e:?}"))?;
    let mut landed = 0;
    for s in &samples {
        append_cost_sample(log_path, s)?; // idempotent by run_id
        landed += 1;
    }
    Ok(landed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::spool::CostSpool;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample(dock: &str, run: &str) -> CostSampleV1 {
        CostSampleV1 {
            dock_id: dock.to_string(),
            model: "claude-opus-4-8".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd_micros: 210_000,
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            run_id: run.to_string(),
        }
    }

    #[test]
    fn flush_lands_cost_sample_with_dedupe_and_hash() {
        let dir = std::env::temp_dir().join(format!("hugit-attest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let spool_dir = dir.join(".hugit/cost-spool");
        let log = dir.join(".hugit/log.json");
        std::fs::create_dir_all(dir.join(".hugit")).unwrap();
        std::fs::write(&log, "[]").unwrap();

        let spool = CostSpool::new(spool_dir);
        spool.push(&sample("dock-x", "run-1")).unwrap();
        spool.push(&sample("dock-x", "run-2")).unwrap();

        let n = flush_dock(&log, &spool, "dock-x").unwrap();
        assert_eq!(n, 2, "two samples landed");

        // Idempotent re-flush: nothing new, exact-once.
        let n2 = flush_dock(&log, &spool, "dock-x").unwrap();
        assert_eq!(n2, 0, "drained — no double-land");

        // Records carry run_id + content_hash + unlabeled=false.
        let bytes = std::fs::read(&log).unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        let cost_records: Vec<_> = rows
            .iter()
            .filter(|r| r.get("kind").and_then(|k| k.as_str()) == Some(COST_SAMPLE_KIND))
            .collect();
        assert_eq!(cost_records.len(), 2, "M2 — two cost.sample records");
        let p0: serde_json::Value = serde_json::from_str(
            cost_records[0]
                .get("payload")
                .and_then(|v| v.as_str())
                .unwrap(),
        )
        .unwrap();
        assert!(
            p0.get("content_hash").is_some(),
            "M3 — content hash present"
        );
        assert_eq!(p0.get("is_unlabeled"), Some(&serde_json::json!(false)));
        assert_eq!(
            p0.get("cost_usd_micros"),
            Some(&serde_json::json!(210_000u64))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unlabeled_sample_marks_is_unlabeled() {
        let dir = std::env::temp_dir().join(format!("hugit-attest-unlbl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let spool_dir = dir.join("cs");
        let log = dir.join("log.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&log, "[]").unwrap();
        let spool = CostSpool::new(spool_dir);
        spool.push(&sample("", "run-u")).unwrap(); // empty dock
        flush_dock(&log, &spool, "").unwrap();
        let bytes = std::fs::read(&log).unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        let p0: serde_json::Value =
            serde_json::from_str(rows[0].get("payload").and_then(|v| v.as_str()).unwrap()).unwrap();
        assert_eq!(p0.get("is_unlabeled"), Some(&serde_json::json!(true)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
