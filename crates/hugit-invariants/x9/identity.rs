//! WP-X9 production surface — cross-phase object identity.
//!
//! This module models the **identity intersection** of four already-built
//! surfaces — phase-B memoization (B2), phase-D verdict panels (D7), the phase-B
//! intent sidecar (B6), and phase-D native intents (D4). It owns the
//! *cross-phase byte-identity* surface for a [`CheckResult`] and the
//! *one-lifecycle-one-id* surface for an intent id; it does NOT re-prove
//! memoization, verdict rendering, or intent resolution — it proves the SAME
//! object / the SAME id crosses the phase boundary unchanged.
//!
//! # The identity law (WP-X9 owned items)
//!
//! ① **Cross-phase `CheckResult` bit-identity.** For the same
//!    `(tree, def, toolchain)` the bytes phase-B memoizes equal the bytes
//!    phase-D serves as evidence. Identity is established by a single canonical
//!    serializer ([`canonical_bytes`]) routed through the production
//!    canonicaliser ([`hugit_refstore::canonical_json`]); both phases serve the
//!    SAME bytes, so [`content_id`] agrees byte-for-byte.
//!
//! ② **Mismatch fails CLOSED + alerts.** [`serve_evidence`] re-derives the
//!    canonical bytes phase-D would serve and compares them to the phase-B memo
//!    bytes; any divergence returns [`IdentityError::EvidenceMismatch`] (a
//!    fail-closed refusal carrying an [`Alert`]) rather than serving the
//!    divergent object.
//!
//! ③ **`intent_id` identity — one lifecycle, one id.** A phase-B
//!    [`IntentSidecar`] mints an `intent_id`; the phase-D native intent (the
//!    [`VerdictObject::intent`] field) for the SAME logical intent must carry the
//!    EXACT same id. [`reconcile_intent`] fails CLOSED on divergence (a second
//!    id minted) and [`IntentRegistry`] fails CLOSED on collision (two distinct
//!    logical intents folding onto one id). Identity ≠ resolution.

use std::collections::BTreeMap;

use hugit_contracts::check_result::CheckResult;
use hugit_contracts::intent_sidecar::IntentSidecar;
use hugit_contracts::verdict_object::VerdictObject;
use hugit_refstore::{canonical_json, compute_memo_key, compute_this_hash};

// ── shared: the alert that every fail-closed path raises ─────────────────────

/// An alert raised whenever a cross-phase identity check fails closed.
///
/// Items ② and ③ both require that a divergence/collision not only be REFUSED
/// but SURFACED — a silent refusal is indistinguishable from a bug. Every
/// fail-closed error in this module therefore carries an `Alert`, and
/// [`IdentityError::alert`] / [`IntentError::alert`] expose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    /// Machine-stable code for routing/alerting (e.g. `"x9.evidence_mismatch"`).
    pub code: String,
    /// Human-readable description of the divergence/collision.
    pub detail: String,
}

impl Alert {
    fn new(code: &str, detail: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            detail: detail.into(),
        }
    }
}

// ── Item ① / ② — cross-phase CheckResult bit-identity ────────────────────────

/// The fixed framing fields used to fold a `CheckResult`'s canonical bytes into
/// the production chain hasher ([`compute_this_hash`]). Reusing the canonical
/// hasher (rather than re-transcribing SHA-256) keeps X9's content-id on the
/// single-sourced production formula.
const CONTENT_ID_KIND: &str = "x9.check_result";
/// Fixed principal framing for the content-id pre-image (identity, not authz).
const CONTENT_ID_PRINCIPAL: &str = "x9.identity";

/// Serialize a [`CheckResult`] to its **canonical bytes** — the single
/// cross-phase wire form.
///
/// The struct is serialized to JSON then re-canonicalised through the production
/// [`canonical_json`] (sorted object keys, no insignificant whitespace), so two
/// phases that authored the SAME logical result with different field-emission
/// order/spacing still produce IDENTICAL bytes. Bit-identity (item ①) is a byte
/// compare of these.
///
/// # Panics
/// Never on a well-formed [`CheckResult`]: `serde_json` cannot fail to serialize
/// the frozen struct, and its output is always valid JSON for `canonical_json`.
pub fn canonical_bytes(result: &CheckResult) -> Vec<u8> {
    let raw = serde_json::to_string(result).expect("CheckResult serializes to JSON");
    let canon = canonical_json(&raw).expect("serde_json output is valid JSON");
    canon.into_bytes()
}

/// The content id of a [`CheckResult`]: the canonical bytes folded through the
/// production chain hasher. Equal `(tree, def, toolchain, …)` results yield the
/// same id; any byte divergence yields a different id.
///
/// This is a convenience over [`canonical_bytes`] for index/compare; the
/// contract's bit-identity guarantee is the BYTE compare, and this id is derived
/// from those exact bytes (so it can never disagree with them).
pub fn content_id(result: &CheckResult) -> String {
    let payload =
        String::from_utf8(canonical_bytes(result)).expect("canonical_json output is valid UTF-8");
    compute_this_hash(
        "",
        CONTENT_ID_KIND,
        &[CONTENT_ID_PRINCIPAL.to_string()],
        &payload,
        0,
    )
}

/// The phase-B memoization surface (B2), consumed as-built.
///
/// A content-addressed memo: `check(tree, def, toolchain)` keyed by the canonical
/// [`compute_memo_key`]. The store holds the SAME [`CheckResult`] object phase-B
/// produced; phase-D reads it back through [`serve_evidence`].
#[derive(Debug, Clone, Default)]
pub struct MemoStore {
    by_key: BTreeMap<String, CheckResult>,
}

impl MemoStore {
    /// A fresh, empty memo store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Memoize a phase-B [`CheckResult`] under its canonical memo key.
    ///
    /// The key is recomputed from the result's three axes via the production
    /// [`compute_memo_key`] (never trusting `result.memo_key` blindly), so a
    /// memo whose stated key disagrees with its axes is normalised to the
    /// canonical key the lookup will use.
    pub fn memoize(&mut self, result: CheckResult) {
        let key = compute_memo_key(
            &result.tree_hash,
            &result.def_digest,
            &result.toolchain_digest,
        );
        self.by_key.insert(key, result);
    }

    /// Look up the memoized result for `(tree, def, toolchain)`.
    pub fn get(
        &self,
        tree_hash: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Option<&CheckResult> {
        let key = compute_memo_key(tree_hash, def_digest, toolchain_digest);
        self.by_key.get(&key)
    }
}

/// Why a cross-phase evidence serve fails the X9 ①/② identity law.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// No phase-B memo exists for the requested `(tree, def, toolchain)`. Phase-D
    /// must not invent evidence — fail closed.
    NoMemo { memo_key: String, alert: Alert },
    /// The bytes phase-D would serve as evidence diverge from the phase-B memo
    /// bytes for the SAME memo key — a re-serialized/tampered object. Fail
    /// closed + alert rather than serve a divergent object as evidence.
    EvidenceMismatch {
        /// The memo key under which the divergence was detected.
        memo_key: String,
        /// Content id of the phase-B memoized object.
        memo_id: String,
        /// Content id of the candidate evidence object phase-D tried to serve.
        evidence_id: String,
        /// The fail-closed alert.
        alert: Alert,
    },
}

impl IdentityError {
    /// The alert this fail-closed error carries.
    pub fn alert(&self) -> &Alert {
        match self {
            IdentityError::NoMemo { alert, .. } => alert,
            IdentityError::EvidenceMismatch { alert, .. } => alert,
        }
    }
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::NoMemo { memo_key, .. } => {
                write!(
                    f,
                    "x9: no phase-B memo for memo_key {memo_key} (fail-closed)"
                )
            }
            IdentityError::EvidenceMismatch {
                memo_key,
                memo_id,
                evidence_id,
                ..
            } => write!(
                f,
                "x9: cross-phase CheckResult mismatch for {memo_key}: memo {memo_id} != evidence {evidence_id} (fail-closed)"
            ),
        }
    }
}

impl std::error::Error for IdentityError {}

/// Serve a phase-D verdict-panel evidence [`CheckResult`] for
/// `(tree, def, toolchain)`, **proving cross-phase byte-identity** against the
/// phase-B memo (items ① + ②).
///
/// `candidate` is what phase-D's panel proposes to serve as evidence. This:
/// 1. Looks up the phase-B memo for the same memo key — absent ⇒ fail closed
///    ([`IdentityError::NoMemo`]).
/// 2. Compares the candidate's canonical bytes to the memo's canonical bytes.
///    Divergent ⇒ fail closed + alert ([`IdentityError::EvidenceMismatch`]) —
///    never serve a divergent object as evidence.
/// 3. Identical ⇒ return the canonical bytes (the ONE object both phases share).
///
/// The match arm is a BYTE compare ([`canonical_bytes`]), exactly as the
/// contract requires ("bit-identity = byte compare, not result-equal").
pub fn serve_evidence(
    memo: &MemoStore,
    tree_hash: &str,
    def_digest: &str,
    toolchain_digest: &str,
    candidate: &CheckResult,
) -> Result<Vec<u8>, IdentityError> {
    let memo_key = compute_memo_key(tree_hash, def_digest, toolchain_digest);

    let memoized = memo
        .get(tree_hash, def_digest, toolchain_digest)
        .ok_or_else(|| IdentityError::NoMemo {
            memo_key: memo_key.clone(),
            alert: Alert::new(
                "x9.no_memo",
                format!("phase-D requested evidence with no phase-B memo for {memo_key}"),
            ),
        })?;

    let memo_bytes = canonical_bytes(memoized);
    let evidence_bytes = canonical_bytes(candidate);

    if memo_bytes != evidence_bytes {
        let memo_id = content_id(memoized);
        let evidence_id = content_id(candidate);
        return Err(IdentityError::EvidenceMismatch {
            memo_key: memo_key.clone(),
            memo_id: memo_id.clone(),
            evidence_id: evidence_id.clone(),
            alert: Alert::new(
                "x9.evidence_mismatch",
                format!(
                    "phase-D evidence {evidence_id} diverges from phase-B memo {memo_id} for {memo_key}"
                ),
            ),
        });
    }

    Ok(memo_bytes)
}

// ── Item ③ — intent_id identity: one lifecycle, one id ───────────────────────

/// Why an intent-id cross-phase reconciliation/registration fails the X9 ③ law.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentError {
    /// The phase-D native intent id diverges from the phase-B sidecar id for the
    /// SAME logical intent — a SECOND id was minted instead of carrying the one
    /// lifecycle id. Fail closed.
    Divergence {
        /// The id minted by the phase-B [`IntentSidecar`].
        sidecar_id: String,
        /// The native id carried by the phase-D [`VerdictObject`].
        native_id: String,
        /// The fail-closed alert.
        alert: Alert,
    },
    /// Two DISTINCT logical intents folded onto the SAME id — an identity
    /// collision. One id must name one lifecycle. Fail closed.
    Collision {
        /// The colliding id.
        intent_id: String,
        /// The logical-intent key already bound to that id.
        existing_logical: String,
        /// The new, distinct logical-intent key that collided onto it.
        incoming_logical: String,
        /// The fail-closed alert.
        alert: Alert,
    },
}

impl IntentError {
    /// The alert this fail-closed error carries.
    pub fn alert(&self) -> &Alert {
        match self {
            IntentError::Divergence { alert, .. } => alert,
            IntentError::Collision { alert, .. } => alert,
        }
    }
}

impl std::fmt::Display for IntentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntentError::Divergence {
                sidecar_id,
                native_id,
                ..
            } => write!(
                f,
                "x9: intent_id divergence: phase-B sidecar {sidecar_id} != phase-D native {native_id} (fail-closed)"
            ),
            IntentError::Collision {
                intent_id,
                existing_logical,
                incoming_logical,
                ..
            } => write!(
                f,
                "x9: intent_id collision on {intent_id}: '{existing_logical}' vs '{incoming_logical}' (fail-closed)"
            ),
        }
    }
}

impl std::error::Error for IntentError {}

/// Reconcile a phase-B sidecar id with a phase-D native intent id for the SAME
/// logical intent (item ③ — identity, NOT resolution).
///
/// `Ok(id)` iff the phase-B [`IntentSidecar::intent_id`] is IDENTICAL to the
/// phase-D [`VerdictObject::intent`]: one logical intent carries exactly one id
/// across its whole lifecycle. Any divergence (a second id minted in phase-D)
/// fails CLOSED with an alert — X14 covers whether a divergent id still
/// *resolves*; X9③ refuses it because it is not the SAME id.
pub fn reconcile_intent(
    sidecar: &IntentSidecar,
    native: &VerdictObject,
) -> Result<String, IntentError> {
    if sidecar.intent_id != native.intent {
        return Err(IntentError::Divergence {
            sidecar_id: sidecar.intent_id.clone(),
            native_id: native.intent.clone(),
            alert: Alert::new(
                "x9.intent_divergence",
                format!(
                    "phase-B sidecar minted {} but phase-D native intent is {}",
                    sidecar.intent_id, native.intent
                ),
            ),
        });
    }
    Ok(sidecar.intent_id.clone())
}

/// A registry that enforces **one id ⇒ one logical intent** across phases.
///
/// Each logical intent (identified by a caller-stable logical key, e.g. the PR's
/// charter content hash) binds to exactly one `intent_id`. Binding the SAME
/// logical key to the same id again is idempotent; binding a DISTINCT logical
/// key onto an already-bound id is a collision and fails CLOSED.
#[derive(Debug, Clone, Default)]
pub struct IntentRegistry {
    /// intent_id → logical-intent key.
    by_id: BTreeMap<String, String>,
}

impl IntentRegistry {
    /// A fresh, empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `logical` ⇒ `intent_id`, enforcing non-collision.
    ///
    /// Idempotent when re-binding the same `(logical, intent_id)` pair (one
    /// lifecycle observed twice). Fails CLOSED with [`IntentError::Collision`]
    /// when a DISTINCT logical intent tries to claim an already-bound id.
    pub fn bind(&mut self, logical: &str, intent_id: &str) -> Result<(), IntentError> {
        match self.by_id.get(intent_id) {
            Some(existing) if existing == logical => Ok(()),
            Some(existing) => Err(IntentError::Collision {
                intent_id: intent_id.to_string(),
                existing_logical: existing.clone(),
                incoming_logical: logical.to_string(),
                alert: Alert::new(
                    "x9.intent_collision",
                    format!(
                        "id {intent_id} already bound to '{existing}', cannot rebind to '{logical}'"
                    ),
                ),
            }),
            None => {
                self.by_id
                    .insert(intent_id.to_string(), logical.to_string());
                Ok(())
            }
        }
    }

    /// The logical intent currently bound to `intent_id`, if any.
    pub fn logical_for(&self, intent_id: &str) -> Option<&str> {
        self.by_id.get(intent_id).map(|s| s.as_str())
    }
}

/// Register a phase-B sidecar and its phase-D native intent as ONE lifecycle
/// under ONE id (item ③, end-to-end).
///
/// This composes [`reconcile_intent`] (the id is the SAME id across phases) with
/// [`IntentRegistry::bind`] (the id names exactly one logical intent). Returns
/// the single shared id on success; fails CLOSED on divergence OR collision.
pub fn register_one_lifecycle(
    registry: &mut IntentRegistry,
    logical: &str,
    sidecar: &IntentSidecar,
    native: &VerdictObject,
) -> Result<String, IntentError> {
    let id = reconcile_intent(sidecar, native)?;
    registry.bind(logical, &id)?;
    Ok(id)
}
