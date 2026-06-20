//! Webhook authentication and ingest (WP-B1 items ① and ⑤).
//!
//! # Signature verification (item ①)
//! Every inbound webhook must carry a valid `X-Hub-Signature-256` header.
//! Mismatches produce HTTP 401 **and** an `EventRecord` of kind
//! `webhook.rejected` (fail-closed audit, whitepaper §9).
//!
//! # Uninstall lifecycle (item ⑤)
//! The `installation.deleted` event revokes the stored installation token,
//! halts all queued processing for that installation, and appends an
//! `EventRecord` of kind `installation.revoked`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hmac::{Hmac, Mac};
use sha2::Sha256;

use hugit_contracts::{AckReceipt, EventRecord, SignedEventEnvelope};
use hugit_refstore::{canonical_json, compute_this_hash};

type HmacSha256 = Hmac<Sha256>;

/// Errors produced by webhook processing.
#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    /// Signature missing or malformed — caller must return HTTP 401.
    #[error("missing or malformed X-Hub-Signature-256 header")]
    MissingSignature,

    /// HMAC verification failed — caller must return HTTP 401.
    #[error("X-Hub-Signature-256 HMAC verification failed: signature mismatch")]
    SignatureMismatch,

    /// JSON payload could not be parsed.
    #[error("payload parse error: {0}")]
    PayloadParse(String),
}

/// Build an `EventRecord` for a rejected (forged/bad-signature) webhook.
///
/// The record has `kind = "webhook.rejected"` as mandated by the contract.
pub fn build_webhook_rejected_record(
    delivery_id: &str,
    prev_hash: &str,
    seq: u64,
    received_at: u64,
) -> EventRecord {
    // Hash = SHA-256 of (prev_hash ‖ kind ‖ principal_chain ‖ payload ‖ seq)
    // as specified in EventRecord doc comment (FROZEN formula).
    let kind = "webhook.rejected";
    let principal_chain: Vec<String> = vec!["github".to_string()];
    let raw_payload = format!(r#"{{"delivery_id":"{delivery_id}"}}"#);
    // The chain hashes canonical-JSON bytes (sorted keys, no insignificant
    // whitespace); canonicalise before chaining so producer ≡ verifier.
    let payload = canonical_json(&raw_payload).unwrap_or(raw_payload);

    let this_hash = compute_this_hash(prev_hash, kind, &principal_chain, &payload, seq);

    EventRecord {
        seq,
        prev_hash: prev_hash.to_string(),
        this_hash,
        kind: kind.to_string(),
        principal_chain,
        payload,
        recorded_at: received_at,
    }
}

/// Build an `EventRecord` for a revoked installation (uninstall, item ⑤).
///
/// The record has `kind = "installation.revoked"` as mandated by the contract.
pub fn build_installation_revoked_record(
    installation_id: &str,
    prev_hash: &str,
    seq: u64,
    recorded_at: u64,
) -> EventRecord {
    let kind = "installation.revoked";
    let principal_chain: Vec<String> = vec!["github".to_string()];
    let raw_payload = format!(r#"{{"installation_id":"{installation_id}"}}"#);
    let payload = canonical_json(&raw_payload).unwrap_or(raw_payload);

    let this_hash = compute_this_hash(prev_hash, kind, &principal_chain, &payload, seq);

    EventRecord {
        seq,
        prev_hash: prev_hash.to_string(),
        this_hash,
        kind: kind.to_string(),
        principal_chain,
        payload,
        recorded_at,
    }
}

/// Verify a `X-Hub-Signature-256: sha256=<hex>` header value against the raw
/// payload bytes using the App webhook secret.
///
/// Returns `Ok(())` on success, or a `WebhookError` on failure.
/// Uses constant-time HMAC comparison to prevent timing attacks.
pub fn verify_x_hub_signature_256(
    secret: &[u8],
    payload: &[u8],
    signature_header: Option<&str>,
) -> Result<(), WebhookError> {
    // An empty secret is a misconfiguration: any payload could be verified
    // with a trivially computed HMAC, making the gate forgeable. Fail closed.
    if secret.is_empty() {
        return Err(WebhookError::MissingSignature);
    }

    let header = signature_header.ok_or(WebhookError::MissingSignature)?;

    // Header format: "sha256=<hex>"
    let hex_sig = header
        .strip_prefix("sha256=")
        .ok_or(WebhookError::MissingSignature)?;

    let sig_bytes = hex::decode(hex_sig).map_err(|_| WebhookError::SignatureMismatch)?;

    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length; infallible");
    mac.update(payload);
    mac.verify_slice(&sig_bytes)
        .map_err(|_| WebhookError::SignatureMismatch)
}

/// Ingest an inbound webhook envelope.
///
/// On a bad signature, returns `Err(WebhookError::SignatureMismatch)`.
/// The caller is responsible for producing the 401 HTTP response and writing
/// a `webhook.rejected` EventRecord.
///
/// On success, returns the `SignedEventEnvelope` ready for persistence.
pub fn ingest_webhook(
    secret: &[u8],
    raw_payload: &[u8],
    signature_header: Option<&str>,
    delivery_id: &str,
    event_type: &str,
    received_at: u64,
) -> Result<SignedEventEnvelope, WebhookError> {
    // Capture the header value before consuming it into verify — the header
    // IS the canonical signature GitHub sent; we store it verbatim for
    // auditability rather than re-computing our own copy.
    let verified_header = signature_header.ok_or(WebhookError::MissingSignature)?;
    verify_x_hub_signature_256(secret, raw_payload, Some(verified_header))?;

    let payload_str = std::str::from_utf8(raw_payload)
        .map(str::to_string)
        .map_err(|e| WebhookError::PayloadParse(e.to_string()))?;

    Ok(SignedEventEnvelope {
        delivery_id: delivery_id.to_string(),
        event_type: event_type.to_string(),
        signature: verified_header.to_string(),
        payload: payload_str,
        received_at,
    })
}

/// Webhook processor — orchestrates ingest, signature check, ack, and
/// lifecycle dispatch.
///
/// Holds a real token store: `Arc<Mutex<HashMap<installation_id, Option<String>>>>`.
/// `None` in the map means the slot exists but has been revoked/zeroed.
/// Absent from the map means no token was ever registered.
pub struct WebhookProcessor {
    /// Webhook secret (HMAC-SHA256 key).
    secret: Vec<u8>,
    /// In-memory installation token store.
    /// Key: installation_id string.
    /// Value: `Some(token)` while active, `None` after revocation.
    token_store: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl WebhookProcessor {
    /// Create a new processor with the given webhook secret.
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        Self {
            secret: secret.into(),
            token_store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register (or update) the installation token for an installation.
    pub fn store_token(&self, installation_id: &str, token: &str) {
        // Recover a poisoned lock via into_inner() — the token cache is
        // best-effort state; a prior panic inside the critical section is
        // not a reason to crash an incoming webhook request.
        let mut store = self
            .token_store
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.insert(installation_id.to_string(), Some(token.to_string()));
    }

    /// Retrieve the installation token, if present and not revoked.
    pub fn get_token(&self, installation_id: &str) -> Option<String> {
        // Same poison-recovery policy as store_token — cache is best-effort.
        let store = self
            .token_store
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.get(installation_id).and_then(Clone::clone)
    }

    /// Process a raw inbound webhook request.
    ///
    /// Returns:
    /// - `Ok(envelope)` when the signature is valid; the envelope is
    ///   ready for persistence.
    /// - `Err(WebhookError::SignatureMismatch | MissingSignature)` when the
    ///   signature is absent or wrong; the caller MUST return HTTP 401 and
    ///   write a `webhook.rejected` EventRecord.
    pub fn process(
        &self,
        raw_payload: &[u8],
        signature_header: Option<&str>,
        delivery_id: &str,
        event_type: &str,
        received_at: u64,
    ) -> Result<SignedEventEnvelope, WebhookError> {
        ingest_webhook(
            &self.secret,
            raw_payload,
            signature_header,
            delivery_id,
            event_type,
            received_at,
        )
    }

    /// Build a `webhook.rejected` audit record for a bad-signature attempt.
    pub fn rejected_record(
        &self,
        delivery_id: &str,
        prev_hash: &str,
        seq: u64,
        received_at: u64,
    ) -> EventRecord {
        build_webhook_rejected_record(delivery_id, prev_hash, seq, received_at)
    }

    /// Build an `installation.revoked` audit record for an uninstall event.
    pub fn revoked_record(
        &self,
        installation_id: &str,
        prev_hash: &str,
        seq: u64,
        recorded_at: u64,
    ) -> EventRecord {
        build_installation_revoked_record(installation_id, prev_hash, seq, recorded_at)
    }

    /// Handle an `installation.deleted` lifecycle event (item ⑤).
    ///
    /// - Revokes the stored installation token: removes the entry from the
    ///   in-memory store and zeros the slot (sets to `None`). Returns
    ///   `token_revoked = true` iff a `Some(token)` was present before
    ///   revocation; `false` if the slot was already absent or already `None`.
    /// - Sets `processing_halted = true` unconditionally (the caller is
    ///   responsible for calling `PersistenceAdapter::halt_installation`).
    /// - Builds and returns an `installation.revoked` EventRecord.
    pub fn handle_uninstall(
        &self,
        installation_id: &str,
        prev_hash: &str,
        seq: u64,
        recorded_at: u64,
    ) -> (RevokeOutcome, EventRecord) {
        let record =
            build_installation_revoked_record(installation_id, prev_hash, seq, recorded_at);

        // Remove/zero the token slot; token_revoked = true only if there was
        // an active (Some) token.
        let token_revoked = {
            let mut store = self
                .token_store
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match store.remove(installation_id) {
                Some(Some(_)) => {
                    // Zero the slot after removal (tombstone).
                    store.insert(installation_id.to_string(), None);
                    true
                }
                Some(None) => {
                    // Already revoked — re-insert tombstone, return false.
                    store.insert(installation_id.to_string(), None);
                    false
                }
                None => false,
            }
        };

        let outcome = RevokeOutcome {
            installation_id: installation_id.to_string(),
            token_revoked,
            processing_halted: true,
        };
        (outcome, record)
    }
}

/// Outcome of an uninstall/revoke operation.
#[derive(Debug, Clone, PartialEq)]
pub struct RevokeOutcome {
    /// The installation that was revoked.
    pub installation_id: String,
    /// Whether the stored token was successfully revoked.
    pub token_revoked: bool,
    /// Whether queued processing was halted.
    pub processing_halted: bool,
}

/// Build an `AckReceipt` for a successfully ingested webhook.
pub fn build_ack_receipt(delivery_id: &str, acked_at: u64) -> AckReceipt {
    let processing_id = format!("proc-{delivery_id}");
    AckReceipt {
        delivery_id: delivery_id.to_string(),
        processing_id,
        acked_at,
    }
}
