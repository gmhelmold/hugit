//! The `/v1` error envelope: `{ "code": "<MACHINE_CODE>", "reason": "<pt-BR>" }`.
//!
//! Matches the frozen client contract (backend-API-v1 §0): the `githugr-live`
//! `EngineError` reads exactly `code` + `reason`. **401 is ALWAYS auth, 403
//! authz, 404 not-found-or-no-access (no existence leak), 503 engine
//! unavailable.** A tampered/unreadable log is `503 ENGINE_UNAVAILABLE` —
//! fail-honest (never serve a fake-empty VM as if real).

use serde_json::json;

/// A structured engine error → an HTTP status + the `{code, reason}` body.
#[derive(Debug, Clone)]
pub struct EngineErr {
    /// HTTP status code (401 / 404 / 503 / …).
    pub status: u16,
    /// Stable machine code the client switches on (`NOT_FOUND`, `TOKEN_INVALID`, …).
    pub code: &'static str,
    /// Human pt-BR reason (the house voice; the client may show it).
    pub reason: String,
}

impl EngineErr {
    /// 404 — the target does not exist OR the principal cannot see it. The two are
    /// INDISTINGUISHABLE from outside (no existence oracle): same code, same reason.
    #[must_use]
    pub fn not_found() -> Self {
        Self {
            status: 404,
            code: "NOT_FOUND",
            reason: "não encontrado".to_string(),
        }
    }

    /// 401 — missing or invalid Bearer token (authentication). Real Clerk JWKS /
    /// RFC-8693 validation is the P2 identity seam; this is the dev-token stub.
    #[must_use]
    pub fn token_invalid() -> Self {
        Self {
            status: 401,
            code: "TOKEN_INVALID",
            reason: "token de engine ausente ou inválido".to_string(),
        }
    }

    /// 401 — a valid engine token that has EXPIRED. The client calls
    /// `POST /v1/token` for a fresh engine token, then retries once. Distinct
    /// from `TOKEN_INVALID` (which requires re-login).
    #[must_use]
    pub fn token_expired() -> Self {
        Self {
            status: 401,
            code: "TOKEN_EXPIRED",
            reason: "token de engine expirado — renove via /v1/token".to_string(),
        }
    }

    /// 401 — the request AUTHENTICATED, but its principal is NOT entitled to the
    /// action (e.g. `POST /v1/repos` by the platform operator or an anonymous
    /// caller — no god-create / anon-create over the public door: a real user
    /// creates their OWN repo). Distinct from `TOKEN_INVALID` (a bad/missing
    /// bearer): the bearer is valid, the principal is just not a self-provisioning
    /// tenant.
    #[must_use]
    pub fn unauthorized(reason: impl Into<String>) -> Self {
        Self {
            status: 401,
            code: "UNAUTHORIZED",
            reason: reason.into(),
        }
    }

    /// 503 — the engine cannot serve trustworthy data (log unreadable, parse
    /// failure, or a TAMPERED hash chain). Fail-honest: never a fake-empty VM.
    #[must_use]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: 503,
            code: "ENGINE_UNAVAILABLE",
            reason: reason.into(),
        }
    }

    /// 400 — a mutating verb arrived with no `Idempotency-Key` (spec §3). The
    /// write-door rejects BEFORE any side effect.
    #[must_use]
    pub fn idempotency_required() -> Self {
        Self {
            status: 400,
            code: "IDEMPOTENCY_REQUIRED",
            reason: "toda escrita exige um Idempotency-Key".to_string(),
        }
    }

    /// 409 — the same `Idempotency-Key` was replayed with a DIFFERENT body. The
    /// verb is NEVER re-executed; the conflict is reported (spec §3).
    #[must_use]
    pub fn idem_mismatch() -> Self {
        Self {
            status: 409,
            code: "IDEM_MISMATCH",
            reason: "Idempotency-Key reutilizada com um corpo diferente".to_string(),
        }
    }

    /// 403 — a step-up-gated verb (policy, erasure) without fresh reauth (spec §3).
    /// 403 is authZ/step-up; 401 is always authN — they never overlap.
    #[must_use]
    pub fn step_up_required() -> Self {
        Self {
            status: 403,
            code: "STEP_UP_REQUIRED",
            reason: "esta ação exige reautenticação recente".to_string(),
        }
    }

    /// 403 — a policy rule refused the verb SYNCHRONOUSLY (spec §3: a recusa nunca
    /// chega depois). `reason` is the pt-BR "a regra exige <x>" line.
    #[must_use]
    pub fn policy_denied(reason: impl Into<String>) -> Self {
        Self {
            status: 403,
            code: "POLICY_DENIED",
            reason: reason.into(),
        }
    }

    /// 403 — the credential AUTHENTICATED but its SCOPE is insufficient for a
    /// mutation: a `repo:read`-only PAT attempting a write/push. This is the
    /// read-authz ≠ write-authz law at the TOKEN layer — the caller may well OWN the
    /// target (so this is NOT the ownership 404, which hides existence), the token
    /// simply lacks `repo:write`. Distinct from 401 (authN) and the ownership 404.
    #[must_use]
    pub fn scope_insufficient() -> Self {
        Self {
            status: 403,
            code: "SCOPE_INSUFFICIENT",
            reason: "este token não tem escopo de escrita (repo:write)".to_string(),
        }
    }

    /// 403 — a PAT tried to MINT another token. Token creation requires a session
    /// (browser/Clerk) credential, GitHub-style — a PAT is a git/API credential, not a
    /// session, so it cannot spawn survivor tokens that would outlive its own
    /// revocation. Distinct from `SCOPE_INSUFFICIENT` (that is about write scope; this
    /// is about the credential TYPE).
    #[must_use]
    pub fn pat_cannot_mint() -> Self {
        Self {
            status: 403,
            code: "PAT_CANNOT_MINT",
            reason: "um PAT não pode criar tokens — use uma sessão de navegador".to_string(),
        }
    }

    /// 400 — a malformed/invalid request body (e.g. an unknown `mode`/`verdict`
    /// enum value). Distinct from the auth/idempotency/policy refusals.
    #[must_use]
    pub fn invalid_request(reason: impl Into<String>) -> Self {
        Self {
            status: 400,
            code: "INVALID_REQUEST",
            reason: reason.into(),
        }
    }

    /// 409 — a compare-and-swap head mismatch: the durable log moved between the
    /// write-door's `load` and its `persist` (a concurrent writer won the race).
    /// This is an INTERNAL retry signal — `with_write` catches it (via
    /// [`Self::is_cas_conflict`]), reloads, and re-runs; it only surfaces to a
    /// client after the bounded retry budget is exhausted, and then as a 503
    /// (transient contention), never as this code.
    #[must_use]
    pub fn cas_conflict() -> Self {
        Self {
            status: 409,
            code: "CAS_CONFLICT",
            reason: "escrita concorrente — o log mudou durante a operação".to_string(),
        }
    }

    /// Whether this is the [`Self::cas_conflict`] head-mismatch signal — the
    /// write-door's cue to reload + retry rather than fail.
    #[must_use]
    pub fn is_cas_conflict(&self) -> bool {
        self.code == "CAS_CONFLICT"
    }

    /// 429 — the upstream `/v1/session/exchange` throttled the per-principal mint
    /// (its `429`). Surfaced honestly so the client backs off + retries, rather
    /// than collapsing to a generic 503. The Clerk JWT is never echoed.
    #[must_use]
    pub fn rate_limited() -> Self {
        Self {
            status: 429,
            code: "RATE_LIMITED",
            reason: "muitas solicitações de token — tente novamente em instantes".to_string(),
        }
    }

    /// 429 — the per-principal ENGINE rate limit (G10) refused this request: the
    /// caller (tenant/anonymous edge) exceeded its req/s budget. Rejected INLINE in
    /// the accept-loop preamble BEFORE any body read or worker spawn. Distinct from
    /// [`Self::rate_limited`] (the upstream token-mint throttle).
    #[must_use]
    pub fn too_many_requests() -> Self {
        Self {
            status: 429,
            code: "RATE_LIMITED",
            reason: "muitas solicitações — reduza o ritmo e tente de novo".to_string(),
        }
    }

    /// The `{code, reason}` JSON body (UTF-8). Exactly the two fields the frozen
    /// client deserializes; extra fields are never added.
    #[must_use]
    pub fn to_body(&self) -> String {
        json!({ "code": self.code, "reason": self.reason }).to_string()
    }
}
