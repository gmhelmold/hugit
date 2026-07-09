//! GitHub App webhook ingress (WP-1) — mounted PRE-AUTH on the accept loop.
//!
//! `POST /v1/github/webhook` is a FAST-ACK, FAIL-CLOSED door:
//!
//! 1. Verify `X-Hub-Signature-256` (HMAC-SHA256 of the raw body) against the App
//!    webhook secret via the frozen [`hugit_app::WebhookProcessor`].
//! 2. On a valid signature, DURABLY enqueue the [`SignedEventEnvelope`] and return
//!    **202 IMMEDIATELY**. The heavy work (ingest / arbitration / mirror /
//!    git-push) is NEVER run inline — that would starve the single-threaded accept
//!    loop (the DoS surface). This door does verify → enqueue → ack, nothing more.
//! 3. On a bad/absent signature, return **401** and durably append a
//!    `webhook.rejected` audit record (fail-closed audit, whitepaper §9).
//! 4. `installation.deleted` additionally runs [`WebhookProcessor::handle_uninstall`]
//!    then [`PersistenceAdapter::halt_installation`], and durably records an
//!    `installation.revoked` audit record — still fast-ack (202).
//!
//! The App webhook secret is read from the App secret dir
//! (`HUGIT_GITHUB_APP_SECRET_DIR`/`~/.hugit/secrets/github-app-dev`, the same
//! resolution the outbound `AppAuth` loader uses), file `webhook-secret`. When the
//! secret is absent/empty the route is DISABLED (404) — never a half-configured
//! verify that could be forged with a trivial HMAC.
//!
//! The route rides the server's existing bounded-body-read + shed/503 admission
//! model (it is reached via `route_write` after the loop's capped body read), so a
//! webhook flood is shed like any other POST and cannot wedge the thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tiny_http::Header;

use hugit_app::persistence::PersistenceAdapter;
use hugit_app::webhook::{WebhookError, WebhookProcessor, build_ack_receipt};
use hugit_contracts::{EventRecord, SignedEventEnvelope};

use crate::error::EngineErr;

/// A durable, append-only NDJSON journal for the webhook ingress. Each accepted
/// [`SignedEventEnvelope`] is appended (one JSON object per line) to
/// `<dir>/pending.ndjson`; each audit [`EventRecord`] to `<dir>/audit.ndjson`. An
/// append opens in append mode, writes one line, and flushes — durable across a
/// restart (in production the enqueue target is the CoreLink CAS; this on-disk
/// journal is the local-testable durable seam, and it is genuinely fsync-durable).
///
/// The directory is resolved from `HUGIT_GITHUB_WEBHOOK_QUEUE_DIR` (an explicit
/// override; tests point it at a temp dir) or the default
/// `~/.hugit/webhook-queue`. An enqueue failure is surfaced (the caller then 503s
/// rather than falsely ack a lost event) — never swallowed.
#[derive(Debug, Clone)]
pub struct WebhookQueue {
    dir: PathBuf,
}

impl WebhookQueue {
    /// Build a queue rooted at `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Resolve the queue directory from the environment (override → default). The
    /// directory is created lazily on the first append.
    fn from_env() -> Self {
        if let Ok(p) = std::env::var("HUGIT_GITHUB_WEBHOOK_QUEUE_DIR") {
            let p = p.trim().to_string();
            if !p.is_empty() {
                return Self::new(PathBuf::from(p));
            }
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::new(home.join(".hugit").join("webhook-queue"))
    }

    /// Append one NDJSON line (`value` + `\n`) to `<dir>/<file>`, creating the
    /// directory if needed. `sync` controls the durability barrier: `true` fsyncs
    /// (`sync_all`) so the caller may safely ack; `false` skips the fsync (an
    /// OS-buffered append) — used for BEST-EFFORT audit records so an
    /// unauthenticated flood on the PRE-AUTH reject path can never force a blocking
    /// per-request fsync on the single-threaded accept loop (the audit DoS the
    /// convergence re-audit flagged). Fail-closed either way: any I/O error is
    /// returned so a durability-required caller does not ack a non-durable event.
    fn append_line(
        &self,
        file: &str,
        value: &serde_json::Value,
        sync: bool,
    ) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(&self.dir)?;
        let mut line = serde_json::to_string(value).map_err(std::io::Error::other)?;
        line.push('\n');
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(file))?;
        f.write_all(line.as_bytes())?;
        f.flush()?;
        if sync {
            f.sync_all()?;
        }
        Ok(())
    }

    /// Durably enqueue an accepted envelope (fsync'd — the caller acks 202 only
    /// after this returns `Ok`). Returns `Err` (→ the caller 503s, GitHub retries)
    /// if the durable write fails. This is the ONLY webhook path that fsyncs; it is
    /// reached only AFTER a valid HMAC + the per-principal rate limit.
    pub fn enqueue_envelope(&self, env: &SignedEventEnvelope) -> std::io::Result<()> {
        let value = serde_json::to_value(env).map_err(std::io::Error::other)?;
        self.append_line("pending.ndjson", &value, true)
    }

    /// Append an audit record (`webhook.rejected` / `installation.revoked`) WITHOUT
    /// an fsync barrier — best-effort at the call site (a 401/202 is returned
    /// regardless), so a rejected request never blocks the accept loop on disk. A
    /// crash may lose the last few OS-buffered audit lines; that is acceptable for a
    /// monitoring record of (often unauthenticated) rejected attempts, and it
    /// removes the pre-auth per-request fsync DoS. Still fail-honest: returns `Err`
    /// rather than pretend it persisted.
    pub fn append_audit(&self, rec: &EventRecord) -> std::io::Result<()> {
        let value = serde_json::to_value(rec).map_err(std::io::Error::other)?;
        self.append_line("audit.ndjson", &value, false)
    }
}

/// The mounted webhook ingress: the shared [`WebhookProcessor`] (token store +
/// HMAC secret), the [`PersistenceAdapter`] halt gate, the durable [`WebhookQueue`],
/// and a best-effort in-process audit chain (seq + prev_hash). Held once per engine
/// lifetime behind [`ingress`]; unit-tested directly by constructing an instance.
pub struct WebhookIngress {
    processor: WebhookProcessor,
    persistence: Mutex<PersistenceAdapter>,
    queue: WebhookQueue,
    /// Monotonic audit sequence (in-process; resets on restart — the durable
    /// journal keeps the records, only the counter is per-lifetime).
    audit_seq: AtomicU64,
    /// The previous audit record's `this_hash` (genesis = ""), for chaining.
    audit_prev: Mutex<String>,
}

impl WebhookIngress {
    /// Construct an ingress with an explicit secret + durable queue (the test
    /// constructor; production goes through [`ingress`]).
    pub fn new(secret: impl Into<Vec<u8>>, queue: WebhookQueue) -> Self {
        Self {
            processor: WebhookProcessor::new(secret),
            persistence: Mutex::new(PersistenceAdapter::new_local()),
            queue,
            audit_seq: AtomicU64::new(0),
            audit_prev: Mutex::new(String::new()),
        }
    }

    /// Take the next `(seq, prev_hash)` for an audit record.
    fn next_audit_coords(&self) -> (u64, String) {
        let seq = self.audit_seq.fetch_add(1, Ordering::SeqCst);
        let prev = self
            .audit_prev
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        (seq, prev)
    }

    /// Commit an audit record's hash as the new chain tip and durably append it.
    fn commit_audit(&self, rec: &EventRecord) {
        *self.audit_prev.lock().unwrap_or_else(|e| e.into_inner()) = rec.this_hash.clone();
        // Fail-honest but non-fatal for the request: a failed audit append is
        // logged, but a rejected request still returns 401 / an accepted one 202.
        if let Err(e) = self.queue.append_audit(rec) {
            eprintln!(
                "hugit-serve: webhook audit append failed (non-fatal): {}",
                e.kind()
            );
        }
    }

    /// Process one raw webhook request. Pure w.r.t. the socket (I/O only via the
    /// durable queue): verify → (uninstall halt) → durably enqueue → ack. NEVER
    /// runs ingest/mirror/git-push inline.
    ///
    /// Returns the `(status, json_body)` the door writes: 401 (bad/absent sig),
    /// 202 (accepted + durably enqueued), or 503 (the durable enqueue failed —
    /// GitHub retries; we never falsely ack a lost event).
    pub fn process_raw(
        &self,
        body: &[u8],
        signature_header: Option<&str>,
        delivery_id: &str,
        event_type: &str,
        now_ms: u64,
    ) -> (u16, String) {
        match self
            .processor
            .process(body, signature_header, delivery_id, event_type, now_ms)
        {
            Err(WebhookError::SignatureMismatch | WebhookError::MissingSignature) => {
                // Fail-closed audit: build + durably record `webhook.rejected`, 401.
                let (seq, prev) = self.next_audit_coords();
                let rec = self
                    .processor
                    .rejected_record(delivery_id, &prev, seq, now_ms);
                self.commit_audit(&rec);
                let e = EngineErr::unauthorized("assinatura de webhook inválida ou ausente");
                (e.status, e.to_body())
            }
            Err(WebhookError::PayloadParse(_)) => {
                // A non-UTF-8 payload can't be a valid GitHub webhook — treat as a
                // bad request (never enqueue undecodable bytes).
                let e = EngineErr::invalid_request("payload de webhook inválido");
                (e.status, e.to_body())
            }
            Ok(envelope) => {
                // `installation.deleted` → revoke + halt + durable audit, still 202.
                if event_type == "installation"
                    && let Some("deleted") = payload_action(body)
                    && let Some(iid) = payload_installation_id(body)
                {
                    let (seq, prev) = self.next_audit_coords();
                    let (_outcome, rec) = self.processor.handle_uninstall(&iid, &prev, seq, now_ms);
                    self.persistence
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .halt_installation(&iid);
                    self.commit_audit(&rec);
                }
                // DURABLY enqueue BEFORE ack — fail-closed: a failed durable write
                // returns 503 (GitHub retries) rather than a false 202.
                match self.queue.enqueue_envelope(&envelope) {
                    Ok(()) => {
                        let ack = build_ack_receipt(delivery_id, now_ms);
                        let body = serde_json::to_string(&ack)
                            .unwrap_or_else(|_| r#"{"accepted":true}"#.to_string());
                        (202, body)
                    }
                    Err(_) => {
                        let e = EngineErr::unavailable(
                            "não foi possível enfileirar o webhook de forma durável",
                        );
                        (e.status, e.to_body())
                    }
                }
            }
        }
    }

    /// Whether a given installation has been halted (test/inspection hook).
    #[cfg(test)]
    fn is_halted(&self, installation_id: &str) -> bool {
        // `persist_event` rejects a halted installation; probe with a throwaway.
        let probe = SignedEventEnvelope {
            delivery_id: "probe".to_string(),
            event_type: "ping".to_string(),
            signature: "sha256=probe".to_string(),
            payload: "{}".to_string(),
            received_at: 0,
        };
        self.persistence
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .persist_event(&probe, Some(installation_id))
            .is_err()
    }
}

/// Resolve the App webhook secret bytes from the App secret dir (the same
/// resolution `AppAuth::resolve_secret_dir` uses): `HUGIT_GITHUB_APP_SECRET_DIR`
/// override, else `~/.hugit/secrets/github-app-dev`, file `webhook-secret`. Returns
/// `None` when the dir/file is absent or the secret is empty — the route is then
/// DISABLED (404), never a forgeable half-config.
fn resolve_webhook_secret() -> Option<Vec<u8>> {
    // Container/env secret path (takes precedence): the DEPLOYED engine has no on-disk
    // secret dir — the App webhook secret arrives as a wrangler-forwarded env var. A
    // non-empty `HUGIT_GITHUB_APP_WEBHOOK_SECRET` is used directly; empty/absent falls
    // through to the on-disk dir (the dev/local path). Fail-closed either way.
    if let Ok(s) = std::env::var("HUGIT_GITHUB_APP_WEBHOOK_SECRET") {
        let t = s.trim();
        if !t.is_empty() {
            return Some(t.as_bytes().to_vec());
        }
    }
    let dir = if let Ok(p) = std::env::var("HUGIT_GITHUB_APP_SECRET_DIR") {
        let p = p.trim().to_string();
        if p.is_empty() {
            default_app_secret_dir()?
        } else {
            PathBuf::from(p)
        }
    } else {
        default_app_secret_dir()?
    };
    let raw = std::fs::read(dir.join("webhook-secret")).ok()?;
    // Trim a trailing newline (a secret file is usually `echo`d); reject empty.
    let trimmed: Vec<u8> = {
        let mut v = raw;
        while matches!(v.last(), Some(b'\n' | b'\r')) {
            v.pop();
        }
        v
    };
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// `~/.hugit/secrets/github-app-dev` from `$HOME` (never a hardcoded literal).
fn default_app_secret_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".hugit")
            .join("secrets")
            .join("github-app-dev")
    })
}

/// The process-wide ingress, resolved once. `None` when no webhook secret is
/// configured → the route is disabled (404). The engine is single-instance +
/// single-threaded, so a single shared ingress (its token store + halt set) is the
/// correct lifetime for cross-request state; the durable journal survives restart.
fn ingress() -> Option<&'static WebhookIngress> {
    static INGRESS: OnceLock<Option<WebhookIngress>> = OnceLock::new();
    INGRESS
        .get_or_init(|| {
            resolve_webhook_secret()
                .map(|secret| WebhookIngress::new(secret, WebhookQueue::from_env()))
        })
        .as_ref()
}

/// Case-insensitive header lookup (local; mirrors `server::header_val`).
fn header_value(headers: &[Header], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

/// Extract the top-level string `action` from a raw JSON webhook payload.
fn payload_action(body: &[u8]) -> Option<&'static str> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    match v.get("action").and_then(|a| a.as_str()) {
        Some("deleted") => Some("deleted"),
        _ => None,
    }
}

/// Extract `installation.id` (GitHub sends it as a number) as a string.
fn payload_installation_id(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let id = v.get("installation")?.get("id")?;
    id.as_u64()
        .map(|n| n.to_string())
        .or_else(|| id.as_str().map(|s| s.to_string()))
}

/// The route entry point (`POST /v1/github/webhook`). Resolves the mounted
/// ingress; when no secret is configured the route is DISABLED → 404 (no
/// existence oracle, consistent with the rest of the door). Otherwise verifies +
/// enqueues + acks. Socket-free `(status, body)` so it rides `route_write`.
pub fn handle(headers: &[Header], body: &[u8]) -> (u16, String) {
    let Some(ingress) = ingress() else {
        let e = EngineErr::not_found();
        return (e.status, e.to_body());
    };
    let signature = header_value(headers, "X-Hub-Signature-256");
    let delivery_id = header_value(headers, "X-GitHub-Delivery").unwrap_or_default();
    let event_type = header_value(headers, "X-GitHub-Event").unwrap_or_default();
    ingress.process_raw(
        body,
        signature.as_deref(),
        &delivery_id,
        &event_type,
        now_ms(),
    )
}

/// Wall-clock epoch-ms (best-effort).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    const SECRET: &[u8] = b"webhook-secret-do-not-leak";

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn tmp_queue(tag: &str) -> WebhookQueue {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "hugit-webhook-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        WebhookQueue::new(p)
    }

    #[test]
    fn valid_hmac_is_202_and_durably_enqueued() {
        let q = tmp_queue("valid");
        let ingress = WebhookIngress::new(SECRET, q.clone());
        let body = br#"{"action":"opened","number":1}"#;
        let sig = sign(SECRET, body);
        let (status, _) = ingress.process_raw(body, Some(&sig), "delivery-1", "pull_request", 42);
        assert_eq!(status, 202);
        // The envelope was durably written to pending.ndjson.
        let pending = std::fs::read_to_string(q.dir.join("pending.ndjson")).unwrap();
        assert!(pending.contains("delivery-1"));
        assert!(pending.contains("pull_request"));
        std::fs::remove_dir_all(&q.dir).ok();
    }

    #[test]
    fn bad_signature_is_401_and_writes_rejected_audit() {
        let q = tmp_queue("bad");
        let ingress = WebhookIngress::new(SECRET, q.clone());
        let body = br#"{"action":"opened"}"#;
        let (status, _) = ingress.process_raw(
            body,
            Some("sha256=deadbeef"),
            "delivery-2",
            "pull_request",
            7,
        );
        assert_eq!(status, 401);
        // A `webhook.rejected` audit record was durably appended.
        let audit = std::fs::read_to_string(q.dir.join("audit.ndjson")).unwrap();
        assert!(audit.contains("webhook.rejected"));
        // Nothing was enqueued (fail-closed).
        assert!(!q.dir.join("pending.ndjson").exists());
        std::fs::remove_dir_all(&q.dir).ok();
    }

    #[test]
    fn absent_signature_is_401() {
        let q = tmp_queue("absent");
        let ingress = WebhookIngress::new(SECRET, q.clone());
        let (status, _) = ingress.process_raw(b"{}", None, "d", "ping", 1);
        assert_eq!(status, 401);
        std::fs::remove_dir_all(&q.dir).ok();
    }

    #[test]
    fn installation_deleted_halts_and_records_revoked() {
        let q = tmp_queue("uninstall");
        let ingress = WebhookIngress::new(SECRET, q.clone());
        let body = br#"{"action":"deleted","installation":{"id":424242}}"#;
        let sig = sign(SECRET, body);
        let (status, _) = ingress.process_raw(body, Some(&sig), "delivery-3", "installation", 99);
        // Fast-ack even on the uninstall path.
        assert_eq!(status, 202);
        // The installation is halted (durable-state gate flipped).
        assert!(ingress.is_halted("424242"));
        // An `installation.revoked` audit record was durably appended.
        let audit = std::fs::read_to_string(q.dir.join("audit.ndjson")).unwrap();
        assert!(audit.contains("installation.revoked"));
        std::fs::remove_dir_all(&q.dir).ok();
    }

    #[test]
    fn empty_secret_processor_fails_closed() {
        // Defense-in-depth: even if an empty secret reached the processor (the
        // route resolver already 404s that config), an empty secret rejects.
        let q = tmp_queue("emptysecret");
        let ingress = WebhookIngress::new(Vec::<u8>::new(), q.clone());
        let body = br#"{"action":"opened"}"#;
        let sig = sign(b"", body); // an attacker-forgeable HMAC over empty key
        let (status, _) = ingress.process_raw(body, Some(&sig), "d", "pull_request", 1);
        assert_eq!(status, 401);
        std::fs::remove_dir_all(&q.dir).ok();
    }

    #[test]
    fn audit_chain_links_prev_hash() {
        let q = tmp_queue("chain");
        let ingress = WebhookIngress::new(SECRET, q.clone());
        // Two rejects — the second record's prev_hash must equal the first's hash.
        ingress.process_raw(b"{}", Some("sha256=00"), "d1", "pull_request", 1);
        ingress.process_raw(b"{}", Some("sha256=00"), "d2", "pull_request", 2);
        let audit = std::fs::read_to_string(q.dir.join("audit.ndjson")).unwrap();
        let recs: Vec<EventRecord> = audit
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].seq, 0);
        assert_eq!(recs[1].seq, 1);
        assert_eq!(recs[1].prev_hash, recs[0].this_hash);
        std::fs::remove_dir_all(&q.dir).ok();
    }
}
