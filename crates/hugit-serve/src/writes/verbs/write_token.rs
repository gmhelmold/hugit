//! Personal Access Tokens (PATs) — the durable per-account store + create/revoke,
//! PLUS the git-auth resolution primitives (slice 2b): the [`PatAuth`] resolver
//! ([`index_account_log`]/[`resolve_pat`]) and the wire credential-extraction
//! ([`candidate_pat_secrets`] — Bearer AND the git-CLI HTTP-Basic password). The
//! hot-path WIRING that consults these ([`AppState::resolve_pat_from_auth`] +
//! Tier-1.5 in `two_tier_auth`/`clone_principal`/receive-pack) is gated behind
//! `HUGIT_SERVE_PAT_AUTH` (default OFF) and an adversarial review before live enable
//! — see `docs/design/2026-07-05-pat-git-auth-wire.md`.
//!
//! ## Storage (reuses the GDPR account seam)
//!
//! PATs live on the per-account event log (`_accounts/{org}`, the same durable store
//! the erasure lifecycle uses) as append-only `pat.created` / `pat.revoked` records.
//! The token SECRET NEVER touches the log — only its SHA-256 hash (ADR-0002: a PAT
//! never reaches the browser, and the engine stores no recoverable secret). The raw
//! secret is returned EXACTLY ONCE, on create.
//!
//! ## Identity
//!
//! A PAT belongs to a specific USER within an org (`clerk:{org}:{user}`). The account
//! log is keyed by the ORG; each `pat.created` records the full `user` principal, so
//! the projection ([`project_pats`]) returns only the caller's OWN tokens.

use hugit_http_contracts::account::{CreateTokenReq, CreatedTokenVm, PatMetaVm};
use hugit_refstore::{Endpoint, EventLog};
use sha2::{Digest, Sha256};

use crate::error::EngineErr;
use crate::state::AppState;
use crate::writes::{AccountLogSink, MAX_CAS_ATTEMPTS, asserted_class};

/// A `pat.created` record — the token exists (metadata + the SECRET HASH, never the
/// secret). The matching `pat.revoked` (by id) tombstones it.
pub const PAT_CREATED_KIND: &str = "pat.created";
/// A `pat.revoked` record — tombstones a `pat.created` by id (idempotent).
pub const PAT_REVOKED_KIND: &str = "pat.revoked";

/// The secret prefix identifying a hugit PAT on the wire (the fast-reject discriminant
/// the future git-auth path keys on).
pub const PAT_SECRET_PREFIX: &str = "ghgr_pat_";

/// The valid PAT scopes (v0 — the minimal git set). A create request's scopes MUST be
/// a subset; an unknown scope is rejected (fail-closed).
pub const VALID_SCOPES: &[&str] = &["repo:read", "repo:write"];

/// Max PATs one account may hold (a DoS / log-bloat bound). Over-cap create → 429.
pub const MAX_PATS_PER_ACCOUNT: usize = 50;

/// Hex SHA-256 of the raw secret — the ONLY form stored.
fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// Mint a fresh PAT secret: `ghgr_pat_<64 hex>` from the OS CSPRNG (`/dev/urandom`;
/// no `rand` dep, mirrors the session-token minter). Returned to the caller ONCE.
fn mint_secret() -> Result<String, EngineErr> {
    use std::fs::File;
    use std::io::Read;
    let mut buf = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| EngineErr::unavailable(format!("entropy source unavailable: {e}")))?;
    Ok(format!("{PAT_SECRET_PREFIX}{}", hex::encode(buf)))
}

/// A short opaque token id derived from the secret hash (the first 16 hex chars) — a
/// stable handle for revoke/list that is NOT the secret and NOT reversible to it.
fn token_id(secret_hash: &str) -> String {
    format!("pat_{}", &secret_hash[..16.min(secret_hash.len())])
}

/// Validate + normalize the requested scopes. Empty → the default `["repo:read"]`.
/// An unknown scope → `400` (fail-closed).
fn resolve_scopes(req: &CreateTokenReq) -> Result<Vec<String>, EngineErr> {
    if req.scopes.is_empty() {
        return Ok(vec!["repo:read".to_string()]);
    }
    for s in &req.scopes {
        if !VALID_SCOPES.contains(&s.as_str()) {
            return Err(EngineErr::invalid_request(format!(
                "escopo inválido: {s} (válidos: {})",
                VALID_SCOPES.join(", ")
            )));
        }
    }
    Ok(req.scopes.clone())
}

/// Project the caller's OWN live (non-revoked) PATs from an account log → metadata
/// (never the secret hash / another user's tokens). Latest-wins is not needed: a
/// `pat.revoked` for an id removes it. `user` = the caller's full `clerk:{org}:{user}`.
#[must_use]
pub fn project_pats(log: &EventLog, user: &str) -> Vec<PatMetaVm> {
    // Collect revoked ids first (a tombstone anywhere kills the token).
    let mut revoked = std::collections::BTreeSet::new();
    for r in log.records().iter().filter(|r| r.kind == PAT_REVOKED_KIND) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload)
            && let Some(id) = v.get("id").and_then(|x| x.as_str())
        {
            revoked.insert(id.to_string());
        }
    }
    let mut out = Vec::new();
    for r in log.records().iter().filter(|r| r.kind == PAT_CREATED_KIND) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload) else {
            continue;
        };
        // Only the caller's OWN tokens (cross-user isolation within the org).
        if v.get("user").and_then(|x| x.as_str()) != Some(user) {
            continue;
        }
        let Some(id) = v.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        if revoked.contains(id) {
            continue;
        }
        let scopes = v
            .get("scopes")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        out.push(PatMetaVm {
            id: id.to_string(),
            name: v
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            created_at: v.get("created_at").and_then(|x| x.as_u64()).unwrap_or(0),
            // last_used_at tracking is slice 2b (the auth wire); 0 = never (so far).
            last_used_at: 0,
            scopes,
        });
    }
    out
}

/// Count a caller's live PATs (for the per-account cap).
fn live_pat_count(log: &EventLog, user: &str) -> usize {
    project_pats(log, user).len()
}

// ── the AUTH RESOLUTION foundation (slice 2b — pure; NOT yet wired to the live
//    auth hot path) ─────────────────────────────────────────────────────────────
//
// A PAT authenticates git/API by resolving its SECRET to the owning principal +
// scopes. The engine stores only `sha256(secret)`, so resolution is: hash the
// presented secret → look it up among the live (non-revoked, non-expired) tokens →
// the owning `clerk:{org}:{user}` + scopes. This module provides the PURE resolver +
// an in-memory index builder; wiring it into `two_tier_auth`/`clone_principal` (the
// hot path) + the Basic-auth (git-CLI) decode is the reviewed follow-on (see the
// design `docs/design/2026-07-05-pat-git-auth-wire.md`).

/// What a resolved PAT authorizes — the owning principal + its scopes. NEVER the
/// operator: `principal` is the token owner's FULL `clerk:{org}:{user}` chain tail (as
/// stored on `pat.created`), so a resolved PAT is structurally a clerk principal and
/// can NEVER be the god-path (the resolver is inserted BEFORE the dev-token tier).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatAuth {
    /// The owning principal — the full `clerk:{org}:{user}` string.
    pub principal: String,
    /// The granted scopes (`repo:read`/`repo:write`).
    pub scopes: Vec<String>,
    /// Unix ms expiry; `0` = never.
    pub expires_at: u64,
}

impl PatAuth {
    /// The full engine principal chain a resolved PAT authenticates as.
    #[must_use]
    pub fn principal_chain(&self) -> Vec<String> {
        vec![self.principal.clone()]
    }

    /// Whether the PAT is expired at `now_ms` (never for `expires_at == 0`).
    #[must_use]
    pub fn is_expired(&self, now_ms: u64) -> bool {
        self.expires_at != 0 && now_ms >= self.expires_at
    }

    /// Whether the PAT grants write access (`repo:write`). A read-only PAT
    /// (`repo:read` only) MUST NOT authenticate a mutation — the write paths gate on
    /// this once wired.
    #[must_use]
    pub fn can_write(&self) -> bool {
        self.scopes.iter().any(|s| s == "repo:write")
    }
}

/// Build the `secret_hash → PatAuth` index for ONE account log. Skips revoked tokens.
/// This is composed across all `_accounts/*` logs at boot + refreshed on create/revoke
/// to form the engine-wide index. The owning principal is taken verbatim from each
/// record's `user` field (the full `clerk:{org}:{user}`).
#[must_use]
pub fn index_account_log(log: &EventLog) -> std::collections::HashMap<String, PatAuth> {
    let mut revoked = std::collections::BTreeSet::new();
    for r in log.records().iter().filter(|r| r.kind == PAT_REVOKED_KIND) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload)
            && let Some(id) = v.get("id").and_then(|x| x.as_str())
        {
            revoked.insert(id.to_string());
        }
    }
    let mut out = std::collections::HashMap::new();
    for r in log.records().iter().filter(|r| r.kind == PAT_CREATED_KIND) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload) else {
            continue;
        };
        let (Some(id), Some(hash), Some(user)) = (
            v.get("id").and_then(|x| x.as_str()),
            v.get("secret_hash").and_then(|x| x.as_str()),
            v.get("user").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        if revoked.contains(id) {
            continue;
        }
        let scopes = v
            .get("scopes")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        out.insert(
            hash.to_string(),
            PatAuth {
                principal: user.to_string(),
                scopes,
                expires_at: v.get("expires_at").and_then(|x| x.as_u64()).unwrap_or(0),
            },
        );
    }
    out
}

/// Resolve a presented raw secret against a `secret_hash → PatAuth` index at `now_ms`.
/// `None` for: not a hugit PAT (wrong prefix), unknown/revoked (absent from the
/// index), or expired. `Some` ONLY for a live, valid PAT → its owning principal +
/// scopes. The lookup keys on `sha256(secret)`, so knowing a hash is useless without
/// the secret (a stored-hash leak cannot forge a token).
#[must_use]
pub fn resolve_pat(
    index: &std::collections::HashMap<String, PatAuth>,
    raw_secret: &str,
    now_ms: u64,
) -> Option<PatAuth> {
    // Fast-reject anything that is not shaped like a hugit PAT (avoids hashing every
    // random Bearer). A real session token / dev-token never has this prefix.
    if !raw_secret.starts_with(PAT_SECRET_PREFIX) {
        return None;
    }
    let auth = index.get(&hash_secret(raw_secret))?;
    if auth.is_expired(now_ms) {
        return None; // expired → treated as no credential (the caller 401s / degrades)
    }
    Some(auth.clone())
}

/// Decode a standard base64 string (RFC 4648 standard alphabet, `=` padding tolerated).
/// UNTRUSTED-input safe: any invalid character or an impossible length returns `None`,
/// NEVER a panic (this parses the git-CLI `Authorization: Basic` header from the wire).
pub(crate) fn decode_base64_std(s: &str) -> Option<Vec<u8>> {
    fn sextet(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    // Strip trailing padding; the remaining length mod 4 == 1 is impossible for valid
    // base64 (a single leftover sextet can't encode a byte) → reject.
    let body = s.trim_end_matches('=');
    if body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in body.as_bytes() {
        acc = (acc << 6) | sextet(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Extract the candidate PAT secret(s) an `Authorization` header VALUE could carry:
/// - `Bearer <secret>` — the `/v1` API + `http.extraHeader` git.
/// - `Basic base64(user:secret)` — the DEFAULT git CLI, which puts the PAT in the
///   PASSWORD field (the username is ignored, GitHub-style).
///
/// The scheme match is case-insensitive (RFC 7235). Every malformed shape (no scheme,
/// bad base64, missing `:`, non-UTF-8) yields an EMPTY vec — never a panic, never a
/// spurious credential. Returned secrets are pre-filter candidates; [`resolve_pat`]
/// applies the `ghgr_pat_` fast-reject, so feeding a session/dev Bearer here is inert.
#[must_use]
pub fn candidate_pat_secrets(auth_value: &str) -> Vec<String> {
    let Some((scheme, rest)) = auth_value.split_once(' ') else {
        return Vec::new();
    };
    let rest = rest.trim();
    if scheme.eq_ignore_ascii_case("Bearer") {
        return vec![rest.to_string()];
    }
    if scheme.eq_ignore_ascii_case("Basic")
        && let Some(bytes) = decode_base64_std(rest)
        && let Ok(text) = std::str::from_utf8(&bytes)
        && let Some((_user, secret)) = text.split_once(':')
    {
        return vec![secret.to_string()];
    }
    Vec::new()
}

/// Append one record to the account log via a bounded compare-and-swap (mirrors the
/// account-door CAS loop). Fail-closed on a durable fault.
fn append_account(
    sink: &dyn AccountLogSink,
    account: &str,
    kind: &str,
    principal_chain: &[String],
    payload: String,
    at: u64,
    // A pre-persist guard evaluated against the freshly-loaded log (e.g. the per-account
    // cap, or the "token exists / not already revoked" check) — re-checked on each CAS
    // attempt so it stays correct under contention. `Ok(())` proceeds; `Err` aborts.
    guard: impl Fn(&EventLog) -> Result<(), EngineErr>,
) -> Result<(), EngineErr> {
    let class = asserted_class(principal_chain)?;
    for _attempt in 0..MAX_CAS_ATTEMPTS {
        let (mut log, token) = sink.load_account(account)?;
        guard(&log)?;
        log.append_authorized(
            class,
            Endpoint::Land,
            kind,
            principal_chain.to_vec(),
            payload.clone(),
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!("{kind} append denied: {}", d.reason.code()))
        })?;
        match sink.persist_account(account, &log, &token) {
            Ok(()) => return Ok(()),
            Err(e) if e.is_cas_conflict() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(EngineErr::unavailable(
        "registro de token sob contenção — tente novamente",
    ))
}

/// The account (org) + full user principal for the caller, or a fail-closed error.
/// A PAT belongs to a real tenant USER; operator/anon are refused (no god/anon token).
fn caller_identity(principal_chain: &[String]) -> Result<(String, String), EngineErr> {
    let user = principal_chain.last().cloned().unwrap_or_default();
    let account = crate::writes::verbs::write_provision::derive_owner_tenant(principal_chain)?;
    Ok((account, user))
}

/// `POST /v1/me/tokens` — mint a new PAT for the caller. Returns the raw secret EXACTLY
/// ONCE (never stored, never shown again). NOT idempotent (each call mints a distinct
/// token — mirrors GitHub; a lost-response retry yields a second token the user can
/// revoke). The account log stores only the SHA-256 hash.
///
/// # Errors
/// - `401` — operator/anon (no own account).
/// - `400` — bad body / invalid scope / empty name.
/// - `429` — the account is at [`MAX_PATS_PER_ACCOUNT`].
/// - `503` — durable append fault (fail-closed).
pub fn token_create(
    state: &AppState,
    body: &[u8],
    principal_chain: &[String],
    at: u64,
) -> Result<CreatedTokenVm, EngineErr> {
    let (account, user) = caller_identity(principal_chain)?;
    let req: CreateTokenReq = serde_json::from_slice(body)
        .map_err(|e| EngineErr::invalid_request(format!("corpo inválido: {e}")))?;
    let name = req.name.trim();
    if name.is_empty() || name.len() > 128 {
        return Err(EngineErr::invalid_request(
            "nome do token: 1..=128 caracteres",
        ));
    }
    let scopes = resolve_scopes(&req)?;
    let expires_at = if req.ttl_secs == 0 {
        0
    } else {
        at.saturating_add(req.ttl_secs.saturating_mul(1000))
    };

    let secret = mint_secret()?;
    let secret_hash = hash_secret(&secret);
    let id = token_id(&secret_hash);
    let payload_value = serde_json::json!({
        "created_at": at,
        "expires_at": expires_at,
        "id": id,
        "name": name,
        "scopes": scopes,
        "secret_hash": secret_hash,
        "user": user,
    });
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());

    let sink: &dyn AccountLogSink = state;
    let user_for_guard = user.clone();
    append_account(
        sink,
        &account,
        PAT_CREATED_KIND,
        principal_chain,
        payload,
        at,
        move |log| {
            if live_pat_count(log, &user_for_guard) >= MAX_PATS_PER_ACCOUNT {
                return Err(EngineErr {
                    status: 429,
                    code: "TOO_MANY_TOKENS",
                    reason: format!("limite de {MAX_PATS_PER_ACCOUNT} tokens por conta atingido"),
                });
            }
            Ok(())
        },
    )?;

    // Keep the live in-memory PAT auth index warm (the hot-path resolver) — insert the
    // freshly-minted token so it authenticates immediately, no reboot (mirrors the ref
    // hot-swap). A no-op when `HUGIT_SERVE_PAT_AUTH` is off (the index is never
    // consulted then). Done AFTER the durable append succeeds → the index never leads
    // the log.
    state.pat_index_insert_if_enabled(
        secret_hash.clone(),
        PatAuth {
            principal: user.clone(),
            scopes: scopes.clone(),
            expires_at,
        },
    );

    Ok(CreatedTokenVm {
        id,
        name: name.to_string(),
        secret, // returned ONCE — never stored, never shown again
        scopes,
        created_at: at,
        expires_at,
    })
}

/// `DELETE /v1/me/tokens/{id}` — revoke the caller's PAT `id`. IDEMPOTENT (already
/// revoked / absent → a no-op success). A caller can revoke ONLY their OWN token
/// (cross-user isolation): revoking an id the caller does not own is a `404`.
///
/// # Errors
/// - `401` — operator/anon.
/// - `404` — the id is not one of the caller's live tokens.
/// - `503` — durable append fault.
pub fn token_revoke(
    state: &AppState,
    id: &str,
    principal_chain: &[String],
    at: u64,
) -> Result<(), EngineErr> {
    let (account, user) = caller_identity(principal_chain)?;
    let sink: &dyn AccountLogSink = state;

    // Ownership + existence pre-check (also the CAS guard): the id must be one of the
    // CALLER'S live tokens. Absent/foreign → 404 (no cross-user oracle); already
    // revoked → idempotent no-op (handled by the guard returning a sentinel).
    let id_owned = |log: &EventLog| project_pats(log, &user).iter().any(|p| p.id == id);

    // First load: if the token is not the caller's live token, decide 404 vs no-op.
    let (log0, _) = sink.load_account(&account)?;
    if !id_owned(&log0) {
        // Distinguish "already revoked by the caller" (idempotent OK) from "never the
        // caller's" (404). A prior pat.created by this user with this id ⇒ it was
        // theirs and is now revoked ⇒ no-op success.
        let was_ever_mine = log0.records().iter().any(|r| {
            r.kind == PAT_CREATED_KIND
                && serde_json::from_str::<serde_json::Value>(&r.payload)
                    .ok()
                    .map(|v| {
                        v.get("id").and_then(|x| x.as_str()) == Some(id)
                            && v.get("user").and_then(|x| x.as_str()) == Some(user.as_str())
                    })
                    .unwrap_or(false)
        });
        return if was_ever_mine {
            Ok(()) // already revoked — idempotent
        } else {
            Err(EngineErr::not_found()) // never the caller's — no oracle
        };
    }

    // The secret_hash for the revoked id (from its `pat.created` record) — needed to
    // DROP the entry from the in-memory PAT index so the token stops authenticating
    // IMMEDIATELY (no reboot). Captured from the pre-check load before the append.
    let revoked_hash = log0.records().iter().find_map(|r| {
        if r.kind != PAT_CREATED_KIND {
            return None;
        }
        let v = serde_json::from_str::<serde_json::Value>(&r.payload).ok()?;
        if v.get("id").and_then(|x| x.as_str()) == Some(id) {
            v.get("secret_hash")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        } else {
            None
        }
    });

    let payload_value = serde_json::json!({ "id": id, "user": user });
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    append_account(
        sink,
        &account,
        PAT_REVOKED_KIND,
        principal_chain,
        payload,
        at,
        // Under contention a concurrent revoke may have landed first — treat a
        // now-absent live token as an idempotent no-op (re-check owns the race).
        move |log| {
            let _ = log;
            Ok(())
        },
    )?;

    // Durable revoke landed → drop the live index entry (immediate deny; a no-op when
    // PAT auth is off). Ordered AFTER the append so the index never lags toward MORE
    // access than the log grants.
    if let Some(h) = revoked_hash {
        state.pat_index_remove_if_enabled(&h);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_dir() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("hugit-pat-{}-{nanos}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn state() -> AppState {
        AppState::new(scratch_dir(), "dev-token".to_string())
    }
    fn user(org: &str, u: &str) -> Vec<String> {
        vec![format!("clerk:{org}:{u}")]
    }
    fn body(name: &str, scopes: &[&str]) -> Vec<u8> {
        serde_json::json!({"name": name, "scopes": scopes})
            .to_string()
            .into_bytes()
    }
    fn pats_of(st: &AppState, org: &str, u: &str) -> Vec<PatMetaVm> {
        let (log, _) = st.load_account_log(org).unwrap();
        project_pats(&log, &format!("clerk:{org}:{u}"))
    }

    #[test]
    fn create_returns_secret_once_and_stores_only_the_hash() {
        let st = state();
        let created =
            token_create(&st, &body("ci", &["repo:write"]), &user("org-a", "u1"), 10).unwrap();
        assert!(
            created.secret.starts_with(PAT_SECRET_PREFIX),
            "secret is prefixed"
        );
        assert_eq!(created.scopes, vec!["repo:write"]);
        // The account log stores the HASH, never the secret.
        let (log, _) = st.load_account_log("org-a").unwrap();
        let rec = log
            .records()
            .iter()
            .find(|r| r.kind == PAT_CREATED_KIND)
            .unwrap();
        assert!(
            !rec.payload.contains(&created.secret),
            "the raw secret is NEVER stored"
        );
        assert!(
            rec.payload.contains(&hash_secret(&created.secret)),
            "the hash IS stored"
        );
        // It projects into the caller's list (metadata only, no secret/hash).
        let mine = pats_of(&st, "org-a", "u1");
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].id, created.id);
        assert_eq!(mine[0].last_used_at, 0);
    }

    #[test]
    fn tokens_are_isolated_per_user_within_an_org() {
        let st = state();
        token_create(&st, &body("u1-tok", &[]), &user("org-a", "u1"), 1).unwrap();
        token_create(&st, &body("u2-tok", &[]), &user("org-a", "u2"), 2).unwrap();
        assert_eq!(pats_of(&st, "org-a", "u1").len(), 1, "u1 sees only its own");
        assert_eq!(pats_of(&st, "org-a", "u2").len(), 1, "u2 sees only its own");
        assert_eq!(pats_of(&st, "org-a", "u1")[0].name, "u1-tok");
    }

    #[test]
    fn revoke_removes_the_token_and_is_idempotent() {
        let st = state();
        let c = token_create(&st, &body("t", &[]), &user("org-a", "u1"), 1).unwrap();
        token_revoke(&st, &c.id, &user("org-a", "u1"), 2).unwrap();
        assert!(
            pats_of(&st, "org-a", "u1").is_empty(),
            "revoked token is gone from the list"
        );
        // A second revoke of the same id is an idempotent no-op (not a 404).
        token_revoke(&st, &c.id, &user("org-a", "u1"), 3).expect("re-revoke is idempotent");
    }

    #[test]
    fn revoking_a_foreign_or_unknown_token_is_404_no_oracle() {
        let st = state();
        let c = token_create(&st, &body("mine", &[]), &user("org-a", "u1"), 1).unwrap();
        // Another user cannot revoke u1's token.
        assert_eq!(
            token_revoke(&st, &c.id, &user("org-a", "u2"), 2)
                .unwrap_err()
                .status,
            404
        );
        // An unknown id is 404.
        assert_eq!(
            token_revoke(&st, "pat_deadbeef00000000", &user("org-a", "u1"), 3)
                .unwrap_err()
                .status,
            404
        );
        // u1's token still lives (the foreign revoke did nothing).
        assert_eq!(pats_of(&st, "org-a", "u1").len(), 1);
    }

    #[test]
    fn operator_and_anon_cannot_mint_tokens() {
        let st = state();
        assert_eq!(
            token_create(&st, &body("x", &[]), &["orchestrator:hugit".into()], 1)
                .unwrap_err()
                .status,
            401
        );
        assert_eq!(
            token_create(&st, &body("x", &[]), &[], 1)
                .unwrap_err()
                .status,
            401
        );
    }

    #[test]
    fn invalid_scope_and_empty_name_are_400() {
        let st = state();
        assert_eq!(
            token_create(&st, &body("t", &["repo:admin"]), &user("org-a", "u1"), 1)
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            token_create(&st, &body("  ", &[]), &user("org-a", "u1"), 1)
                .unwrap_err()
                .status,
            400
        );
    }

    // ── the AUTH RESOLUTION foundation (slice 2b) — adversarial ───────────────

    /// Create a PAT and return (secret, the org's index).
    fn make_pat(
        st: &AppState,
        org: &str,
        u: &str,
        scopes: &[&str],
        ttl_secs: u64,
        at: u64,
    ) -> (String, std::collections::HashMap<String, PatAuth>) {
        let b = serde_json::json!({"name":"t","scopes":scopes,"ttl_secs":ttl_secs})
            .to_string()
            .into_bytes();
        let created = token_create(st, &b, &user(org, u), at).unwrap();
        let (log, _) = st.load_account_log(org).unwrap();
        (created.secret, index_account_log(&log))
    }

    #[test]
    fn resolve_maps_a_live_pat_to_its_owning_principal_never_operator() {
        let st = state();
        let (secret, idx) = make_pat(&st, "org-a", "u1", &["repo:write"], 0, 10);
        let auth = resolve_pat(&idx, &secret, 20).expect("a live PAT resolves");
        assert_eq!(auth.principal_chain(), vec!["clerk:org-a:u1".to_string()]);
        assert!(auth.can_write(), "repo:write PAT can write");
        // A PAT is STRUCTURALLY a clerk principal — never the operator.
        assert!(auth.principal_chain()[0].starts_with("clerk:"));
        assert!(!auth.principal_chain()[0].starts_with("orchestrator:"));
    }

    #[test]
    fn resolve_rejects_wrong_prefix_unknown_and_revoked() {
        let st = state();
        let (secret, idx) = make_pat(&st, "org-a", "u1", &[], 0, 10);
        // A non-PAT bearer (session token / garbage) is fast-rejected (never hashed).
        assert!(resolve_pat(&idx, "sess_whatever", 20).is_none());
        // A well-formed-but-unknown PAT secret → None.
        assert!(resolve_pat(&idx, &format!("{PAT_SECRET_PREFIX}deadbeef"), 20).is_none());
        // After revoke, the token is ABSENT from a freshly-built index → None.
        let id = format!("pat_{}", &hash_secret(&secret)[..16]);
        token_revoke(&st, &id, &user("org-a", "u1"), 30).unwrap();
        let (log2, _) = st.load_account_log("org-a").unwrap();
        let idx2 = index_account_log(&log2);
        assert!(
            resolve_pat(&idx2, &secret, 40).is_none(),
            "a revoked PAT no longer resolves"
        );
    }

    #[test]
    fn resolve_rejects_an_expired_pat() {
        let st = state();
        // ttl 5s → expires_at = created_at(1000) + 5000 = 6000ms.
        let (secret, idx) = make_pat(&st, "org-a", "u1", &[], 5, 1000);
        assert!(
            resolve_pat(&idx, &secret, 5999).is_some(),
            "valid before expiry"
        );
        assert!(
            resolve_pat(&idx, &secret, 6000).is_none(),
            "expired exactly at expires_at → no credential"
        );
        assert!(
            resolve_pat(&idx, &secret, 9999).is_none(),
            "still expired later"
        );
    }

    #[test]
    fn a_read_only_pat_cannot_write() {
        let st = state();
        let (secret, idx) = make_pat(&st, "org-a", "u1", &["repo:read"], 0, 10);
        let auth = resolve_pat(&idx, &secret, 20).unwrap();
        assert!(
            !auth.can_write(),
            "a repo:read PAT must NOT authorize a write"
        );
        // The default scope (empty request) is repo:read → also read-only.
        let (s2, i2) = make_pat(&st, "org-a", "u2", &[], 0, 11);
        assert!(!resolve_pat(&i2, &s2, 20).unwrap().can_write());
    }

    #[test]
    fn the_index_isolates_users_and_carries_no_secret() {
        let st = state();
        let (sa, _) = make_pat(&st, "org-a", "u1", &["repo:write"], 0, 10);
        make_pat(&st, "org-a", "u2", &["repo:read"], 0, 11);
        let (log, _) = st.load_account_log("org-a").unwrap();
        let idx = index_account_log(&log);
        // Two tokens indexed; each resolves to its OWN user.
        assert_eq!(idx.len(), 2);
        assert_eq!(
            resolve_pat(&idx, &sa, 20).unwrap().principal,
            "clerk:org-a:u1"
        );
        // The index keys on the HASH — the raw secret appears nowhere.
        assert!(
            !idx.keys().any(|k| k.contains(&sa)),
            "the index stores hashes, not secrets"
        );
    }

    // ── slice-2b wiring: base64 + credential extraction + AppState maintenance ──

    #[test]
    fn base64_decodes_known_vectors_and_rejects_garbage() {
        // Authoritative vectors (python base64).
        assert_eq!(
            decode_base64_std("dXNlcjpnaGdyX3BhdF9zZWNyZXQxMjM=").unwrap(),
            b"user:ghgr_pat_secret123"
        );
        assert_eq!(
            decode_base64_std("AP8QIEA=").unwrap(),
            vec![0, 255, 16, 32, 64]
        );
        assert_eq!(decode_base64_std("").unwrap(), Vec::<u8>::new());
        // Invalid character / impossible length → None, never a panic.
        assert!(decode_base64_std("not base64!!").is_none());
        assert!(decode_base64_std("A").is_none()); // len%4==1 is impossible
        assert!(decode_base64_std("====").unwrap().is_empty());
    }

    #[test]
    fn candidate_secrets_from_bearer_and_basic() {
        // Bearer → the token verbatim.
        assert_eq!(
            candidate_pat_secrets("Bearer ghgr_pat_xyz"),
            vec!["ghgr_pat_xyz".to_string()]
        );
        // Basic → the PASSWORD (git puts the PAT there; username ignored).
        assert_eq!(
            candidate_pat_secrets("Basic dXNlcjpnaGdyX3BhdF9zZWNyZXQxMjM="),
            vec!["ghgr_pat_secret123".to_string()]
        );
        // Basic with an EMPTY username still yields the password.
        assert_eq!(
            candidate_pat_secrets("Basic OmdoZ3JfcGF0X25vdXNlcg=="),
            vec!["ghgr_pat_nouser".to_string()]
        );
        // Case-insensitive scheme (RFC 7235).
        assert_eq!(
            candidate_pat_secrets("bearer ghgr_pat_x"),
            vec!["ghgr_pat_x".to_string()]
        );
    }

    #[test]
    fn candidate_secrets_malformed_inputs_are_panic_safe_and_empty() {
        // No scheme, bad base64, missing colon, non-UTF-8 → empty, never a panic.
        assert!(candidate_pat_secrets("").is_empty());
        assert!(candidate_pat_secrets("Bearer").is_empty()); // no space
        assert!(candidate_pat_secrets("Basic !!!notbase64").is_empty());
        assert!(candidate_pat_secrets("Basic bm9Db2xvbkhlcmU=").is_empty()); // "noColonHere"
        assert!(candidate_pat_secrets("Digest abc").is_empty()); // unsupported scheme
    }

    /// A state with PAT auth ENABLED (the flag flip a test needs; prod is env-gated).
    fn state_pat_on() -> AppState {
        let mut st = state();
        st.pat_auth_enabled = true;
        st
    }

    #[test]
    fn resolve_from_auth_returns_none_when_pat_auth_disabled() {
        let st = state(); // pat_auth_enabled == false
        let created =
            token_create(&st, &body("ci", &["repo:write"]), &user("org-a", "u1"), 10).unwrap();
        // Even a REAL, valid secret does not authenticate while the flag is off.
        assert!(
            st.resolve_pat_from_auth(&format!("Bearer {}", created.secret), 20)
                .is_none(),
            "PAT auth disabled → no resolution (a ghgr_pat_ credential authenticates nothing)"
        );
    }

    #[test]
    fn create_warms_the_index_and_revoke_evicts_it_immediately() {
        let st = state_pat_on();
        let created =
            token_create(&st, &body("ci", &["repo:write"]), &user("org-a", "u1"), 10).unwrap();
        let bearer = format!("Bearer {}", created.secret);
        // Minted → authenticates immediately, no reboot (insert-on-create).
        let auth = st
            .resolve_pat_from_auth(&bearer, 20)
            .expect("a freshly-minted PAT authenticates via the live index");
        assert_eq!(auth.principal, "clerk:org-a:u1");
        assert!(auth.can_write());
        // (The git-CLI Basic form — password = the PAT — is covered by
        // `candidate_secrets_from_bearer_and_basic`, which `resolve_pat_from_auth`
        // funnels through; here we exercise the create/revoke index maintenance.)
        // Revoke → evicted from the live index immediately (remove-on-revoke).
        token_revoke(&st, &created.id, &user("org-a", "u1"), 30).unwrap();
        assert!(
            st.resolve_pat_from_auth(&bearer, 40).is_none(),
            "a revoked PAT stops authenticating immediately (index eviction)"
        );
    }

    #[test]
    fn expired_pat_does_not_resolve_via_auth_header() {
        let st = state_pat_on();
        let created = token_create(
            &st,
            &serde_json::json!({"name":"t","scopes":["repo:read"],"ttl_secs":5})
                .to_string()
                .into_bytes(),
            &user("org-a", "u1"),
            1000,
        )
        .unwrap();
        let bearer = format!("Bearer {}", created.secret);
        assert!(st.resolve_pat_from_auth(&bearer, 5999).is_some());
        assert!(
            st.resolve_pat_from_auth(&bearer, 6000).is_none(),
            "expired at expires_at → no credential"
        );
    }
}
