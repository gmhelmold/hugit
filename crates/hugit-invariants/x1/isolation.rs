//! X1 tenant-isolation red-team surface.
//!
//! WP-X1 proves the CoreLink-inherited tenant boundary holds for hugit:
//!
//! - private bytes never cross tenants ([`TenantNamespace`] + [`MemoStore`]);
//! - a forged / collision memo key is denied + alerted and **never poisons** an
//!   existing entry ([`MemoStore::admit`]);
//! - a public-deterministic artifact IS shared cross-tenant, with proof no
//!   private principal/runner byte rode along ([`SharedRegistry`], leak-checked
//!   against the frozen [`AttestationChain`](hugit_contracts::AttestationChain));
//! - private artifacts emit **no cross-tenant existence/timing signal** — a
//!   private miss is byte-shape- and latency-class-identical to a genuine
//!   absent ([`Lookup`]); the existence signal inherent to public-deterministic
//!   sharing is the only one, and it is **by-design disclosure** (documented).
//!
//! # The tenant boundary under test = HMAC-derived prefixes
//!
//! Mirroring the CoreLink keyspace model (whitepaper §9, "HMAC-derived
//! prefixes, fail-closed audit"), every tenant holds a private root secret. A
//! memo key's *physical* storage location is `HMAC(tenant_secret, base_key)` —
//! the **namespaced key**. Two tenants asking for the same logical `base_key`
//! land on different physical keys, because each derivation is keyed by a secret
//! the other tenant does not have. A tenant cannot compute another tenant's
//! namespaced key without that tenant's secret, so a cross-tenant *private*
//! lookup physically cannot hit. This is the partition X1 attacks; it is
//! consumed here as a surface and proven sound, never weakened.
//!
//! The canonical memo *base* key (`H(tree‖def‖toolchain)`) is computed by the
//! single-source [`hugit_refstore::compute_memo_key`] — never re-transcribed.
//!
//! Everything in this module is verification logic over the consumed surfaces;
//! there is no production behaviour shipped from this crate.

use std::collections::HashMap;

use hmac::{Hmac, Mac};
use hugit_contracts::AttestationChain;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

// ── tenant identity + HMAC-derived namespace ─────────────────────────────────

/// A tenant's stable, public identifier (e.g. `"tenant-a"`). Carries no secret;
/// it is safe to log and to surface in audit events.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantId(pub String);

impl TenantId {
    /// Construct a tenant id from anything string-like.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The public identifier bytes.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A tenant's HMAC root secret — the keyspace-partition key. This is the byte
/// material another tenant **cannot** possess; possessing it is equivalent to
/// being the tenant. It never leaves the namespace abstraction and is never
/// surfaced in an audit event or response.
///
/// (In production this is the CoreLink-issued per-tenant prefix key; here it is
/// the modelled equivalent, used to prove the partition is sound.)
#[derive(Clone)]
pub struct TenantSecret(Vec<u8>);

impl TenantSecret {
    /// Wrap raw secret bytes. Distinct tenants MUST hold distinct secrets; the
    /// caller (the platform key-issuer in production) guarantees that.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }
}

// A secret must never leak through Debug.
impl std::fmt::Debug for TenantSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TenantSecret(<redacted>)")
    }
}

/// A tenant-scoped memo namespace: derives the physical storage key for a
/// logical memo base key under this tenant's HMAC secret.
///
/// The derivation `HMAC(secret, base_key)` is the keyspace partition. Because
/// the secret is per-tenant and private, the namespaced key for tenant A's
/// private entry is unforgeable by tenant B — B cannot even *name* A's storage
/// slot, let alone read it.
#[derive(Debug, Clone)]
pub struct TenantNamespace {
    id: TenantId,
    secret: TenantSecret,
}

impl TenantNamespace {
    /// Bind a tenant id to its private HMAC secret.
    pub fn new(id: TenantId, secret: TenantSecret) -> Self {
        Self { id, secret }
    }

    /// This namespace's public tenant id.
    pub fn id(&self) -> &TenantId {
        &self.id
    }

    /// Derive the physical, tenant-namespaced storage key for a logical memo
    /// `base_key` — `lower_hex(HMAC-SHA256(tenant_secret, base_key))`.
    ///
    /// This is the ONLY way a private entry is keyed. Two tenants deriving the
    /// same `base_key` produce different namespaced keys (different secrets), so
    /// a private entry is physically partitioned by construction.
    pub fn namespaced_key(&self, base_key: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.secret.0)
            .expect("HMAC-SHA256 accepts a key of any length");
        mac.update(base_key.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

// ── audit trail ──────────────────────────────────────────────────────────────

/// One fail-closed audit event. Every deny / alert path emits one; an admit
/// emits one too, so the trail is complete. A reason NEVER contains tenant
/// secret bytes — only public ids and the (public) physical key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    /// What was attempted: `"lookup"`, `"admit"`.
    pub action: &'static str,
    /// The requesting tenant's public id.
    pub tenant: String,
    /// Whether the attempt was admitted / served.
    pub admitted: bool,
    /// Whether this event is an ALERT (a forgery/collision/cross-tenant attack
    /// was detected, not merely a benign miss).
    pub alert: bool,
    /// Machine-readable reason (public; never a secret).
    pub reason: String,
}

/// An append-only audit sink. The store writes every decision here so the
/// red-team oracle can assert deny/alert paths are recorded.
#[derive(Debug, Default, Clone)]
pub struct AuditLog {
    events: Vec<AuditEvent>,
}

impl AuditLog {
    /// A fresh empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one event.
    pub fn record(&mut self, ev: AuditEvent) {
        self.events.push(ev);
    }

    /// All recorded events, in order.
    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    /// All events flagged as ALERTs.
    pub fn alerts(&self) -> impl Iterator<Item = &AuditEvent> {
        self.events.iter().filter(|e| e.alert)
    }
}

// ── the lookup verdict (side-channel-flat) ────────────────────────────────────

/// The verdict of a memo lookup.
///
/// Item ④ (side-channels): [`Miss`](Lookup::Miss) is returned IDENTICALLY for a
/// genuinely-absent key AND for a key that exists but belongs to another tenant
/// — there is no `Forbidden`/`Exists-but-denied` variant that would betray
/// existence, and (see [`MemoStore::lookup`]) the work performed is the same in
/// both cases (one HMAC derivation + one map probe), so latency is the same
/// class. A private artifact thus produces NO cross-tenant existence or timing
/// signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// The (tenant-namespaced) key resolved to bytes — served.
    Hit(Vec<u8>),
    /// The key did not resolve in this tenant's namespace. Indistinguishable
    /// for "never existed" vs "exists for another tenant". (PUBLIC shared hits
    /// are served via [`SharedRegistry`], a SEPARATE, by-design-disclosed path.)
    Miss,
}

impl Lookup {
    /// True iff bytes were served.
    pub fn is_hit(&self) -> bool {
        matches!(self, Lookup::Hit(_))
    }

    /// True iff the lookup did not serve (absent OR cross-tenant-private —
    /// indistinguishable).
    pub fn is_miss(&self) -> bool {
        matches!(self, Lookup::Miss)
    }
}

/// Why an admission was refused. Every variant denies fail-closed and the
/// caller emits an ALERT audit event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmitError {
    /// The supplied namespaced key does not match what the tenant's secret
    /// derives for the claimed base key — a FORGED key (the attacker tried to
    /// write to a slot they did not legitimately derive).
    ForgedKey,
    /// The namespaced key is already occupied — admitting would OVERWRITE an
    /// existing entry. A genuine memo write is idempotent on identical bytes;
    /// admitting *different* bytes to an occupied slot is a poisoning attempt
    /// and is refused. The existing bytes are left untouched.
    CollisionWouldPoison,
}

impl std::fmt::Display for AdmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdmitError::ForgedKey => {
                write!(
                    f,
                    "forged namespaced key (does not match tenant derivation)"
                )
            }
            AdmitError::CollisionWouldPoison => {
                write!(f, "collision: admitting would poison an existing entry")
            }
        }
    }
}

impl std::error::Error for AdmitError {}

// ── the tenant-partitioned memo store (B2's AC surface, modelled) ─────────────

/// An in-process, tenant-partitioned content store that models the CoreLink
/// AC/memo surface X1 attacks. Entries are keyed by their HMAC-derived,
/// tenant-namespaced physical key; nothing tenant-private is reachable without
/// the owning tenant's namespace.
///
/// This is the surface the red-team drives: it must DENY cross-tenant private
/// lookups, DENY+ALERT forged/collision writes without poisoning, and emit no
/// existence/timing side-channel for private artifacts.
#[derive(Debug, Default)]
pub struct MemoStore {
    /// physical (namespaced) key → bytes.
    entries: HashMap<String, Vec<u8>>,
}

impl MemoStore {
    /// A fresh empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit a private memo entry for `ns`, keyed by the logical `base_key`,
    /// guarding against forgery and poisoning. `claimed_key` is the namespaced
    /// key the caller presents; it MUST equal `ns.namespaced_key(base_key)` or
    /// the write is a forgery (deny + alert). An occupied slot is only re-writable
    /// with byte-identical content (idempotent memo); differing bytes are a
    /// poisoning attempt (deny + alert), and the existing bytes are preserved.
    ///
    /// Returns the physical key on success.
    pub fn admit(
        &mut self,
        ns: &TenantNamespace,
        base_key: &str,
        claimed_key: &str,
        bytes: &[u8],
        audit: &mut AuditLog,
    ) -> Result<String, AdmitError> {
        let derived = ns.namespaced_key(base_key);

        // ── forgery check: the claimed physical key MUST be the one this
        //    tenant's secret derives. A mismatch means the caller is trying to
        //    write to a slot it did not legitimately derive. ──────────────────
        if claimed_key != derived {
            audit.record(AuditEvent {
                action: "admit",
                tenant: ns.id().as_str().to_string(),
                admitted: false,
                alert: true,
                reason: AdmitError::ForgedKey.to_string(),
            });
            return Err(AdmitError::ForgedKey);
        }

        // ── poisoning check: an occupied slot is only re-writable with
        //    byte-identical content. Differing bytes are refused; the existing
        //    entry is NEVER mutated on the refusal path. ──────────────────────
        if let Some(existing) = self.entries.get(&derived)
            && existing.as_slice() != bytes
        {
            audit.record(AuditEvent {
                action: "admit",
                tenant: ns.id().as_str().to_string(),
                admitted: false,
                alert: true,
                reason: AdmitError::CollisionWouldPoison.to_string(),
            });
            return Err(AdmitError::CollisionWouldPoison);
        }

        self.entries.insert(derived.clone(), bytes.to_vec());
        audit.record(AuditEvent {
            action: "admit",
            tenant: ns.id().as_str().to_string(),
            admitted: true,
            alert: false,
            reason: String::new(),
        });
        Ok(derived)
    }

    /// Look up a logical `base_key` in `ns`'s namespace.
    ///
    /// The derivation + single map probe are performed UNCONDITIONALLY and
    /// IDENTICALLY whether the key is absent or belongs to another tenant —
    /// there is exactly one HMAC derivation and one `HashMap::get`, so the
    /// "private miss" and "absent" paths are the same shape and latency class
    /// (item ④). The result is [`Lookup::Hit`] only when THIS tenant's
    /// namespaced key resolves; otherwise [`Lookup::Miss`] — never a distinct
    /// "exists-but-forbidden" verdict.
    pub fn lookup(&self, ns: &TenantNamespace, base_key: &str, audit: &mut AuditLog) -> Lookup {
        let physical = ns.namespaced_key(base_key);
        let result = match self.entries.get(&physical) {
            Some(bytes) => Lookup::Hit(bytes.clone()),
            None => Lookup::Miss,
        };
        audit.record(AuditEvent {
            action: "lookup",
            tenant: ns.id().as_str().to_string(),
            admitted: result.is_hit(),
            alert: false,
            reason: String::new(),
        });
        result
    }

    /// Read back the raw bytes at a physical key — test/forensic accessor used
    /// by the poisoning oracle to prove an existing entry is unchanged after a
    /// refused write. Not a tenant-facing path (it takes the physical key).
    pub fn raw_get(&self, physical_key: &str) -> Option<&[u8]> {
        self.entries.get(physical_key).map(Vec::as_slice)
    }

    /// Number of stored entries — used by the oracle to prove a refused write
    /// added nothing.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True iff the store holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── public-deterministic shared registry (item ③) ─────────────────────────────

/// The sentinel principal carried by a shared public-deterministic artifact's
/// attestation. It names the platform, never a tenant — surfacing it leaks
/// nothing.
pub const PLATFORM_PRINCIPAL: &str = "platform:hugit";

/// The sentinel runner identity carried by a shared artifact's attestation —
/// it discloses no producing-tenant runner.
pub const PLATFORM_RUNNER: &str = "platform:shared-deterministic";

/// A registry of public-deterministic artifacts that ARE shared cross-tenant
/// (item ③). Keyed by the *base* memo key directly (NOT tenant-namespaced):
/// a public-deterministic artifact has no tenant identity, so it lives in a
/// single global slot any tenant resolves identically.
///
/// The shared object's attestation is the PLATFORM-anonymized form — the
/// public-deterministic links (`tree`/`def`/`model`) are preserved (they carry
/// no tenant identity) while `runner` and `principal` are platform sentinels.
/// This is the no-leak side of the X2④ honesty property: a shared hit carries
/// NO byte of any producing tenant's private principal/runner.
#[derive(Debug, Default)]
pub struct SharedRegistry {
    /// base memo key → (bytes, platform attestation).
    entries: HashMap<String, (Vec<u8>, AttestationChain)>,
}

impl SharedRegistry {
    /// A fresh empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a public-deterministic artifact derived from `producer`'s
    /// attestation. The producer's private `runner`/`principal` links are
    /// DROPPED and replaced by platform sentinels before storage — not one byte
    /// of them is carried into the shared slot.
    ///
    /// `base_key` is the canonical, tenant-independent memo key
    /// ([`hugit_refstore::compute_memo_key`]).
    pub fn publish(&mut self, base_key: &str, bytes: &[u8], producer: &AttestationChain) {
        let anonymized = anonymize(producer);
        self.entries
            .insert(base_key.to_string(), (bytes.to_vec(), anonymized));
    }

    /// Resolve a shared artifact by its base key. Returns the bytes AND the
    /// platform-anonymized attestation. ANY tenant resolves the SAME slot
    /// identically — this is the by-design existence disclosure inherent to
    /// public-deterministic sharing (documented; not a leak).
    pub fn get(&self, base_key: &str) -> Option<(&[u8], &AttestationChain)> {
        self.entries.get(base_key).map(|(b, a)| (b.as_slice(), a))
    }
}

/// Produce the platform-anonymized attestation for a shared artifact: keep the
/// public-deterministic links (`tree`/`def`/`model`), replace the
/// tenant-identifying links (`runner`/`principal`) with platform sentinels, and
/// clear the producer's signature (a shared object is re-attested by the
/// platform, never carrying the producer's signature).
///
/// The producer chain is read ONLY to copy the public-deterministic links; its
/// `runner`/`principal`/`sig` are never carried through.
pub fn anonymize(producer: &AttestationChain) -> AttestationChain {
    AttestationChain {
        tree: producer.tree.clone(),
        def: producer.def.clone(),
        model: producer.model.clone(),
        runner: PLATFORM_RUNNER.to_string(),
        principal: vec![PLATFORM_PRINCIPAL.to_string()],
        sig: String::new(),
    }
}

/// True iff `chain`'s tenant-identifying fields carry NONE of the producing
/// tenant's private bytes — the no-leak check for item ③. Used to prove a shared
/// object carries none of the producing tenant's private principal/runner bytes.
///
/// This is an **allow-list** assertion, not a substring deny-scan. A deny-scan
/// only catches a leak that appears *verbatim*; it silently passes any encoded,
/// truncated, or otherwise transformed leak. Instead we assert the only
/// acceptable outcome: the tenant-identifying fields (`runner`, `principal`,
/// `sig`) MUST be EXACTLY the platform sentinel values that [`anonymize`] is
/// contracted to produce. Anything other than the sentinels — including an
/// encoded leak that no substring deny-scan would catch — fails.
///
/// The public-deterministic links (`tree`/`def`/`model`) carry no tenant
/// identity (they are reproducible hashes / the model name) and are preserved by
/// design, so they are not part of the allow-list.
pub fn carries_none_of(chain: &AttestationChain) -> bool {
    chain.runner == PLATFORM_RUNNER
        && chain.principal == [PLATFORM_PRINCIPAL.to_string()]
        && chain.sig.is_empty()
}
