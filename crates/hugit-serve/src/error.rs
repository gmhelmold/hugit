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

    /// The `{code, reason}` JSON body (UTF-8). Exactly the two fields the frozen
    /// client deserializes; extra fields are never added.
    #[must_use]
    pub fn to_body(&self) -> String {
        json!({ "code": self.code, "reason": self.reason }).to_string()
    }
}
