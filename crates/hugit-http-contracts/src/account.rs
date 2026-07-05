//! `GET /v1/me/account` → `AccountVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

/// One PAT row — METADATA ONLY (token value never reaches the browser — ADR-0002).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatVm {
    pub name: String,
    pub meta: String,
}

/// Account settings page — ALL render data, no Option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountVm {
    pub name: String,
    pub user: String,
    pub subtitle: String,
    pub org: String,
    pub bio: String,
    pub company: String,
    pub user_lock_note: String,
    pub email: String,
    pub email_verified: bool,
    pub email_help: String,
    pub language: String,
    pub languages: Vec<String>,
    pub pat_note: String,
    pub pats: Vec<PatVm>,
    pub notifications_note: String,
    pub repos_note: String,
    pub danger_title: String,
    pub danger_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_vm_round_trips() {
        let vm = AccountVm {
            name: "Test User".into(),
            user: "gustavo".into(),
            subtitle: "Sua conta HuGR — uma conta pra família toda".into(),
            org: "humangr".into(),
            bio: "fundador · HuGR".into(),
            company: "HuGR".into(),
            user_lock_note: "fixo — identidade HuGR".into(),
            email: "owner@example.com".into(),
            email_verified: true,
            email_help: "É o e-mail da conta HuGR.".into(),
            language: "Português (Brasil)".into(),
            languages: vec!["Português (Brasil)".into(), "English".into()],
            pat_note: "PATs são criados via CLI e nunca aparecem no navegador.".into(),
            pats: vec![PatVm {
                name: "ci-runner-hetzner".into(),
                meta: "escopo org · criado 05 jun · último uso há 2 h".into(),
            }],
            notifications_note: "githugr não tem notificações — tem a Atenção ◎".into(),
            repos_note: "4 repositórios na conta · gerencie no dashboard →".into(),
            danger_title: "Excluir conta".into(),
            danger_note: "Remove permanentemente sua conta e dados.".into(),
        };
        let json = serde_json::to_string(&vm).expect("serialize");
        let reparsed: AccountVm = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(vm, reparsed, "AccountVm round-trip is lossless");
    }

    #[test]
    fn me_account_vm_round_trips() {
        let vm = MeAccountVm {
            usage: AccountUsageVm {
                repos_count: 4,
                log_footprint_bytes: 20_480,
            },
            pats: vec![PatMetaVm {
                id: "pat_abc".into(),
                name: "ci-runner".into(),
                created_at: 1_720_000_000,
                last_used_at: 1_720_100_000,
                scopes: vec!["repo:read".into(), "repo:write".into()],
            }],
        };
        let json = serde_json::to_string(&vm).expect("serialize");
        let reparsed: MeAccountVm = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(vm, reparsed, "MeAccountVm round-trip is lossless");
    }
}

// ── the per-principal DATA read (githugr ASK 2026-07-05) ─────────────────────
//
// Distinct from the presentational [`AccountVm`] above (display strings, window
// locale): this is the STRUCTURED per-caller data `GET /v1/me/account` serves —
// machine values only, so the window composes its own copy (the githugr TL's
// clean-contract rule: machine value + window-side i18n). Scoped by the caller's
// session Bearer; another principal's data is never returned (same isolation as
// `me/dashboard`).

/// Per-principal account USAGE (informational — githugr is FREE, NOT billing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUsageVm {
    /// The exact count of repos the caller owns (the authorized `me` set).
    pub repos_count: u64,
    /// The caller's DURABLE event-log/metadata footprint in bytes — the sum of the
    /// serialized per-repo event logs. NOTE: the git CONTENT objects are
    /// content-addressed + cross-tenant DEDUPLICATED, so they are NOT a clean
    /// per-account figure and are NOT counted here (a true content-storage meter is
    /// a deferred CoreLink-CAS seam). This is the honest, cheap, per-account figure.
    pub log_footprint_bytes: u64,
}

/// One PAT's STRUCTURED metadata (the token SECRET NEVER leaves the engine —
/// ADR-0002; the window renders metadata only). Sentinel `0` for `last_used_at`
/// means never used (the module's no-`Option` house style).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatMetaVm {
    /// Stable opaque id (never the secret).
    pub id: String,
    /// The user-chosen label.
    pub name: String,
    /// Unix seconds when the PAT was minted.
    pub created_at: u64,
    /// Unix seconds of last use; `0` = never used.
    pub last_used_at: u64,
    /// The granted scopes (machine values, e.g. `repo:read`).
    pub scopes: Vec<String>,
}

/// The `GET /v1/me/account` structured DATA read — the caller's own usage + PAT
/// metadata. `pats` is empty until the engine PAT store lands (a fresh caller has
/// none); the usage figures are real today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeAccountVm {
    pub usage: AccountUsageVm,
    pub pats: Vec<PatMetaVm>,
}

/// `POST /v1/me/tokens` request — mint a PAT. `scopes` empty → the engine default
/// (`repo:read`); `ttl_secs` `0` → never expires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTokenReq {
    pub name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub ttl_secs: u64,
}

/// `POST /v1/me/tokens` response — the created token. **`secret` is returned EXACTLY
/// ONCE** (the engine stores only its hash; it can never be shown again — ADR-0002).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedTokenVm {
    pub id: String,
    pub name: String,
    /// The raw secret — shown ONCE, never recoverable. The client MUST capture it now.
    pub secret: String,
    pub scopes: Vec<String>,
    pub created_at: u64,
    /// Unix ms when it expires; `0` = never.
    pub expires_at: u64,
}
