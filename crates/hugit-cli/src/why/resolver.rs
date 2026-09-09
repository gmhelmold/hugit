//! `why` provenance resolver (WP-D10 ①④).
//!
//! Resolves a file/line or symbol reference to the originating [`Intent`] +
//! the full attestation provenance (charter / author / model / cost) by
//! walking the event log. The answer is asserted to MATCH the event log —
//! there is no divergent second source.
//!
//! ## Derived-bytes honesty (item ④ / R6)
//!
//! When the queried file is regenerated/derived, `why` resolves to the
//! REGEN/DERIVATION event, and [`AuthorKind`] is set to
//! [`AuthorKind::Derived`]. It NEVER fabricates a human author for machine-
//! produced bytes.

use hugit_contracts::{AttestationChain, EventRecord, IntentSidecar};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Query shape
// ---------------------------------------------------------------------------

/// A query to `hugit why`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhyQuery {
    /// The file path being queried.
    pub path: String,
    /// Optional 1-based line number within the file.
    pub line: Option<u64>,
    /// Optional symbol name (function / type / const).
    pub symbol: Option<String>,
}

// ---------------------------------------------------------------------------
// Answer shape
// ---------------------------------------------------------------------------

/// Whether the originating author was a human intent or a machine derivation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorKind {
    /// A human-authored landed intent.  `principal_chain` names the humans /
    /// agents that produced it.
    Intent,
    /// Regenerated / derived bytes — produced by a regen/derivation event, not
    /// by a human. The event `kind` is stored in `event_kind`.
    Derived { event_kind: String },
}

/// The full provenance answer returned by `resolve_why`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceAnswer {
    // ── originating intent (item ①) ─────────────────────────────────────
    /// The stable intent_id of the originating intent, if this is an
    /// `AuthorKind::Intent`.  `None` for derived/regen events.
    pub intent_id: Option<String>,

    /// Human-readable charter describing what the intent/regen did.
    pub charter: String,

    /// Ordered chain of principals (humans / runners / models) that produced
    /// this event — the "author" field.
    pub author: Vec<String>,

    /// Model identifier that participated in the pipeline (`""` if none).
    pub model: String,

    /// Cost annotation as recorded on the attestation chain (`""` if none).
    ///
    /// Whitepaper §9: cost is carried in the attestation, not re-derived.
    pub cost: String,

    // ── event-log anchor ─────────────────────────────────────────────────
    /// Log sequence number of the originating event.
    pub event_seq: u64,

    /// Event kind string (`"intent.landed"`, `"regen.derived"`, etc.).
    pub event_kind: String,

    /// SHA-256 hex digest of the originating event (from the log).
    pub event_hash: String,

    // ── author kind (item ④ / R6) ────────────────────────────────────────
    /// Whether this is a human intent or a machine derivation.
    pub author_kind: AuthorKind,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from `resolve_why`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhyError {
    /// No event in the log matches the queried path.
    NotFound { path: String },
    /// An event payload could not be parsed.
    BadPayload { seq: u64 },
    /// The path is known, but the requested line could not be attributed to any
    /// specific event range. A line/symbol query that cannot resolve to a
    /// concrete event range is REJECTED, never silently widened to file level —
    /// answering the wrong event is worse than admitting "unknown".
    LineUnresolved { path: String, line: u64 },
    /// The path is known, but the requested symbol is not attributed to any
    /// event. Rejected rather than mis-attributed.
    SymbolUnresolved { path: String, symbol: String },
}

/// Exact result after Git has blamed a committed-tree line. It never selects an
/// event by path recency: caller supplies Git's owning commit oid.
#[derive(Debug, Clone, PartialEq)]
pub enum PreciseLineResolution {
    Attributed {
        commit: String,
        answer: ProvenanceAnswer,
    },
    Unattributed {
        commit: String,
    },
    RangeUnavailable {
        commit: String,
    },
    RangeMismatch {
        commit: String,
    },
}

impl std::fmt::Display for WhyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WhyError::NotFound { path } => write!(f, "no provenance found for `{path}`"),
            WhyError::BadPayload { seq } => {
                write!(f, "malformed event payload at seq {seq}")
            }
            WhyError::LineUnresolved { path, line } => {
                write!(f, "line {line} of `{path}` resolves to no specific event")
            }
            WhyError::SymbolUnresolved { path, symbol } => {
                write!(
                    f,
                    "symbol `{symbol}` in `{path}` resolves to no specific event"
                )
            }
        }
    }
}

impl std::error::Error for WhyError {}

// ---------------------------------------------------------------------------
// Event-log entry used by the resolver
// ---------------------------------------------------------------------------

/// A self-contained event-log entry presented to the resolver.
///
/// Combines the frozen [`EventRecord`] with the optional attestation chain
/// and sidecar that a real implementation would load from the refstore.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub record: EventRecord,
    pub attestation: Option<AttestationChain>,
    pub sidecar: Option<IntentSidecar>,
}

// ---------------------------------------------------------------------------
// Event-kind constants
// ---------------------------------------------------------------------------

/// The event kind for a landed intent (mirrors D4's `INTENT_LANDED_KIND`).
pub const INTENT_LANDED_KIND: &str = "intent.landed";

/// The event kind for a regen / derivation event.
pub const REGEN_DERIVED_KIND: &str = "regen.derived";

// ---------------------------------------------------------------------------
// Provenance chain (the `why --walk` view)
// ---------------------------------------------------------------------------

/// One link in the provenance chain for a queried path: an event that captured
/// the path, projected raw (the walk does NOT merge/attribute — the CLI does).
///
/// The chain is the answer to "how did this path evolve": every captured commit
/// / checkout / merge / push-attempt that touched it, most-recent first. The
/// single-event [`ProvenanceAnswer`] (origin) is the head of this chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainEntry {
    /// Log sequence number of the captured event.
    pub seq: u64,
    /// SHA-256 hex digest of the event (from the log).
    pub event_hash: String,
    /// Event kind (always `ref.update` for captured git activity).
    pub kind: String,
    /// The recorder principal (defaults to `orchestrator:hugit-hook`).
    pub author: Vec<String>,
    /// Wall-clock recorded_at (committer date for commit/merge, delivery time
    /// for checkout/push-attempt).
    pub recorded_at: u64,
    /// The ref/branch the event describes (`refs/heads/<branch>` or `HEAD`).
    pub reference: Option<String>,
    /// The target oid: commit/merge `target`, checkout `to`.
    pub oid: Option<String>,
    /// The branch name, when the event carries one.
    pub branch: Option<String>,
    /// Qualifiers: `checkout`, `attempt`, `merged_from`, as recorded — the
    /// frozen `ref.update` kind distinguishes hook source by these.
    pub qualifiers: Option<serde_json::Value>,
    /// The files the event touched (commit `files` list; empty for checkout /
    /// merge / push-attempt which do not scrub paths).
    pub files: Vec<String>,
}

/// Resolve a [`WhyQuery`] to the FULL provenance chain: every event on the log
/// that cites `query.path`, most-recent first, projected raw.
///
/// The walk is the provenance history behind the single-event origin. It never
/// flattens or fuses events — each captured commit stays a distinct link
/// (identical to how `git log -- <path>` keeps each commit separate). A path
/// that no event ever cites yields an empty chain (NOT an error): "no captured
/// git activity touched this" is a true, distinct answer from "corrupt log".
///
/// The chain is only meaningful for captured git activity (the hooks write kind
/// `ref.update` + a `files` array), so attribution reuses the same
/// [`payload_attribution`] resolver a path-only query uses; the walk is
/// strictly path-level (no line/symbol narrowing).
pub fn resolve_why_chain(query: &WhyQuery, entries: &[LogEntry]) -> Vec<ChainEntry> {
    entries
        .iter()
        .rev()
        .filter_map(|entry| {
            payload_attribution(&entry.record.payload, &query.path)?;
            let payload: serde_json::Value =
                serde_json::from_str(&entry.record.payload).unwrap_or(serde_json::Value::Null);
            let qualifiers = {
                let mut q = serde_json::Map::new();
                if payload.get("checkout").is_some() {
                    q.insert(
                        "checkout".into(),
                        payload
                            .get("checkout")
                            .cloned()
                            .unwrap_or(serde_json::Value::Bool(true)),
                    );
                }
                if payload.get("attempt").is_some() {
                    q.insert(
                        "attempt".into(),
                        payload
                            .get("attempt")
                            .cloned()
                            .unwrap_or(serde_json::Value::Bool(true)),
                    );
                }
                if let Some(from) = payload.get("merged_from") {
                    q.insert("merged_from".into(), from.clone());
                }
                if q.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(q))
                }
            };
            Some(ChainEntry {
                seq: entry.record.seq,
                event_hash: entry.record.this_hash.clone(),
                kind: entry.record.kind.clone(),
                author: entry.record.principal_chain.clone(),
                recorded_at: entry.record.recorded_at,
                reference: payload
                    .get("ref")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                oid: payload
                    .get("target")
                    .and_then(|v| v.as_str())
                    .or_else(|| payload.get("to").and_then(|v| v.as_str()))
                    .map(String::from),
                branch: payload
                    .get("branch")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                qualifiers,
                files: parse_files(&payload),
            })
        })
        .collect()
}

/// Extract the `files` array from a payload, if present.
fn parse_files(v: &serde_json::Value) -> Vec<String> {
    v.get("files")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    f.as_str()
                        .or_else(|| f.get("path").and_then(|path| path.as_str()))
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// resolve_why
// ---------------------------------------------------------------------------

/// Resolve a [`WhyQuery`] against a slice of log entries.
///
/// ## Algorithm
///
/// 1. Walk entries in reverse log order (most-recent first).
/// 2. For each entry, check whether its payload references `query.path`.
/// 3. The first matching entry is the originating event.
/// 4. Build a [`ProvenanceAnswer`] from the matching entry — never from a
///    second source.
/// 5. If the event kind is a regen/derivation kind, set
///    `author_kind = AuthorKind::Derived` — NEVER fabricate a human author
///    (item ④ / R6).
///
/// ## Line + symbol attribution (the precise-`why` contract)
///
/// When `query.line` or `query.symbol` is set, an event matches the queried
/// path ONLY when its payload attributes that specific line range or symbol.
/// Two different lines on the same file therefore resolve to DIFFERENT events
/// (each owns its own range), and a line/symbol that no event range claims is
/// REJECTED ([`WhyError::LineUnresolved`] / [`WhyError::SymbolUnresolved`]) —
/// never silently widened to a file-level answer, which would mis-attribute.
///
/// File-level back-compat: a payload that names the path but carries NO range /
/// symbol attribution still matches a path-only query (and a line/symbol query
/// falls back to it only when NO event in the log carries precise attribution
/// for that path — i.e. the corpus simply has no line granularity yet).
///
/// ## Fixture contract
///
/// `entries` is the complete log slice.  In production this would come from the
/// refstore; in tests it is a hand-crafted fixture that doubles as the oracle.
pub fn resolve_why(query: &WhyQuery, entries: &[LogEntry]) -> Result<ProvenanceAnswer, WhyError> {
    // Does ANY event carry precise (range/symbol) attribution for this path?
    // If so, a line/symbol query MUST resolve precisely or be rejected — it
    // may not fall back to a file-level match (which would mis-attribute).
    let path_has_precise_attribution = entries
        .iter()
        .any(|e| payload_attribution(&e.record.payload, &query.path).is_some_and(|a| a.precise()));

    // Walk in reverse so we get the most-recent (=originating for the current
    // state) event among the candidates that match the query precision.
    let mut file_level_fallback: Option<&LogEntry> = None;

    for entry in entries.iter().rev() {
        let Some(attr) = payload_attribution(&entry.record.payload, &query.path) else {
            continue;
        };

        // Precise match: this event's ranges/symbols claim the queried line/symbol.
        if attr.matches(query.line, query.symbol.as_deref()) {
            return build_answer(entry).map_err(|_| WhyError::BadPayload {
                seq: entry.record.seq,
            });
        }

        // Remember the first (most-recent) file-level path match for fallback.
        if !attr.precise() && file_level_fallback.is_none() {
            file_level_fallback = Some(entry);
        }
    }

    // No precise match. If the query was line/symbol-specific AND the path has
    // precise attribution somewhere, reject — answering the wrong event is worse
    // than admitting the line/symbol is unattributed.
    if path_has_precise_attribution {
        if let Some(line) = query.line {
            return Err(WhyError::LineUnresolved {
                path: query.path.clone(),
                line,
            });
        }
        if let Some(symbol) = &query.symbol {
            return Err(WhyError::SymbolUnresolved {
                path: query.path.clone(),
                symbol: symbol.clone(),
            });
        }
    }

    // File-level fallback (path-only query, or a path with no line granularity).
    if let Some(entry) = file_level_fallback {
        return build_answer(entry).map_err(|_| WhyError::BadPayload {
            seq: entry.record.seq,
        });
    }

    Err(WhyError::NotFound {
        path: query.path.clone(),
    })
}

pub fn resolve_precise_line(
    path: &str,
    line: u64,
    blamed_commit: &str,
    entries: &[LogEntry],
) -> Result<PreciseLineResolution, WhyError> {
    let event = entries.iter().rev().find(|entry| {
        let payload: serde_json::Value = match serde_json::from_str(&entry.record.payload) {
            Ok(payload) => payload,
            Err(_) => return false,
        };
        payload.get("target").and_then(|target| target.as_str()) == Some(blamed_commit)
            && payload.get("files").is_some()
    });
    let Some(event) = event else {
        return Ok(PreciseLineResolution::Unattributed {
            commit: blamed_commit.to_string(),
        });
    };
    let payload: serde_json::Value =
        serde_json::from_str(&event.record.payload).map_err(|_| WhyError::BadPayload {
            seq: event.record.seq,
        })?;
    if payload.get("hunk_capture").and_then(|value| value.as_str()) != Some("complete") {
        return Ok(PreciseLineResolution::RangeUnavailable {
            commit: blamed_commit.to_string(),
        });
    }
    let Some(attribution) = payload_attribution(&event.record.payload, path) else {
        return Ok(PreciseLineResolution::RangeMismatch {
            commit: blamed_commit.to_string(),
        });
    };
    if attribution.matches(Some(line), None) {
        return build_answer(event)
            .map(|answer| PreciseLineResolution::Attributed {
                commit: blamed_commit.to_string(),
                answer,
            })
            .map_err(|_| WhyError::BadPayload {
                seq: event.record.seq,
            });
    }
    Ok(PreciseLineResolution::RangeMismatch {
        commit: blamed_commit.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// One event's attribution for a single queried path: the line ranges and
/// symbols it claims. An empty attribution (path named, no ranges/symbols) is a
/// file-level match only.
struct PathAttribution {
    ranges: Vec<(u64, u64)>,
    symbols: Vec<String>,
}

impl PathAttribution {
    /// Whether this attribution is precise (claims specific lines or symbols).
    fn precise(&self) -> bool {
        !self.ranges.is_empty() || !self.symbols.is_empty()
    }

    /// Whether this attribution satisfies the query's line/symbol constraints.
    ///
    /// - A path-only query (no line, no symbol) matches any attribution.
    /// - A line query matches iff some range covers the line.
    /// - A symbol query matches iff the symbol is among the claimed symbols.
    /// - When both are set, BOTH must hold.
    fn matches(&self, line: Option<u64>, symbol: Option<&str>) -> bool {
        if line.is_none() && symbol.is_none() {
            // Path-only query: a non-precise (file-level) attribution matches.
            // A precise attribution also matches at file level.
            return true;
        }
        let line_ok = match line {
            None => true,
            Some(l) => self
                .ranges
                .iter()
                .any(|(start, end)| l >= *start && l <= *end),
        };
        let symbol_ok = match symbol {
            None => true,
            Some(s) => self.symbols.iter().any(|sym| sym == s),
        };
        line_ok && symbol_ok
    }
}

/// Extract this payload's attribution for `path`, or `None` if the payload does
/// not reference the path at all.
///
/// Supported payload shapes:
/// - `{ "path": "<p>", "ranges": [{"start":N,"end":M}], "symbols": ["foo"] }`
/// - `{ "files": [ "<p>", { "path":"<p>", "ranges":[…], "symbols":[…] } ] }`
fn payload_attribution(payload: &str, path: &str) -> Option<PathAttribution> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;

    // Top-level single-path form.
    if v.get("path").and_then(|x| x.as_str()) == Some(path) {
        return Some(PathAttribution {
            ranges: parse_ranges(v.get("ranges")),
            symbols: parse_symbols(v.get("symbols")),
        });
    }

    // `files` array: either bare strings or per-file objects.
    if let Some(files) = v.get("files").and_then(|x| x.as_array()) {
        for f in files {
            if f.as_str() == Some(path) {
                return Some(PathAttribution {
                    ranges: vec![],
                    symbols: vec![],
                });
            }
            if f.get("path").and_then(|x| x.as_str()) == Some(path) {
                return Some(PathAttribution {
                    ranges: parse_ranges(f.get("ranges")),
                    symbols: parse_symbols(f.get("symbols")),
                });
            }
        }
    }

    None
}

fn parse_ranges(v: Option<&serde_json::Value>) -> Vec<(u64, u64)> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|r| {
            let start = r.get("start").and_then(|x| x.as_u64())?;
            let end = r.get("end").and_then(|x| x.as_u64())?;
            Some((start, end))
        })
        .collect()
}

fn parse_symbols(v: Option<&serde_json::Value>) -> Vec<String> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|s| s.as_str().map(String::from))
        .collect()
}

fn build_answer(entry: &LogEntry) -> Result<ProvenanceAnswer, ()> {
    let record = &entry.record;

    // Determine author kind (item ④ / R6).
    let author_kind = if is_derived_kind(&record.kind) {
        AuthorKind::Derived {
            event_kind: record.kind.clone(),
        }
    } else {
        AuthorKind::Intent
    };

    // Extract intent_id and charter from the payload (best-effort).
    let payload_val: serde_json::Value = serde_json::from_str(&record.payload).map_err(|_| ())?;
    let intent_id = payload_val
        .get("intent_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    // Redact the surfaced charter at the view boundary — parity with the
    // ledger projection (`ledger/mod.rs:104`) and the envelope write path.
    // `why` is a read surface; one redaction law across every read surface.
    let charter = hugit_ledger::redact_apply(
        payload_val
            .get("charter")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    );

    // Pull model + cost from the attestation chain (if provided).
    let (model, cost) = entry
        .attestation
        .as_ref()
        .map(|a| (a.model.clone(), a.def.clone()))
        .unwrap_or_default();

    Ok(ProvenanceAnswer {
        intent_id,
        charter,
        author: record.principal_chain.clone(),
        model,
        cost,
        event_seq: record.seq,
        event_kind: record.kind.clone(),
        event_hash: record.this_hash.clone(),
        author_kind,
    })
}

/// True when the event kind represents a regen / derivation (machine-produced)
/// event rather than a human-authored intent.
pub fn is_derived_kind(kind: &str) -> bool {
    kind == REGEN_DERIVED_KIND || kind.starts_with("regen.") || kind.starts_with("derived.")
}

// ---------------------------------------------------------------------------
// Fixture helpers (used by integration tests only)
// ---------------------------------------------------------------------------

/// Build a minimal [`EventRecord`] for integration tests.
///
/// This function is **test infrastructure only** — it is `pub` solely because
/// integration tests in `tests/` must access it through the public crate API.
/// It must never be called from production code.
#[doc(hidden)]
pub fn fixture_event(
    seq: u64,
    kind: &str,
    principal_chain: Vec<String>,
    payload: serde_json::Value,
) -> EventRecord {
    EventRecord {
        seq,
        prev_hash: format!("{:064x}", seq.saturating_sub(1)),
        this_hash: format!("{:064x}", seq),
        kind: kind.to_string(),
        principal_chain,
        payload: payload.to_string(),
        recorded_at: 1_700_000_000_000 + seq * 1000,
    }
}
