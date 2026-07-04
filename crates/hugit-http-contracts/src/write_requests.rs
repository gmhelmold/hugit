//! Write-verb REQUEST bodies (backend-API-v1 §3). Transcribed byte-for-field from
//! the `Actions` trait in the companion web frontend — one struct
//! per POST verb, the JSON body the window sends. The success body is
//! [`crate::actions::Accepted`]; the error body is `{code, reason}` (`EngineErr`).
//!
//! Every write also carries an `Idempotency-Key` HEADER (NOT in these bodies) —
//! mandatory; the engine's write-door enforces it (`400 IDEMPOTENCY_REQUIRED`).
//! Free-text fields (`note`, `body`, `ask`, `content`, `param`, `description`) are
//! scrubbed at the write boundary before they are persisted.
//!
//! Additive-only: changing a shape after dispatch is an integration break (lead-only).

use serde::{Deserialize, Serialize};

/// `POST /v1/repos/{repo}/prs/{n}/land` — enqueue/settle a PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandReq {
    /// `"union" | "serial" | "window"`.
    pub mode: String,
}

/// `POST /v1/repos/{repo}/prs/{n}/verdict` — record an adversarial verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictReq {
    /// `"approve" | "request-changes"`.
    pub verdict: String,
    /// Free-text reviewer note (scrubbed at the write boundary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `POST /v1/repos/{repo}/intents/{id}/usage` — record an authoring run's REAL
/// provider token usage against an intent (the engine mirror of `hugit ctx usage`).
///
/// The engine appends a canonical `ctx.usage` record byte-identical to the CLI's;
/// the cost read-fold parses it back, so the field names below are FROZEN. hugit
/// records verbatim + prices nowhere: the figures land as-submitted (checked only
/// for the trivially-consistent `total = input+output+cache_read+cache_write`,
/// fail-closed on overflow).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReq {
    /// The model id the usage was measured on (e.g. `claude-opus-4-8`). Scrubbed
    /// at the write boundary (a secret-shaped value redacts, a real slug survives).
    pub model: String,
    /// Input tokens (non-cached), read from the provider's `/usage`.
    pub input: u64,
    /// Output tokens, read from the provider's `/usage`.
    pub output: u64,
    /// Tokens read from the prompt cache.
    pub cache_read: u64,
    /// Tokens written to the prompt cache.
    pub cache_write: u64,
    /// Optional content-address digest pinning the exact model build (scrubbed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_digest: Option<String>,
    /// Optional authoring-finish timestamp (unix ms) stamped on the payload.
    /// Defaults to `0` — the canonical forever-log is clock-untrusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<u64>,
}

/// `POST /v1/repos/{repo}/prs/{n}/comments` — a PR comment, optionally anchored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentReq {
    /// Free-text comment body (scrubbed at the write boundary).
    pub body: String,
    /// The diff anchor, e.g. `"token.rs:42"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

/// `POST /v1/repos/{repo}/dispatch` — ask for work; the orchestrator plans + opens a PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchReq {
    /// Free-text ask (scrubbed at the write boundary).
    pub ask: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign: Option<String>,
    /// Open the PR as a draft (no auto-spawn until a human/owner gate).
    pub draft: bool,
}

/// `POST /v1/repos/{repo}/prs` — open a PR from a pushed branch (head vs base).
///
/// The GitHub-faithful "open a PR on a branch's diff" verb: `head` and `base` are
/// ref-ish names (a short branch/tag, a full `refs/heads/…`, or a raw 40-hex oid)
/// resolved against the repo's LIVE refs at open time; the engine pins each tip's
/// SHA into the `pr.opened` event so the PR renders a REAL head-vs-base diff on
/// read-back. `title`/`body` are free text (scrubbed at the write boundary). The
/// success body is [`crate::actions::Accepted`] carrying the fresh `pr_number`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrCreateReq {
    /// The source (feature) branch/ref-ish the change lives on, e.g. `"feat/x"`.
    pub head: String,
    /// The target branch/ref-ish to open the PR against, e.g. `"main"`.
    pub base: String,
    /// The PR title (scrubbed at the write boundary).
    pub title: String,
    /// Optional PR description/body (scrubbed at the write boundary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// `POST /v1/repos/{repo}/issues/{n}/transition` — move an issue's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueTransitionReq {
    /// `"backlog" | "open" | "closed" | "dispatch"`.
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// `POST /v1/repos/{repo}/policy` — toggle a policy rule. STEP-UP gated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyReq {
    pub rule_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Optional rule parameter (scrubbed at the write boundary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
}

/// `POST /v1/repos/{repo}/erasure/{id}/decide` — approve/deny an erasure. STEP-UP gated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureDecideReq {
    pub approve: bool,
}

/// `POST /v1/repos/{repo}/edit/{path}/propose` — a web edit → signed branch + PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditProposeReq {
    /// The FULL edited file body the user typed (scrubbed at the write boundary;
    /// dropping it would silently lose the user's work — spec §3).
    pub content: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `POST /v1/repos/{repo}/undo` — revert a prior op by its event sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoReq {
    pub op_seq: u64,
}

/// `POST /v1/repos/{repo}/repo/meta` — set a repo's visibility. Operator/owner-only.
///
/// Sets the authz predicate the read gate projects from the log (`repo.meta` record).
/// `visibility` MUST be `"public"` or `"private"` (fail-closed on any other value).
/// `owner_tenant` assigns the owning org; omit to leave the current owner unchanged
/// (the latest `repo.meta` record wins — each call is a full-replace of the projection).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMetaReq {
    /// `"public"` or `"private"`.
    pub visibility: String,
    /// The owning org (e.g. `"org-a"`). `None` leaves no owner assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_tenant: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn land_req_round_trips() {
        let v = LandReq {
            mode: "union".into(),
        };
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<LandReq>(&s).unwrap(), v);
    }

    #[test]
    fn optional_fields_omitted_when_none() {
        // `note`/`anchor` absent ⇒ not serialized (clean wire), and an absent key
        // deserializes back to None (the window may omit them).
        let s = serde_json::to_string(&VerdictReq {
            verdict: "approve".into(),
            note: None,
        })
        .unwrap();
        assert_eq!(s, r#"{"verdict":"approve"}"#);
        let back: VerdictReq = serde_json::from_str(r#"{"verdict":"approve"}"#).unwrap();
        assert_eq!(back.note, None);
    }

    #[test]
    fn usage_req_round_trips_and_omits_optionals() {
        let v = UsageReq {
            model: "claude-opus-4-8".into(),
            input: 1,
            output: 2,
            cache_read: 3,
            cache_write: 4,
            model_digest: None,
            recorded_at: None,
        };
        let s = serde_json::to_string(&v).unwrap();
        // Optionals absent when None (clean wire) and round-trip back.
        assert!(!s.contains("model_digest"));
        assert!(!s.contains("recorded_at"));
        assert_eq!(serde_json::from_str::<UsageReq>(&s).unwrap(), v);
        // A body carrying the optionals parses them back.
        let with: UsageReq = serde_json::from_str(
            r#"{"model":"m","input":0,"output":0,"cache_read":0,"cache_write":0,"model_digest":"d","recorded_at":9}"#,
        )
        .unwrap();
        assert_eq!(with.model_digest.as_deref(), Some("d"));
        assert_eq!(with.recorded_at, Some(9));
    }

    #[test]
    fn pr_create_req_round_trips_and_omits_optional_body() {
        let v = PrCreateReq {
            head: "feat/x".into(),
            base: "main".into(),
            title: "add x".into(),
            body: None,
        };
        let s = serde_json::to_string(&v).unwrap();
        // The optional body is absent from the wire when None, and round-trips back.
        assert!(!s.contains("body"));
        assert_eq!(serde_json::from_str::<PrCreateReq>(&s).unwrap(), v);
        // A body-carrying request parses the description back.
        let with: PrCreateReq =
            serde_json::from_str(r#"{"head":"feat/x","base":"main","title":"t","body":"why"}"#)
                .unwrap();
        assert_eq!(with.body.as_deref(), Some("why"));
    }

    #[test]
    fn comment_requires_body_allows_optional_anchor() {
        let with =
            serde_json::from_str::<CommentReq>(r#"{"body":"lgtm","anchor":"x.rs:1"}"#).unwrap();
        assert_eq!(with.anchor.as_deref(), Some("x.rs:1"));
        let without = serde_json::from_str::<CommentReq>(r#"{"body":"lgtm"}"#).unwrap();
        assert_eq!(without.anchor, None);
    }
}
