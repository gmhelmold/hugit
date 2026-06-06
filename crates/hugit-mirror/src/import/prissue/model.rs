//! Data model for PR/issue import.
//!
//! Defines the fixture/import types used to project GitHub PR and issue
//! metadata into **proposed, non-authoritative** intents.

use hugit_contracts::intent_sidecar::IntentSidecar;

// ── Fidelity contract ──────────────────────────────────────────────────────────
//
// The stated set of preserved elements (per-element provenance required):
//   • body
//   • comment/review threads
//   • state
//   • labels
//   • cross-refs
//
// NON_IMPORTED — explicitly enumerated; nothing is silently dropped:

/// Elements that are explicitly NOT imported from a PR/issue.
///
/// No element is silently dropped — anything not imported is named here.
/// (cf. hugit whitepaper §10: social-graph signals ride the mirror, not
/// stormed.)
pub const NON_IMPORTED: &[&str] = &[
    "reactions",
    "social_graph_signals",
    "emoji_reactions",
    "user_follow_graph",
    "star_count",
    "fork_count",
    "watch_count",
    "assignees_social",
    "milestone_progress_percent",
    "project_cards",
    "timeline_events_non_substantive",
    "lock_reason",
    "active_lock_reason",
];

// ── Provenance ─────────────────────────────────────────────────────────────────

/// Per-element provenance: records the origin of a single imported element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementProvenance {
    /// The source URL of the GitHub PR or issue this element originated from.
    pub source_url: String,
    /// The specific element being traced (e.g. "body", "comment/42", "label/bug").
    pub element_origin: String,
}

impl ElementProvenance {
    /// Construct element provenance from a source URL and element name.
    pub fn new(source_url: impl Into<String>, element_origin: impl Into<String>) -> Self {
        Self {
            source_url: source_url.into(),
            element_origin: element_origin.into(),
        }
    }
}

// ── Cross-ref resolution ───────────────────────────────────────────────────────

/// A cross-reference to another PR or issue.
///
/// When both ends of the reference are imported, `resolved_to` is `Some`.
/// When only one end is imported (the other was not fetched), the cross-ref is
/// recorded as an explicit **residual** — never fabricated or silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossRef {
    /// The raw cross-reference text as it appeared in the source (e.g. `#42`).
    pub raw: String,
    /// The source URL where this cross-ref was found.
    pub provenance: ElementProvenance,
    /// Resolved object id when the target was also imported; `None` when the
    /// target is not part of this import batch — recorded as a dangling
    /// residual, never fabricated.
    pub resolved_to: Option<String>,
    /// Whether this cross-ref is unresolved (dangling / residual).
    pub residual: bool,
}

impl CrossRef {
    /// A resolved cross-ref: both ends were imported.
    pub fn resolved(
        raw: impl Into<String>,
        provenance: ElementProvenance,
        target: impl Into<String>,
    ) -> Self {
        Self {
            raw: raw.into(),
            provenance,
            resolved_to: Some(target.into()),
            residual: false,
        }
    }

    /// An unresolved (dangling) cross-ref: only one end was imported.
    ///
    /// Recorded as an explicit residual; never fabricated.
    pub fn dangling(raw: impl Into<String>, provenance: ElementProvenance) -> Self {
        Self {
            raw: raw.into(),
            provenance,
            resolved_to: None,
            residual: true,
        }
    }
}

// ── Comment / review thread ───────────────────────────────────────────────────

/// A single comment or review-thread entry imported from a PR or issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedComment {
    /// Stable id of the comment in the source system (GitHub comment id).
    pub id: u64,
    /// Body text of the comment.
    pub body: String,
    /// Author login.
    pub author: String,
    /// Unix epoch milliseconds when the comment was created.
    pub created_at: u64,
    /// Per-element provenance.
    pub provenance: ElementProvenance,
}

// ── PrIssueState ──────────────────────────────────────────────────────────────

/// The open/closed/merged state of the imported PR or issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrIssueState {
    Open,
    Closed,
    Merged,
}

impl PrIssueState {
    /// String representation matching GitHub API values.
    pub fn as_str(&self) -> &str {
        match self {
            PrIssueState::Open => "open",
            PrIssueState::Closed => "closed",
            PrIssueState::Merged => "merged",
        }
    }
}

// ── Source kind ───────────────────────────────────────────────────────────────

/// Whether the import source is a Pull Request or an Issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    PullRequest,
    Issue,
}

// ── ImportedPrIssue ───────────────────────────────────────────────────────────

/// A PR or issue imported from GitHub, ready to be projected into a proposed
/// non-authoritative intent.
///
/// Every preserved element carries its own [`ElementProvenance`]. Elements in
/// [`NON_IMPORTED`] are explicitly excluded and never appear here.
#[derive(Debug, Clone)]
pub struct ImportedPrIssue {
    /// Source kind (PR or issue).
    pub kind: SourceKind,
    /// Numeric id in the source system.
    pub source_id: u64,
    /// Full URL of the source PR or issue.
    pub source_url: String,
    /// Body text, with provenance.
    pub body: String,
    /// Provenance of the body element.
    pub body_provenance: ElementProvenance,
    /// Open/closed/merged state, with provenance.
    pub state: PrIssueState,
    /// Provenance of the state element.
    pub state_provenance: ElementProvenance,
    /// Labels applied to this PR/issue, each with its own provenance.
    pub labels: Vec<(String, ElementProvenance)>,
    /// Comment and review thread entries, each with per-element provenance.
    pub comments: Vec<ImportedComment>,
    /// Cross-references found in the body or comments; dangling ones are
    /// recorded as explicit residuals, never fabricated.
    pub cross_refs: Vec<CrossRef>,
}

impl ImportedPrIssue {
    /// Project this imported PR/issue into a proposed, non-authoritative
    /// [`ProposedIntent`].
    ///
    /// The sidecar is flagged `authoritative = false` and carries the source
    /// URL as `context_ref` so the provenance chain is preserved end-to-end.
    /// The `intent_id` is minted from the source URL to ensure stability
    /// across re-imports (same source → same id).
    pub fn to_proposed_intent(&self) -> ProposedIntent {
        // Stable intent_id: content-derived from source_url so re-import is idempotent.
        let intent_id = format!("proposed::{}", self.source_url);

        let charter = match self.kind {
            SourceKind::PullRequest => {
                format!("PR import: {} [{}]", self.source_url, self.state.as_str())
            }
            SourceKind::Issue => {
                format!(
                    "Issue import: {} [{}]",
                    self.source_url,
                    self.state.as_str()
                )
            }
        };

        let acceptance: Vec<String> = self
            .labels
            .iter()
            .map(|(lbl, _prov)| format!("label:{lbl}"))
            .collect();

        let sidecar = IntentSidecar {
            intent_id,
            charter,
            acceptance,
            context_ref: self.source_url.clone(),
            // Non-authoritative by design — never gates landing.
            authoritative: false,
        };

        ProposedIntent {
            sidecar,
            source_url: self.source_url.clone(),
            element_provenance: ProposedIntentProvenance {
                body_provenance: self.body_provenance.clone(),
                state_provenance: self.state_provenance.clone(),
                label_provenances: self.labels.iter().map(|(_, p)| p.clone()).collect(),
                comment_provenances: self.comments.iter().map(|c| c.provenance.clone()).collect(),
                cross_ref_provenances: self
                    .cross_refs
                    .iter()
                    .map(|cr| cr.provenance.clone())
                    .collect(),
            },
            cross_refs: self.cross_refs.clone(),
            non_imported: NON_IMPORTED,
        }
    }
}

// ── ProposedIntent ────────────────────────────────────────────────────────────

/// The output of projecting a PR/issue: a proposed, non-authoritative intent
/// with full per-element provenance and the explicit non-imported enumeration.
#[derive(Debug, Clone)]
pub struct ProposedIntent {
    /// The proposed intent sidecar (always `authoritative = false`).
    pub sidecar: IntentSidecar,
    /// The source URL for this intent's provenance.
    pub source_url: String,
    /// Per-element provenance for each preserved fidelity element.
    pub element_provenance: ProposedIntentProvenance,
    /// Cross-references (resolved or dangling/residual).
    pub cross_refs: Vec<CrossRef>,
    /// The explicit non-imported elements list.
    pub non_imported: &'static [&'static str],
}

impl ProposedIntent {
    /// Verify the proposed/non-authoritative invariant holds.
    ///
    /// Returns `true` when the sidecar is correctly flagged as non-authoritative.
    pub fn is_proposed_non_authoritative(&self) -> bool {
        !self.sidecar.authoritative
    }
}

/// Per-element provenance for every preserved fidelity element.
#[derive(Debug, Clone)]
pub struct ProposedIntentProvenance {
    /// Provenance for the body element.
    pub body_provenance: ElementProvenance,
    /// Provenance for the state element.
    pub state_provenance: ElementProvenance,
    /// Provenance for each label element.
    pub label_provenances: Vec<ElementProvenance>,
    /// Provenance for each comment/review-thread entry.
    pub comment_provenances: Vec<ElementProvenance>,
    /// Provenance for each cross-reference.
    pub cross_ref_provenances: Vec<ElementProvenance>,
}
