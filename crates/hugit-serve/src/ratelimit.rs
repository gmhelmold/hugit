//! Per-principal engine rate limit (go-live gap **G10**).
//!
//! The deployed engine is a SINGLE-THREADED synchronous `tiny_http` accept loop
//! ([`crate::server::serve_on`]). Its only pre-existing backpressure is the GLOBAL
//! saturation load-shed ([`crate::metrics::ShedGate`] → 503) — it protects the
//! engine as a whole but is NOT per-tenant: one noisy org can consume the whole
//! serial loop and starve every other tenant while the loop is still "fast enough"
//! to never trip the saturation gate.
//!
//! [`RateLimiter`] adds a per-PRINCIPAL fairness gate placed in the accept-loop
//! preamble, right beside `should_shed` and BEFORE the POST body is read — so a
//! flood from one org is rejected 429 without the loop ever buffering its bodies or
//! spawning a worker. Same discipline as `should_shed`: the decision is PURE
//! saturating integer arithmetic — no I/O, no syscall, and (in steady state) no
//! allocation on the hot path. A per-key `HashMap` insert (only the FIRST time a key
//! is seen) is the sole allocation; every repeat request for a live key is a
//! `get_mut` + arithmetic. The map is BOUNDED (per-map cap with oldest-first
//! eviction) so it can never itself become an unbounded-growth vector under a flood
//! of distinct principals.
//!
//! Keying (derived from the engine-resolved principal chain, identical vocabulary to
//! [`crate::authz`]):
//!   - `orchestrator:*` (the platform/dev OPERATOR) → EXEMPT (no gate at all — the
//!     orchestrator drives the control plane and must never throttle itself).
//!   - `clerk:{org}:{user}` → keyed by the owning `{org}` tenant. A push
//!     (`git-receive-pack`) uses a STRICTER sub-bucket than ordinary requests.
//!   - anything else — a genuinely anonymous request (git clone over the wire, no
//!     Bearer), a malformed/unknown principal → the strict ANONYMOUS bucket, keyed
//!     by the peer edge IP (falling back to a single GLOBAL bucket when the socket
//!     exposes no address). An anonymous flood therefore cannot borrow a tenant's
//!     budget, and each edge IP is throttled independently.
//!
//! Interior mutability (a small `Mutex` per shard) lets the loop share
//! `&RateLimiter`; on the single-threaded loop every lock is uncontended.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::Mutex;

/// How many shards the bucket map is split into. Sharding keeps each per-map
/// eviction scan small and gives the (single-threaded, so uncontended) locks fine
/// granularity. A power of two so the index mask is cheap.
const NUM_SHARDS: usize = 16;

/// The verdict for one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RlDecision {
    /// Within budget — serve.
    Allow,
    /// Over budget — reject INLINE with a 429 (no body read, no worker spawn).
    TooMany,
}

/// A token-bucket configuration. All quantities are in MILLI-tokens (tokens × 1000)
/// so the refill is exact integer arithmetic at millisecond resolution — no floats.
#[derive(Clone, Copy, Debug)]
struct BucketCfg {
    /// Burst capacity, in milli-tokens (`burst × 1000`).
    capacity_milli: u64,
    /// Refill in milli-tokens PER MILLISECOND. This equals the requests-per-second
    /// rate: `rate` tokens/s = `rate/1000` tokens/ms = `rate` milli-tokens/ms.
    refill_milli_per_ms: u64,
}

impl BucketCfg {
    fn new(rps: u64, burst: u64) -> Self {
        Self {
            capacity_milli: burst.saturating_mul(1000),
            refill_milli_per_ms: rps,
        }
    }
}

/// One principal's token bucket. `Copy` + 16 bytes — cheap to store per key.
#[derive(Clone, Copy, Debug)]
struct Bucket {
    /// Current balance, in milli-tokens.
    tokens_milli: u64,
    /// Monotonic ms (loop clock) of the last refill.
    last_ms: u64,
}

impl Bucket {
    fn full(cfg: &BucketCfg, now_ms: u64) -> Self {
        Self {
            tokens_milli: cfg.capacity_milli,
            last_ms: now_ms,
        }
    }

    /// Refill for the elapsed time, then try to spend one whole token. Returns
    /// `true` iff a token was available (ALLOW). PURE saturating arithmetic — no
    /// I/O, no alloc, cannot panic, monotonic-clock-safe (a backwards `now_ms`
    /// saturates to a zero refill, never a panic or an over-credit).
    fn take(&mut self, cfg: &BucketCfg, now_ms: u64) -> bool {
        let elapsed = now_ms.saturating_sub(self.last_ms);
        let refill = elapsed.saturating_mul(cfg.refill_milli_per_ms);
        self.tokens_milli = self
            .tokens_milli
            .saturating_add(refill)
            .min(cfg.capacity_milli);
        self.last_ms = now_ms;
        if self.tokens_milli >= 1000 {
            self.tokens_milli -= 1000;
            true
        } else {
            false
        }
    }
}

/// One shard's bucket maps — three keyspaces that never collide (a tenant's ordinary
/// budget, that tenant's stricter push budget, and the anonymous per-edge budget).
#[derive(Default)]
struct Shard {
    /// `{org}` → ordinary-request bucket.
    tenant: HashMap<String, Bucket>,
    /// `{org}` → push (`git-receive-pack`) bucket — a stricter sub-bucket.
    push: HashMap<String, Bucket>,
    /// peer edge IP (`None` = the single global fallback) → anonymous bucket.
    anon: HashMap<Option<IpAddr>, Bucket>,
}

/// The per-principal rate limiter. Cheap to share as `&RateLimiter` across the loop.
pub struct RateLimiter {
    enabled: bool,
    shards: Vec<Mutex<Shard>>,
    tenant_cfg: BucketCfg,
    push_cfg: BucketCfg,
    anon_cfg: BucketCfg,
    /// Max live keys PER MAP PER SHARD — the boundedness invariant. When a map is
    /// full and a NEW key arrives, the oldest (least-recently-touched) entry is
    /// evicted first, so total memory is capped at
    /// `NUM_SHARDS × per_map_cap × 3 × sizeof(entry)` regardless of how many
    /// distinct principals ever appear.
    per_map_cap: usize,
}

impl RateLimiter {
    /// Build from the process env (called once at loop start).
    ///
    ///   - `HUGIT_RATELIMIT` — `0`/`false`/`off` disables the gate entirely
    ///     (default ON, mirroring the load-shed).
    ///   - `HUGIT_RL_TENANT_RPS` (default `60`) / `HUGIT_RL_TENANT_BURST` (`120`).
    ///   - `HUGIT_RL_ANON_RPS` (default `10`) / `HUGIT_RL_ANON_BURST` (`10`).
    ///   - `HUGIT_RL_PUSH_RPS` (default `5`) / `HUGIT_RL_PUSH_BURST` (`10`).
    ///   - `HUGIT_RL_MAP_CAP` — max live keys per map per shard (default `4096`).
    #[must_use]
    pub fn from_env() -> Self {
        let enabled = env_bool("HUGIT_RATELIMIT", true);
        let tenant_cfg = BucketCfg::new(
            env_u64("HUGIT_RL_TENANT_RPS", 60),
            env_u64("HUGIT_RL_TENANT_BURST", 120),
        );
        let anon_cfg = BucketCfg::new(
            env_u64("HUGIT_RL_ANON_RPS", 10),
            env_u64("HUGIT_RL_ANON_BURST", 10),
        );
        let push_cfg = BucketCfg::new(
            env_u64("HUGIT_RL_PUSH_RPS", 5),
            env_u64("HUGIT_RL_PUSH_BURST", 10),
        );
        let per_map_cap = env_u64("HUGIT_RL_MAP_CAP", 4096).max(1) as usize;
        Self::new(enabled, tenant_cfg, anon_cfg, push_cfg, per_map_cap)
    }

    fn new(
        enabled: bool,
        tenant_cfg: BucketCfg,
        anon_cfg: BucketCfg,
        push_cfg: BucketCfg,
        per_map_cap: usize,
    ) -> Self {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        for _ in 0..NUM_SHARDS {
            shards.push(Mutex::new(Shard::default()));
        }
        Self {
            enabled,
            shards,
            tenant_cfg,
            push_cfg,
            anon_cfg,
            per_map_cap,
        }
    }

    /// Construct explicitly with the given limits (tests).
    #[must_use]
    pub fn for_test(
        tenant_rps: u64,
        tenant_burst: u64,
        anon_rps: u64,
        anon_burst: u64,
        push_rps: u64,
        push_burst: u64,
        per_map_cap: usize,
    ) -> Self {
        Self::new(
            true,
            BucketCfg::new(tenant_rps, tenant_burst),
            BucketCfg::new(anon_rps, anon_burst),
            BucketCfg::new(push_rps, push_burst),
            per_map_cap,
        )
    }

    /// Whether the gate is armed at all.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// THE gate. Given the engine-resolved `principal` chain (as
    /// [`crate::server::two_tier_auth`] yields it), the request's peer edge IP
    /// (`None` when the socket exposes none), whether this is a push
    /// (`git-receive-pack`), and the monotonic loop clock `now_ms`, decide ALLOW /
    /// TOO-MANY.
    ///
    /// PURE + NON-BLOCKING: an uncontended lock + saturating integer arithmetic. No
    /// I/O, no syscall, no allocation on the steady-state (repeat-key) path; the sole
    /// allocation is a one-time `HashMap` insert the first time a key is seen. The
    /// operator (`orchestrator:*`) is EXEMPT and returns instantly without touching a
    /// bucket.
    #[must_use]
    pub fn check(
        &self,
        principal: &[String],
        peer_ip: Option<IpAddr>,
        is_push: bool,
        now_ms: u64,
    ) -> RlDecision {
        if !self.enabled {
            return RlDecision::Allow;
        }
        match classify(principal) {
            // The operator drives the control plane — never throttle it.
            RlClass::Operator => RlDecision::Allow,
            RlClass::Tenant(org) => {
                let shard = &mut *self.shard_for(org.as_bytes());
                let allowed = if is_push {
                    take_str(
                        &mut shard.push,
                        org,
                        &self.push_cfg,
                        now_ms,
                        self.per_map_cap,
                    )
                } else {
                    take_str(
                        &mut shard.tenant,
                        org,
                        &self.tenant_cfg,
                        now_ms,
                        self.per_map_cap,
                    )
                };
                decision(allowed)
            }
            RlClass::Anon => {
                // Key by the peer IP so each edge is throttled independently; `None`
                // (no socket address) collapses to a single shared global bucket.
                let seed = match peer_ip {
                    Some(ip) => ip_seed(ip),
                    None => 0,
                };
                let shard = &mut *self.shard_at(seed);
                let allowed = take_key(
                    &mut shard.anon,
                    peer_ip,
                    &self.anon_cfg,
                    now_ms,
                    self.per_map_cap,
                );
                decision(allowed)
            }
        }
    }

    fn shard_for(&self, key_bytes: &[u8]) -> std::sync::MutexGuard<'_, Shard> {
        self.shard_at(fnv1a(key_bytes))
    }

    fn shard_at(&self, seed: u64) -> std::sync::MutexGuard<'_, Shard> {
        let idx = (seed as usize) & (NUM_SHARDS - 1);
        // A poisoned lock (a panic while held) must NEVER take down the accept loop —
        // recover the inner state and carry on. The bucket data is plain integers, so
        // a partially-updated bucket is at worst a slightly-off balance, never unsafe.
        match self.shards[idx].lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }
}

fn decision(allowed: bool) -> RlDecision {
    if allowed {
        RlDecision::Allow
    } else {
        RlDecision::TooMany
    }
}

/// Take a token for a `&str` key (tenant/push maps), inserting a fresh full bucket on
/// first sight and evicting the oldest entry first when the map is at capacity.
fn take_str(
    map: &mut HashMap<String, Bucket>,
    key: &str,
    cfg: &BucketCfg,
    now_ms: u64,
    cap: usize,
) -> bool {
    if let Some(b) = map.get_mut(key) {
        return b.take(cfg, now_ms);
    }
    if map.len() >= cap {
        evict_oldest(map);
    }
    let mut b = Bucket::full(cfg, now_ms);
    let allowed = b.take(cfg, now_ms);
    map.insert(key.to_string(), b);
    allowed
}

/// Take a token for a `Copy`/owned key (the anonymous map), same bounded semantics.
fn take_key<K: Eq + Hash + Copy>(
    map: &mut HashMap<K, Bucket>,
    key: K,
    cfg: &BucketCfg,
    now_ms: u64,
    cap: usize,
) -> bool {
    if let Some(b) = map.get_mut(&key) {
        return b.take(cfg, now_ms);
    }
    if map.len() >= cap {
        evict_oldest(map);
    }
    let mut b = Bucket::full(cfg, now_ms);
    let allowed = b.take(cfg, now_ms);
    map.insert(key, b);
    allowed
}

/// Evict the least-recently-touched entry so the map stays bounded. O(n) over ONE
/// small already-capped map, and only when that map is full — never on the
/// steady-state repeat-key path.
fn evict_oldest<K: Eq + Hash + Clone>(map: &mut HashMap<K, Bucket>) {
    if let Some(oldest) = map
        .iter()
        .min_by_key(|(_, b)| b.last_ms)
        .map(|(k, _)| k.clone())
    {
        map.remove(&oldest);
    }
}

/// The rate-limit class of a principal, mirroring [`crate::authz`]'s `caller`
/// vocabulary but yielding only what the gate keys on (the org is BORROWED — no
/// allocation to classify).
enum RlClass<'a> {
    Operator,
    Tenant(&'a str),
    Anon,
}

fn classify(principal: &[String]) -> RlClass<'_> {
    match principal.first().map(String::as_str) {
        // No credential at all → anonymous (git wire, no Bearer).
        None => RlClass::Anon,
        Some(p) if p.starts_with("orchestrator:") => RlClass::Operator,
        Some(p) => match p.strip_prefix("clerk:") {
            Some(rest) => match rest.split(':').next().unwrap_or("") {
                // "clerk:" / "clerk::user" — malformed → strict anonymous bucket.
                "" => RlClass::Anon,
                org => RlClass::Tenant(org),
            },
            // A present but unrecognized prefix → strict anonymous bucket (never a
            // tenant's budget — fail closed, same spirit as authz's `Unknown`).
            None => RlClass::Anon,
        },
    }
}

/// A stable 64-bit FNV-1a over the key bytes (shard selection). Deterministic (no
/// per-process seed) — the shard for a key is stable across the loop's lifetime.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A cheap shard seed for an IP (its octets), avoiding a string allocation.
fn ip_seed(ip: IpAddr) -> u64 {
    match ip {
        IpAddr::V4(v4) => fnv1a(&v4.octets()),
        IpAddr::V6(v6) => fnv1a(&v6.octets()),
    }
}

/// Parse a `u64` env var; missing/unparseable → `default`.
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

/// Parse a boolean env var (`0`/`false`/`off`/`no` → false; anything else present →
/// true); missing → `default`.
fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::time::Instant;

    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }
    fn operator() -> Vec<String> {
        vec!["orchestrator:hugit".to_string()]
    }

    /// A generous limiter for the isolation/bypass tests (tenant 5/s burst 5).
    fn rl() -> RateLimiter {
        RateLimiter::for_test(5, 5, 3, 3, 2, 2, 1024)
    }

    /// (1) ISOLATION: a flood from org A is throttled while org B, on the SAME
    /// limiter at the SAME instant, stays fully served. One noisy tenant cannot
    /// consume another tenant's budget.
    #[test]
    fn per_tenant_isolation() {
        let rl = rl();
        let a = tenant("org-a");
        let b = tenant("org-b");
        let now = 0; // no refill within the burst

        // Drain org A's burst of 5.
        for _ in 0..5 {
            assert_eq!(rl.check(&a, None, false, now), RlDecision::Allow);
        }
        // The 6th from org A (same instant) is over budget.
        assert_eq!(rl.check(&a, None, false, now), RlDecision::TooMany);

        // org B is UNAFFECTED — its own full burst still serves.
        for _ in 0..5 {
            assert_eq!(
                rl.check(&b, None, false, now),
                RlDecision::Allow,
                "org B must not be throttled by org A's flood"
            );
        }
        assert_eq!(rl.check(&b, None, false, now), RlDecision::TooMany);
    }

    /// (3) OPERATOR BYPASS: the platform operator is never throttled, however many
    /// requests it fires at a single instant.
    #[test]
    fn operator_is_exempt() {
        let rl = rl();
        let op = operator();
        for _ in 0..10_000 {
            assert_eq!(rl.check(&op, None, false, 0), RlDecision::Allow);
        }
    }

    /// (4) BOUNDEDNESS: feeding thousands of DISTINCT anonymous edge IPs never grows
    /// any bucket map beyond its per-map cap — the map cannot become an
    /// unbounded-growth vector.
    #[test]
    fn map_stays_bounded_under_distinct_principals() {
        let cap = 64;
        let rl = RateLimiter::for_test(60, 120, 10, 10, 5, 10, cap);
        for i in 0..10_000u32 {
            let ip = IpAddr::V4(Ipv4Addr::from(i)); // 10k distinct edges
            let _ = rl.check(&[], Some(ip), false, i as u64);
        }
        let total: usize = rl
            .shards
            .iter()
            .map(|s| {
                let g = s.lock().unwrap();
                g.tenant.len() + g.push.len() + g.anon.len()
            })
            .sum();
        assert!(
            total <= NUM_SHARDS * cap,
            "bucket maps must stay bounded: {total} > {}",
            NUM_SHARDS * cap
        );
    }

    /// Distinct TENANT orgs are likewise bounded (the tenant map cap holds).
    #[test]
    fn tenant_map_stays_bounded() {
        let cap = 32;
        let rl = RateLimiter::for_test(60, 120, 10, 10, 5, 10, cap);
        for i in 0..5_000u32 {
            let _ = rl.check(&tenant(&format!("org-{i}")), None, false, i as u64);
        }
        let tenant_total: usize = rl
            .shards
            .iter()
            .map(|s| s.lock().unwrap().tenant.len())
            .sum();
        assert!(tenant_total <= NUM_SHARDS * cap);
    }

    /// The push sub-bucket is STRICTER than (and INDEPENDENT of) the same tenant's
    /// ordinary bucket: a push flood is throttled at the tighter push rate while the
    /// tenant's ordinary budget is untouched.
    #[test]
    fn push_subbucket_is_separate_and_stricter() {
        let rl = RateLimiter::for_test(60, 120, 10, 10, 2, 2, 1024); // push burst 2
        let t = tenant("org-a");
        // Drain the push burst of 2.
        assert_eq!(rl.check(&t, None, true, 0), RlDecision::Allow);
        assert_eq!(rl.check(&t, None, true, 0), RlDecision::Allow);
        assert_eq!(rl.check(&t, None, true, 0), RlDecision::TooMany);
        // The ordinary bucket for the SAME tenant is independent → still serves.
        assert_eq!(rl.check(&t, None, false, 0), RlDecision::Allow);
    }

    /// Anonymous edges are keyed per-IP: one flooding IP is throttled while a
    /// different IP stays served.
    #[test]
    fn anon_keyed_per_edge_ip() {
        let rl = RateLimiter::for_test(60, 120, 2, 2, 5, 10, 1024); // anon burst 2
        let ip1 = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        let ip2 = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));
        assert_eq!(rl.check(&[], ip1, false, 0), RlDecision::Allow);
        assert_eq!(rl.check(&[], ip1, false, 0), RlDecision::Allow);
        assert_eq!(rl.check(&[], ip1, false, 0), RlDecision::TooMany);
        // A different edge IP has its own budget.
        assert_eq!(rl.check(&[], ip2, false, 0), RlDecision::Allow);
    }

    /// The bucket refills over time: after being drained, waiting long enough for one
    /// token to accrue lets exactly one more request through.
    #[test]
    fn bucket_refills_over_time() {
        let rl = RateLimiter::for_test(10, 1, 10, 1, 5, 10, 1024); // tenant 10/s burst 1
        let t = tenant("org-a");
        assert_eq!(rl.check(&t, None, false, 0), RlDecision::Allow); // spend the 1
        assert_eq!(rl.check(&t, None, false, 0), RlDecision::TooMany); // empty
        // 10 tokens/s → 1 token accrues in 100 ms.
        assert_eq!(rl.check(&t, None, false, 100), RlDecision::Allow);
        assert_eq!(rl.check(&t, None, false, 100), RlDecision::TooMany);
    }

    /// A disabled limiter is a hard kill-switch: it always allows.
    #[test]
    fn disabled_always_allows() {
        let rl = RateLimiter::new(
            false,
            BucketCfg::new(1, 1),
            BucketCfg::new(1, 1),
            BucketCfg::new(1, 1),
            1024,
        );
        assert!(!rl.enabled());
        for _ in 0..1000 {
            assert_eq!(
                rl.check(&tenant("org-a"), None, false, 0),
                RlDecision::Allow
            );
        }
    }

    /// A backwards monotonic clock (a quirk) cannot panic — `saturating_sub` yields a
    /// zero refill, so the decision stays well-defined.
    #[test]
    fn backwards_clock_does_not_panic() {
        let rl = rl();
        let t = tenant("org-a");
        assert_eq!(rl.check(&t, None, false, 10_000), RlDecision::Allow);
        // now_ms < last_ms → no over-credit, no panic.
        let _ = rl.check(&t, None, false, 5_000);
    }

    /// (5) PURE / NON-BLOCKING: the gate does no I/O and returns in sub-millisecond
    /// time even across many distinct keys. A generous wall-clock ceiling asserts the
    /// path never blocks (no socket, no syscall) — it is arithmetic + an uncontended
    /// lock only.
    #[test]
    fn gate_is_pure_and_non_blocking() {
        let rl = RateLimiter::for_test(
            1_000_000, 1_000_000, 1_000_000, 1_000_000, 1_000_000, 1_000_000, 8192,
        );
        let start = Instant::now();
        for i in 0..50_000u32 {
            // Mix tenant + anonymous keys; all within budget (huge limits).
            if i % 2 == 0 {
                let _ = rl.check(&tenant("org-a"), None, false, i as u64);
            } else {
                let ip = IpAddr::V4(Ipv4Addr::from(i));
                let _ = rl.check(&[], Some(ip), false, i as u64);
            }
        }
        let elapsed = start.elapsed();
        // 50k pure-arithmetic gate calls must finish comfortably under this ceiling;
        // any real I/O/block in the path would blow past it by orders of magnitude.
        assert!(
            elapsed.as_millis() < 500,
            "gate must be non-blocking arithmetic; 50k calls took {elapsed:?}"
        );
    }
}
