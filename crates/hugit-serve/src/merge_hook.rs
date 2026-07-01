//! KungFu merge-event webhook (WP W-WEBHOOK, pilot event #72).
//!
//! On a SUCCESSFUL `git-receive-pack` finalize (create / update / delete) the
//! engine emits a signed [`KungFuMergeEvent`] to a configured KungFu webhook. This
//! module owns the WHOLE egress: the frozen event shape, the base→head tree-diff
//! that fills `touched_paths`, HMAC signing, the SSRF-safe outbound POST, and the
//! bounded off-loop delivery queue. The finalize call site (`git.rs`) does exactly
//! one thing: call [`emit_merge_event`] AFTER the push is durable and the `ok` has
//! been written to the client.
//!
//! ## Non-negotiable invariants (this is outbound egress on the LIVE write path)
//!
//! 1. **Post-durable, never regresses the push.** [`emit_merge_event`] is called
//!    only after `finalize_cas_push` / `finalize_cas_delete` returned `Ok` AND the
//!    `report-status` `ok` was already flushed to the client. A hook fault (bad
//!    config, full queue, hanging endpoint, SSRF refusal) can NEVER change the push
//!    result — the client already has its `ok`.
//! 2. **Off the accept loop.** The network egress (the hang-prone, retrying part)
//!    runs on a dedicated worker thread draining a BOUNDED in-memory queue. Enqueue
//!    is non-blocking: a full queue DROPS THE OLDEST pending delivery and logs a
//!    running counter — it never blocks the single-threaded accept loop.
//!    (The one on-loop cost is the `touched_paths` tree-diff, which is WALL-CLOCK
//!    bounded — see [`HOOK_DIFF_BUDGET`] — and runs only AFTER the client has `ok`,
//!    so it delays at most the NEXT accept, never this push, and can never wedge.)
//! 3. **SSRF-safe.** The webhook URL is operator config. It MUST be `https://` and
//!    MUST resolve to a PUBLIC address — loopback / link-local (incl. the
//!    `169.254.169.254` cloud-metadata IP) / RFC-1918 / CGNAT / ULA / unspecified /
//!    multicast / documentation ranges are all REFUSED. The check is enforced by a
//!    custom `ureq` [`PublicOnlyResolver`] so `ureq` connects to EXACTLY the
//!    validated addresses (closing the DNS-rebind TOCTOU window — see its docs), and
//!    redirects are disabled (a 3xx is refused, never followed to any target).
//! 4. **HMAC-signed, secret never logged.** The canonical event body is signed with
//!    `HUGIT_KUNGFU_WEBHOOK_SECRET` (HMAC-SHA256, `X-Hugit-Signature: sha256=<hex>`).
//!    The secret lives only in [`Config`] (redacted in `Debug`), never in a queue
//!    item, never in a log line.
//! 5. **Fail-closed config.** Unset URL → the feature is OFF and every emit is a
//!    pure no-op (a missing config NEVER errors a push). URL set but not `https://`,
//!    or the secret missing → DISABLED (we never emit an unsigned or plaintext
//!    event); a one-line warning, then no-op.
//! 6. **At-least-once with idempotency.** Delivery retries a bounded number of times
//!    with backoff; the receiver de-dups on the `X-Hugit-Idempotency-Key`
//!    (`(repo, head_sha)`). The queue is in-memory, so "at-least-once" is bounded to
//!    the process lifetime — an undelivered event is lost on an engine restart
//!    (best-effort, decoupled, as specified).

use std::collections::{HashSet, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use hugit_proto::ObjectSource;
use hugit_proto::read::pack::{MAX_DIFF_FILES, tree_diff_until};
use serde::Serialize;
use sha2::Sha256;

// ─────────────────────────── tunables ───────────────────────────

/// Bounded capacity of the in-memory delivery queue. A burst of pushes past this
/// drops the OLDEST pending delivery (never blocks the push); low, because one
/// undelivered merge event is best-effort and the receiver can reconcile.
const QUEUE_CAPACITY: usize = 1024;

/// WALL-CLOCK ceiling on the `touched_paths` tree-diff. Runs on the accept loop
/// (post-`ok`), so it MUST be bounded — an unbounded cold-cache diff of a large
/// commit would occupy the single-threaded loop (the read-latency-DoS class). Kept
/// below `hugit_proto::DIFF_BUDGET` (2 s): the diff is informational, a partial /
/// truncated result is honestly marked in `cv_hint`.
const HOOK_DIFF_BUDGET: Duration = Duration::from_millis(1_500);

/// Max `touched_paths` emitted. Mirrors the `tree_diff` row cap so the event body
/// is bounded regardless of diff size.
const MAX_TOUCHED_PATHS: usize = MAX_DIFF_FILES;

/// Bounded delivery attempts per event (at-least-once, not at-any-cost). After
/// this many transient failures the event is dropped with a log line.
const MAX_DELIVERY_ATTEMPTS: u32 = 4;

/// Outbound connect / overall request timeouts. A slow/hanging endpoint must not
/// pin the worker thread indefinitely.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Marker embedded in the SSRF refusal `io::Error` so the delivery layer can tell a
/// policy refusal (permanent, do NOT retry) from a transient network error.
const SSRF_ERR_MARKER: &str = "hugit-merge-hook-ssrf-blocked";

// `cv_hint` values — an honest hint about how `touched_paths` was derived. The
// frozen event shape has no dedicated "truncated" field, so completeness is
// conveyed here (a defined use of the free-form hint field; see the return card).
const CV_FULL: &str = "diff:full";
const CV_TRUNCATED_CAP: &str = "diff:truncated-cap";
const CV_TRUNCATED_DEADLINE: &str = "diff:truncated-deadline";
const CV_UNAVAILABLE: &str = "diff:unavailable";
const CV_BRANCH_CREATE: &str = "branch-create";
const CV_REF_DELETE: &str = "ref-delete";

// ─────────────────────────── the frozen event ───────────────────────────

/// The pilot merge event (#72, owner-approved). FROZEN shape — field order is
/// load-bearing: it defines the canonical JSON bytes that are HMAC-signed and sent.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KungFuMergeEvent {
    /// Repo slug the push landed on.
    pub repo: String,
    /// The updated ref (e.g. `refs/heads/main`). `ref` is a keyword → `r#ref`.
    #[serde(rename = "ref")]
    pub r#ref: String,
    /// The pre-image tip (all-zero on a branch CREATE).
    pub base_sha: String,
    /// The post-image tip (all-zero on a branch DELETE).
    pub head_sha: String,
    /// Files changed base→head (bounded; see `cv_hint` for completeness).
    pub touched_paths: Vec<String>,
    /// Unix epoch milliseconds the push finalized.
    pub landed_at: u64,
    /// The pusher — the first element of the authenticated principal chain.
    pub actor: String,
    /// Derivation hint for `touched_paths` (`diff:full` / `diff:truncated-*` /
    /// `diff:unavailable` / `branch-create` / `ref-delete`).
    pub cv_hint: String,
}

// ─────────────────────────── touched_paths (base→head diff) ───────────────────────────

fn is_zero_oid(sha: &str) -> bool {
    !sha.is_empty() && sha.bytes().all(|b| b == b'0')
}

fn parse_oid(sha: &str) -> Option<gix_hash::ObjectId> {
    gix_hash::ObjectId::from_hex(sha.as_bytes()).ok()
}

/// Compute `touched_paths` for a finalized push via a base→head tree-diff, bounded
/// by [`HOOK_DIFF_BUDGET`]. Honest by construction:
/// * DELETE (`head` all-zero) → empty + `ref-delete` (the ref itself is the change;
///   we do NOT walk the whole deleted tree — an unbounded cost for no signal).
/// * CREATE (`base` all-zero) → empty + `branch-create` (no base to diff against;
///   enumerating the entire new tree is an unbounded full-tree walk).
/// * UPDATE → the real base→head diff. Truncation (row cap or deadline) is marked
///   in `cv_hint`. Any unresolvable oid / tree / source fault → empty + `unavailable`
///   (fail-closed; never a fabricated or partial-as-if-complete list).
pub fn compute_touched_paths(
    src: &dyn ObjectSource,
    base_sha: &str,
    head_sha: &str,
) -> (Vec<String>, String) {
    use hugit_proto::commit_root_tree;

    if is_zero_oid(head_sha) {
        return (Vec::new(), CV_REF_DELETE.to_string());
    }
    if is_zero_oid(base_sha) {
        return (Vec::new(), CV_BRANCH_CREATE.to_string());
    }
    let (Some(base), Some(head)) = (parse_oid(base_sha), parse_oid(head_sha)) else {
        return (Vec::new(), CV_UNAVAILABLE.to_string());
    };
    let base_tree = match commit_root_tree(src, &base) {
        Ok(Some(t)) => t,
        _ => return (Vec::new(), CV_UNAVAILABLE.to_string()),
    };
    let head_tree = match commit_root_tree(src, &head) {
        Ok(Some(t)) => t,
        _ => return (Vec::new(), CV_UNAVAILABLE.to_string()),
    };

    let started = Instant::now();
    let diffs = match tree_diff_until(src, &base_tree, &head_tree, started + HOOK_DIFF_BUDGET) {
        Ok(d) => d,
        Err(_) => return (Vec::new(), CV_UNAVAILABLE.to_string()),
    };
    // Truncation classification. The row cap is authoritative (`tree_diff` caps at
    // `MAX_DIFF_FILES`); the deadline is heuristic (the diff API returns the partial
    // result silently, so we infer a deadline hit from elapsed wall-clock).
    let capped = diffs.len() >= MAX_DIFF_FILES;
    let deadline_hit = started.elapsed() >= HOOK_DIFF_BUDGET;
    let mut paths: Vec<String> = diffs.into_iter().map(|f| f.path).collect();
    paths.truncate(MAX_TOUCHED_PATHS);
    let cv = if capped {
        CV_TRUNCATED_CAP
    } else if deadline_hit {
        CV_TRUNCATED_DEADLINE
    } else {
        CV_FULL
    };
    (paths, cv.to_string())
}

// ─────────────────────────── signing + idempotency ───────────────────────────

type HmacSha256 = Hmac<Sha256>;

/// HMAC-SHA256 of `body` under `secret`, lowercase hex. The signed material is the
/// EXACT canonical body bytes that are POSTed.
fn sign(secret: &[u8], body: &[u8]) -> String {
    // `new_from_slice` is infallible for HMAC (any key length is accepted).
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Idempotency key `(repo, head_sha)` — the receiver de-dups on it under
/// at-least-once delivery. NUL-joined so the two components can't be confused.
fn idempotency_key(repo: &str, head_sha: &str) -> String {
    format!("{repo}\u{0}{head_sha}")
}

// ─────────────────────────── SSRF guard ───────────────────────────

/// Is `ip` a GLOBALLY-ROUTABLE (public) address? Conservative denylist — anything
/// not provably public is refused (fail-closed). `std`'s `is_global` is unstable,
/// so the ranges are enumerated explicitly. Covers the cloud-metadata endpoint
/// (`169.254.169.254` ∈ link-local) and its IPv6 analogues (link-local / ULA).
pub fn is_global_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_global_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) reaches the SAME host as
            // the bare v4 — validate under the v4 rules, never as an opaque v6.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_global_v4(&v4);
            }
            is_global_v6(v6)
        }
    }
}

fn is_global_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    if ip.is_unspecified()          // 0.0.0.0
        || ip.is_loopback()          // 127.0.0.0/8
        || ip.is_private()           // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local()        // 169.254.0.0/16  (incl. 169.254.169.254 metadata)
        || ip.is_broadcast()         // 255.255.255.255
        || ip.is_multicast()         // 224.0.0.0/4
        || ip.is_documentation()
    // 192.0.2/24, 198.51.100/24, 203.0.113/24
    {
        return false;
    }
    // Ranges `std` has no predicate for:
    if o[0] == 0 {
        return false; // 0.0.0.0/8  "this network"
    }
    if o[0] == 100 && (64..=127).contains(&o[1]) {
        return false; // 100.64.0.0/10  CGNAT / shared address space
    }
    if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        return false; // 192.0.0.0/24  IETF protocol assignments
    }
    if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
        return false; // 198.18.0.0/15  benchmarking
    }
    true
}

fn is_global_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return false; // ::, ::1, ff00::/8
    }
    let seg = ip.segments();
    if (seg[0] & 0xfe00) == 0xfc00 {
        return false; // fc00::/7  unique-local
    }
    if (seg[0] & 0xffc0) == 0xfe80 {
        return false; // fe80::/10 link-local
    }
    if seg[0] == 0x2001 && seg[1] == 0x0db8 {
        return false; // 2001:db8::/32 documentation
    }

    // ── transitional / IPv4-embedding ranges ──────────────────────────────────
    // A v6 that carries an embedded IPv4 reaches the SAME host as that v4 (e.g.
    // `::7f00:1` is loopback, a 6to4 wrapper of `192.168.1.1` is that LAN host), so
    // the embedded v4 MUST be extracted and re-checked under the v4 rules — a strict
    // SSRF allowlist (like std's unstable `is_global`) rejects all of these.

    // Teredo 2001:0000::/32 — the client v4 lives in the low bits under an XOR
    // obfuscation; rather than trust a reversible unwrap, reject the whole range.
    if seg[0] == 0x2001 && seg[1] == 0x0000 {
        return false;
    }
    // 6to4 2002::/16 — the embedded IPv4 is bits 16–48 (`seg[1]:seg[2]`).
    if seg[0] == 0x2002 {
        return is_global_v4(&embedded_v4(seg[1], seg[2]));
    }
    // NAT64 well-known prefix 64:ff9b::/96 — the embedded IPv4 is the low 32 bits.
    if seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2..6] == [0, 0, 0, 0] {
        return is_global_v4(&embedded_v4(seg[6], seg[7]));
    }
    // IPv4-compatible (deprecated) `::a.b.c.d` — high 96 bits zero, low 32 = v4.
    // (`::` and `::1` are already handled by the unspecified/loopback checks.)
    if seg[..6] == [0, 0, 0, 0, 0, 0] {
        return is_global_v4(&embedded_v4(seg[6], seg[7]));
    }

    true
}

/// Reassemble the IPv4 address embedded in two consecutive IPv6 segments
/// (`hi`=high 16 bits, `lo`=low 16 bits of the v4).
fn embedded_v4(hi: u16, lo: u16) -> Ipv4Addr {
    Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8)
}

/// The SSRF gate, installed as `ureq`'s DNS resolver. `ureq` calls `resolve` with
/// the connection's `host:port`, then connects to EXACTLY the `SocketAddr`s we
/// return — so validating here and returning only public addresses both enforces
/// the policy AND pins the resolution (there is no second, unchecked re-resolve at
/// connect time → the DNS-rebind TOCTOU window is closed). If ANY resolved address
/// is non-public the whole netloc is refused (a mixed public/private answer can't
/// slip a private target through).
struct PublicOnlyResolver;

impl ureq::Resolver for PublicOnlyResolver {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        let addrs: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        if addrs.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{SSRF_ERR_MARKER}: {netloc} resolved to no addresses"),
            ));
        }
        for a in &addrs {
            if !is_global_ip(&a.ip()) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "{SSRF_ERR_MARKER}: {netloc} resolves to non-public address {}",
                        a.ip()
                    ),
                ));
            }
        }
        Ok(addrs)
    }
}

/// `https://`-only fast reject at config time (the resolver is the authoritative
/// address gate; this is a cheap scheme guard). A `@` in the authority (userinfo)
/// is refused — it is an SSRF-confusion smell and hugit never needs credentials in
/// the URL.
fn validate_webhook_url(url: &str) -> Result<(), &'static str> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("webhook URL must be https://");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err("webhook URL has no host");
    }
    if authority.contains('@') {
        return Err("webhook URL must not contain userinfo (@)");
    }
    Ok(())
}

// ─────────────────────────── delivery sink (egress) ───────────────────────────

/// Why a single delivery attempt failed — governs retry.
#[derive(Debug)]
enum DeliveryError {
    /// The SSRF guard refused the target. PERMANENT — never retry.
    Ssrf(String),
    /// A non-2xx, non-5xx status (4xx, or a refused 3xx redirect). PERMANENT.
    Client(u16),
    /// A 5xx or a transport/timeout error. Transient — retry with backoff.
    Transient(String),
}

/// The egress abstraction, injectable so delivery is testable without a network.
trait EventSink: Send + Sync {
    fn deliver(
        &self,
        url: &str,
        body: &[u8],
        signature: &str,
        idempotency_key: &str,
    ) -> Result<(), DeliveryError>;
}

/// The real HTTPS sink: a per-delivery `ureq` agent pinned to the SSRF resolver,
/// with redirects disabled and bounded timeouts.
struct HttpEventSink;

impl EventSink for HttpEventSink {
    fn deliver(
        &self,
        url: &str,
        body: &[u8],
        signature: &str,
        idempotency_key: &str,
    ) -> Result<(), DeliveryError> {
        let agent = ureq::AgentBuilder::new()
            .resolver(PublicOnlyResolver)
            .redirects(0)
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build();

        let result = agent
            .post(url)
            .set("Content-Type", "application/json")
            .set("User-Agent", "hugit-merge-hook/1")
            .set("X-Hugit-Event", "kungfu.merge")
            .set("X-Hugit-Signature", &format!("sha256={signature}"))
            .set("X-Hugit-Idempotency-Key", idempotency_key)
            .send_bytes(body);

        // Normalize to a status code. `ureq` returns `Ok` for <400 (incl. a 3xx that
        // was NOT followed because redirects are disabled) and `Err(Status)` for
        // >=400; a transport/DNS/SSRF error is `Err(Transport)`.
        let status = match result {
            Ok(resp) => resp.status(),
            Err(ureq::Error::Status(code, _)) => code,
            Err(ureq::Error::Transport(t)) => {
                let msg = t.to_string();
                return Err(if msg.contains(SSRF_ERR_MARKER) {
                    DeliveryError::Ssrf(msg)
                } else {
                    DeliveryError::Transient(msg)
                });
            }
        };
        if (200..300).contains(&status) {
            Ok(())
        } else if (500..600).contains(&status) {
            Err(DeliveryError::Transient(format!(
                "endpoint status {status}"
            )))
        } else {
            // 4xx, or a 3xx redirect we refuse to follow.
            Err(DeliveryError::Client(status))
        }
    }
}

// ─────────────────────────── config ───────────────────────────

/// Validated, enabled configuration. The secret is redacted in `Debug`.
struct Config {
    url: String,
    secret: Vec<u8>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// Build a [`Config`] from raw values (env-decoupled for testing). Returns `None`
/// (feature OFF) when: the URL is unset/blank; the URL is not a valid `https://`
/// URL; or the secret is unset/blank (we never emit unsigned). Each disabling
/// reason logs one line; a `None` is always a NO-OP, never a push error.
fn config_from_values(url: Option<String>, secret: Option<String>) -> Option<Config> {
    let url = url
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    if let Err(reason) = validate_webhook_url(&url) {
        eprintln!("hugit-serve: merge-hook DISABLED — {reason}");
        return None;
    }
    let secret = match secret.filter(|s| !s.is_empty()) {
        Some(s) => s.into_bytes(),
        None => {
            eprintln!(
                "hugit-serve: merge-hook DISABLED — HUGIT_KUNGFU_WEBHOOK_URL set but \
                 HUGIT_KUNGFU_WEBHOOK_SECRET missing (never emit unsigned)"
            );
            return None;
        }
    };
    Some(Config { url, secret })
}

// ─────────────────────────── the hook (queue + worker) ───────────────────────────

struct PendingDelivery {
    idempotency_key: String,
    body: Vec<u8>,
    signature: String,
}

struct QueueState {
    items: VecDeque<PendingDelivery>,
    /// Idempotency keys currently queued — collapse a duplicate `(repo, head_sha)`
    /// enqueue while one is still pending (reduces redundant deliveries; delivery is
    /// still at-least-once via the header).
    pending_keys: HashSet<String>,
    dropped: u64,
    shutdown: bool,
}

struct Shared {
    config: Config,
    transport: Box<dyn EventSink>,
    queue: Mutex<QueueState>,
    cv: Condvar,
}

/// The merge-event hook: a bounded queue + a drain worker. Cheap to hold; cloning
/// is not needed (a single instance lives in the process-global [`GLOBAL`]).
pub struct MergeHook {
    inner: Arc<Shared>,
}

impl MergeHook {
    fn build(config: Config, transport: Box<dyn EventSink>, spawn_worker: bool) -> MergeHook {
        let inner = Arc::new(Shared {
            config,
            transport,
            queue: Mutex::new(QueueState {
                items: VecDeque::new(),
                pending_keys: HashSet::new(),
                dropped: 0,
                shutdown: false,
            }),
            cv: Condvar::new(),
        });
        if spawn_worker {
            let worker = Arc::clone(&inner);
            // A failure to spawn the worker is non-fatal: enqueue still succeeds
            // (bounded), it just won't drain — best-effort, never a push regression.
            let _ = std::thread::Builder::new()
                .name("hugit-merge-hook".to_string())
                .spawn(move || run_worker(&worker));
        }
        MergeHook { inner }
    }

    /// Non-blocking enqueue. Serializes + signs the event, then pushes onto the
    /// bounded queue (drop-oldest if full, dedup a still-pending duplicate). Returns
    /// immediately regardless of endpoint health — the whole point of the queue.
    pub fn enqueue(&self, event: &KungFuMergeEvent) {
        let body = match serde_json::to_vec(event) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("hugit-serve: merge-hook — event serialize failed, dropped: {e}");
                return;
            }
        };
        let key = idempotency_key(&event.repo, &event.head_sha);
        let signature = sign(&self.inner.config.secret, &body);
        let item = PendingDelivery {
            idempotency_key: key.clone(),
            body,
            signature,
        };

        let mut q = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.shutdown {
            return;
        }
        if q.pending_keys.contains(&key) {
            return; // an identical (repo, head_sha) is already pending → collapse
        }
        if q.items.len() >= QUEUE_CAPACITY
            && let Some(old) = q.items.pop_front()
        {
            q.pending_keys.remove(&old.idempotency_key);
            q.dropped += 1;
            eprintln!(
                "hugit-serve: merge-hook — queue full (cap {QUEUE_CAPACITY}), dropped oldest \
                 (total dropped {})",
                q.dropped
            );
        }
        q.pending_keys.insert(key);
        q.items.push_back(item);
        drop(q);
        self.inner.cv.notify_one();
    }

    /// Test/diagnostic: `(queued_len, total_dropped)`.
    #[cfg(test)]
    fn queue_stats(&self) -> (usize, u64) {
        let q = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
        (q.items.len(), q.dropped)
    }
}

fn run_worker(shared: &Arc<Shared>) {
    loop {
        let item = {
            let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(it) = q.items.pop_front() {
                    q.pending_keys.remove(&it.idempotency_key);
                    break it;
                }
                if q.shutdown {
                    return;
                }
                q = shared.cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        deliver_with_retry(shared, &item);
    }
}

fn deliver_with_retry(shared: &Shared, item: &PendingDelivery) {
    let mut backoff = Duration::from_millis(500);
    for attempt in 1..=MAX_DELIVERY_ATTEMPTS {
        match shared.transport.deliver(
            &shared.config.url,
            &item.body,
            &item.signature,
            &item.idempotency_key,
        ) {
            Ok(()) => return,
            Err(DeliveryError::Ssrf(reason)) => {
                eprintln!(
                    "hugit-serve: merge-hook — egress REFUSED by SSRF guard, dropped: {reason}"
                );
                return;
            }
            Err(DeliveryError::Client(code)) => {
                eprintln!(
                    "hugit-serve: merge-hook — endpoint rejected ({code}), dropped (not retried)"
                );
                return;
            }
            Err(DeliveryError::Transient(reason)) => {
                if attempt == MAX_DELIVERY_ATTEMPTS {
                    eprintln!(
                        "hugit-serve: merge-hook — delivery failed after {attempt} attempts, dropped: {reason}"
                    );
                    return;
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(4));
            }
        }
    }
}

// ─────────────────────────── process-global wiring ───────────────────────────

/// Lazily-initialized process-global hook. `None` = the feature is OFF (URL unset
/// / misconfigured). Initialized once from the environment on first emit.
///
/// A process-global (rather than an `AppState` field) is a DELIBERATE, flagged
/// scope choice: WP W-WEBHOOK owns only `git.rs` + this module, NOT `state.rs`, so
/// the hook cannot hang off `AppState`. The global is read-only after init and the
/// hook is `Send + Sync`, so this is safe; the ergonomic cost is that config is read
/// from env once at first use.
static GLOBAL: OnceLock<Option<MergeHook>> = OnceLock::new();

fn global() -> Option<&'static MergeHook> {
    GLOBAL
        .get_or_init(|| {
            config_from_values(
                std::env::var("HUGIT_KUNGFU_WEBHOOK_URL").ok(),
                std::env::var("HUGIT_KUNGFU_WEBHOOK_SECRET").ok(),
            )
            .map(|cfg| MergeHook::build(cfg, Box::new(HttpEventSink), true))
        })
        .as_ref()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Emit a merge event for a just-finalized push. **Best-effort, post-durable:** call
/// ONLY after the push is durable AND the client `ok` has been written. When the
/// feature is OFF this returns immediately without even computing the diff. Any
/// downstream fault (queue full, endpoint down) is swallowed here — it can never
/// regress the push.
///
/// * `base_sha` — the pusher's pre-image tip (all-zero on a CREATE).
/// * `head_sha` — the new tip (all-zero on a DELETE).
/// * `actor_chain` — the authenticated principal chain (first element = the actor).
pub fn emit_merge_event(
    git_source: &Arc<dyn ObjectSource + Send + Sync>,
    repo: &str,
    ref_name: &str,
    base_sha: &str,
    head_sha: &str,
    actor_chain: &[String],
) {
    let Some(hook) = global() else {
        return; // feature OFF → no-op, no diff computed
    };
    let (touched_paths, cv_hint) = compute_touched_paths(git_source.as_ref(), base_sha, head_sha);
    let event = KungFuMergeEvent {
        repo: repo.to_string(),
        r#ref: ref_name.to_string(),
        base_sha: base_sha.to_string(),
        head_sha: head_sha.to_string(),
        touched_paths,
        landed_at: now_ms(),
        actor: actor_chain.first().cloned().unwrap_or_default(),
        cv_hint,
    };
    hook.enqueue(&event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use ureq::Resolver; // bring the trait into scope to call `resolve` directly

    // ── git fixture builders (mirror compare.rs / diff.rs test helpers) ──
    fn blob(src: &mut CasObjectSource, body: &str) -> gix_hash::ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(
        src: &mut CasObjectSource,
        mut entries: Vec<(&str, &str, gix_hash::ObjectId)>,
    ) -> gix_hash::ObjectId {
        entries.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &entries {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }
    fn commit(
        src: &mut CasObjectSource,
        tree: gix_hash::ObjectId,
        parent: Option<gix_hash::ObjectId>,
    ) -> gix_hash::ObjectId {
        let mut body = format!("tree {tree}\n");
        if let Some(p) = parent {
            body.push_str(&format!("parent {p}\n"));
        }
        body.push_str("author a <a@x> 0 +0000\ncommitter a <a@x> 0 +0000\n\nmsg\n");
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    const ZERO: &str = "0000000000000000000000000000000000000000";

    // ── touched_paths ──────────────────────────────────────────────

    #[test]
    fn touched_paths_real_base_to_head_diff() {
        let mut src = CasObjectSource::new();
        let b_old = blob(&mut src, "one\n");
        let keep = blob(&mut src, "keep\n");
        let t_old = tree(
            &mut src,
            vec![("100644", "a.txt", b_old), ("100644", "keep.txt", keep)],
        );
        let c_base = commit(&mut src, t_old, None);
        // change a.txt, add b.txt, keep.txt unchanged (shared subtree pruned).
        let b_new = blob(&mut src, "two\n");
        let b_added = blob(&mut src, "new\n");
        let t_new = tree(
            &mut src,
            vec![
                ("100644", "a.txt", b_new),
                ("100644", "keep.txt", keep),
                ("100644", "b.txt", b_added),
            ],
        );
        let c_head = commit(&mut src, t_new, Some(c_base));

        let (paths, cv) = compute_touched_paths(&src, &c_base.to_string(), &c_head.to_string());
        assert_eq!(cv, CV_FULL);
        assert_eq!(paths, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[test]
    fn create_and_delete_are_honest_markers_not_full_tree_walks() {
        let src = CasObjectSource::new();
        let head = "1111111111111111111111111111111111111111";
        let (paths, cv) = compute_touched_paths(&src, ZERO, head);
        assert!(paths.is_empty());
        assert_eq!(cv, CV_BRANCH_CREATE);

        let base = "2222222222222222222222222222222222222222";
        let (paths, cv) = compute_touched_paths(&src, base, ZERO);
        assert!(paths.is_empty());
        assert_eq!(cv, CV_REF_DELETE);
    }

    #[test]
    fn unresolvable_oids_are_honest_empty_unavailable() {
        let src = CasObjectSource::new(); // empty CAS → nothing resolves
        let (paths, cv) = compute_touched_paths(
            &src,
            "3333333333333333333333333333333333333333",
            "4444444444444444444444444444444444444444",
        );
        assert!(paths.is_empty());
        assert_eq!(cv, CV_UNAVAILABLE);
        // a garbage (non-hex) oid also fails closed
        let (_, cv) = compute_touched_paths(&src, "not-a-sha", "also-bad");
        assert_eq!(cv, CV_UNAVAILABLE);
    }

    // ── HMAC (authoritative RFC 4231 vector) ───────────────────────

    #[test]
    fn hmac_sha256_matches_rfc4231_test_case_2() {
        // RFC 4231 §4.3: key="Jefe", data="what do ya want for nothing?"
        let sig = sign(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            sig,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn signature_is_over_the_exact_canonical_body() {
        let ev = KungFuMergeEvent {
            repo: "hugit".into(),
            r#ref: "refs/heads/main".into(),
            base_sha: "a".repeat(40),
            head_sha: "b".repeat(40),
            touched_paths: vec!["src/lib.rs".into()],
            landed_at: 123,
            actor: "clerk:org:user".into(),
            cv_hint: CV_FULL.into(),
        };
        let body = serde_json::to_vec(&ev).unwrap();
        let expected = sign(b"topsecret", &body);
        // recompute independently
        let mut mac = HmacSha256::new_from_slice(b"topsecret").unwrap();
        mac.update(&body);
        assert_eq!(expected, hex::encode(mac.finalize().into_bytes()));
        // canonical field order is stable + `ref` is emitted (not `r#ref`)
        let json = String::from_utf8(body).unwrap();
        assert!(
            json.starts_with(r#"{"repo":"hugit","ref":"refs/heads/main""#),
            "canonical shape: {json}"
        );
    }

    // ── SSRF guard ─────────────────────────────────────────────────

    #[test]
    fn is_global_ip_rejects_every_private_class() {
        for s in [
            "127.0.0.1",        // loopback
            "0.0.0.0",          // unspecified
            "10.0.0.1",         // RFC1918
            "172.16.5.4",       // RFC1918
            "192.168.1.1",      // RFC1918
            "169.254.169.254",  // link-local / cloud metadata
            "100.64.0.1",       // CGNAT
            "192.0.0.1",        // IETF
            "198.18.0.1",       // benchmarking
            "192.0.2.5",        // documentation
            "255.255.255.255",  // broadcast
            "224.0.0.1",        // multicast
            "::1",              // v6 loopback
            "::",               // v6 unspecified
            "fc00::1",          // ULA
            "fe80::1",          // v6 link-local
            "2001:db8::1",      // v6 documentation
            "::ffff:127.0.0.1", // v4-mapped loopback
            "::ffff:10.0.0.1",  // v4-mapped RFC1918
        ] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(!is_global_ip(&ip), "{s} must be refused as non-public");
        }
    }

    #[test]
    fn is_global_ip_accepts_public() {
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(is_global_ip(&ip), "{s} must be accepted as public");
        }
    }

    #[test]
    fn is_global_v6_rejects_v4_embedding_and_transitional() {
        for s in [
            // NAT64 well-known 64:ff9b::/96 wrapping a private/loopback v4
            "64:ff9b::7f00:1",    // → 127.0.0.1 (loopback)
            "64:ff9b::a00:1",     // → 10.0.0.1  (RFC1918)
            "64:ff9b::a9fe:a9fe", // → 169.254.169.254 (metadata)
            // 6to4 2002::/16 wrapping a private/loopback v4
            "2002:7f00:1::",    // → 127.0.0.1
            "2002:c0a8:101::",  // → 192.168.1.1
            "2002:a9fe:a9fe::", // → 169.254.169.254 (metadata)
            // Teredo 2001:0000::/32 — rejected outright
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            // IPv4-compatible (deprecated) ::a.b.c.d
            "::7f00:1", // ::127.0.0.1
            "::a00:1",  // ::10.0.0.1
        ] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(
                !is_global_ip(&ip),
                "{s} must be refused (v4-embedding SSRF)"
            );
        }
    }

    #[test]
    fn is_global_v6_allows_genuine_global() {
        // A 6to4 wrapper of a GLOBAL v4 stays global; plain global v6 unaffected.
        for s in [
            "2606:4700::",          // Cloudflare
            "2001:4860:4860::8888", // Google DNS (2001: but not Teredo/doc)
            "2002:808:808::",       // 6to4 of 8.8.8.8 (global v4)
        ] {
            let ip: IpAddr = s.parse().unwrap();
            assert!(is_global_ip(&ip), "{s} must be accepted as public");
        }
    }

    #[test]
    fn resolver_refuses_private_literal_targets() {
        let r = PublicOnlyResolver;
        for netloc in [
            "127.0.0.1:443",
            "10.0.0.1:443",
            "169.254.169.254:80",
            "[::1]:443",
        ] {
            let err = r.resolve(netloc).unwrap_err();
            assert!(err.to_string().contains(SSRF_ERR_MARKER), "{netloc}: {err}");
        }
        // a public literal is allowed through (returns the pinned addr)
        let ok = r.resolve("8.8.8.8:443").unwrap();
        assert_eq!(ok.len(), 1);
        assert!(is_global_ip(&ok[0].ip()));
    }

    #[test]
    fn non_https_and_userinfo_urls_are_refused() {
        assert!(validate_webhook_url("http://kungfu.example/hook").is_err());
        assert!(validate_webhook_url("ftp://kungfu.example/hook").is_err());
        assert!(validate_webhook_url("https://user:pw@kungfu.example/hook").is_err());
        assert!(validate_webhook_url("https:///nohost").is_err());
        assert!(validate_webhook_url("https://kungfu.example/hook").is_ok());
    }

    // ── config gating (feature flag) ───────────────────────────────

    #[test]
    fn config_gating_is_fail_closed() {
        // unset URL → OFF (no-op)
        assert!(config_from_values(None, Some("s".into())).is_none());
        // blank URL → OFF
        assert!(config_from_values(Some("  ".into()), Some("s".into())).is_none());
        // non-https URL → OFF
        assert!(config_from_values(Some("http://x/y".into()), Some("s".into())).is_none());
        // https but no secret → OFF (never emit unsigned)
        assert!(config_from_values(Some("https://x/y".into()), None).is_none());
        assert!(config_from_values(Some("https://x/y".into()), Some(String::new())).is_none());
        // both present → ON
        assert!(config_from_values(Some("https://x/y".into()), Some("s".into())).is_some());
    }

    // ── secret never leaks ─────────────────────────────────────────

    #[test]
    fn secret_is_redacted_in_debug_and_never_in_artifacts() {
        const SECRET: &str = "S3CR3T-do-not-leak-xyz";
        let cfg = config_from_values(Some("https://kungfu.example/h".into()), Some(SECRET.into()))
            .unwrap();
        assert!(
            !format!("{cfg:?}").contains(SECRET),
            "secret leaked in Config Debug"
        );

        // The emitted artifacts (canonical body + signature header) must not equal /
        // contain the raw secret.
        let ev = KungFuMergeEvent {
            repo: "r".into(),
            r#ref: "refs/heads/main".into(),
            base_sha: "a".repeat(40),
            head_sha: "b".repeat(40),
            touched_paths: vec![],
            landed_at: 1,
            actor: "x".into(),
            cv_hint: CV_REF_DELETE.into(),
        };
        let body = serde_json::to_vec(&ev).unwrap();
        let sig = sign(SECRET.as_bytes(), &body);
        assert!(!String::from_utf8_lossy(&body).contains(SECRET));
        assert!(!sig.contains(SECRET));
    }

    // ── the hook: non-blocking + drop-oldest + never regresses ─────

    /// A sink that BLOCKS forever (models a hanging endpoint).
    struct HangingSink;
    impl EventSink for HangingSink {
        fn deliver(&self, _u: &str, _b: &[u8], _s: &str, _k: &str) -> Result<(), DeliveryError> {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
    }

    /// A sink that records every delivered body (for the happy-path test).
    /// `(body, signature, idempotency_key)` of the last delivery.
    type Recorded = (Vec<u8>, String, String);
    #[derive(Clone)]
    struct RecordingSink {
        count: Arc<AtomicUsize>,
        last: Arc<Mutex<Option<Recorded>>>,
    }
    impl EventSink for RecordingSink {
        fn deliver(
            &self,
            _u: &str,
            body: &[u8],
            sig: &str,
            key: &str,
        ) -> Result<(), DeliveryError> {
            *self.last.lock().unwrap() = Some((body.to_vec(), sig.to_string(), key.to_string()));
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn test_event(head: &str) -> KungFuMergeEvent {
        KungFuMergeEvent {
            repo: "hugit".into(),
            r#ref: "refs/heads/main".into(),
            base_sha: "a".repeat(40),
            head_sha: head.into(),
            touched_paths: vec![],
            landed_at: 1,
            actor: "clerk:o:u".into(),
            cv_hint: CV_REF_DELETE.into(),
        }
    }

    #[test]
    fn enqueue_never_blocks_even_with_a_hanging_endpoint() {
        let cfg = config_from_values(Some("https://x/y".into()), Some("s".into())).unwrap();
        let hook = MergeHook::build(cfg, Box::new(HangingSink), true);
        // The worker will grab the first item and hang forever in deliver(); every
        // subsequent enqueue must STILL return immediately.
        let start = Instant::now();
        for i in 0..50 {
            hook.enqueue(&test_event(&format!("{i:040}")));
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "enqueue blocked on the hanging sink"
        );
    }

    #[test]
    fn full_queue_drops_oldest_and_never_grows_past_capacity() {
        let cfg = config_from_values(Some("https://x/y".into()), Some("s".into())).unwrap();
        // No worker → the queue can't drain, isolating the drop-oldest logic.
        let hook = MergeHook::build(cfg, Box::new(HangingSink), false);
        let extra = 37;
        for i in 0..(QUEUE_CAPACITY + extra) {
            hook.enqueue(&test_event(&format!("{i:040}")));
        }
        let (len, dropped) = hook.queue_stats();
        assert_eq!(len, QUEUE_CAPACITY, "queue must be bounded at capacity");
        assert_eq!(
            dropped, extra as u64,
            "exactly the overflow was dropped-oldest"
        );
    }

    #[test]
    fn duplicate_repo_head_is_collapsed_while_pending() {
        let cfg = config_from_values(Some("https://x/y".into()), Some("s".into())).unwrap();
        let hook = MergeHook::build(cfg, Box::new(HangingSink), false);
        for _ in 0..10 {
            hook.enqueue(&test_event(&"c".repeat(40))); // same (repo, head_sha)
        }
        let (len, _) = hook.queue_stats();
        assert_eq!(
            len, 1,
            "identical (repo, head_sha) collapsed to one pending delivery"
        );
    }

    #[test]
    fn worker_delivers_signed_body_and_idempotency_key() {
        let cfg = config_from_values(Some("https://x/y".into()), Some("Jefe".into())).unwrap();
        let sink = RecordingSink {
            count: Arc::new(AtomicUsize::new(0)),
            last: Arc::new(Mutex::new(None)),
        };
        let count = Arc::clone(&sink.count);
        let last = Arc::clone(&sink.last);
        let hook = MergeHook::build(cfg, Box::new(sink), true);

        let ev = test_event(&"d".repeat(40));
        hook.enqueue(&ev);

        // poll for the async delivery
        let start = Instant::now();
        while count.load(Ordering::SeqCst) == 0 {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "worker never delivered"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let (body, sig, key) = last.lock().unwrap().clone().unwrap();
        assert_eq!(key, idempotency_key("hugit", &"d".repeat(40)));
        // the signature matches an independent HMAC over the delivered body
        assert_eq!(sig, sign(b"Jefe", &body));
    }
}
