//! Art.17 provenance-PII erasure primitives — the shreddable subject pseudonym.
//!
//! See `docs/adr/0004-art17-provenance-pii-erasure.md` for the decision this
//! module implements the FIRST, additive slice of.
//!
//! ## The gap this closes (and the constraint that shapes it)
//!
//! GDPR erasure ([`crate::writes::erasure`]) operates on the OBJECT/content store
//! (tombstone repos, physically GC the account-exclusive CAS objects), but NEVER on
//! the append-only PROVENANCE chain. Yet the provenance records carry the subject's
//! CLEARTEXT identifiers — the `account` slug in the payload and `clerk:<org>:<user>`
//! in every record's `principal_chain` — which therefore PERSIST after a "completed"
//! Art.17 erasure. Residual cleartext PII after erasure is an Art.17 completeness hole.
//!
//! The chain is **append-only + tamper-evident**: `this_hash` is a frozen SHA-256 over
//! `(prev_hash, kind, principal_chain, payload, seq)` (see
//! [`hugit_refstore::compute_this_hash`]) and [`hugit_refstore::verify_chain`]
//! recomputes it from those exact bytes. You cannot mutate a past record's
//! `principal_chain`/`payload` without breaking `this_hash` — so cleartext PII cannot
//! simply be scrubbed in place.
//!
//! ## The chosen mechanism (ADR-0004: B + A's key-shred)
//!
//! Records store a **pseudonymous subject-ref** — a per-subject *keyed* HMAC of the
//! account slug, `subj:<hex>` — instead of the cleartext identity. The chain hashes
//! the PSEUDONYM, so integrity + tamper-evidence are preserved by construction. The
//! cleartext↔pseudonym linkability lives ONLY in the per-subject KEY held by a
//! [`SubjectKeyStore`]; erasure SHREDS that key ([`SubjectKeyStore::shred`]). After the
//! shred the pseudonym bytes remain in the chain (integrity intact) but:
//! - it cannot be REVERSED to the account (HMAC is one-way, and the key is gone), and
//! - it cannot be RE-DERIVED from a guessed account slug (the per-subject key defeats
//!   the low-entropy-slug brute force that a plain unsalted hash would fall to —
//!   account slugs are `[a-z0-9-]`, ≤64, trivially enumerable).
//!
//! That is a GDPR-recognised crypto-shred: cleartext rendered UNRECOVERABLE while a
//! PSEUDONYMOUS accountability record is RETAINED (lawful + required under Art.5(2) —
//! you must be able to demonstrate you honoured the erasure).
//!
//! ## Scope of THIS slice (additive; see ADR-0004 for the rest)
//!
//! This module ships the shreddable primitive + the retained-accountability record
//! builder, fully unit-tested, WITHOUT touching the hot write path, the identity
//! readers (authz/ownership/projection), or the tamper core. Wiring new writes to
//! emit pseudonyms, the backfill of existing cleartext records, and the
//! redaction-aware verifier are the follow-up legs the ADR specifies.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The stable prefix that marks a pseudonymous subject-ref in a provenance field,
/// distinguishing it from a cleartext `clerk:<org>:<user>` principal. A reader can
/// tell "this record is already pseudonymous" from the prefix alone.
pub const SUBJECT_PSEUDONYM_PREFIX: &str = "subj:";

/// The event kind of the RETAINED, non-reversible Art.17 accountability record — the
/// pseudonymous "subject `<pseudonym>` erasure requested@T / executed@T'" the engine
/// keeps to demonstrate compliance (Art.5(2)) AFTER the cleartext is shredded. Carries
/// NO cleartext account/principal — see [`PseudonymousErasureRecord`].
pub const ERASURE_PII_SHREDDED_KIND: &str = "erasure.pii_shredded";

/// A per-subject secret key — the ONLY thing that links a [`SubjectPseudonym`] back to
/// its cleartext account. 32 bytes from a CSPRNG. Erasure SHREDS it; once gone, the
/// pseudonym is unrecoverable (one-way HMAC + no key to re-derive under).
///
/// `zeroize`-on-drop is intentionally NOT taken as a new dependency in this slice — the
/// in-memory store below drops the boxed bytes on `shred`; a hardened store (the durable
/// follow-up, ADR-0004) owns key-material hygiene. The type is opaque (`Debug` redacts).
#[derive(Clone, PartialEq, Eq)]
pub struct SubjectKey([u8; 32]);

impl SubjectKey {
    /// Wrap raw key bytes (the durable store's job to source from a CSPRNG / KMS).
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw key bytes — used only to key the HMAC below.
    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// REDACTING Debug — key material must never appear in a log/panic.
impl std::fmt::Debug for SubjectKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SubjectKey(<redacted>)")
    }
}

/// A non-reversible pseudonymous subject-ref: `subj:<hex>` where
/// `<hex> = HMAC-SHA256(subject_key, account_slug)`. Stored in the provenance chain in
/// place of the cleartext account/principal. Opaque + stable given the key; irreversible
/// once the key is shredded.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SubjectPseudonym(String);

impl SubjectPseudonym {
    /// Derive the pseudonym for `account_slug` under `key`. Keyed (HMAC), so it is NOT
    /// recomputable without the key — the property that makes the low-entropy account
    /// slug space (`[a-z0-9-]`, ≤64) safe against a brute-force re-identification once
    /// the key is shredded (an unsalted hash would be trivially reversible).
    #[must_use]
    pub fn derive(key: &SubjectKey, account_slug: &str) -> Self {
        let mut mac =
            HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
        mac.update(account_slug.as_bytes());
        let hex = hex::encode(mac.finalize().into_bytes());
        Self(format!("{SUBJECT_PSEUDONYM_PREFIX}{hex}"))
    }

    /// The wire form (`subj:<hex>`), for storing in a `principal_chain` entry or a
    /// payload `subject` field.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// REDACTING-safe Debug: the pseudonym is NOT cleartext PII (that is the point), so it
/// prints in full — useful in tamper diagnostics without exposing the account.
impl std::fmt::Debug for SubjectPseudonym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SubjectPseudonym({})", self.0)
    }
}

/// Whether a provenance field value is ALREADY a pseudonymous subject-ref (vs a
/// cleartext `clerk:…` principal). The backfill (ADR-0004) uses this to skip
/// already-pseudonymised records idempotently.
#[must_use]
pub fn is_pseudonymous(value: &str) -> bool {
    value.starts_with(SUBJECT_PSEUDONYM_PREFIX)
}

/// The per-subject key store — the erasable secret the Art.17 shred deletes. A completed
/// erasure calls [`shred`](SubjectKeyStore::shred); thereafter [`key_for`](SubjectKeyStore::key_for)
/// returns `None`, so the subject's pseudonyms can no longer be linked to cleartext.
///
/// The durable production impl (KMS / a per-subject-keyed secrets table) is the ADR-0004
/// follow-up; this trait is the seam so the erasure executor drives ONE method
/// (`shred`) regardless of backend, and tests use the in-memory double below.
pub trait SubjectKeyStore {
    /// Fetch the subject's key, or `None` if never minted OR already shredded. `None`
    /// after a shred is the whole point — it is what renders the pseudonym irreversible.
    fn key_for(&self, subject: &str) -> Option<SubjectKey>;

    /// Fetch-or-mint the subject's key (idempotent; a repeat returns the SAME key so the
    /// pseudonym is stable across a subject's records). The write path calls this before
    /// deriving a pseudonym.
    fn ensure_key(&self, subject: &str) -> SubjectKey;

    /// SHRED the subject's key — the irreversible Art.17 action. Idempotent (shredding an
    /// absent key is a no-op success). After this the subject's cleartext is
    /// UNRECOVERABLE from the provenance.
    fn shred(&self, subject: &str);
}

/// An in-memory [`SubjectKeyStore`] — the hermetic test double + the reference semantics
/// the durable store must match. Interior-mutable so it is a shared `&dyn` like the other
/// engine stores. NOT for production (keys evaporate on restart); the durable backend is
/// ADR-0004 follow-up.
#[derive(Default)]
pub struct InMemorySubjectKeyStore {
    keys: std::sync::Mutex<std::collections::HashMap<String, SubjectKey>>,
    /// Deterministic key source for tests — a monotonic counter expanded to 32 bytes.
    /// The durable store uses a CSPRNG; determinism here keeps the pseudonym assertions
    /// reproducible without pulling `rand` into this slice.
    next: std::sync::atomic::AtomicU64,
}

impl InMemorySubjectKeyStore {
    /// A fresh, empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a fresh 32-byte key deterministically (test-double behaviour — see the field
    /// doc). Distinct per call so two subjects never collide.
    fn mint(&self) -> SubjectKey {
        let n = self
            .next
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(1);
        let mut bytes = [0u8; 32];
        // Spread the counter across the block so the key is not mostly-zero (defensive;
        // the HMAC is keyed regardless, but a near-zero key reads badly in a dump).
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            let v = n.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(i as u64);
            chunk.copy_from_slice(&v.to_be_bytes());
        }
        SubjectKey::from_bytes(bytes)
    }
}

impl SubjectKeyStore for InMemorySubjectKeyStore {
    fn key_for(&self, subject: &str) -> Option<SubjectKey> {
        self.keys
            .lock()
            .expect("key store mutex")
            .get(subject)
            .cloned()
    }

    fn ensure_key(&self, subject: &str) -> SubjectKey {
        let mut keys = self.keys.lock().expect("key store mutex");
        keys.entry(subject.to_string())
            .or_insert_with(|| self.mint())
            .clone()
    }

    fn shred(&self, subject: &str) {
        self.keys.lock().expect("key store mutex").remove(subject);
    }
}

/// The RETAINED, non-reversible Art.17 accountability record (payload of an
/// [`ERASURE_PII_SHREDDED_KIND`] event). Records THAT an erasure happened and WHEN,
/// keyed on the pseudonym + the opaque DSR legitimacy id — and carries NO cleartext
/// account/principal, so appending it to the append-only account log introduces no new
/// cleartext PII. This is the lawful pseudonymous record kept under Art.5(2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PseudonymousErasureRecord {
    /// The frozen pseudonym label for the erased subject (irreversible post-shred).
    pub subject_pseudonym: SubjectPseudonym,
    /// The opaque DSR legitimacy id (non-PII; githugr's anchor mints it). The regulator-
    /// facing handle that ties this record to the request WITHOUT the account slug.
    pub dsr_id: Option<String>,
    /// Unix-ms the erasure was REQUESTED (the staged `erasure.requested`).
    pub requested_at: u64,
    /// Unix-ms the cleartext PII was SHREDDED (this record's instant).
    pub executed_at: u64,
}

impl PseudonymousErasureRecord {
    /// Serialise to the canonical-JSON payload for the account-log append. Contains ONLY
    /// the pseudonym, the opaque DSR id, and the two timestamps — never a cleartext
    /// account/principal. `cleartext_shredded: true` is the affirmative compliance claim.
    #[must_use]
    pub fn to_canonical_payload(&self) -> String {
        let mut v = serde_json::json!({
            "subject_pseudonym": self.subject_pseudonym.as_str(),
            "requested_at": self.requested_at,
            "executed_at": self.executed_at,
            "cleartext_shredded": true,
        });
        if let Some(d) = self.dsr_id.as_deref().filter(|s| !s.is_empty()) {
            v["dsr_id"] = serde_json::json!(d);
        }
        hugit_refstore::canonical_json(&v.to_string()).unwrap_or_else(|| v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, EventLog, PrincipalClass, verify_chain};

    #[test]
    fn pseudonym_is_stable_under_the_same_key() {
        let store = InMemorySubjectKeyStore::new();
        let key = store.ensure_key("acme");
        let a = SubjectPseudonym::derive(&key, "acme");
        let b = SubjectPseudonym::derive(&key, "acme");
        assert_eq!(
            a, b,
            "same key + same account ⇒ same pseudonym (stable label)"
        );
        assert!(a.as_str().starts_with(SUBJECT_PSEUDONYM_PREFIX));
        assert!(is_pseudonymous(a.as_str()));
        assert!(
            !is_pseudonymous("clerk:acme:user-1"),
            "cleartext principal is not pseudonymous"
        );
    }

    #[test]
    fn distinct_subjects_get_distinct_pseudonyms() {
        let store = InMemorySubjectKeyStore::new();
        let ka = store.ensure_key("acme");
        let kb = store.ensure_key("beta");
        assert_ne!(
            SubjectPseudonym::derive(&ka, "acme"),
            SubjectPseudonym::derive(&kb, "beta"),
            "different subjects ⇒ different pseudonyms"
        );
    }

    #[test]
    fn keyed_pseudonym_is_not_recomputable_without_the_key() {
        // The crux vs a naive unsalted redaction: because the pseudonym is a KEYED HMAC,
        // an attacker who knows the (low-entropy, enumerable) account slug still cannot
        // recompute the pseudonym without the per-subject key.
        let store = InMemorySubjectKeyStore::new();
        let real_key = store.ensure_key("acme");
        let pseudonym = SubjectPseudonym::derive(&real_key, "acme");

        // A guessed key (what a brute-forcer has post-shred: none, but simulate a wrong one)
        // over the KNOWN account slug does not reproduce the pseudonym.
        let wrong_key = SubjectKey::from_bytes([7u8; 32]);
        assert_ne!(
            SubjectPseudonym::derive(&wrong_key, "acme"),
            pseudonym,
            "knowing the account slug is not enough — the key gates re-identification"
        );
    }

    #[test]
    fn shred_makes_the_key_unrecoverable() {
        let store = InMemorySubjectKeyStore::new();
        let _ = store.ensure_key("acme");
        assert!(store.key_for("acme").is_some(), "key present before erase");

        store.shred("acme");
        assert!(
            store.key_for("acme").is_none(),
            "after shred the key is gone ⇒ the pseudonym can no longer be linked to cleartext"
        );
        // Idempotent: a re-shred (idempotent erase replay) is a no-op success.
        store.shred("acme");
        assert!(store.key_for("acme").is_none());
    }

    #[test]
    fn accountability_record_carries_no_cleartext() {
        let store = InMemorySubjectKeyStore::new();
        let key = store.ensure_key("acme-corp");
        let rec = PseudonymousErasureRecord {
            subject_pseudonym: SubjectPseudonym::derive(&key, "acme-corp"),
            dsr_id: Some("dsr-abc123".to_string()),
            requested_at: 1000,
            executed_at: 2000,
        };
        let payload = rec.to_canonical_payload();
        assert!(
            !payload.contains("acme-corp"),
            "the retained record must not carry the account slug"
        );
        assert!(payload.contains("subj:"), "it carries the pseudonym");
        assert!(payload.contains("dsr-abc123"), "and the opaque DSR handle");
        assert!(payload.contains("\"cleartext_shredded\":true"));
    }

    /// The END-STATE property, demonstrated hermetically on a real [`EventLog`] via the
    /// PUBLIC chain API — no core edits: a record written PSEUDONYMOUSLY (pseudonym in
    /// `principal_chain` + payload, never the cleartext) verifies; after the key is
    /// shredded the cleartext is UNRECOVERABLE while the chain STILL verifies and the
    /// pseudonymous accountability record REMAINS.
    #[test]
    fn pseudonymous_record_verifies_and_survives_shred() {
        let account = "acme-corp";
        let store = InMemorySubjectKeyStore::new();
        let key = store.ensure_key(account);
        let pseudonym = SubjectPseudonym::derive(&key, account);

        // Build a chain the way a pseudonymised write path WOULD: the identity in both
        // `principal_chain` and the payload is the pseudonym, never `clerk:acme-corp:user`.
        let mut log = EventLog::new();
        let payload = serde_json::json!({ "subject": pseudonym.as_str(), "state": "requested" });
        let payload = hugit_refstore::canonical_json(&payload.to_string()).unwrap();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "erasure.requested",
            vec![pseudonym.as_str().to_string()],
            payload,
            1000,
        )
        .expect("orchestrator may append on Land");

        // Retain the pseudonymous accountability record on the SAME append-only log.
        let rec = PseudonymousErasureRecord {
            subject_pseudonym: pseudonym.clone(),
            dsr_id: Some("dsr-abc123".to_string()),
            requested_at: 1000,
            executed_at: 2000,
        };
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            ERASURE_PII_SHREDDED_KIND,
            vec![pseudonym.as_str().to_string()],
            rec.to_canonical_payload(),
            2000,
        )
        .expect("orchestrator may append on Land");

        // Happy path unchanged: the chain verifies BEFORE the shred.
        verify_chain(log.records()).expect("pseudonymous chain verifies");

        // ERASE: shred the subject's key.
        store.shred(account);

        // 1. The chain STILL verifies (integrity + tamper-evidence preserved — the
        //    pseudonym bytes hashed into `this_hash` are untouched).
        verify_chain(log.records()).expect("chain still verifies after shred");

        // 2. The subject's CLEARTEXT account/principal is UNRECOVERABLE from the
        //    provenance: no record carries the cleartext slug, and it can no longer be
        //    re-derived to its pseudonym (the key is gone).
        assert!(store.key_for(account).is_none());
        for r in log.records() {
            assert!(
                !r.payload.contains(account),
                "no payload carries the cleartext account slug"
            );
            assert!(
                r.principal_chain
                    .iter()
                    .all(|p| !p.contains(account) && is_pseudonymous(p)),
                "principal_chain is pseudonymous, never cleartext clerk:{account}:…"
            );
        }

        // 3. The pseudonymous accountability record REMAINS (Art.5(2) demonstrability).
        let kept = log
            .records()
            .iter()
            .find(|r| r.kind == ERASURE_PII_SHREDDED_KIND)
            .expect("the accountability record is retained");
        assert!(kept.payload.contains(pseudonym.as_str()));
        assert!(kept.payload.contains("\"executed_at\":2000"));
    }
}
