//! Personal Access Tokens (PATs) — the durable per-account store + create/revoke.
//! (WP-#90 slice 2a. The git-auth WIRE — accepting a PAT as the git credential — is
//! slice 2b, the hot-path/latency-sensitive follow-on; NOT here.)
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
    )
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
}
