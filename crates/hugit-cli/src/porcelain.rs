//! Porcelain shared helpers — **the one error law + the one exit-code law**
//! (WP-WB0, SOTA-fix Wave B; PC0 scaffold before it).
//!
//! # The LLM-first contract
//!
//! hugit's primary typist is an orchestrated agent, so the machine shape IS the
//! contract. Every porcelain verb — flow (`campaign`/`intent`/`pr`) AND legacy
//! (`why`/`impact`/`tournament`/`export`) — converges on ONE error envelope and
//! ONE exit-code law. An agent parsing stdout always gets a machine-parseable
//! signal, never a fake success and never a bare string.
//!
//! ## One error law (THE canonical shape)
//!
//! A structured (non-crash) error is a single JSON object on **stdout**:
//!
//! ```json
//! {"error":{"kind":"…","message":"…","fix":"…", …context}}
//! ```
//!
//! - **nested** under a top-level `"error"` key (never a flat `{"kind":…}`),
//! - `kind` — a stable, machine-matchable error class (snake_case),
//! - `message` — the human/agent-readable description,
//! - **`fix`** is THE remediation key (NEVER `suggested_fix` — the P2 audit's
//!   schism: the legacy `intent` envelope still spells it `suggested_fix`; that
//!   module is the next WP's to converge — see the note in [`PorcelainError`]),
//! - any further keys are flat **context** folded into the `error` object
//!   (e.g. `"detail"`, `"path"`, `"seq"`), via [`PorcelainError::with_context`].
//!
//! ## One exit-code law
//!
//! - **`0`** — success.
//! - **`2`** — a structured user/domain error (the envelope above on stdout).
//!   This is [`PORCELAIN_ERROR_EXIT`].
//! - **`1`** — RESERVED for an internal fault (a bug, not the caller's input).
//!   An internal fault is ALSO emitted as JSON, with `kind:"internal"`, so even
//!   a crash-class fault stays machine-parseable. This is [`INTERNAL_FAULT_EXIT`].
//!
//! Module WPs converge on this law by constructing [`PorcelainError`] (or the
//! [`internal`] helper) and rendering with [`PorcelainError::to_json`] /
//! [`PorcelainError::exit_code`]. The PC0 stub shape ([`not_implemented`]) is a
//! `PorcelainError` of `kind:"not_implemented"` carrying the owning `wp`.

use std::process::ExitCode;

use serde_json::{Value, json};

/// The process exit code for a structured (non-crash) user/domain error — the
/// `{"error":{…}}` envelope on stdout. The one error law's exit code.
pub const PORCELAIN_ERROR_EXIT: u8 = 2;

/// The process exit code RESERVED for an internal fault (a bug, not bad input).
/// Also emitted as JSON (`kind:"internal"`) so even a fault stays parseable.
pub const INTERNAL_FAULT_EXIT: u8 = 1;

/// THE canonical structured porcelain error — rendered as `{"error":{…}}` JSON
/// on stdout, exit [`PORCELAIN_ERROR_EXIT`] (or [`INTERNAL_FAULT_EXIT`] for the
/// `internal` kind).
///
/// One shape for every verb. `fix` is THE remediation key (never
/// `suggested_fix`). Extra structured context is folded flat into the `error`
/// object via [`with_context`](PorcelainError::with_context).
///
/// # Convergence note (the P2 audit)
///
/// The flow porcelain has two pre-existing error types that this law supersedes:
/// `campaign::CampaignError` (already `fix`-keyed, nested — compatible) and
/// `intent`'s `error::PorcelainError` (spells the remediation `suggested_fix` —
/// the divergence the audit flagged). Converging the `intent`/`pr`/`campaign`
/// CALL SITES onto this type is the next WP's; WB0 establishes the shape here +
/// converges the legacy verbs (`why`/`impact`/`tournament`/`export`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorcelainError {
    /// Stable, machine-matchable error class (snake_case).
    kind: &'static str,
    /// Human/agent-readable description of what went wrong.
    message: String,
    /// THE remediation the caller (an agent) can act on. Never `suggested_fix`.
    fix: String,
    /// Extra structured context, folded flat into the `error` object on render.
    context: Vec<(&'static str, Value)>,
    /// Whether this is an internal fault (exit `1`) rather than a user/domain
    /// error (exit `2`). Set only via [`PorcelainError::internal`].
    internal: bool,
}

impl PorcelainError {
    /// A `kind` + `message` + `fix` error (the common case), exit
    /// [`PORCELAIN_ERROR_EXIT`].
    pub fn new(kind: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        PorcelainError {
            kind,
            message: message.into(),
            fix: fix.into(),
            context: Vec::new(),
            internal: false,
        }
    }

    /// An **internal fault** (a bug, not the caller's input): `kind:"internal"`,
    /// exit [`INTERNAL_FAULT_EXIT`]. Still emitted as JSON so a fault stays
    /// machine-parseable.
    pub fn internal(message: impl Into<String>) -> Self {
        PorcelainError {
            kind: "internal",
            message: message.into(),
            fix: "this is an internal hugit bug; report it with the command + inputs".to_string(),
            context: Vec::new(),
            internal: true,
        }
    }

    /// Fold a flat structured context key into the `error` object (e.g.
    /// `("path", json!("src/a.rs"))`). Repeatable; insertion order preserved.
    pub fn with_context(mut self, key: &'static str, value: Value) -> Self {
        self.context.push((key, value));
        self
    }

    /// Render THE canonical `{"error":{"kind","message","fix", …context}}`
    /// envelope (the stable wire shape).
    pub fn to_json(&self) -> String {
        let mut error = json!({
            "kind": self.kind,
            "message": self.message,
            "fix": self.fix,
        });
        if let Some(map) = error.as_object_mut() {
            for (k, v) in &self.context {
                map.insert((*k).to_string(), v.clone());
            }
        }
        json!({ "error": error }).to_string()
    }

    /// The process exit code under the one exit-code law: `1` for an internal
    /// fault, else `2` for a structured user/domain error.
    pub fn exit_code(&self) -> ExitCode {
        if self.internal {
            ExitCode::from(INTERNAL_FAULT_EXIT)
        } else {
            ExitCode::from(PORCELAIN_ERROR_EXIT)
        }
    }

    /// The error's stable `kind` (for call-site convergence + tests).
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// An I/O fault reading/writing a `--log`/`--store` path (user/domain).
    pub fn io(action: &str, path: &std::path::Path, e: &std::io::Error) -> Self {
        PorcelainError::new(
            "io",
            format!("{action} {}: {e}", path.display()),
            "check the --log path exists and is readable/writable",
        )
    }

    /// A malformed/truncated `--log` file (not valid canonical JSON). The one
    /// input-error law's `parse_log` kind — never silently an empty world.
    ///
    /// This helper is the **flow porcelain's** loader fault (campaign / intent /
    /// pr / checks / queue — the verbs that share the canonical
    /// `[EventRecord, …]` log). The legacy verbs `why`/`export` read their OWN
    /// distinct on-disk shapes (a why-entry wrapper array / an `{events:[…]}`
    /// object) and build their own `parse_log` error with a verb-specific shape
    /// hint in `main.rs` — they do NOT call this helper, so its fix text is
    /// accurate for every verb that DOES (no verb claims a shared format it does
    /// not honor — see `--log` consistency, WF item 4).
    pub fn parse_log(path: &std::path::Path, e: &serde_json::Error) -> Self {
        PorcelainError::new(
            "parse_log",
            format!("--log file {} is not valid JSON: {e}", path.display()),
            "the --log file must be a canonical JSON [EventRecord, …] array \
             (the engine's EventLog shape, shared by every FLOW porcelain verb — \
             campaign/intent/pr/checks/queue; why/export read their own shapes); \
             a truncated/corrupt file is rejected, never read as an empty world",
        )
        .with_context("path", json!(path.display().to_string()))
    }

    /// A `--log` FILE that does not exist (the path is absent on disk). Explicit
    /// — NEVER silently an empty world (the P5 audit finding). Exit `2`.
    ///
    /// Shared by the flow porcelain AND the legacy `why`/`export` reads (a
    /// missing file is the same canonical `log_not_found`/exit-2 everywhere);
    /// the canonical-shape hint below is the flow shape — `why`/`export` add
    /// their own verb-specific shape hint on the `parse_log` path in `main.rs`.
    pub fn log_not_found(path: &std::path::Path) -> Self {
        PorcelainError::new(
            "log_not_found",
            format!("--log file does not exist: {}", path.display()),
            "create the log first (e.g. `hugit intent new --log <path>` \
             bootstraps it) or point --log at an existing log in the shape this \
             verb reads (the flow porcelain shares the canonical \
             [EventRecord, …] array; why/export read their own shapes)",
        )
        .with_context("path", json!(path.display().to_string()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scrub-on-append — THE structural redaction seam (WP-WG-SCRUB)
// ─────────────────────────────────────────────────────────────────────────────
//
// THE forbidden path is a porcelain verb that appends a USER-supplied string to
// the hash-chained, append-only `--log` WITHOUT routing it through the redaction
// engine first. The chain is forever: a `ghp_…`/JWT/conn-string appended verbatim
// is unredactable (re-hashing would break the chain), so the secret leaks for the
// life of the log.
//
// Per-FIELD redaction (Wave E) was fragile: each new verb had to remember to
// scrub each new field, and the wedge wave's new verbs (`check`, `verdict`,
// `pr abandon/queued/landed`, …) forgot — two fresh adversaries reproduced a live
// `ghp_…` leak. The fix is STRUCTURAL: every porcelain append builds a
// `serde_json::Value` payload and hands it to [`scrub_payload`], which recursively
// replaces EVERY user string value with the engine's wholesale
// [`REDACTED`](hugit_ledger::redact::REDACTED) sentinel when any detector fires —
// BEFORE the bytes reach the chain. A verb physically cannot forget a field: the
// whole tree is scrubbed by construction.
//
// **Keys are structural, not user content** — the exemption is by KEY, mirroring
// the envelope's own content-address survival rule (see `hugit_ledger::redact`):
// a content-address/digest VALUE must survive verbatim (it is load-bearing,
// never a secret), so the fields that carry one are exempt — [`is_digest_key`]:
// `memo_key`, `tree_hash`, `commit`, any `*_digest`, and a `hash` field
// (`files_read[].hash`). EVERYTHING else scrubs. A bare digest in a non-exempt
// free-text field is (correctly) subject to the engine's entropy scan and may
// redact — that is the engine's WF-1 "err toward redaction in free text" law, not
// a regression.

/// Recursively scrub EVERY user-supplied string VALUE in a porcelain payload
/// through the redaction engine ([`crate::redaction::scrub`] →
/// [`hugit_ledger::redact::apply`]), IN PLACE, BEFORE the payload is serialized
/// and appended to the hash-chained log.
///
/// Object KEYS are structural and never scrubbed. A value under a digest KEY
/// ([`is_digest_key`] — `memo_key`/`tree_hash`/`commit`/`*_digest`/`hash`) is
/// EXEMPT (a content address is load-bearing and must survive verbatim); every
/// other string value scrubs. Arrays and nested objects recurse, re-evaluating
/// the exemption per key as they descend.
///
/// This is THE seam: route every porcelain append through [`scrub_payload`]
/// (directly or via [`scrub_to_canonical`]). A raw append of an un-scrubbed user
/// payload is the forbidden path the WG-SCRUB tests guard against.
pub fn scrub_payload(value: &mut Value) {
    scrub_value(value, false);
}

/// Inner recursion. `exempt` is set when the value is reached via a digest KEY,
/// so its string content (and any nested strings under it — a digest is a leaf in
/// practice, but the flag propagates honestly) survives verbatim.
fn scrub_value(value: &mut Value, exempt: bool) {
    match value {
        Value::String(s) => {
            if !exempt {
                *s = crate::redaction::scrub(s);
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub_value(item, exempt);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                // Re-evaluate exemption at EACH key: a digest key under a
                // non-exempt object exempts its own subtree, and vice-versa.
                scrub_value(child, is_digest_key(key));
            }
        }
        // Numbers / bools / null carry no user free-text — nothing to scrub.
        _ => {}
    }
}

/// True iff `key` names a content-address / digest field whose VALUE must survive
/// the scrub verbatim. This mirrors the envelope's own digest-survival rule
/// (`hugit_ledger::redact`): a digest is load-bearing, never a secret.
///
/// The rule: exact `memo_key` / `tree_hash` / `commit`, any `*_digest` suffix
/// (`def_digest`, `toolchain_digest`, `prompt_digest`, …), or a bare `hash`
/// field (`files_read[].hash`). Everything else scrubs.
pub fn is_digest_key(key: &str) -> bool {
    matches!(key, "memo_key" | "tree_hash" | "commit" | "hash") || key.ends_with("_digest")
}

/// Scrub a payload [`Value`] ([`scrub_payload`]) and return it as a **canonical**
/// JSON string (sorted keys, no insignificant whitespace — the byte shape the
/// hash chain covers), ready to hand to `append`/`append_authorized`.
///
/// This is the one-call convenience for the porcelain append sites: build the
/// `Value`, call [`scrub_to_canonical`], append the result. The canonicalisation
/// re-uses [`hugit_refstore::canonical_json`] so the appended bytes match what the
/// rest of the flow porcelain emits; if (impossibly) re-canonicalisation fails the
/// compact serialization is returned unchanged.
pub fn scrub_to_canonical(mut value: Value) -> String {
    scrub_payload(&mut value);
    let compact = value.to_string();
    hugit_refstore::canonical_json(&compact).unwrap_or(compact)
}

/// Emit the canonical NOT-IMPLEMENTED error as JSON on **stdout** and return the
/// structured-error exit code.
///
/// Shape (stable contract for callers): `{"error":{"kind":"not_implemented",
/// "wp":"…","message":…,"fix":…}}`. Honest stub — never a fake success. `wp`
/// names the work package that will replace the stub with the real projection.
pub fn not_implemented(wp: &str) -> ExitCode {
    println!("{}", not_implemented_json(wp));
    ExitCode::from(PORCELAIN_ERROR_EXIT)
}

/// The canonical NOT-IMPLEMENTED [`PorcelainError`] for `wp` — a `PorcelainError`
/// of `kind:"not_implemented"` carrying the owning WP token as flat context.
fn not_implemented_error(wp: &str) -> PorcelainError {
    PorcelainError::new(
        "not_implemented",
        "this verb is a scaffold stub; its projection is not implemented yet",
        "track the owning work package; the stub never returns a fake success",
    )
    .with_context("wp", json!(wp))
}

/// The canonical NOT-IMPLEMENTED JSON line for `wp` (the stable wire shape).
///
/// Split out from [`not_implemented`] so the exact contract can be asserted in
/// tests without capturing stdout. `wp` is a fixed ASCII WP token, never
/// untrusted input.
pub fn not_implemented_json(wp: &str) -> String {
    not_implemented_error(wp).to_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_json_is_the_one_canonical_shape() {
        let e = PorcelainError::new("k", "m", "f");
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        // nested under "error", with fix (NOT suggested_fix) as THE key.
        assert_eq!(v["error"]["kind"], "k");
        assert_eq!(v["error"]["message"], "m");
        assert_eq!(v["error"]["fix"], "f");
        assert!(v["error"].get("suggested_fix").is_none());
        assert!(v.get("kind").is_none(), "must be nested, never flat");
    }

    #[test]
    fn context_folds_flat_into_the_error_object() {
        let e = PorcelainError::new("k", "m", "f")
            .with_context("path", json!("src/a.rs"))
            .with_context("seq", json!(7));
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["path"], "src/a.rs");
        assert_eq!(v["error"]["seq"], 7);
    }

    #[test]
    fn user_error_is_exit_two() {
        let e = PorcelainError::new("k", "m", "f");
        assert_eq!(e.exit_code(), ExitCode::from(PORCELAIN_ERROR_EXIT));
        assert_eq!(PORCELAIN_ERROR_EXIT, 2);
    }

    #[test]
    fn internal_fault_is_kind_internal_and_exit_one() {
        let e = PorcelainError::internal("the rollup accumulator overflowed");
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "internal");
        assert!(v["error"]["fix"].is_string());
        assert_eq!(e.exit_code(), ExitCode::from(INTERNAL_FAULT_EXIT));
        assert_eq!(INTERNAL_FAULT_EXIT, 1);
    }

    #[test]
    fn log_not_found_is_explicit_not_an_empty_world() {
        let e = PorcelainError::log_not_found(std::path::Path::new("/no/such.json"));
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "log_not_found");
        assert_eq!(v["error"]["path"], "/no/such.json");
    }

    #[test]
    fn parse_log_carries_the_path_context() {
        let bad: serde_json::Error = serde_json::from_str::<Value>("{trunc").unwrap_err();
        let e = PorcelainError::parse_log(std::path::Path::new("/x/log.json"), &bad);
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "parse_log");
        assert_eq!(v["error"]["path"], "/x/log.json");
    }

    #[test]
    fn not_implemented_json_is_the_canonical_envelope() {
        let v: Value = serde_json::from_str(&not_implemented_json("WB2")).unwrap();
        assert_eq!(v["error"]["kind"], "not_implemented");
        assert_eq!(v["error"]["wp"], "WB2");
        assert!(v["error"]["fix"].is_string());
    }
}
