//! The append-only, hash-chained event log core.
//!
//! There is exactly **one** mutating primitive in this WP: [`EventLog::append`].
//! Each appended [`EventRecord`] carries the hash of its predecessor; the chain
//! is the integrity spine. Nothing is ever rewritten.
//!
//! # THE canonical hash/memo/attestation byte-format (single source of truth)
//!
//! This module is the **one** place the hugit hash-chain, memo-key, and
//! attestation pre-images are realised in bytes. `hugit-contracts` froze the
//! *spec* (see the doc-comments on `EventRecord`, `CheckResult`,
//! `AttestationChain`); this module is the spec made executable, and is the
//! ground truth: if a doc and this code ever disagree, **this code wins** and the
//! doc is the bug. Every emitter (refstore, policy, diag, checks, app, …) MUST
//! call these functions rather than re-transcribe the format — re-transcription
//! is exactly the divergence that the 2026-06-07 brutal review (R1) caught.
//!
//! ## Primitive: length-prefixed field — `LP(s)`
//!
//! `LP(s)` = `u32_be(byte_len(s)) ‖ utf8_bytes(s)`. A 4-byte big-endian `u32`
//! byte-length prefix, then the raw UTF-8 bytes. This makes `‖` unambiguous
//! (WP-00): without the prefix `"ab" ‖ "c"` and `"a" ‖ "bc"` would collide.
//!
//! ## Primitive: vector framing — `VEC([e0, e1, …])`
//!
//! `VEC(v)` = `u32_be(len(v)) ‖ LP(e0) ‖ LP(e1) ‖ …`. A 4-byte big-endian
//! `u32` **element count**, then each element as an `LP` field. The count is
//! load-bearing: without it `["a","b"]`, `["ab"]`, and `["a"]` followed by a
//! trailing `"b"` field would all collide on the spine.
//!
//! ## `this_hash` (the event hash-chain)
//!
//! ```text
//! this_hash = lower_hex( SHA-256(
//!     LP(prev_hash) ‖ LP(kind) ‖ VEC(principal_chain) ‖ LP(payload) ‖ u64_be(seq)
//! ) )
//! ```
//!
//! Field order is `prev_hash, kind, principal_chain, payload, seq` (matching the
//! `EventRecord` spec). Notes:
//!
//! - `H` = SHA-256, emitted as a **64-char lowercase** hex digest.
//! - `prev_hash`, `kind`, `payload` are `LP` UTF-8 fields.
//! - `principal_chain` is `VEC(...)` — count-prefixed then per-element `LP`.
//! - `seq` is `u64_be` (an 8-byte big-endian integer, NOT length-prefixed — the
//!   one explicit exception, since its width is fixed).
//! - `payload` MUST already be **canonical JSON** when chained (see
//!   [`canonical_json`]); the chain hashes the payload bytes verbatim, so a
//!   producer that emits non-canonical JSON would hash differently from a
//!   verifier that re-canonicalised. Canonicalise before calling.
//! - `recorded_at` is **deliberately excluded** from the pre-image. It is an
//!   unauthenticated observability annotation; authenticated ordering comes from
//!   `seq` + the `prev_hash` linkage, not from a wall clock.
//! - genesis `prev_hash` = 64 ASCII `'0'` characters ([`GENESIS_PREV_HASH`]).
//!
//! ## `memo_key` (the check memoisation key)
//!
//! ```text
//! memo_key = lower_hex( SHA-256(
//!     LP(tree_hash) ‖ LP(def_digest) ‖ LP(toolchain_digest)
//! ) )
//! ```
//!
//! Field order is `tree_hash, def_digest, toolchain_digest` (the three memo axes
//! of `CheckResult`, in struct order). Each input is the **lowercase-hex** UTF-8
//! string of the respective digest (`LP` framed). Output is a 64-char lowercase
//! hex digest. See [`compute_memo_key`].
//!
//! ## attestation `sig` pre-image (ed25519)
//!
//! The bytes an [`AttestationChain`](hugit_contracts::AttestationChain) signature
//! is computed over (the message handed to ed25519 sign/verify) are:
//!
//! ```text
//! attestation_preimage =
//!     LP(tree) ‖ LP(def) ‖ LP(runner) ‖ LP(model) ‖ VEC(principal)
//! ```
//!
//! Field order is `tree, def, runner, model, principal` (matching the
//! `AttestationChain` struct order). `principal` uses `VEC(...)` framing
//! identical to `principal_chain` above. The result is the raw message; ed25519
//! is computed over it directly (no extra hashing). See
//! [`attestation_sig_preimage`].

use hugit_contracts::event_record::EventRecord;
use sha2::{Digest, Sha256};

/// Genesis predecessor hash: 64 ASCII `'0'` characters (the chain's anchor).
///
/// This is the literal 64-byte ASCII string `"000…0"`, NOT 32 zero bytes — the
/// first event's `prev_hash` field carries these 64 hex-zero characters and they
/// are `LP`-framed into its pre-image like any other `prev_hash`.
pub const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Append a single length-prefixed UTF-8 field — `LP(s)` — to a pre-image.
///
/// 4-byte big-endian `u32` byte-length prefix, then the raw UTF-8 bytes.
fn push_lp_field(buf: &mut Vec<u8>, field: &str) {
    let bytes = field.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(bytes);
}

/// Append a vector-framed field — `VEC(v)` — to a pre-image.
///
/// 4-byte big-endian `u32` element count, then each element as an `LP` field.
fn push_vec_field(buf: &mut Vec<u8>, elems: &[String]) {
    buf.extend_from_slice(&(elems.len() as u32).to_be_bytes());
    for e in elems {
        push_lp_field(buf, e);
    }
}

/// Compute `this_hash` for an event from the four chained inputs, exactly per
/// the canonical byte-format documented at the module level. Returns a 64-char
/// lowercase hex SHA-256 digest.
///
/// Pre-image: `LP(prev_hash) ‖ LP(kind) ‖ VEC(principal_chain) ‖ LP(payload) ‖
/// u64_be(seq)`. `recorded_at` is excluded by design; `payload` must already be
/// canonical JSON ([`canonical_json`]).
///
/// This is the *only* place the formula is realised; [`EventLog::append`] and
/// [`crate::tamper::verify_chain`] both route through it so the producer and the
/// verifier can never drift. Every other crate that emits events MUST call this
/// (re-import: `hugit_refstore::compute_this_hash`) rather than re-transcribe.
pub fn compute_this_hash(
    prev_hash: &str,
    kind: &str,
    principal_chain: &[String],
    payload: &str,
    seq: u64,
) -> String {
    let mut buf: Vec<u8> = Vec::new();

    // prev_hash ‖ kind   (LP UTF-8 fields)
    push_lp_field(&mut buf, prev_hash);
    push_lp_field(&mut buf, kind);

    // principal_chain    (VEC: u32 count, then each element LP)
    push_vec_field(&mut buf, principal_chain);

    // payload            (LP UTF-8 field — caller canonicalises JSON)
    push_lp_field(&mut buf, payload);

    // seq                (u64 big-endian — the fixed-width exception)
    buf.extend_from_slice(&seq.to_be_bytes());

    let digest = Sha256::digest(&buf);
    hex::encode(digest)
}

/// Compute the `CheckResult` `memo_key` from its three memo axes, exactly per
/// the canonical byte-format. Returns a 64-char lowercase hex SHA-256 digest.
///
/// Pre-image: `LP(tree_hash) ‖ LP(def_digest) ‖ LP(toolchain_digest)`. Each
/// input is the lowercase-hex UTF-8 string of the respective digest, in
/// `CheckResult` struct field order. This is the single canonical realisation
/// of the `CheckResult::memo_key` doc — call it, never re-transcribe.
pub fn compute_memo_key(tree_hash: &str, def_digest: &str, toolchain_digest: &str) -> String {
    let mut buf: Vec<u8> = Vec::new();
    push_lp_field(&mut buf, tree_hash);
    push_lp_field(&mut buf, def_digest);
    push_lp_field(&mut buf, toolchain_digest);
    let digest = Sha256::digest(&buf);
    hex::encode(digest)
}

/// Build the ed25519 signing/verification pre-image for an
/// [`AttestationChain`](hugit_contracts::AttestationChain).
///
/// Pre-image: `LP(tree) ‖ LP(def) ‖ LP(runner) ‖ LP(model) ‖ VEC(principal)`,
/// in `AttestationChain` struct field order. The returned bytes are the message
/// handed directly to ed25519 (no extra hashing layer). This is the single
/// canonical realisation of the `AttestationChain::sig` doc.
pub fn attestation_sig_preimage(
    tree: &str,
    def: &str,
    runner: &str,
    model: &str,
    principal: &[String],
) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    push_lp_field(&mut buf, tree);
    push_lp_field(&mut buf, def);
    push_lp_field(&mut buf, runner);
    push_lp_field(&mut buf, model);
    push_vec_field(&mut buf, principal);
    buf
}

/// Canonicalise a JSON string: parse then re-serialise with **sorted object
/// keys** and **no insignificant whitespace**, so equal JSON values map to
/// identical bytes regardless of authoring key-order/spacing.
///
/// `payload` on an [`EventRecord`] MUST be run through this before it is chained
/// via [`compute_this_hash`]; otherwise a producer and a verifier that disagree
/// on key-order/whitespace would compute different `this_hash` for the same
/// logical event. Returns `None` if the input is not valid JSON (the caller
/// decides whether to reject or to chain the raw bytes — but a chained payload
/// is by contract canonical JSON).
///
/// # Feature-proof key ordering (NOT ambient — explicit)
///
/// Key-sorting here does **not** rely on serde_json's default `BTreeMap`-backed
/// object representation. That representation is feature-controlled: a single
/// transitive dependency anywhere in the build graph that enables serde_json's
/// `preserve_order` feature flips objects to an insertion-ordered `IndexMap`,
/// and (because cargo features are additive + unified across the whole graph)
/// it would do so for *this* crate too — silently diverging every `this_hash`
/// fleet-wide without a single line of code changing here. To make that
/// impossible, [`canonicalize_value`] walks the parsed `Value` and explicitly
/// rebuilds every object through a [`BTreeMap`](std::collections::BTreeMap), so
/// the serialized key order is sorted by construction regardless of which
/// backing map serde_json compiled with. The compact `to_string` then strips
/// insignificant whitespace. The byte output is identical to the previous
/// default-feature path (pin: `canonical_format_pin`), so this is a hardening,
/// not a format change.
pub fn canonical_json(input: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(input).ok()?;
    let canonical = canonicalize_value(value);
    serde_json::to_string(&canonical).ok()
}

/// Recursively rebuild a [`serde_json::Value`] with every object's keys in
/// sorted (`BTreeMap`) order, independent of serde_json's `preserve_order`
/// feature. Scalars pass through unchanged; arrays preserve element order
/// (arrays are ordered by JSON semantics) but each element is canonicalised.
///
/// This is the feature-proofing core of [`canonical_json`]: by routing every
/// object through a `BTreeMap`, sorted key order is guaranteed by the data
/// structure rather than inherited from an ambient cargo feature.
fn canonicalize_value(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    use std::collections::BTreeMap;
    match value {
        Value::Object(map) => {
            // BTreeMap sorts keys lexicographically by the same byte ordering
            // serde_json's default object emits; rebuilding through it pins the
            // order explicitly. Re-collect into a serde_json::Map so the result
            // is a Value::Object whatever backing map serde_json compiled with.
            let sorted: BTreeMap<String, Value> = map
                .into_iter()
                .map(|(k, v)| (k, canonicalize_value(v)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonicalize_value).collect()),
        scalar => scalar,
    }
}

/// The append-only, hash-chained event log for one repository.
///
/// In production exactly one Durable Object owns one of these and is the single
/// writer. This type holds the in-memory chain and exposes the one mutating
/// primitive ([`append`](EventLog::append)); persistence/transport is the
/// caller's concern (D1b cold-tier offload preserves the chain semantics
/// sealed here).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventLog {
    records: Vec<EventRecord>,
}

/// Error returned by the append path when an input violates the append-only,
/// monotonic-sequence invariant. Append never rewrites history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendError {
    /// `seq` was not exactly `len()` (the next slot in a 0-based, gap-free log).
    NonMonotonicSeq { expected: u64, got: u64 },
}

impl std::fmt::Display for AppendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppendError::NonMonotonicSeq { expected, got } => write!(
                f,
                "non-monotonic seq: expected {expected}, got {got} (log is append-only)"
            ),
        }
    }
}

impl std::error::Error for AppendError {}

/// Returned by [`EventLog::append_authorized`] when the D14 matrix denies the
/// mutation. The requested event was **not** appended; the carried `audit`
/// record is the `authz.denied` event that *was* appended (③) so the denial is
/// attributable. Carries the [`DenyReason`](crate::authz::DenyReason) for the
/// caller to map to its own structured error.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthzDenied {
    /// Why the mutation was denied.
    pub reason: crate::authz::DenyReason,
    /// The `authz.denied` audit record that was appended for this denial.
    /// Boxed to keep the error variant small (clippy `result_large_err`).
    pub audit: Box<EventRecord>,
}

impl std::fmt::Display for AuthzDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "authorization denied ({}): the mutation was not appended (audited as {})",
            self.reason.code(),
            crate::authz::AUTHZ_DENIED_KIND
        )
    }
}

impl std::error::Error for AuthzDenied {}

impl EventLog {
    /// A fresh, empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of records currently in the log (also the `seq` of the next append).
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the log holds no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The records in chain order (read-only view).
    pub fn records(&self) -> &[EventRecord] {
        &self.records
    }

    /// `this_hash` of the last record, or [`GENESIS_PREV_HASH`] for an empty log.
    /// This is the `prev_hash` the next [`append`](EventLog::append) will chain to.
    pub fn head_hash(&self) -> String {
        self.records
            .last()
            .map(|r| r.this_hash.clone())
            .unwrap_or_else(|| GENESIS_PREV_HASH.to_string())
    }

    /// Append one event to the log, computing its place in the hash chain.
    ///
    /// The new record's `seq` is the current length, its `prev_hash` is the
    /// current [`head_hash`](EventLog::head_hash), and its `this_hash` is the
    /// frozen formula over `(prev_hash, kind, principal_chain, payload, seq)`.
    /// The fully-formed [`EventRecord`] is returned (cloned) for the caller to
    /// persist / broadcast.
    ///
    /// This is the only mutating primitive in the WP. It never rewrites an
    /// existing record.
    ///
    /// # TRUSTED append — bypasses the D14 authorization guard
    ///
    /// This raw entry point performs **no authorization**. It is the trusted
    /// primitive used by (a) internal machinery that has already authorized or
    /// is not a user-facing mutation verb (replay rehydration, the audit-record
    /// emit inside [`append_authorized`]/[`crate::authz::AuditedGuard`] itself,
    /// fixtures, golden pins), and (b) emitters whose principal is system-fixed.
    /// User-facing **mutating forge verbs** (`pr open`/`land`, `push`, `undo`,
    /// `policy`) MUST route through [`append_authorized`](EventLog::append_authorized)
    /// so the D14 matrix gates the mutation and denials are audited. Calling this
    /// directly for a guarded verb is the bypass S3 flagged — don't.
    pub fn append(
        &mut self,
        kind: impl Into<String>,
        principal_chain: Vec<String>,
        payload: impl Into<String>,
        recorded_at: u64,
    ) -> EventRecord {
        let seq = self.records.len() as u64;
        let prev_hash = self.head_hash();
        let kind = kind.into();
        let payload = payload.into();

        let this_hash = compute_this_hash(&prev_hash, &kind, &principal_chain, &payload, seq);

        let record = EventRecord {
            seq,
            prev_hash,
            this_hash,
            kind,
            principal_chain,
            payload,
            recorded_at,
        };
        self.records.push(record.clone());
        record
    }

    /// Append a mutating-verb event **through the D14 authorization guard**.
    ///
    /// This is the guarded entry point that makes the D14 matrix unbypassable on
    /// the mutation path. The caller names the mutating [`Endpoint`](crate::authz::Endpoint)
    /// and the **asserted** [`PrincipalClass`](crate::authz::PrincipalClass) of
    /// the actor driving it; the class is checked against the frozen permission
    /// matrix ([`authorize_class`](crate::authz::authorize_class)):
    ///
    /// - **Allow** → the real event is appended (raw [`append`](EventLog::append))
    ///   and the [`EventRecord`] returned.
    /// - **Deny** → the event is **not** appended; instead an `authz.denied`
    ///   audit record is appended (③ — the denial is attributable, never silent)
    ///   and an [`AuthzDenied`] error is returned carrying the reason and the
    ///   audit record.
    ///
    /// The asserted class is **caller-supplied** until identity rollout binds it
    /// to an authenticated principal (the disclosed seam — see the
    /// [`authz`](crate::authz) module doc). The matrix decision itself is real and
    /// enforced here on the only mutating primitive.
    pub fn append_authorized(
        &mut self,
        class: crate::authz::PrincipalClass,
        endpoint: crate::authz::Endpoint,
        kind: impl Into<String>,
        principal_chain: Vec<String>,
        payload: impl Into<String>,
        recorded_at: u64,
    ) -> Result<EventRecord, AuthzDenied> {
        use crate::authz::{Decision, authorize_class, denial_payload};
        match authorize_class(class, endpoint) {
            Decision::Allow => Ok(self.append(kind, principal_chain, payload, recorded_at)),
            Decision::Deny(reason) => {
                // Audit the denial (③) via the trusted primitive — the audit
                // record is system-emitted, not a user mutation.
                let audit = self.append(
                    crate::authz::AUTHZ_DENIED_KIND,
                    principal_chain,
                    denial_payload(endpoint, &reason),
                    recorded_at,
                );
                Err(AuthzDenied {
                    reason,
                    audit: Box::new(audit),
                })
            }
        }
    }

    /// Append a pre-formed [`EventRecord`] (e.g. rehydrated from storage),
    /// enforcing the monotonic, gap-free `seq` invariant.
    ///
    /// Used when loading a persisted chain. The record's hashes are taken as
    /// given here; integrity is established separately by
    /// [`crate::tamper::verify_chain`].
    pub fn push_record(&mut self, record: EventRecord) -> Result<(), AppendError> {
        let expected = self.records.len() as u64;
        if record.seq != expected {
            return Err(AppendError::NonMonotonicSeq {
                expected,
                got: record.seq,
            });
        }
        self.records.push(record);
        Ok(())
    }
}
