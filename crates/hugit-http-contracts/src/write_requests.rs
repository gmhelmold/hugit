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
    fn comment_requires_body_allows_optional_anchor() {
        let with =
            serde_json::from_str::<CommentReq>(r#"{"body":"lgtm","anchor":"x.rs:1"}"#).unwrap();
        assert_eq!(with.anchor.as_deref(), Some("x.rs:1"));
        let without = serde_json::from_str::<CommentReq>(r#"{"body":"lgtm"}"#).unwrap();
        assert_eq!(without.anchor, None);
    }
}
